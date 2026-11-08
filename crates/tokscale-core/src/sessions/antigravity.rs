use super::UnifiedMessage;
use crate::{pricing, provider_identity, TokenBreakdown};
use serde_json::Value;
use std::path::Path;

fn file_modified_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .map(|time| chrono::DateTime::<chrono::Utc>::from(time).timestamp_millis())
        .filter(|&ms| ms > 0)
}

pub fn parse_antigravity_file(path: &Path) -> Vec<UnifiedMessage> {
    let (mut auth, fallback, _, _) = parse_antigravity_file_inner(path, false);
    auth.extend(fallback);
    auth
}

pub fn parse_antigravity_file_with_v1(
    path: &Path,
) -> (Vec<UnifiedMessage>, Vec<UnifiedMessage>, bool) {
    let (mut auth, fallback, v1, readable) = parse_antigravity_file_inner(path, true);
    auth.extend(fallback);
    (auth, v1, readable)
}

pub fn parse_antigravity_file_partitioned(
    path: &Path,
) -> (Vec<UnifiedMessage>, Vec<UnifiedMessage>) {
    let (auth, fallback, _, _) = parse_antigravity_file_inner(path, false);
    (auth, fallback)
}

pub fn parse_antigravity_file_with_v1_partitioned(
    path: &Path,
) -> (
    Vec<UnifiedMessage>,
    Vec<UnifiedMessage>,
    Vec<UnifiedMessage>,
    bool,
) {
    parse_antigravity_file_inner(path, true)
}

fn parse_antigravity_file_inner(
    path: &Path,
    include_v1: bool,
) -> (
    Vec<UnifiedMessage>,
    Vec<UnifiedMessage>,
    Vec<UnifiedMessage>,
    bool,
) {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return (Vec::new(), Vec::new(), Vec::new(), false),
    };

    let fallback_file_ts = file_modified_ms(path);
    let mut authoritative_messages = Vec::new();
    let mut fallback_messages = Vec::new();
    let mut v1_messages = Vec::new();
    let mut session_model: Option<String> = None;
    let mut session_timestamp: Option<i64> = None;
    let mut is_v2_session = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        let row_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        match row_type {
            "session_meta" => {
                session_model = value
                    .get("modelId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(|id| id.to_string());
                let meta_ts = to_safe_i64(value.get("timestamp"));
                session_timestamp = if meta_ts > 0 { Some(meta_ts) } else { None };
                is_v2_session = value.get("version").and_then(Value::as_u64) == Some(2);
            }
            "usage" => {
                let fallback_ts = session_timestamp.filter(|&ts| ts > 0).or(fallback_file_ts);
                let row_is_v2 = is_v2_session
                    || value.get("v").and_then(Value::as_u64) == Some(2)
                    || value
                        .get("responseOutputTokens")
                        .filter(|v| !v.is_null())
                        .is_some();
                let raw_timestamp = to_safe_i64(value.get("timestamp"));
                let is_fallback_ts = raw_timestamp <= 0;
                if let Some(message) = parse_usage_row(
                    &value,
                    session_model.as_deref(),
                    fallback_ts,
                    false,
                    row_is_v2,
                ) {
                    if is_fallback_ts {
                        fallback_messages.push(message);
                    } else {
                        authoritative_messages.push(message);
                    }
                }
                if include_v1 {
                    if let Some(v1_message) =
                        parse_usage_row(&value, session_model.as_deref(), None, true, row_is_v2)
                    {
                        v1_messages.push(v1_message);
                    }
                }
            }
            _ => {}
        }
    }

    (authoritative_messages, fallback_messages, v1_messages, true)
}

