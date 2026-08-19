# codex-reasoning-double-count

Finding and fix for Codex reasoning tokens being counted and billed twice, plus the disproof of the mechanism proposed in issue #1148.

## summary

Codex reports `reasoning_output_tokens` as a subset of `output_tokens`, the same containment relationship `cached_input_tokens` has with `input_tokens`. The Codex parser carved cached out of input but passed reasoning through unchanged, while both `TokenBreakdown::total()` and `compute_cost` treat `reasoning` as additive to `output`. Every Codex reasoning token was therefore counted once in the token total and billed once at the output rate, on top of the output tokens that already contained it.

## evidence

Codex `token_count` events carry both a cumulative `total_token_usage` and a per-turn `last_token_usage`. Sampled event from a real session:

```json
{"total_token_usage":{"input_tokens":12610,"cached_input_tokens":7040,"output_tokens":108,"reasoning_output_tokens":90,"total_tokens":12718}}
```

`12610 + 108 = 12718`, so the vendor's own total adds input and output only. The 90 reasoning tokens live inside the 108 output tokens.

Measured across every `token_count` event in `~/.codex/sessions` and `~/.codex/archived_sessions` on this machine (1028 session files, 115,428 events with `last_token_usage`, 99,085 of them with non-zero reasoning):

| check | result |
|---|---|
| `reasoning_output_tokens <= output_tokens` | 115,428 of 115,428 (100.00%) |
| `total_tokens == input_tokens + output_tokens` | 114,460 of 115,428 (99.16%) |

The cost side was confirmed independently, before reading the pricing code, by recomputing each model's cost from its published rates and inspecting the residual. Residual divided by that model's output rate lands on the model's reasoning token count exactly:

| model | reported cost | reasoning tokens | residual / output rate |
|---|---|---|---|
| gpt-5.5 | $2,689.57 | 3,983,099 | 3,983,099 |
| gpt-5.6-sol | $1,862.30 | 2,594,200 | 2,594,200 |
| gpt-5.6-terra | $592.29 | 3,105,122 | 3,105,122 |
| codex-auto-review | $27.80 | 204,546 | 204,546 |
| gpt-5.6-luna | $7.99 | 275,235 | 337,206 |

Four of the five models with a resolvable single rate reconcile exactly. `gpt-5.6-luna` does not, and the gap is in the absolute reconciliation rather than in the mechanism: the before-and-after measurement below moves luna's cost by exactly its reasoning count times its output rate, so its residual mismatch comes from something else in the absolute-cost path (tiered rates over a long-context threshold are the most likely candidate) and not from the reasoning bucket.

The mechanism is `crates/tokscale-core/src/pricing/lookup.rs:1788`:

```rust
let output_clamped = output.max(0).saturating_add(reasoning.max(0)) as f64;
```

## the contract this violates

`TokenBreakdown::total()` (`crates/tokscale-core/src/lib.rs:270`) sums all five buckets, and `compute_cost` adds reasoning to output before applying the output rate. A parser may therefore place in `reasoning` only tokens that are not already inside `output`. Two clients already encode this rule in their test names: `sessions::senpi::tests::test_parse_senpi_does_not_double_count_reasoning_into_output` and `sessions::grok::tests::parses_unified_log_token_breakdown_without_double_counting_reasoning`. `sessions/goose.rs:202` derives reasoning as the excess beyond `input + output`, which is additive by construction and also correct. Codex was the outlier.

## fix

`crates/tokscale-core/src/sessions/codex.rs`, in `CodexTotals::into_tokens`, mirroring the cached-out-of-input clamp that already sat two lines above:

```rust
let clamped_reasoning = self.reasoning.min(self.output).max(0);
TokenBreakdown {
    input: (self.input - clamped_cached).max(0),
    output: (self.output - clamped_reasoning).max(0),
    cache_read: clamped_cached,
    cache_write: 0,
    reasoning: clamped_reasoning,
}
```

After the fix `TokenBreakdown::total()` for a Codex row equals the vendor's own `total_tokens`, and the output rate is applied to exactly `output_tokens`.

The `min` also hardens against a malformed row claiming more reasoning than output, which would otherwise drive `output` negative.

## tests

Three new tests in `crates/tokscale-core/src/sessions/codex.rs`, each red before the fix and green after:

- `test_into_tokens_splits_reasoning_out_of_output` asserts the split and that `total()` matches the vendor total.
- `test_into_tokens_clamps_reasoning_to_output` covers the malformed over-report.
- `test_reasoning_is_not_billed_twice_at_the_output_rate` parses a fixture line and asserts `compute_cost` charges the output rate once.

Seventeen existing Codex tests asserted the old inflated `output` values. Each expectation was reduced by exactly that message's asserted reasoning count, and the duration fixture's `timed_tokens` dropped from 170 to 160 against a fixture reasoning total of 10.

## cache invalidation

Both parse caches hold pre-fix breakdowns, so the fix does not reach an existing install without an explicit invalidation.

- `crates/tokscale-core/src/message_cache.rs` keys entries on a per-client `parser_version`, which exists for exactly this case. Codex moves from 6 to 7. Without the bump, a cached entry whose source file has not changed keeps its doubled split forever, and only newly appended bytes get the corrected one.
- `crates/tokscale-cli/src/tui/cache.rs` snapshots already-aggregated `HourlyUsage`, independently of the core cache. `CACHE_SCHEMA_VERSION` moves from 10 to 11, which marks a v10 snapshot `Stale` rather than `Fresh`, so the TUI refreshes it in the background instead of serving inflated numbers for the full five-minute staleness window.

## impact

Measured on the sample machine, cold cache, pre-fix binary against post-fix binary, `tokscale models --client codex --json`:

| field | before | after | delta |
|---|---|---|---|
| input | 328,284,709 | 328,284,709 | 0 |
| cache read | 8,759,587,968 | 8,759,587,968 | 0 |
| output | 32,956,587 | 20,463,048 | -12,493,539 |
| messages | 76,214 | 76,214 | 0 |
| cost | $5,423.19 | $5,160.16 | -$263.02 |

The output delta equals the recorded reasoning count exactly, input and cache read are untouched, the message count is unchanged, and each model's cost delta equals that model's reasoning count times its output rate to the cent. On this machine the overcharge was 4.85 percent of the Codex estimate.

Historical usage already sent to the leaderboard through `submit` carries the inflated numbers server-side. Correcting stored submissions is out of scope here and is the same class of problem as issue #960.

## relation to issue #1148

Issue #1148 reports Codex usage and cost inflated to billions of tokens per hour and attributes it to summing streaming deltas or re-parsing sub-agent events. That proposed mechanism does not hold up:

- The parser uses `last_token_usage` (the per-turn delta) as its increment source and uses `total_token_usage` only for dedup and monotonicity checks (`crates/tokscale-core/src/sessions/codex.rs:530`).
- Sessions present in both `sessions/` and `archived_sessions/` are deduplicated by a key derived from in-file `session_meta` content rather than the file path.
- Forked and resumed sessions that replay a parent's history are suppressed by the inherited-snapshot skip.
- Run against 1028 real session files, tokscale reported roughly two thirds of a naive sum of every `last_token_usage`, which is under-counting relative to the naive sum, not the inflation the issue describes.

The reasoning double-count documented here is a real inflation of the same two quantities the issue names, at a few percent rather than three orders of magnitude. It is not a full explanation of the reported screenshot. The remaining gap is most likely legitimate volume from heavy parallel Codex usage, but that is unverified and no reproduction of a 1000x inflation was found.
