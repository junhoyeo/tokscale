# reasoning-containment-audit

Follow-up queue from the Codex reasoning double-count fix. Except where marked resolved, these are unverified leads rather than confirmed defects. Each needs the same treatment Codex got before it is called a bug: establish containment from the vendor's own source or real data, then fix with a test that is red first. Lead 1 has been worked and came back negative, which is the outcome this standard is designed to produce when the suspicion is wrong.

## the rule being audited

`TokenBreakdown::total()` (`crates/tokscale-core/src/lib.rs:270`) sums all five buckets and `compute_cost` (`crates/tokscale-core/src/pricing/lookup.rs:1788`) adds `reasoning` to `output` before applying the output rate. A parser may therefore place in `reasoning` only tokens that are not already inside `output`. See [codex-reasoning-double-count](codex-reasoning-double-count.md) for the case that was confirmed and fixed.

## decisions

2026-08-19: the Codex fix and its delivery were approved, and PR [#1149](https://github.com/junhoyeo/tokscale/pull/1149) was opened against `junhoyeo/tokscale` from the `klappe-pm` fork. The seven leads below were deferred out of that diff by the same decision, with lead 1 (`jcode.rs:126`) picked up next. This file stays untracked: it holds unverified hypotheses about other people's parsers and should not travel upstream looking like findings.

2026-08-19: closed out and approved after lead 1 resolved negative. Two commits are on the PR: `574fb341` (the Codex fix) and `a7dd8c5a` (the `TokenBreakdown` contract doc comment). Leads 2 through 7 remain unworked. Whoever picks them up should read the vendor's source first, the way lead 1 was settled, rather than reasoning from the field names in tokscale's own deserializers.

## leads, strongest first

| # | Site | Source field | Why it is suspected |
|---|---|---|---|
| ~~1~~ | `crates/tokscale-core/src/sessions/jcode.rs:126` | `reasoning_output_tokens` | **Resolved, not a defect.** jcode never writes the field. See the section below. |
| 2 | `crates/tokscale-core/src/sessions/copilot_desktop.rs:615` | `reasoningTokens` | The file handles cache-inclusive `inputTokens` carefully and documents it, then stores `outputTokens` and `reasoningTokens` as independent raw buckets with no symmetric handling. Its fixtures are OpenAI codex-family models, the same vendor family as the confirmed case. |
| 3 | `crates/tokscale-core/src/sessions/tencent_buddy.rs:111` | `completion_thinking_tokens`, `reasoningTokens` | `input_exclusive()` already computes an exclusive total to detect cache folded into input, but applies no equivalent check for reasoning folded into output. |
| 4 | `crates/tokscale-core/src/sessions/copilot.rs:437` | `gen_ai.usage.reasoning.output_tokens` | The attribute name reads as a nested sub-field of output tokens, and `normalize_input_tokens` fixes the input side only. Counter-evidence: some fixtures carry reasoning greater than output, which a strict subset cannot produce, so the fixtures may be synthetic. |
| 5 | `crates/tokscale-core/src/sessions/opencode.rs:253`, `kilo.rs:145`, `micode.rs:254` | `reasoning` | One shared schema across an OpenCode fork family. No subtraction in any of the three. No in-repo evidence either way. |
| 6 | `crates/tokscale-core/src/sessions/droid.rs:222` | `thinking_tokens` | Droid tracks Claude thinking models, and this repo's own `claudecode.rs` always sets `reasoning: 0` because Anthropic's output tokens are inclusive of thinking. |
| 7 | `crates/tokscale-core/src/sessions/junie.rs:249` | `reasoningTokens`, `reasoningOutputTokens`, `thinkingTokens` | Aliases mix OpenAI-style and Claude-style naming, both known-inclusive patterns elsewhere in this repo. Weakest evidence of the group. |

`crates/tokscale-core/src/sessions/hermes.rs:196` reads `reasoning_tokens` from Hermes's own pre-aggregated SQLite rather than a raw vendor payload. Containment cannot be determined from this repo either way.

## lead 1, jcode: not a defect

Resolved 2026-08-19 against the vendor's source at `1jehuang/jcode` commit `37272c9150c5759575acf16c892bb3458439dc7a`. **jcode never writes `reasoning_output_tokens`**, so the suspected double count is unreachable.

`StoredTokenUsage` is the struct jcode serializes into `~/.jcode/sessions/session_*.json`, at `crates/jcode-session-types/src/lib.rs:251`:

```rust
pub struct StoredTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
}
```

Four fields, no reasoning. The only two sites that construct it (`crates/jcode-app-core/src/agent/turn_streaming_mpsc.rs:1087` and `crates/jcode-app-core/src/agent/turn_loops.rs:791`) set those same four and nothing else. The stream event `TokenUsage` at `crates/jcode-message-types/src/lib.rs:721` carries the same four. Searching the entire jcode tree for `reasoning_output_tokens` or `reasoning_tokens` returns nothing at all.

So tokscale's `JcodeTokenUsage.reasoning_output_tokens` (`jcode.rs:77`) always deserializes to `None`, `tokens_from_usage` always yields `reasoning: 0`, and the three fixtures asserting `reasoning == 25` describe a payload jcode does not produce. Reasoning tokens a jcode user spends are folded into `output_tokens` by the provider and billed once at the output rate, which is correct.

No fix, and specifically no speculative clamp: if jcode ever adds the field, whether it is contained or additive has to be established then, and guessing now would be the same error in the other direction. The forward risk is that jcode adopts the OpenAI name for a contained value and tokscale silently starts double counting, which is what the `TokenBreakdown` doc comment added alongside the Codex fix exists to catch at review time.

### what the in-repo evidence had shown

Recorded because it explains why field names were never enough to settle this, and because the same gaps apply to the remaining leads.

What the repo does establish:

- `JcodeTokenUsage` (`jcode.rs:71`) has five fields and **no total**. Codex was provable because its events carry `total_tokens`, so `input + output == total` demonstrated that reasoning is not added on top. jcode offers no equivalent anchor in its schema or in any fixture.
- Every jcode fixture in the repo is synthetic. Three carry a non-zero `reasoning_output_tokens`, all with the value 25 and all near-identical, most likely copied from the original feature PR. A fixture where reasoning is smaller than output is equally consistent with both hypotheses and proves nothing.
- No test distinguishes the two readings. `tokens_from_usage` (`jcode.rs:108`) passes `output_tokens` and `reasoning_output_tokens` through as independent buckets with no interaction, and every assertion simply confirms that pass-through.
- `crates/tokscale-core/tests/jcode.rs` asserts cost as `(250.0 + 25.0) * output_rate` under the comment "Reasoning tokens are intentionally billed through the output-token price". That records the current billing behavior as deliberate, but it does not record any finding about whether the vendor's output already contained those tokens, which is the question that decides whether the behavior is right.
- No jcode commit message, inline comment, or `parser_version` note across seven commits mentions reasoning containment. jcode's v5, v6 and v7 bumps cover timestamps, cache-read overlap, and lenient parsing.
- `README.md` lists jcode's five fields without stating their relationship.

One structural correlation, offered at the time as a lead rather than as evidence: all three non-zero-reasoning fixtures also carry `cache_creation_input_tokens`, which `uses_split_cache_accounting` (`jcode.rs:99`) reads as the Anthropic-style route. Anthropic's API exposes no separate reasoning counter, so a `reasoning_output_tokens` value on an Anthropic route would have to be something jcode computed itself. The vendor source shows the simpler explanation: jcode computes no such value on any route, and the fixtures were written to exercise parser mechanics rather than to mirror real output.

The lesson for the remaining leads is that field names in a consumer's deserializer prove nothing about a producer. A field the vendor never emits looks identical, in this repo, to a field the vendor emits as a contained subset. Only the producer's source or a real session file separates the two.

## already correct, for calibration

`gemini` and `qwen` prove additivity by matching an inclusive total against the vendor's own total field. `grok`, `zcode`, `reasonix` and `senpi` all subtract explicitly. `goose` derives reasoning as the excess beyond input plus output, additive by construction. `antigravity` and `antigravity_cli` rest on an empirical cross-check recorded in their module docs. `pi`, `kimi` and `zed` deliberately set `reasoning: 0` with a comment rather than pass through a possibly-contained value.

## unrelated findings

`tui::cache::tests::load_cache_falls_back_to_legacy_dot_cache_path` and its two siblings were updated in the Codex fix because the TUI cache schema version moved. Separately, `trae::sync::tests::test_losing_contender_leaves_no_orphan_after_release` failed once under full-suite parallel load with `another trae sync is in progress; aborting`, then passed three times in a row when run alone. It is a lock-contention flake in the Trae sync test, unrelated to the Codex change and outside its blast radius.

`parse_codex_headless_line` (`crates/tokscale-core/src/sessions/codex.rs:1126`) builds a `TokenBreakdown` directly rather than through `CodexTotals::into_tokens`, and `extract_headless_usage` never reads `reasoning_output_tokens`, so the headless path hardcodes `reasoning: 0`. The double-count never existed there, but reasoning tokens from headless-format Codex output are dropped rather than counted. Pre-existing and outside the fix's blast radius.

`cargo clippy --workspace --all-targets --all-features -- -D warnings` fails on two `needless_borrows_for_generic_args` errors at `crates/tokscale-core/src/pricing/lookup.rs:587` and `:600`, both `.map(&annotate_direct)`. Verified pre-existing: stashing every change in this cycle and re-running clippy reproduces both. A newer clippy than the one the code was written against flags them, and `lib.rs:1` denies `clippy::all`. Left alone here because the file is outside this change's blast radius, but it makes the lint gate red on the default branch.