fn parse_usage_row(
    value: &Value,
    fallback_model: Option<&str>,
    fallback_timestamp: Option<i64>,
    v1_compat: bool,
    is_v2_format: bool,
) -> Option<UnifiedMessage> {
    let session_id = value.get("sessionId").and_then(Value::as_str)?.to_string();
    let raw_timestamp = to_safe_i64(value.get("timestamp"));
    let timestamp = if v1_compat || raw_timestamp > 0 {
        raw_timestamp
    } else {
        fallback_timestamp.unwrap_or(0)
    };
    if timestamp <= 0 {
        return None;
    }

    let model_id = value
        .get("modelId")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(|text| text.to_string())
        .or_else(|| fallback_model.map(|text| text.to_string()))
        .unwrap_or_else(|| "unknown".to_string());
    let raw_model = model_id.clone();
    let model_id = pricing::aliases::resolve_alias(&model_id)
        .unwrap_or(model_id.as_str())
        .to_string();

    let provider_id = value
        .get("providerId")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(|text| text.to_string())
        .unwrap_or_else(|| infer_provider(&model_id).to_string());
    let provider_id = provider_identity::canonical_provider(&provider_id).unwrap_or(provider_id);

    let input = to_safe_i64(value.get("input"));
    let raw_output = value
        .get("responseOutputTokens")
        .filter(|v| !v.is_null())
        .map(|v| to_safe_i64(Some(v)))
        .unwrap_or_else(|| to_safe_i64(value.get("output")));
    let cache_read = to_safe_i64(value.get("cacheRead"));
    let reasoning = to_safe_i64(value.get("reasoning"));
    let (output, cache_write) = if v1_compat {
        // Pre-PR CLI artifacts already stored outputTokens (total output including reasoning)
        // in "output". Only new v2 CLI artifacts store responseOutputTokens (text-only) in
        // "output" and therefore need reasoning added back to emulate v1.
        let v1_output = if is_v2_format {
            raw_output.saturating_add(reasoning)
        } else {
            raw_output
        };
        (v1_output, 0)
    } else {
        let output = if is_v2_format {
            raw_output
        } else {
            raw_output.saturating_sub(reasoning)
        };
        (output, to_safe_i64(value.get("cacheWrite")))
    };
    if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 && reasoning == 0 {
        return None;
    }

    let dedup_key = value
        .get("responseId")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(|text| text.to_string());

    Some(
        UnifiedMessage::new_with_dedup(
            "antigravity",
            model_id,
            provider_id,
            session_id,
            timestamp,
            TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                cache_write_1h: 0,
                reasoning,
            },
            0.0,
            dedup_key,
        )
        .with_billing(
            value.get("providerId").and_then(Value::as_str),
            super::BillingIdentityOrigin::ExplicitRecord,
        )
        .with_pricing_model(&raw_model),
    )
}

fn infer_provider(model: &str) -> &'static str {
    provider_identity::inferred_provider_from_model(model).unwrap_or("antigravity")
}

