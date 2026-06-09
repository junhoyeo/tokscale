//! Roo Code task parser
//!
//! Parses task-based logs from VS Code globalStorage directories:
//! - tasks/<taskId>/ui_messages.json
//! - tasks/<taskId>/api_conversation_history.json

use super::utils::{extract_i64, parse_timestamp_str, read_file_or_none};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct UiMessageEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    say: Option<String>,
    text: Option<String>,
    ts: Option<Value>,
}

pub fn parse_roocode_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_roo_kilo_file(path, "roocode")
}

pub(crate) fn parse_roo_kilo_file(path: &Path, source: &str) -> Vec<UnifiedMessage> {
    let Some(data) = read_file_or_none(path) else {
        return Vec::new();
    };

    let mut bytes = data;
    let entries: Vec<UiMessageEntry> = match simd_json::from_slice(&mut bytes) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let session_id = extract_session_id(path);
    let (model_id, agent) = read_task_metadata(path);

    let mut messages = Vec::new();
    for entry in entries {
        if entry.entry_type.as_deref() != Some("say")
            || entry.say.as_deref() != Some("api_req_started")
        {
            continue;
        }

        let text = match entry.text {
            Some(t) => t,
            None => continue,
        };

        let timestamp = match parse_entry_timestamp(entry.ts.as_ref()) {
            Some(ts) => ts,
            None => continue,
        };

        let payload = match parse_api_req_started_payload(&text) {
            Some(p) => p,
            None => continue,
        };

        let provider = provider_from_api_protocol(payload.api_protocol.as_deref());

        messages.push(UnifiedMessage::new_with_agent(
            source,
            model_id.clone(),
            provider,
            session_id.clone(),
            timestamp,
            TokenBreakdown {
                input: payload.tokens_in,
                output: payload.tokens_out,
                cache_read: payload.cache_reads,
                cache_write: payload.cache_writes,
                reasoning: 0,
            },
            payload.cost,
            agent.clone(),
        ));
    }

    messages
}

fn extract_session_id(path: &Path) -> String {
    path.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn read_task_metadata(ui_messages_path: &Path) -> (String, Option<String>) {
    let history_path = sibling_history_path(ui_messages_path);
    let content = match std::fs::read_to_string(&history_path) {
        Ok(c) => c,
        Err(_) => return ("unknown".to_string(), None),
    };

    extract_model_and_agent(&content)
}

fn sibling_history_path(ui_messages_path: &Path) -> PathBuf {
    ui_messages_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("api_conversation_history.json")
}

fn extract_model_and_agent(content: &str) -> (String, Option<String>) {
    const ENV_START: &str = "<environment_details>";
    const ENV_END: &str = "</environment_details>";

    let mut offset = 0usize;
    let mut last_model: Option<String> = None;
    let mut last_slug: Option<String> = None;
    let mut last_name: Option<String> = None;

    while let Some(start_rel) = content[offset..].find(ENV_START) {
        let start_idx = offset + start_rel + ENV_START.len();
        let rest = &content[start_idx..];

        let Some(end_rel) = rest.find(ENV_END) else {
            break;
        };
        let end_idx = start_idx + end_rel;
        let block = &content[start_idx..end_idx];

        if let Some(model) = extract_tag_value(block, "model") {
            last_model = Some(model);
        }
        if let Some(slug) = extract_tag_value(block, "slug") {
            last_slug = Some(slug);
        }
        if let Some(name) = extract_tag_value(block, "name") {
            last_name = Some(name);
        }

        offset = end_idx + ENV_END.len();
    }

    let model = last_model.unwrap_or_else(|| "unknown".to_string());
    let agent = last_slug.or(last_name);
    (model, agent)
}

fn extract_tag_value(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);

    let start_idx = block.find(&open)? + open.len();
    let rest = &block[start_idx..];
    let end_rel = rest.find(&close)?;
    let value = rest[..end_rel].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_entry_timestamp(ts: Option<&Value>) -> Option<i64> {
    let value = ts?;
    let ts_str = if let Some(s) = value.as_str() {
        s.to_string()
    } else if let Some(i) = value.as_i64() {
        i.to_string()
    } else if let Some(u) = value.as_u64() {
        u.to_string()
    } else {
        return None;
    };

    parse_timestamp_str(&ts_str)
}

struct ApiReqStartedPayload {
    cost: f64,
    tokens_in: i64,
    tokens_out: i64,
    cache_reads: i64,
    cache_writes: i64,
    api_protocol: Option<String>,
}

fn parse_api_req_started_payload(text: &str) -> Option<ApiReqStartedPayload> {
    let mut bytes = text.as_bytes().to_vec();
    let value: Value = simd_json::from_slice(&mut bytes).ok()?;

    let cost = extract_f64(value.get("cost")).unwrap_or(0.0).max(0.0);
    let tokens_in = extract_i64(value.get("tokensIn")).unwrap_or(0).max(0);
    let tokens_out = extract_i64(value.get("tokensOut")).unwrap_or(0).max(0);
    let cache_reads = extract_i64(value.get("cacheReads")).unwrap_or(0).max(0);
    let cache_writes = extract_i64(value.get("cacheWrites")).unwrap_or(0).max(0);
    let api_protocol = value
        .get("apiProtocol")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Some(ApiReqStartedPayload {
        cost,
        tokens_in,
        tokens_out,
        cache_reads,
        cache_writes,
        api_protocol,
    })
}

fn extract_f64(value: Option<&Value>) -> Option<f64> {
    value.and_then(|val| {
        val.as_f64()
            .or_else(|| val.as_i64().map(|v| v as f64))
            .or_else(|| val.as_u64().map(|v| v as f64))
            .or_else(|| val.as_str().and_then(|s| s.parse::<f64>().ok()))
    })
}

fn provider_from_api_protocol(api_protocol: Option<&str>) -> String {
    api_protocol
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_task(
        dir: &TempDir,
        task_id: &str,
        ui_messages_content: &str,
        history_content: Option<&str>,
    ) -> PathBuf {
        let task_dir = dir.path().join("tasks").join(task_id);
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("ui_messages.json"), ui_messages_content).unwrap();
        if let Some(history) = history_content {
            fs::write(task_dir.join("api_conversation_history.json"), history).unwrap();
        }
        task_dir.join("ui_messages.json")
    }

    #[test]
    fn test_parse_roocode_valid_api_req_started() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.12,\"tokensIn\":100,\"tokensOut\":50,\"cacheReads\":20,\"cacheWrites\":5,\"apiProtocol\":\"anthropic\"}"
  },
  {
    "type": "say",
    "say": "assistant_message",
    "ts": "2026-02-18T12:00:01Z",
    "text": "{}"
  }
]"#;
        let history = r#"before
