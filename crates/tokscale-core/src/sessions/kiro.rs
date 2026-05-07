//! Kiro session parser
//!
//! Parses session files from ~/.kiro/sessions/cli/*.json. Kiro writes a
//! session metadata JSON record followed by JSONL message records. Turn-level
//! token counts are currently zero, so usage is estimated from prompt and
//! assistant text with a chars/4 approximation.

use super::utils::file_modified_timestamp_ms;
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

const CLIENT_ID: &str = "kiro";
const PROVIDER_ID: &str = "amazon-bedrock";
const UNKNOWN_MODEL: &str = "unknown";

#[derive(Debug, Deserialize)]
struct KiroSessionHeader {
    session_id: Option<String>,
    cwd: Option<String>,
    session_state: Option<KiroSessionState>,
}

#[derive(Debug, Deserialize)]
struct KiroSessionState {
    rts_model_state: Option<KiroRtsModelState>,
    conversation_metadata: Option<KiroConversationMetadata>,
}

#[derive(Debug, Deserialize)]
struct KiroRtsModelState {
    model_info: Option<KiroModelInfo>,
}

#[derive(Debug, Deserialize)]
struct KiroModelInfo {
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KiroConversationMetadata {
    user_turn_metadatas: Option<Vec<KiroTurnMetadata>>,
}

#[derive(Debug, Deserialize)]
struct KiroTurnMetadata {
    input_token_count: Option<i64>,
    output_token_count: Option<i64>,
    end_timestamp: Option<serde_json::Value>,
    total_request_count: Option<i32>,
    message_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct KiroJsonlEntry {
    kind: String,
    data: Option<KiroJsonlData>,
}

#[derive(Debug, Deserialize)]
struct KiroJsonlData {
    message_id: Option<String>,
    content: Option<Vec<KiroContentPart>>,
    meta: Option<KiroEntryMeta>,
}

#[derive(Debug, Deserialize)]
struct KiroContentPart {
    kind: Option<String>,
    data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KiroEntryMeta {
    timestamp: Option<f64>,
}

#[derive(Debug, Clone, Default)]
struct KiroMessageContent {
    prompt_chars: usize,
    assistant_chars: usize,
    prompt_timestamp_ms: Option<i64>,
}

pub fn parse_kiro_file(path: &Path) -> Vec<UnifiedMessage> {
    let fallback_timestamp = file_modified_timestamp_ms(path);

    let mut json_bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };

    let header = match simd_json::from_slice::<KiroSessionHeader>(&mut json_bytes) {
        Ok(header) => header,
        Err(_) => return Vec::new(),
    };

    let session_id = header
        .session_id
        .unwrap_or_else(|| session_id_from_path(path));
    let model_id = header
        .session_state
        .as_ref()
        .and_then(|state| state.rts_model_state.as_ref())
        .and_then(|state| state.model_info.as_ref())
        .and_then(|info| info.model_id.as_deref())
        .filter(|model| !model.trim().is_empty())
        .unwrap_or(UNKNOWN_MODEL)
        .to_string();
    let workspace_key = header.cwd.as_deref().and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
    let turns = header
        .session_state
        .and_then(|state| state.conversation_metadata)
        .and_then(|metadata| metadata.user_turn_metadatas)
        .unwrap_or_default();

    let jsonl_path = path.with_extension("jsonl");
    let mut content_by_message_id: HashMap<String, KiroMessageContent> = HashMap::new();

    if let Ok(jsonl_file) = std::fs::File::open(&jsonl_path) {
        let reader = BufReader::new(jsonl_file);
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let mut bytes = trimmed.as_bytes().to_vec();
            let entry = match simd_json::from_slice::<KiroJsonlEntry>(&mut bytes) {
                Ok(entry) => entry,
                Err(_) => continue,
            };

            let Some(data) = entry.data else {
                continue;
            };
            let Some(message_id) = data.message_id else {
                continue;
            };

            let text_chars = text_char_count(data.content.as_deref());
            if text_chars == 0 {
                continue;
            }

            let message = content_by_message_id.entry(message_id).or_default();
            match entry.kind.as_str() {
                "Prompt" => {
                    message.prompt_chars += text_chars;
                    if message.prompt_timestamp_ms.is_none() {
                        message.prompt_timestamp_ms = data
                            .meta
                            .and_then(|meta| meta.timestamp)
                            .map(seconds_to_millis);
                    }
                }
                "AssistantMessage" => {
                    message.assistant_chars += text_chars;
                }
                _ => {}
            }
        }
    }

