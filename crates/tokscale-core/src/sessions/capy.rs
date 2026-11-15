//! Capy (capy.ai) usage, read from the cache `tokscale capy sync` writes.
//!
//! Capy runs its agent loop on Capy's servers, including threads that run
//! tools on the local machine and threads on a connected subscription such as
//! Codex. Nothing that carries token counts is written locally, so the only
//! source is Capy's public `GET /api/v1/usage/series` endpoint. The CLI stores
//! its rows verbatim (`items`) under `capy-cache/usage.json`; this parser turns
//! each `llm` row into one message.
//!
//! Rows are aggregates: one per time bucket, thread, user and model. Capy's
//! `inputTokens` includes cache reads and writes, so they are split out of the
//! input count here. Cost is left to Tokscale's pricing engine so a Codex
//! subscription row is priced the same way a local Codex CLI session is;
//! `billedDollars` is only Capy's own charge and is zero for subscription
//! routes.

use super::UnifiedMessage;
use crate::provider_identity::inferred_provider_from_model_delimited;
use crate::TokenBreakdown;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::Path;

pub const CLIENT_ID: &str = "capy";

/// Day buckets are UTC days. Stamping a day row at its UTC midnight would put
/// it on the previous calendar day for every timezone west of UTC, so day rows
/// are placed at the bucket's midpoint instead. Hour rows keep their start.
const DAY_BUCKET_MIDPOINT_MS: i64 = 12 * 60 * 60 * 1000;
const HOUR_MS: i64 = 60 * 60 * 1000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapyUsageCache {
    #[serde(default)]
    items: Vec<CapyUsageRow>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapyUsageRow {
    start: String,
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    dimensions: CapyDimensions,
    #[serde(default)]
    tokens: CapyTokens,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapyDimensions {
    user_id: Option<String>,
    thread_id: Option<String>,
    item_type: Option<String>,
    item_id: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapyTokens {
    #[serde(default, deserialize_with = "lenient_count")]
    input_tokens: i64,
    #[serde(default, deserialize_with = "lenient_count")]
    output_tokens: i64,
    #[serde(default, deserialize_with = "lenient_count")]
    cache_read_tokens: i64,
    #[serde(default, deserialize_with = "lenient_count")]
    cache_write_tokens: i64,
}

/// Capy's encoder writes non-finite numbers as the strings `"Infinity"`,
/// `"-Infinity"` and `"NaN"`. None of them is a usable token count.
fn lenient_count<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Number(number) => number
            .as_i64()
            .or_else(|| {
                number
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .map(|value| value.round() as i64)
            })
            .unwrap_or(0)
            .max(0),
        _ => 0,
    })
}

pub fn parse_capy_file(path: &Path) -> Vec<UnifiedMessage> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(cache) = serde_json::from_str::<CapyUsageCache>(&content) else {
        return Vec::new();
    };
    cache.items.iter().filter_map(row_to_message).collect()
}

fn row_to_message(row: &CapyUsageRow) -> Option<UnifiedMessage> {
    let dims = &row.dimensions;
    if dims.item_type.as_deref() != Some("llm") {
        return None;
    }
    let raw_model = dims
        .model
        .as_deref()
        .or(dims.item_id.as_deref())
        .map(str::trim)
        .filter(|model| !model.is_empty())?;

    let tokens = &row.tokens;
    let cache_read = tokens.cache_read_tokens;
    let cache_write = tokens.cache_write_tokens;
    let breakdown = TokenBreakdown {
        // Counts are clamped to `0..=i64::MAX` on read, so saturate rather
        // than overflow when a corrupt row reports huge cache buckets.
        input: tokens
            .input_tokens
            .saturating_sub(cache_read)
            .saturating_sub(cache_write)
            .max(0),
        output: tokens.output_tokens,
        cache_read,
        cache_write,
        cache_write_1h: 0,
        reasoning: 0,
    };
    if breakdown.total() <= 0 {
        return None;
    }

    let start = parse_rfc3339_millis(&row.start)?;
    let end = row.end.as_deref().and_then(parse_rfc3339_millis);
    let timestamp = match end {
        Some(end) if end - start > HOUR_MS => start + DAY_BUCKET_MIDPOINT_MS.min((end - start) / 2),
        _ => start,
    };

    let (provider, model) = split_model(raw_model);
    let thread = dims
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|thread| !thread.is_empty())
        .unwrap_or("no-thread");
    let session_id = format!("capy-{thread}");
    let user = dims.user_id.as_deref().unwrap_or("-");

    let mut message = UnifiedMessage::new(
        CLIENT_ID, model, provider, session_id, timestamp, breakdown, 0.0,
    );
    let granularity = match end.map(|end| end - start) {
        Some(duration) if duration <= HOUR_MS => "hour",
        Some(_) => "day",
        None => "unknown",
    };
    // Use normalized interval boundaries rather than the source strings so
    // equivalent RFC 3339 offsets deduplicate, while a midnight hour and its
    // residual day bucket remain distinct. JSON tuple encoding keeps arbitrary
    // user/thread/model strings from creating delimiter collisions.
    let interval_key = serde_json::to_string(&(start, end, granularity, user, thread, raw_model))
        .expect("Capy dedup tuple is serializable");
    message.dedup_key = Some(format!("capy:{interval_key}"));
    Some(message)
}

/// Capy names models `provider/model` (`openai/gpt-5.5`,
/// `anthropic/claude-opus-5-5`). Tokscale prices by the bare model id and keeps
/// the provider separately.
fn split_model(raw: &str) -> (String, String) {
    if let Some((provider, model)) = raw.split_once('/') {
        let provider = provider.trim().to_ascii_lowercase();
        let model = model.trim();
        if !provider.is_empty() && !model.is_empty() {
            return (provider, model.to_string());
        }
    }
    let provider = inferred_provider_from_model_delimited(raw)
        .unwrap_or(CLIENT_ID)
        .to_string();
    (provider, raw.to_string())
}