<environment_details>
<model>claude-sonnet-4</model>
<slug>architect</slug>
<name>Architect</name>
</environment_details>
after"#;
        let path = setup_task(&dir, "task-abc", ui_messages, Some(history));

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "roocode");
        assert_eq!(messages[0].model_id, "claude-sonnet-4");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].session_id, "task-abc");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.cache_write, 5);
        assert_eq!(messages[0].cost, 0.12);
        assert_eq!(messages[0].agent.as_deref(), Some("architect"));
    }

    #[test]
    fn test_parse_roocode_skips_malformed_payload_entry() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "not-json"
  },
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:02Z",
    "text": "{\"cost\":0.03,\"tokensIn\":10,\"tokensOut\":2,\"cacheReads\":1,\"cacheWrites\":0,\"apiProtocol\":\"openai\"}"
  }
]"#;
        let path = setup_task(&dir, "task-def", ui_messages, None);

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].model_id, "unknown");
        assert_eq!(messages[0].agent, None);
    }

    #[test]
    fn test_parse_roocode_preserves_nested_reseller_api_protocol() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.12,\"tokensIn\":100,\"tokensOut\":50,\"cacheReads\":20,\"cacheWrites\":5,\"apiProtocol\":\"bedrock/anthropic\"}"
  }
]"#;
        let history = r#"before
<environment_details>
<model>claude-sonnet-4</model>
</environment_details>
after"#;
        let path = setup_task(&dir, "task-nested-provider", ui_messages, Some(history));

        let messages = parse_roocode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "bedrock/anthropic");
    }

    #[test]
    fn test_parse_roocode_skips_invalid_timestamp() {
        let dir = TempDir::new().unwrap();
        let ui_messages = r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "not-a-time",
    "text": "{\"cost\":0.12,\"tokensIn\":100,\"tokensOut\":50,\"cacheReads\":20,\"cacheWrites\":5,\"apiProtocol\":\"anthropic\"}"
  }
]"#;
        let path = setup_task(&dir, "task-time", ui_messages, None);

        let messages = parse_roocode_file(&path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_roocode_invalid_file_json_is_ignored() {
        let dir = TempDir::new().unwrap();
        let path = setup_task(&dir, "task-invalid", "{not-json", None);

        let messages = parse_roocode_file(&path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_extract_model_and_agent_prefers_slug_then_name() {
        let content = r#"
<environment_details>
<model>gpt-5</model>
<name>Builder</name>
</environment_details>
<environment_details>
<model>gpt-5.1</model>
<slug>reviewer</slug>
<name>Reviewer</name>
</environment_details>
"#;

        let (model, agent) = extract_model_and_agent(content);
        assert_eq!(model, "gpt-5.1");
        assert_eq!(agent.as_deref(), Some("reviewer"));
    }
}
