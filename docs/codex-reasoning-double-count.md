# codex-reasoning-double-count

Verification record for the Codex reasoning split, and the one invalidation gap the split left behind.

## what was already fixed

`CodexTotals::into_tokens` now carves `reasoning_output_tokens` out of `output_tokens` rather than passing it through additively, and `ClientId::Codex` moved to `parser_version` 7 so an existing message cache cannot keep replaying pre-split rows. This document does not change either of those. It records the independent measurement behind them, because the numbers are useful when judging the remaining gap and any future report of inflated Codex usage.

## the containment measurement

Codex `token_count` events carry both a cumulative `total_token_usage` and a per-turn `last_token_usage`. A real event:

```json
{"total_token_usage":{"input_tokens":12610,"cached_input_tokens":7040,"output_tokens":108,"reasoning_output_tokens":90,"total_tokens":12718}}
```

`12610 + 108 = 12718`, so the vendor's own total adds input and output only. The 90 reasoning tokens live inside the 108 output tokens.

Measured across every `token_count` event in `~/.codex/sessions` and `~/.codex/archived_sessions` on one machine (1,028 session files, 115,428 events carrying `last_token_usage`, 99,085 of them with non-zero reasoning):

| check | result |
|---|---|
| `reasoning_output_tokens <= output_tokens` | 115,428 of 115,428 (100.00%) |
| `total_tokens == input_tokens + output_tokens` | 114,460 of 115,428 (99.16%) |

The cost half was confirmed separately, by recomputing each model's cost from its published rates and dividing the residual by that model's output rate. The result lands on the model's reasoning token count exactly:

| model | reported cost | reasoning tokens | residual / output rate |
|---|---|---|---|
| gpt-5.5 | $2,689.57 | 3,983,099 | 3,983,099 |
| gpt-5.6-sol | $1,862.30 | 2,594,200 | 2,594,200 |
| gpt-5.6-terra | $592.29 | 3,105,122 | 3,105,122 |
| codex-auto-review | $27.80 | 204,546 | 204,546 |

Measured effect of the split, cold cache, pre-split binary against post-split binary, `tokscale models --client codex --json`:

| field | before | after | delta |
|---|---|---|---|
| input | 328,284,709 | 328,284,709 | 0 |
| cache read | 8,759,587,968 | 8,759,587,968 | 0 |
| output | 32,956,587 | 20,463,048 | -12,493,539 |
| messages | 76,214 | 76,214 | 0 |
| cost | $5,423.19 | $5,160.16 | -$263.02 |

The output delta equals the recorded reasoning count exactly, input and cache read are untouched, the message count is unchanged, and each model's cost delta equals that model's reasoning count times its output rate to the cent. The overcharge was 4.85 percent of the Codex estimate on that machine.

## the remaining gap: the TUI snapshot cache

`parser_version` invalidates the core message cache, but `crates/tokscale-cli/src/tui/cache.rs` keeps a second, independent snapshot of already-aggregated `HourlyUsage` and friends. Its freshness is decided by `CACHE_SCHEMA_VERSION` and a five-minute age threshold, not by anything the core parser knows about.

At `CACHE_SCHEMA_VERSION` 10 a snapshot written before the split is still `Fresh`, so a user who upgrades sees the pre-split totals and cost until the age threshold expires, on every tab the snapshot feeds. Moving the constant to 11 makes such a snapshot `Stale` instead, which serves it once and refreshes in the background rather than presenting stale inflated numbers as current.

## the contract this sat on

`TokenBreakdown::total()` sums all five buckets and `compute_cost` charges `output + reasoning` at the output rate, so a parser may place in `reasoning` only tokens that are not already inside `output`. That rule was not written down anywhere: not in `CONTRIBUTING.md`, not in `AGENTS.md`, not on the struct. It existed only in test names such as `test_parse_senpi_does_not_double_count_reasoning_into_output` and in four hand-rolled subtractions that do not reference one another, so a parser author could only learn it by tripping over it.

The doc comment added on `TokenBreakdown` states it, names both overlaps that recur across vendors (cached inside input, reasoning inside output), and points at the parsers that establish each shape, including `sessions::pi`, `sessions::kimi` and `sessions::zed`, which deliberately leave `reasoning` at zero rather than guess.

## note on issue #1148

That issue reports Codex usage and cost inflated to billions of tokens per hour and attributes it to summing streaming deltas or re-parsing sub-agent events. That mechanism does not hold: the parser uses `last_token_usage` as its increment source, deduplicates sessions present in both `sessions/` and `archived_sessions/` by a key derived from in-file `session_meta` content, and suppresses forked or resumed sessions that replay a parent's history. Run against 1,028 real session files it reported roughly two thirds of a naive sum of every `last_token_usage`, which is under-counting relative to that sum rather than the inflation the issue describes.

The reasoning split corrects a real inflation of the same two quantities, at a few percent rather than three orders of magnitude, so it does not fully explain that report.
