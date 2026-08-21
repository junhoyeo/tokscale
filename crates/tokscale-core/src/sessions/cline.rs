//! Cline task parser
//!
//! Cline is the upstream project that Roo Code and Kilo forked from, so it
//! shares the same VS Code globalStorage task-log format and reuses the same
//! parser helper.

use super::roocode::parse_roo_kilo_file;
use super::utils::{extract_i64, file_modified_timestamp_ms, parse_timestamp_value};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn parse_cline_file(path: &Path) -> Vec<UnifiedMessage> {
    if is_cline_cli_messages_path(path) {
        return parse_cline_cli_file(path);
    }

    parse_roo_kilo_file(path, "cline")
}

#[derive(Debug, Deserialize)]
struct ClineCliMessagesFile {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    agent: Option<String>,
    messages: Option<Vec<ClineCliMessage>>,
}

#[derive(Debug, Deserialize)]
struct ClineCliMessage {
    id: Option<String>,
    role: Option<String>,
    ts: Option<Value>,
    content: Option<Vec<Value>>,
    #[serde(rename = "modelInfo")]
    model_info: Option<ClineCliModelInfo>,
    metrics: Option<ClineCliMetrics>,
}

fn is_human_user_prompt(content: Option<&[Value]>) -> bool {
    let Some(content) = content else {
        return false;
    };

    let mut has_text_block = false;
    for block in content {
        match block.get("type").and_then(Value::as_str) {
            Some("tool_result") => return false,
            Some("text") => has_text_block = true,
            _ => {}
        }
    }
    has_text_block
}

#[derive(Debug, Deserialize)]
struct ClineCliModelInfo {
    id: Option<String>,
    provider: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClineCliMetrics {
    #[serde(rename = "inputTokens")]
    input_tokens: Option<Value>,
    #[serde(rename = "outputTokens")]
    output_tokens: Option<Value>,
    #[serde(rename = "cacheReadTokens")]
    cache_read_tokens: Option<Value>,
    #[serde(rename = "cacheWriteTokens")]
    cache_write_tokens: Option<Value>,
    cost: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
struct ClineCliManifest {
    session_id: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    cwd: Option<String>,
    workspace_root: Option<String>,
    metadata: Option<Value>,
}

pub(crate) fn is_cline_cli_messages_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".messages.json"))
}

pub(crate) fn cline_cli_manifest_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let session_stem = stem.strip_suffix(".messages").unwrap_or(stem);
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{session_stem}.json"))
}

fn read_cline_cli_manifest(path: &Path) -> ClineCliManifest {
    let manifest_path = cline_cli_manifest_path(path);
    let Ok(mut bytes) = std::fs::read(manifest_path) else {
        return ClineCliManifest::default();
    };

    simd_json::from_slice(&mut bytes).unwrap_or_default()
}

fn non_empty_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn extract_f64(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_i64().map(|value| value as f64))
            .or_else(|| value.as_u64().map(|value| value as f64))
            .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()))
    })
}

fn extract_non_negative_finite_f64(value: Option<&Value>) -> Option<f64> {
    extract_f64(value).filter(|value| value.is_finite() && *value >= 0.0)
}