fn parse_rfc3339_millis(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc).timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn parse_json(json: &str) -> Vec<UnifiedMessage> {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(json.as_bytes()).unwrap();
        parse_capy_file(file.path())
    }

    #[test]
    fn llm_rows_become_messages_with_cache_split_out_of_input() {
        let messages = parse_json(
            r#"{"version":1,"items":[{
                "start":"2026-11-14T09:00:00.000Z","end":"2026-11-14T10:00:00.000Z",
                "dimensions":{"userId":"user_1","threadId":"thr_1","itemType":"llm",
                    "itemId":"openai/gpt-5.5-codex","model":"openai/gpt-5.5-codex","label":"GPT-5.5 Codex"},
                "llmDollars":0,"billedDollars":0.01,
                "tokens":{"inputTokens":1000,"outputTokens":50,"cacheReadTokens":700,"cacheWriteTokens":100}
            }]}"#,
        );
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client.as_ref(), "capy");
        assert_eq!(message.model_id.as_ref(), "gpt-5.5-codex");
        assert_eq!(message.provider_id.as_ref(), "openai");
        assert_eq!(message.session_id.as_ref(), "capy-thr_1");
        assert_eq!(message.tokens.input, 200);
        assert_eq!(message.tokens.output, 50);
        assert_eq!(message.tokens.cache_read, 700);
        assert_eq!(message.tokens.cache_write, 100);
        assert_eq!(
            message.timestamp,
            parse_rfc3339_millis("2026-11-14T09:00:00Z").unwrap()
        );
        assert_eq!(message.cost, 0.0);
    }

    #[test]
    fn day_rows_are_stamped_at_the_bucket_midpoint() {
        let messages = parse_json(
            r#"{"items":[{
                "start":"2026-11-14T00:00:00.000Z","end":"2026-11-15T00:00:00.000Z",
                "dimensions":{"threadId":null,"itemType":"llm","itemId":"anthropic/claude-opus-5-5","model":"anthropic/claude-opus-5-5"},
                "tokens":{"inputTokens":10,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0}
            }]}"#,
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].timestamp,
            parse_rfc3339_millis("2026-11-14T12:00:00Z").unwrap()
        );
        assert_eq!(messages[0].session_id.as_ref(), "capy-no-thread");
    }

    #[test]
    fn non_llm_and_tokenless_rows_are_skipped() {
        let messages = parse_json(
            r#"{"items":[
                {"start":"2026-11-14T00:00:00Z","dimensions":{"itemType":"machine","itemId":"small","model":null},
                 "tokens":{"inputTokens":0,"outputTokens":0,"cacheReadTokens":0,"cacheWriteTokens":0}},
                {"start":"2026-11-14T00:00:00Z","dimensions":{"itemType":"llm","itemId":"openai/gpt-5.5","model":"openai/gpt-5.5"},
                 "tokens":{"inputTokens":0,"outputTokens":0,"cacheReadTokens":0,"cacheWriteTokens":0}},
                {"start":"2026-11-14T00:00:00Z","dimensions":{"itemType":"llm","itemId":"openai/gpt-5.5","model":"openai/gpt-5.5"},
                 "tokens":{"inputTokens":"NaN","outputTokens":3,"cacheReadTokens":0,"cacheWriteTokens":0}}
            ]}"#,
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 0);
        assert_eq!(messages[0].tokens.output, 3);
    }

    #[test]
    fn dedup_key_distinguishes_bucket_user_thread_and_model() {
        let messages = parse_json(
            r#"{"items":[
                {"start":"2026-11-14T09:00:00Z","dimensions":{"userId":"u1","threadId":"t1","itemType":"llm","model":"openai/gpt-5.5"},
                 "tokens":{"inputTokens":5,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0}},
                {"start":"2026-11-14T09:00:00Z","dimensions":{"userId":"u1","threadId":"t1","itemType":"llm","model":"openai/gpt-5.5-codex"},
                 "tokens":{"inputTokens":5,"outputTokens":1,"cacheReadTokens":0,"cacheWriteTokens":0}}
            ]}"#,
        );
        assert_eq!(messages.len(), 2);
        assert_ne!(messages[0].dedup_key, messages[1].dedup_key);
    }

    #[test]
    fn unprefixed_model_falls_back_to_inferred_provider() {
        let (provider, model) = split_model("gpt-5.5");
        assert_eq!(provider, "openai");
        assert_eq!(model, "gpt-5.5");
    }

    #[test]
    fn huge_cache_counts_saturate_instead_of_overflowing() {
        let messages = parse_json(
            r#"{"items":[{"start":"2026-11-14T09:00:00Z",
                "dimensions":{"threadId":"t","itemType":"llm","model":"openai/gpt-5.5"},
                "tokens":{"inputTokens":0,"outputTokens":1,
                    "cacheReadTokens":1e300,"cacheWriteTokens":1e300}}]}"#,
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 0);
    }

    #[test]
    fn unprefixed_provider_inference_respects_family_boundaries() {
        // A family name inside a longer word is not that family: plain
        // substring inference reads these as OpenAI and xAI.
        for raw in ["megpt-1", "groker-1"] {
            let (provider, model) = split_model(raw);
            assert_eq!(provider, CLIENT_ID, "{raw}");
            assert_eq!(model, raw);
        }
        assert_eq!(split_model("gpt-5.5").0, "openai");
    }

    #[test]
    fn malformed_cache_yields_nothing() {
        assert!(parse_json("not json").is_empty());
        assert!(parse_json(r#"{"items":"nope"}"#).is_empty());
    }
}