    turns
        .into_iter()
        .enumerate()
        .filter_map(|(index, turn)| {
            let message_ids = turn.message_ids.unwrap_or_default();
            let mut prompt_chars = 0;
            let mut assistant_chars = 0;
            let mut prompt_timestamp_ms = None;

            for message_id in &message_ids {
                let Some(content) = content_by_message_id.get(message_id) else {
                    continue;
                };
                prompt_chars += content.prompt_chars;
                assistant_chars += content.assistant_chars;
                if prompt_timestamp_ms.is_none() {
                    prompt_timestamp_ms = content.prompt_timestamp_ms;
                }
            }

            let explicit_input = turn.input_token_count.unwrap_or(0).max(0);
            let explicit_output = turn.output_token_count.unwrap_or(0).max(0);
            let input = if explicit_input > 0 {
                explicit_input
            } else {
                estimate_tokens(prompt_chars)
            };
            let output = if explicit_output > 0 {
                explicit_output
            } else {
                estimate_tokens(assistant_chars)
            };

            if input + output == 0 {
                return None;
            }

            let timestamp = prompt_timestamp_ms
                .or_else(|| parse_timestamp_value(turn.end_timestamp.as_ref()))
                .unwrap_or(fallback_timestamp);

            let mut message = UnifiedMessage::new_with_dedup(
                CLIENT_ID,
                model_id.clone(),
                PROVIDER_ID,
                session_id.clone(),
                timestamp,
                TokenBreakdown {
                    input,
                    output,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.0,
                Some(format!("{}:{}", session_id, index)),
            );
            message.message_count = turn.total_request_count.unwrap_or(1).max(1);
            message.is_turn_start = true;
            message.set_workspace(workspace_key.clone(), workspace_label.clone());
            Some(message)
        })
        .collect()
}

fn text_char_count(content: Option<&[KiroContentPart]>) -> usize {
    content
        .unwrap_or_default()
        .iter()
        .filter(|part| part.kind.as_deref().is_none_or(|kind| kind == "text"))
        .filter_map(|part| part.data.as_deref())
        .map(str::chars)
        .map(Iterator::count)
        .sum()
}

fn estimate_tokens(chars: usize) -> i64 {
    chars.div_ceil(4) as i64
}

fn seconds_to_millis(seconds: f64) -> i64 {
    (seconds * 1000.0) as i64
}

fn parse_timestamp_value(value: Option<&serde_json::Value>) -> Option<i64> {
    match value? {
        serde_json::Value::Number(number) => number.as_f64().map(|timestamp| {
            if timestamp.abs() < 1_000_000_000_000.0 {
                seconds_to_millis(timestamp)
            } else {
                timestamp as i64
            }
        }),
        serde_json::Value::String(timestamp) => chrono::DateTime::parse_from_rfc3339(timestamp)
            .ok()
            .map(|dt| dt.timestamp_millis())
            .or_else(|| timestamp.parse::<f64>().ok().map(seconds_to_millis)),
        _ => None,
    }
}

fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_session_files(dir: &TempDir, stem: &str, json: &str, jsonl: &str) -> std::path::PathBuf {
        let json_path = dir.path().join(format!("{}.json", stem));
        let jsonl_path = dir.path().join(format!("{}.jsonl", stem));
        let mut f = std::fs::File::create(&json_path).unwrap();
        f.write_all(json.as_bytes()).unwrap();
        let mut f = std::fs::File::create(&jsonl_path).unwrap();
        f.write_all(jsonl.as_bytes()).unwrap();
        json_path
    }

    #[test]
    fn test_parse_kiro_estimates_tokens_from_jsonl_content() {
        let dir = TempDir::new().unwrap();
        let json = r#"{"session_id":"session-1","cwd":"/tmp/project","session_state":{"rts_model_state":{"model_info":{"model_id":"claude-sonnet-4-5"}},"conversation_metadata":{"user_turn_metadatas":[{"input_token_count":0,"output_token_count":0,"turn_duration":123,"end_timestamp":1770983427,"total_request_count":2,"message_ids":["prompt-1","assistant-1"]}]}}}"#;
        let jsonl = r#"{"version":"v1","kind":"Prompt","data":{"message_id":"prompt-1","content":[{"kind":"text","data":"hello world"}],"meta":{"timestamp":1770983426.420942}}}
{"version":"v1","kind":"AssistantMessage","data":{"message_id":"assistant-1","content":[{"kind":"text","data":"response text"}]}}"#;
        let path = create_session_files(&dir, "session-1", json, jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "kiro");
        assert_eq!(messages[0].provider_id, "amazon-bedrock");
        assert_eq!(messages[0].model_id, "claude-sonnet-4-5");
        assert_eq!(messages[0].session_id, "session-1");
        assert_eq!(messages[0].tokens.input, 3);
        assert_eq!(messages[0].tokens.output, 4);
        assert_eq!(messages[0].message_count, 2);
        assert!(messages[0].is_turn_start);
        assert_eq!(messages[0].timestamp, 1770983426420);
        assert_eq!(messages[0].workspace_key, Some("/tmp/project".to_string()));
        assert_eq!(messages[0].workspace_label, Some("project".to_string()));
    }

    #[test]
    fn test_parse_kiro_skips_zero_content_turns() {
        let dir = TempDir::new().unwrap();
        let json = r#"{"session_id":"session-2","cwd":"/tmp","session_state":{"rts_model_state":{"model_info":{"model_id":"model"}},"conversation_metadata":{"user_turn_metadatas":[{"input_token_count":0,"output_token_count":0,"message_ids":["missing"]}]}}}"#;
        let jsonl = "";
        let path = create_session_files(&dir, "session-2", json, jsonl);

        let messages = parse_kiro_file(&path);

        assert!(messages.is_empty());
    }
}
