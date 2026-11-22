//! Empryo usage ledger parser.
//!
//! Empryo is an AI coding harness for the desktop and the terminal that maps a
//! repository, edits code by symbol, and checks its own work. Every model
//! request it sends — main turns, subagent runs, and its internal lanes such as
//! the session namer — is metered into a local append-only ledger at
//! `~/.empryo/usage/usage-<YYYY-MM>.jsonl`, one JSON object per line. That
//! ledger is the only surface that attributes tokens to a model, so it is what
//! this parser reads; `~/.empryo/sessions/<id>/session.jsonl` holds the
//! transcripts but records per-turn usage without a model identity.
//!
//! # Token Semantics
//!
//! - **Cached tokens:** the ledger's `input` is Empryo's cache-inclusive prompt
//!   total — it adds the reported uncached prompt (`noCacheTokens`) to
//!   `cacheRead`/`cacheWrite`, and when a provider reports only an inclusive
//!   prompt count it leaves `input` at or above that cache total. The cached
//!   subset is therefore split out into `cache_read`/`cache_write`, so Tokscale
//!   counts and prices the prompt once.
//! - **Reasoning:** recorded as `0`. The ledger keeps a single `output` count
//!   and no reasoning split, and providers that stream reasoning bill it inside
//!   the completion count, so inventing a share here would double count it.
//! - **Cost:** the ledger's `cost` is the amount Empryo recorded for that
//!   request — its own price-table answer, or the provider's `costUSD` when one
//!   was reported. A positive value is treated like OpenCode's per-message cost:
//!   authoritative, and immune to Tokscale's repricing pass. A zero value means
//!   Empryo itself had no price for the model, so it is left `Unknown` for
//!   Tokscale to estimate.
//! - **Failed or empty calls:** Empryo's writer never appends a row without
//!   tokens, but the ledger is a plain file a user can edit, so rows that state
//!   no usage are skipped here as well.

use super::utils::lossy_lines;
use super::{BillingIdentityOrigin, UnifiedMessage};
use crate::TokenBreakdown;
use serde::Deserialize;
use std::io::BufReader;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct EmpryoUsageRecord {
    ts: i64,
    #[serde(rename = "modelId")]
    model_id: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    input: i64,
    #[serde(default)]
    output: i64,
    #[serde(default, rename = "cacheRead")]
    cache_read: i64,
    #[serde(default, rename = "cacheWrite")]
    cache_write: i64,
    #[serde(default)]
    cost: f64,
    #[serde(default, rename = "durationMs")]
    duration_ms: Option<i64>,
    #[serde(default)]
    lane: Option<String>,
}

/// The model reference Empryo actually asked for.
///
/// A ledger `modelId` is provider-qualified (`<provider>/<model>`), and the
/// provider prefix is also written to its own `provider` field. Tokscale prices
/// and groups by the model, so the leading segment is dropped when it is the
/// record's provider and something follows it. A prefix that disagrees with the
/// provider field is part of the model identity (for example a gateway model
/// keyed `od-go/...` under provider `local-20128`) and is kept verbatim.
fn model_id_without_provider_prefix<'a>(model_id: &'a str, provider: &str) -> &'a str {
    let model_id = model_id.trim();
    let provider = provider.trim();
    match model_id.split_once('/') {
        Some((prefix, rest)) if !provider.is_empty() && prefix.eq_ignore_ascii_case(provider) => {
            let rest = rest.trim_start_matches('/');
            if rest.is_empty() {
                model_id
            } else {
                rest
            }
        }
        _ => model_id,
    }
}