fn to_safe_i64(value: Option<&Value>) -> i64 {
    value
        .and_then(|inner| {
            inner
                .as_i64()
                .or_else(|| inner.as_u64().and_then(|number| i64::try_from(number).ok()))
                .or_else(|| inner.as_str().and_then(|text| text.parse::<i64>().ok()))
        })
        .unwrap_or(0)
        .max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_usage_row_with_meta_fallback() {
        let input = r#"{"type":"session_meta","sessionId":"abc","modelId":"claude-sonnet-4.6"}
{"type":"usage","sessionId":"abc","timestamp":1711200000000,"input":12,"output":4,"cacheRead":2,"cacheWrite":0,"reasoning":1,"responseId":"resp-1"}
"#;

        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), input).unwrap();

        let messages = parse_antigravity_file(path.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "antigravity");
        assert_eq!(messages[0].model_id, "claude-sonnet-4-6");
        assert_eq!(messages[0].tokens.input, 12);
        assert_eq!(messages[0].tokens.reasoning, 1);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("resp-1"));
    }

    #[test]
    fn parse_usage_row_resolves_placeholder_model_alias() {
        let input = r#"{"type":"usage","sessionId":"abc","modelId":"MODEL_PLACEHOLDER_M26","timestamp":1711200000000,"input":12,"output":4,"cacheRead":2,"cacheWrite":0,"reasoning":1}
"#;

        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), input).unwrap();

        let messages = parse_antigravity_file(path.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-opus-4-6");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(
            messages[0].billing_identity,
            super::super::BillingIdentity::Unknown
        );
        assert_eq!(
            messages[0].pricing_model_id.as_deref(),
            Some("MODEL_PLACEHOLDER_M26")
        );
    }

    #[test]
    fn explicit_billing_channel_keeps_raw_model_separate_from_display_alias() {
        let input = r#"{"type":"usage","sessionId":"abc","modelId":"MODEL_PLACEHOLDER_M26","providerId":" VERTEX ","timestamp":1711200000000,"input":12,"output":4}"#;
        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), input).unwrap();
        let messages = parse_antigravity_file(path.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-opus-4-6");
        assert_eq!(
            messages[0].pricing_model_id.as_deref(),
            Some("MODEL_PLACEHOLDER_M26")
        );
        assert_eq!(
            messages[0].billing_identity,
            super::super::BillingIdentity::Known {
                id: "vertex".into(),
                origin: super::super::BillingIdentityOrigin::ExplicitRecord,
            }
        );
    }

    #[test]
    fn parse_usage_row_resolves_current_placeholder_models() {
        let input = r#"{"type":"usage","sessionId":"abc","modelId":"model_placeholder_m84","timestamp":1711200000000,"input":12,"output":4,"cacheRead":2,"cacheWrite":0,"reasoning":1}
{"type":"usage","sessionId":"abc","modelId":"model_placeholder_m16","timestamp":1711200000001,"input":8,"output":3,"cacheRead":0,"cacheWrite":0,"reasoning":0}
"#;

        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), input).unwrap();

        let messages = parse_antigravity_file(path.path());
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].model_id, "gemini-3-flash-preview");
        assert_eq!(messages[0].provider_id, "google");
        assert_eq!(messages[1].model_id, "gemini-3.1-pro");
        assert_eq!(messages[1].provider_id, "google");
    }

    #[test]
    fn parse_usage_row_falls_back_to_session_meta_or_file_mtime_when_timestamp_is_null() {
        let input = r#"{"type":"session_meta","sessionId":"abc","modelId":"claude-sonnet-4.6","timestamp":1711200000000}
{"type":"usage","sessionId":"abc","timestamp":null,"input":12,"output":4,"cacheRead":2,"cacheWrite":3,"reasoning":1,"responseId":"resp-null-ts"}
"#;

        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), input).unwrap();

        let (v2_messages, v1_messages, ok) = parse_antigravity_file_with_v1(path.path());
        assert!(ok);
        assert_eq!(v2_messages.len(), 1);
        assert_eq!(v2_messages[0].timestamp, 1_711_200_000_000);
        // Legacy cache format (no version: 2): output is 4 - 1 = 3
        assert_eq!(v2_messages[0].tokens.output, 3);
        assert_eq!(v2_messages[0].tokens.cache_write, 3);
        assert_eq!(v2_messages[0].tokens.reasoning, 1);
        assert!(v1_messages.is_empty());
    }

    #[test]
    fn parse_usage_row_falls_back_to_file_mtime_and_handles_v1_legacy_vs_v2() {
        let input = r#"{"type":"usage","sessionId":"abc","modelId":"claude-sonnet-4.6","timestamp":null,"input":12,"output":4,"cacheRead":2,"cacheWrite":0,"reasoning":1,"responseId":"resp-legacy"}
{"type":"usage","sessionId":"abc","modelId":"claude-sonnet-4.6","timestamp":1711200000002,"input":10,"output":4,"cacheRead":0,"cacheWrite":2,"reasoning":1,"responseId":"resp-v2","v":2}
"#;

        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), input).unwrap();

        let (v2_messages, v1_messages, ok) = parse_antigravity_file_with_v1(path.path());
        assert!(ok);
        assert_eq!(v2_messages.len(), 2);
        // Authoritative messages come first:
        // Message 0 is resp-v2 (timestamp 1711200000002, v: 2, so output is raw_output = 4).
        assert_eq!(v2_messages[0].timestamp, 1_711_200_000_002);
        assert_eq!(v2_messages[0].tokens.output, 4);
        assert_eq!(v2_messages[0].tokens.reasoning, 1);
        assert_eq!(v2_messages[0].tokens.cache_write, 2);
        assert_eq!(v2_messages[0].dedup_key.as_deref(), Some("resp-v2"));

        // Message 1 is resp-legacy (timestamp null, fell back to file mtime > 0, legacy format so output is 4 - 1 = 3).
        assert!(v2_messages[1].timestamp > 0);
        assert_eq!(v2_messages[1].tokens.output, 3);
        assert_eq!(v2_messages[1].tokens.reasoning, 1);
        assert_eq!(v2_messages[1].tokens.cache_write, 0);
        assert_eq!(v2_messages[1].dedup_key.as_deref(), Some("resp-legacy"));

        // In v1_messages, only rows with explicit timestamp > 0 are accepted (first row had null ts, so rejected in v1).
        // The second row has v=2, so v1 emulation adds reasoning back (4 + 1 = 5) and zeroes cache_write.
        assert_eq!(v1_messages.len(), 1);
        assert_eq!(v1_messages[0].tokens.output, 5);
        assert_eq!(v1_messages[0].tokens.cache_write, 0);

        // Verify partitioning:
        let (auth, fallback) = parse_antigravity_file_partitioned(path.path());
        assert_eq!(auth.len(), 1);
        assert_eq!(auth[0].dedup_key.as_deref(), Some("resp-v2"));
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].dedup_key.as_deref(), Some("resp-legacy"));
    }

    #[test]
    fn parse_legacy_cache_file_does_not_double_count_reasoning() {
        // Pre-v2 cache format: output stores total tokens including reasoning.
        let input = r#"{"type":"session_meta","sessionId":"legacy-sess","modelId":"claude-sonnet-4.6"}
{"type":"usage","sessionId":"legacy-sess","timestamp":1711200000000,"input":100,"output":50,"cacheRead":20,"reasoning":15,"responseId":"resp-1"}
"#;

        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), input).unwrap();

        let (v2_messages, v1_messages, ok) = parse_antigravity_file_with_v1(path.path());
        assert!(ok);
        assert_eq!(v2_messages.len(), 1);
        // v2 output must be text only: 50 - 15 = 35
        assert_eq!(v2_messages[0].tokens.output, 35);
        assert_eq!(v2_messages[0].tokens.reasoning, 15);
        // Total tokens: 100 + 35 + 15 = 150 (not 100 + 50 + 15 = 165)
        let total = v2_messages[0].tokens.input
            + v2_messages[0].tokens.output
            + v2_messages[0].tokens.reasoning;
        assert_eq!(total, 150);

        // In v1 emulation: output is raw_output = 50. Total is 100 + 50 = 150.
        assert_eq!(v1_messages.len(), 1);
        assert_eq!(v1_messages[0].tokens.output, 50);
        assert_eq!(v1_messages[0].tokens.reasoning, 15);
    }

    #[test]
    fn parse_usage_row_handles_explicit_null_response_output_tokens_in_legacy_mode() {
        // When responseOutputTokens is explicit null in JSON, it should not mark
        // the row as v2; legacy output must subtract reasoning.
        let input = r#"{"type":"usage","sessionId":"sess-null-out","modelId":"claude-sonnet-4.6","timestamp":1711200000000,"input":100,"output":50,"cacheRead":20,"reasoning":15,"responseOutputTokens":null,"responseId":"resp-null"}
"#;

        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), input).unwrap();

        let (v2_messages, v1_messages, ok) = parse_antigravity_file_with_v1(path.path());
        assert!(ok);
        assert_eq!(v2_messages.len(), 1);
        // Must be text only: 50 - 15 = 35 (not 50)
        assert_eq!(v2_messages[0].tokens.output, 35);
        assert_eq!(v2_messages[0].tokens.reasoning, 15);
        assert_eq!(v1_messages.len(), 1);
        assert_eq!(v1_messages[0].tokens.output, 50);
    }

    #[test]
    fn parse_antigravity_file_resets_session_meta_state_across_sessions() {
        // Session 1 is v2 format with explicit timestamp
        // Session 2 is legacy format without version or timestamp
        let input = r#"{"type":"session_meta","sessionId":"sess-1","modelId":"claude-sonnet-4.6","timestamp":1711200000000,"version":2}
{"type":"usage","sessionId":"sess-1","timestamp":1711200000001,"input":50,"output":20,"reasoning":5,"responseId":"resp-1","v":2}
{"type":"session_meta","sessionId":"sess-2","modelId":"claude-opus-4.6"}
{"type":"usage","sessionId":"sess-2","timestamp":null,"input":80,"output":40,"reasoning":10,"responseId":"resp-2"}
"#;

        let path = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(path.path(), input).unwrap();

        let (v2_messages, _, ok) = parse_antigravity_file_with_v1(path.path());
        assert!(ok);
        assert_eq!(v2_messages.len(), 2);

        // sess-1 is authoritative, v2 format: output is raw 20
        let s1 = v2_messages
            .iter()
            .find(|m| m.session_id == "sess-1")
            .unwrap();
        assert_eq!(s1.tokens.output, 20);
        assert_eq!(s1.tokens.reasoning, 5);
        assert_eq!(s1.timestamp, 1_711_200_000_001);

        // sess-2 is legacy format: output is 40 - 10 = 30 (not 40!)
        // and its timestamp must NOT borrow sess-1's 1711200000000
        let s2 = v2_messages
            .iter()
            .find(|m| m.session_id == "sess-2")
            .unwrap();
        assert_eq!(s2.tokens.output, 30);
        assert_eq!(s2.tokens.reasoning, 10);
        assert_ne!(s2.timestamp, 1_711_200_000_000);
        assert!(s2.timestamp > 0); // fell back to file mtime
    }
}