/// Parse Cline CLI's persisted assistant messages from
/// `~/.cline/data/sessions/<session>/<session>.messages.json`.
pub fn parse_cline_cli_file(path: &Path) -> Vec<UnifiedMessage> {
    let Some(data) = super::utils::read_file_or_none(path) else {
        return Vec::new();
    };

    let mut bytes = data;
    let Ok(file) = simd_json::from_slice::<ClineCliMessagesFile>(&mut bytes) else {
        return Vec::new();
    };
    let manifest = read_cline_cli_manifest(path);

    let session_id = file
        .session_id
        .or(manifest.session_id)
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(|name| name.trim_end_matches(".messages").to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let workspace_key = manifest
        .workspace_root
        .as_deref()
        .or(manifest.cwd.as_deref())
        .and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
    let session_title = manifest
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("title"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    let mut current_model =
        non_empty_string(manifest.model.as_deref()).unwrap_or_else(|| "unknown".to_string());
    let mut current_provider =
        non_empty_string(manifest.provider.as_deref()).unwrap_or_else(|| "unknown".to_string());
    let mut pending_turn_start = false;
    let mut assistant_index = 0usize;
    let mut messages = Vec::new();

    for entry in file.messages.unwrap_or_default() {
        if entry.role.as_deref() == Some("user") {
            if is_human_user_prompt(entry.content.as_deref()) {
                pending_turn_start = true;
            }
            continue;
        }
        if entry.role.as_deref() != Some("assistant") {
            continue;
        }

        if let Some(model_info) = entry.model_info.as_ref() {
            if let Some(model) = non_empty_string(model_info.id.as_deref()) {
                current_model = model;
            }
            if let Some(provider) = non_empty_string(model_info.provider.as_deref()) {
                current_provider = provider;
            }
        }

        let Some(metrics) = entry.metrics else {
            continue;
        };
        let input_tokens = extract_i64(metrics.input_tokens.as_ref())
            .unwrap_or(0)
            .max(0);
        let output = extract_i64(metrics.output_tokens.as_ref())
            .unwrap_or(0)
            .max(0);
        let cache_read = extract_i64(metrics.cache_read_tokens.as_ref())
            .unwrap_or(0)
            .max(0);
        let cache_write = extract_i64(metrics.cache_write_tokens.as_ref())
            .unwrap_or(0)
            .max(0);
        let input = input_tokens
            .saturating_sub(cache_read)
            .saturating_sub(cache_write);
        let reported_cost = extract_non_negative_finite_f64(metrics.cost.as_ref());
        let cost = reported_cost.unwrap_or(0.0);

        let total_tokens = input
            .saturating_add(output)
            .saturating_add(cache_read)
            .saturating_add(cache_write);
        if total_tokens == 0 && reported_cost.is_none() {
            continue;
        }

        let timestamp = entry
            .ts
            .as_ref()
            .and_then(parse_timestamp_value)
            .unwrap_or(fallback_timestamp);
        let dedup_key = entry
            .id
            .filter(|id| !id.trim().is_empty())
            .map(|id| format!("cline-cli:{session_id}:{id}"))
            .unwrap_or_else(|| format!("cline-cli:{session_id}:{assistant_index}"));
        let mut message = UnifiedMessage::new_with_agent(
            "cline",
            current_model.clone(),
            current_provider.clone(),
            session_id.clone(),
            timestamp,
            TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                reasoning: 0,
            },
            cost,
            file.agent.clone(),
        );
        message.dedup_key = Some(dedup_key);
        message.is_turn_start = pending_turn_start;
        message.session_title = session_title.clone();
        message.set_workspace(workspace_key.clone(), workspace_label.clone());
        if reported_cost.is_some() {
            message.mark_provider_reported_cost();
        }
        messages.push(message);
        assistant_index += 1;
        pending_turn_start = false;
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_cline_valid_api_req_started() {
        let dir = TempDir::new().unwrap();
        let task_dir = dir.path().join("tasks").join("cline-task-1");
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            task_dir.join("ui_messages.json"),
            r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.05,\"tokensIn\":40,\"tokensOut\":15,\"cacheReads\":7,\"cacheWrites\":3,\"apiProtocol\":\"anthropic\"}"
  }
]"#,
        )
        .unwrap();
        fs::write(
            task_dir.join("api_conversation_history.json"),
            r#"
<environment_details>
<model>claude-sonnet-4</model>
<name>ClineAgent</name>
</environment_details>
"#,
        )
        .unwrap();

        let messages = parse_cline_file(&task_dir.join("ui_messages.json"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "cline");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].model_id, "claude-sonnet-4");
        assert_eq!(messages[0].session_id, "cline-task-1");
        assert_eq!(messages[0].agent.as_deref(), Some("ClineAgent"));
        assert_eq!(messages[0].tokens.input, 40);
        assert_eq!(messages[0].tokens.output, 15);
        assert_eq!(messages[0].tokens.cache_read, 7);
        assert_eq!(messages[0].tokens.cache_write, 3);
        assert_eq!(messages[0].cost, 0.05);
    }

    #[test]
    fn test_parse_cline_ignores_non_api_req_started_events() {
        let dir = TempDir::new().unwrap();
        let task_dir = dir.path().join("tasks").join("cline-task-2");
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            task_dir.join("ui_messages.json"),
            r#"[
  {
    "type": "say",
    "say": "assistant_message",
    "ts": "2026-02-18T12:00:00Z",
    "text": "{\"cost\":0.2,\"tokensIn\":10,\"tokensOut\":1,\"cacheReads\":0,\"cacheWrites\":0,\"apiProtocol\":\"anthropic\"}"
  }
]"#,
        )
        .unwrap();

        let messages = parse_cline_file(&task_dir.join("ui_messages.json"));
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_cline_cli_messages() {
        let dir = TempDir::new().unwrap();
        let session_dir = dir.path().join("sessions").join("cline-cli-session");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("cline-cli-session.json"),
            r#"{
  "session_id": "cline-cli-session",
  "provider": "cline-pass",
  "model": "cline-pass/glm-5.2",
  "workspace_root": "/home/example/project",
  "metadata": {"title": "CLI task"}
}"#,
        )
        .unwrap();
        fs::write(
            session_dir.join("cline-cli-session.messages.json"),
            r#"{
  "sessionId": "cline-cli-session",
  "agent": "lead",
  "messages": [
    {
      "role": "user",
      "ts": 1785320464923,
      "content": [{"type": "text", "text": "Inspect this project."}]
    },
    {
      "id": "msg-1",
      "role": "assistant",
      "ts": 1785320475705,
      "modelInfo": {"id": "cline-free/glm-5.2", "provider": "cline-pass"},
      "metrics": {
        "inputTokens": 7507,
        "outputTokens": 131,
        "cacheReadTokens": 50,
        "cacheWriteTokens": 0,
        "cost": 0.0110232
      }
    },
    {"role": "assistant", "metrics": {"inputTokens": 0, "outputTokens": 0}}
  ]
}"#,
        )
        .unwrap();

        let messages = parse_cline_file(&session_dir.join("cline-cli-session.messages.json"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "cline");
        assert_eq!(messages[0].provider_id, "cline-pass");
        assert_eq!(messages[0].model_id, "cline-free/glm-5.2");
        assert_eq!(messages[0].session_id, "cline-cli-session");
        assert_eq!(messages[0].agent.as_deref(), Some("lead"));
        assert_eq!(messages[0].tokens.input, 7457);
        assert_eq!(messages[0].tokens.output, 131);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.cache_write, 0);
        assert_eq!(messages[0].cost, 0.0110232);
        assert_eq!(
            messages[0].cost_source,
            crate::sessions::CostSource::ProviderReported
        );
        assert_eq!(messages[0].workspace_label.as_deref(), Some("project"));
        assert_eq!(messages[0].session_title.as_deref(), Some("CLI task"));
        assert!(messages[0].is_turn_start);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("cline-cli:cline-cli-session:msg-1")
        );
    }

    #[test]
    fn test_parse_cline_cli_turn_starts_ignore_tool_results() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("turns.messages.json");
        fs::write(
            &path,
            r#"{
  "sessionId": "turns",
  "messages": [
    {
      "id": "prompt-1",
      "role": "user",
      "content": [
        {"type": "text", "text": "Please inspect the repository."}
      ]
    },
    {
      "id": "assistant-tool-use",
      "role": "assistant",
      "content": [
        {"type": "text", "text": "I will inspect the repository."},
        {
          "type": "tool_use",
          "id": "tool-1",
          "name": "read_file",
          "input": {"path": "README.md"}
        },
        {"type": "future_block", "payload": {"priority": "low"}}
      ],
      "modelInfo": {"id": "provider/model", "provider": "provider"},
      "metrics": {
        "inputTokens": 100,
        "outputTokens": 20,
        "cacheReadTokens": 10,
        "cacheWriteTokens": 5,
        "cost": 0.02
      }
    },
    {
      "id": "tool-result",
      "role": "user",
      "content": [
        {
          "type": "tool_result",
          "tool_use_id": "tool-1",
          "content": [{"type": "text", "text": "README contents"}]
        }
      ]
    },
    {
      "id": "assistant-final",
      "role": "assistant",
      "content": [
        {"type": "text", "text": "The repository is ready."}
      ],
      "metrics": {
        "inputTokens": 15,
        "outputTokens": 6,
        "cacheReadTokens": 0,
        "cacheWriteTokens": 0
      }
    }
  ]
}"#,
        )
        .unwrap();

        let messages = parse_cline_cli_file(&path);

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("cline-cli:turns:assistant-tool-use")
        );
        assert_eq!(
            messages[1].dedup_key.as_deref(),
            Some("cline-cli:turns:assistant-final")
        );
        assert!(messages[0].is_turn_start);
        assert!(!messages[1].is_turn_start);
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.is_turn_start)
                .count(),
            1
        );
    }

    #[test]
    fn test_parse_cline_cli_normalizes_cache_tokens_and_preserves_zero_cost() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.messages.json");
        fs::write(
            &path,
            r#"{
  "sessionId": "session-1",
  "messages": [
    {
      "id": "zero-cost",
      "role": "assistant",
      "metrics": {
        "inputTokens": 12,
        "outputTokens": 0,
        "cacheReadTokens": 5,
        "cacheWriteTokens": 2,
        "cost": 0
      }
    },
    {
      "id": "invalid-cost",
      "role": "assistant",
      "metrics": {
        "inputTokens": 1,
        "outputTokens": 2,
        "cost": "NaN"
      }
    }
  ]
}"#,
        )
        .unwrap();

        let messages = parse_cline_cli_file(&path);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 5);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].tokens.cache_write, 2);
        assert_eq!(messages[0].cost, 0.0);
        assert_eq!(
            messages[0].cost_source,
            crate::sessions::CostSource::ProviderReported
        );
        assert_eq!(messages[1].cost, 0.0);
        assert_eq!(
            messages[1].cost_source,
            crate::sessions::CostSource::Unknown
        );
    }
}