/// Parse one Empryo monthly usage ledger file.
///
/// Malformed rows, rows without a usable timestamp or model, and rows stating no
/// usage are skipped defensively rather than failing the file.
pub fn parse_empryo_file(path: &Path) -> Vec<UnifiedMessage> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };

    // A monthly ledger is one session for Tokscale's Sessions tab: the rows
    // carry no session or thread identity of their own.
    let session_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("empryo-usage")
        .to_string();

    lossy_lines(BufReader::new(file))
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }

            let record: EmpryoUsageRecord = serde_json::from_str(line).ok()?;

            if record.ts <= 0 {
                return None;
            }
            let model_id = record.model_id.trim();
            if model_id.is_empty() {
                return None;
            }

            let output = record.output.max(0);
            let cache_read = record.cache_read.max(0);
            let cache_write = record.cache_write.max(0);
            let reported_input = record.input.max(0);
            let cache_total = cache_read.saturating_add(cache_write);
            // `input` is the cache-inclusive prompt total; sature instead of
            // widening the prompt when a hand-edited row claims more cache than
            // input, since the cache count is then not a valid subset.
            let input = if cache_total > 0 && cache_total <= reported_input {
                reported_input - cache_total
            } else {
                reported_input
            };
            if input == 0 && output == 0 && cache_total == 0 {
                return None;
            }

            let tokens = TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                cache_write_1h: 0,
                reasoning: 0,
            };

            let provider = record.provider.trim();
            let model = model_id_without_provider_prefix(model_id, provider);
            let cost = if record.cost.is_finite() && record.cost > 0.0 {
                record.cost
            } else {
                0.0
            };

            // Every row is one request, keyed by the fields that identify it so
            // a row repeated in a later file (a replayed or hand-copied ledger)
            // cannot be counted twice.
            let dedup_key = format!(
                "empryo:{}:{}:{}:{}:{}:{}:{}:{}",
                record.ts,
                model_id,
                provider,
                record.source.trim(),
                record.lane.as_deref().unwrap_or("").trim(),
                input,
                output,
                cache_read,
            );

            let mut message = UnifiedMessage::new_with_dedup(
                "empryo",
                model,
                provider,
                &session_id,
                record.ts,
                tokens,
                cost,
                Some(dedup_key),
            )
            .with_billing(
                (!provider.is_empty()).then_some(provider),
                BillingIdentityOrigin::ExplicitRecord,
            );

            message.duration_ms = record.duration_ms.filter(|duration| *duration > 0);
            if cost > 0.0 {
                message.mark_provider_reported_cost();
            }

            Some(message)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::EstimateStatus;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    /// Two rows in Empryo's real ledger format: a cache-heavy main turn billed
    /// at the gateway's own `costUSD`, and a namer-lane call Empryo could not
    /// price.
    const SAMPLE_FIXTURE: &str = r#"{"ts":1794963601989,"modelId":"local-20128/od-go/deepseek-v4.1-flash-eco","provider":"local-20128","source":"other","input":136,"output":127,"cacheRead":0,"cacheWrite":0,"cost":0.00022748,"durationMs":2966,"costNoCache":0.00022748,"lane":"namer"}
{"ts":1794963608083,"modelId":"local-20128/od-go/deepseek-v4.1-flash-eco","provider":"local-20128","source":"main","input":24785,"output":214,"cacheRead":21120,"cacheWrite":0,"cost":0.00219076,"durationMs":7695,"costNoCache":0.01118788,"lane":"main","effort":"high"}
"#;

    fn write_temp_ledger(name: &str, contents: &str) -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file.flush().unwrap();
        (dir, path)
    }

    #[test]
    fn parses_ledger_rows_and_splits_cache_inclusive_input() {
        let (_dir, path) = write_temp_ledger("usage-2026-11.jsonl", SAMPLE_FIXTURE);
        let messages = parse_empryo_file(&path);
        assert_eq!(messages.len(), 2);

        let first = &messages[0];
        assert_eq!(first.client, "empryo");
        // The provider-qualified ledger id drops its `local-20128/` prefix.
        assert_eq!(first.model_id, "od-go/deepseek-v4.1-flash-eco");
        assert_eq!(first.provider_id, "local-20128");
        assert_eq!(first.session_id, "usage-2026-11");
        assert_eq!(first.timestamp, 1794963601989);
        assert_eq!(first.tokens.input, 136);
        assert_eq!(first.tokens.output, 127);
        assert_eq!(first.tokens.cache_read, 0);
        assert_eq!(first.tokens.cache_write, 0);
        assert_eq!(first.tokens.reasoning, 0);
        assert_eq!(first.duration_ms, Some(2966));
        assert_eq!(first.message_count, 1);
        assert_eq!(first.cost, 0.00022748);
        assert_eq!(
            first.cost_source,
            crate::sessions::CostSource::ProviderReported
        );
        assert_eq!(first.estimate_status, EstimateStatus::Authoritative);

        let second = &messages[1];
        // 24785 cache-inclusive prompt tokens with 21120 of them cached leaves
        // 3665 uncached prompt tokens for pricing.
        assert_eq!(second.tokens.input, 3665);
        assert_eq!(second.tokens.output, 214);
        assert_eq!(second.tokens.cache_read, 21120);
        assert_eq!(second.tokens.total(), 3665 + 214 + 21120);
        assert_eq!(second.duration_ms, Some(7695));
        assert_eq!(second.estimate_status, EstimateStatus::Authoritative);
        assert_ne!(first.dedup_key, second.dedup_key);
    }

    #[test]
    fn leaves_unpriced_rows_for_tokscale_pricing() {
        let row = r#"{"ts":1794963601989,"modelId":"anthropic/claude-sonnet-4-5","provider":"anthropic","source":"main","input":1000,"output":200,"cacheRead":800,"cacheWrite":0,"cost":0}
"#;
        let (_dir, path) = write_temp_ledger("usage-2026-11.jsonl", row);
        let messages = parse_empryo_file(&path);
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.model_id, "claude-sonnet-4-5");
        assert_eq!(message.tokens.input, 200);
        assert_eq!(message.tokens.cache_read, 800);
        assert_eq!(message.cost, 0.0);
        assert_eq!(message.cost_source, crate::sessions::CostSource::Unknown);
        assert_ne!(message.estimate_status, EstimateStatus::Authoritative);
    }

    #[test]
    fn keeps_a_model_prefix_that_disagrees_with_the_provider() {
        // The leading segment is part of the model key here, not a provider
        // stamp: dropping it would alias the gateway's route onto the bare slug.
        let row = r#"{"ts":1794963601989,"modelId":"od-go/deepseek-v4.1-flash-eco","provider":"local-20128","source":"main","input":100,"output":10,"cost":0}
"#;
        let (_dir, path) = write_temp_ledger("usage-2026-11.jsonl", row);
        let messages = parse_empryo_file(&path);
        assert_eq!(messages[0].model_id, "od-go/deepseek-v4.1-flash-eco");
    }

    #[test]
    fn skips_malformed_and_usage_free_rows() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"not json at all\n").unwrap();
        file.write_all(b"{\"ts\":\"nope\",\"modelId\":\"m\"}\n")
            .unwrap();
        file.write_all(b"\n").unwrap();
        file.write_all(b"{\"ts\":1794963601989,\"modelId\":\"m\"}\n")
            .unwrap();
        file.write_all(b"{\"ts\":0,\"modelId\":\"m\",\"input\":10}\n")
            .unwrap();
        file.write_all(b"{\"ts\":1794963601989,\"modelId\":\"  \",\"input\":10}\n")
            .unwrap();
        file.write_all(SAMPLE_FIXTURE.as_bytes()).unwrap();
        file.write_all(b"{\"ts\":1794963609999,\"modelId\":\"trunc")
            .unwrap();
        file.flush().unwrap();

        let messages = parse_empryo_file(file.path());
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn missing_file_yields_no_messages() {
        let dir = TempDir::new().unwrap();
        let messages = parse_empryo_file(&dir.path().join("usage-2026-11.jsonl"));
        assert!(messages.is_empty());
    }

    #[test]
    fn provider_prefix_stripping_is_case_insensitive_and_keeps_bare_ids() {
        assert_eq!(
            model_id_without_provider_prefix("OpenRouter/anthropic/claude-x", "openrouter"),
            "anthropic/claude-x"
        );
        assert_eq!(
            model_id_without_provider_prefix("claude-sonnet-4-5", "anthropic"),
            "claude-sonnet-4-5"
        );
        assert_eq!(
            model_id_without_provider_prefix("anthropic/", "anthropic"),
            "anthropic/"
        );
        assert_eq!(
            model_id_without_provider_prefix("cached/model", "unknown"),
            "cached/model"
        );
    }
}
