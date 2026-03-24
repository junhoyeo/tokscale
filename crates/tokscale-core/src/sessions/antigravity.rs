use super::UnifiedMessage;
use crate::{pricing, provider_identity, TokenBreakdown};
use serde_json::Value;
use std::path::Path;

pub fn parse_antigravity_file(path: &Path) -> Vec<UnifiedMessage> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };

    let mut messages = Vec::new();
    let mut session_model: Option<String> = None;

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
                if let Some(model_id) = value
                    .get("modelId")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                {
                    session_model = Some(model_id.to_string());
                }
            }
            "usage" => {
                if let Some(message) = parse_usage_row(&value, session_model.as_deref()) {
                    messages.push(message);
                }
            }
            _ => {}
        }
    }

    messages
}

fn parse_usage_row(value: &Value, fallback_model: Option<&str>) -> Option<UnifiedMessage> {
    let session_id = value.get("sessionId").and_then(Value::as_str)?.to_string();
    let timestamp = to_safe_i64(value.get("timestamp"));
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
    let model_id = pricing::aliases::resolve_alias(&model_id)
        .unwrap_or(model_id.as_str())
        .to_string();

    let provider_id = value
        .get("providerId")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(|text| text.to_string())
        .unwrap_or_else(|| infer_provider(&model_id).to_string());

    let input = to_safe_i64(value.get("input"));
    let output = to_safe_i64(value.get("output"));
    let cache_read = to_safe_i64(value.get("cacheRead"));
    let cache_write = to_safe_i64(value.get("cacheWrite"));
    let reasoning = to_safe_i64(value.get("reasoning"));
    if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 && reasoning == 0 {
        return None;
    }

    let dedup_key = value
        .get("responseId")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(|text| text.to_string());

    Some(UnifiedMessage::new_with_dedup(
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
            reasoning,
        },
        0.0,
        dedup_key,
    ))
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
    }
}
