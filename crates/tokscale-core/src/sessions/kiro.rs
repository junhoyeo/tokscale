//! Kiro session parser
//!
//! Parses session data from three sources:
//! 1. File-based: ~/.kiro/sessions/cli/*.json + *.jsonl
//! 2. Kiro IDE globalStorage snapshots
//! 3. SQLite-based: ~/Library/Application Support/kiro-cli/data.sqlite3
//!    (conversations_v2 table with history[*].request_metadata)
//!
//! Turn-level token counts are currently zero in both sources, so usage is
//! estimated from context_usage_percentage * context_window (input) and
//! response_size / 4 (output).

use super::utils::file_modified_timestamp_ms;
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use tracing::warn;

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
    context_window_tokens: Option<i64>,
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
    message_ids: Option<Vec<Option<String>>>,
    context_usage_percentage: Option<f64>,
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
    if is_kiro_global_storage_path(path)
        || path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("chat"))
            .unwrap_or(false)
    {
        return parse_kiro_global_storage_file(path);
    }

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
    let context_window = header
        .session_state
        .as_ref()
        .and_then(|state| state.rts_model_state.as_ref())
        .and_then(|state| state.model_info.as_ref())
        .and_then(|info| info.context_window_tokens)
        .unwrap_or(0);
    let turns = header
        .session_state
        .and_then(|state| state.conversation_metadata)
        .and_then(|metadata| metadata.user_turn_metadatas)
        .unwrap_or_default();

    let jsonl_path = path.with_extension("jsonl");
    let mut content_by_message_id: HashMap<String, KiroMessageContent> = HashMap::new();

    if let Ok(jsonl_file) = std::fs::File::open(&jsonl_path) {
        let reader = BufReader::new(jsonl_file);
        let mut pending_prompt: Option<(usize, Option<i64>)> = None;

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
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

            match entry.kind.as_str() {
                "Prompt" => {
                    let timestamp_ms = data
                        .meta
                        .and_then(|meta| meta.timestamp)
                        .map(seconds_to_millis);
                    pending_prompt = Some((text_chars, timestamp_ms));
                }
                "AssistantMessage" => {
                    let message = content_by_message_id.entry(message_id).or_default();
                    if let Some((prompt_chars, prompt_ts)) = pending_prompt.take() {
                        message.prompt_chars += prompt_chars;
                        if message.prompt_timestamp_ms.is_none() {
                            message.prompt_timestamp_ms = prompt_ts;
                        }
                    }
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

            for message_id in message_ids.iter().flatten() {
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
            } else if context_window > 0 {
                let ctx_pct = turn.context_usage_percentage.unwrap_or(0.0);
                if ctx_pct > 0.0 {
                    ((context_window as f64) * ctx_pct / 100.0) as i64
                } else {
                    estimate_tokens(prompt_chars)
                }
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

            let end_timestamp_ms = parse_timestamp_value(turn.end_timestamp.as_ref());
            let duration_ms = duration_between_ms(prompt_timestamp_ms, end_timestamp_ms);
            let timestamp = prompt_timestamp_ms
                .or(end_timestamp_ms)
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
            message.duration_ms = duration_ms;
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

fn duration_between_ms(start_ms: Option<i64>, end_ms: Option<i64>) -> Option<i64> {
    let duration = end_ms?.saturating_sub(start_ms?);
    (duration > 0).then_some(duration)
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

fn is_kiro_global_storage_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains("globalStorage") && path_str.contains("kiro.kiroagent")
}

/// Extract the workspace folder name from a Kiro globalStorage path.
///
/// Snapshots live under `.../globalStorage/kiro.kiroagent/<workspace>/...`,
/// so the workspace folder is the path segment immediately following the
/// `kiro.kiroagent` component. Returns `None` when no such segment exists.
fn kiro_global_storage_workspace(path: &Path) -> Option<String> {
    let mut components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned());
    while let Some(component) = components.next() {
        if component == "kiro.kiroagent" {
            return components.next();
        }
    }
    None
}

#[derive(Debug, Default)]
struct KiroSnapshotTextCounts {
    prompt_chars: usize,
    assistant_chars: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KiroSnapshotRole {
    Prompt,
    Assistant,
}

fn collect_kiro_snapshot_text(
    value: &Value,
    counts: &mut KiroSnapshotTextCounts,
    mut role: Option<KiroSnapshotRole>,
) {
    match value {
        Value::Object(map) => {
            if let Some(kind) = map.get("role").and_then(|v| v.as_str()) {
                role = match kind {
                    "user" | "prompt" => Some(KiroSnapshotRole::Prompt),
                    "assistant" | "response" => Some(KiroSnapshotRole::Assistant),
                    _ => role,
                };
            }
            if let Some(kind) = map.get("type").and_then(|v| v.as_str()) {
                role = match kind {
                    "user" | "prompt" => Some(KiroSnapshotRole::Prompt),
                    "assistant" | "response" => Some(KiroSnapshotRole::Assistant),
                    _ => role,
                };
            }

            for key in ["prompt", "response", "content", "text", "message"] {
                if let Some(item) = map.get(key) {
                    collect_kiro_snapshot_text(item, counts, role);
                }
            }

            for key in [
                "messages",
                "conversation",
                "chat",
                "transcript",
                "entries",
                "events",
                "history",
            ] {
                if let Some(item) = map.get(key) {
                    collect_kiro_snapshot_text(item, counts, role);
                }
            }

            for key in ["parts", "items", "nodes"] {
                if let Some(item) = map.get(key) {
                    collect_kiro_snapshot_text(item, counts, role);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_kiro_snapshot_text(item, counts, role);
            }
        }
        Value::String(text) => match role {
            Some(KiroSnapshotRole::Assistant) => counts.assistant_chars += text.chars().count(),
            Some(KiroSnapshotRole::Prompt) => counts.prompt_chars += text.chars().count(),
            None => {}
        },
        _ => {}
    }
}

fn find_kiro_snapshot_model_id(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in ["model_id", "modelId", "model"] {
                if let Some(model) = map.get(key).and_then(|v| v.as_str()) {
                    let model = model.trim();
                    if !model.is_empty() {
                        return Some(model.to_string());
                    }
                }
            }

            for key in [
                "messages",
                "conversation",
                "chat",
                "transcript",
                "entries",
                "events",
                "history",
                "content",
                "text",
                "message",
            ] {
                if let Some(item) = map.get(key) {
                    if let Some(model) = find_kiro_snapshot_model_id(item) {
                        return Some(model);
                    }
                }
            }

            None
        }
        Value::Array(items) => items.iter().find_map(find_kiro_snapshot_model_id),
        _ => None,
    }
}

fn parse_kiro_global_storage_file(path: &Path) -> Vec<UnifiedMessage> {
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let json = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return Vec::new(),
    };

    let value: Value = match serde_json::from_str(&json) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    let file_stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    // Attribute the snapshot to its IDE workspace folder, mirroring how the
    // file/sqlite Kiro paths derive workspace identity from `cwd`.
    let workspace = kiro_global_storage_workspace(path);
    let workspace_key = workspace.as_deref().and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
    // Namespace the session id by workspace so two workspaces that both contain
    // e.g. `execution.chat` do not collapse into one session / dedup_key.
    let session_id = match workspace.as_deref() {
        Some(ws) => format!("{}/{}", ws, file_stem),
        None => file_stem.to_string(),
    };
    let model_id = find_kiro_snapshot_model_id(&value).unwrap_or_else(|| UNKNOWN_MODEL.to_string());

    let mut counts = KiroSnapshotTextCounts::default();
    collect_kiro_snapshot_text(&value, &mut counts, None);

    let input = estimate_tokens(counts.prompt_chars);
    let output = estimate_tokens(counts.assistant_chars);
    if input + output == 0 {
        return Vec::new();
    }

    let mut message = UnifiedMessage::new_with_dedup(
        CLIENT_ID,
        model_id,
        PROVIDER_ID,
        session_id.clone(),
        fallback_timestamp,
        TokenBreakdown {
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        },
        0.0,
        // dedup_key is structurally disjoint from the sqlite (CLI) source: this
        // key is `<workspace>/<file_stem>:globalstorage` (always the literal
        // `:globalstorage` suffix), whereas `parse_kiro_sqlite` emits
        // `<conversation_id>:<turn_index>` (always a numeric-index suffix).
        // IDE globalStorage snapshots and the kiro-cli `data.sqlite3` are
        // distinct surfaces, so the same conversation cannot appear in both and
        // these keys can never collide — no cross-source dedup is needed.
        Some(format!("{}:globalstorage", session_id)),
    );
    message.message_count = 1;
    message.is_turn_start = true;
    message.set_workspace(workspace_key, workspace_label);
    vec![message]
}

pub fn parse_kiro_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let conn = match Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open Kiro CLI database"
            );
            return Vec::new();
        }
    };

    let query = "SELECT key, conversation_id, value FROM conversations_v2";
    let mut stmt = match conn.prepare(query) {
        Ok(s) => s,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to prepare Kiro conversations query"
            );
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) {
        Ok(r) => r,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to execute Kiro conversations query"
            );
            return Vec::new();
        }
    };

    let mut messages = Vec::new();

    for row in rows.flatten() {
        let (cwd, conversation_id, json_str) = row;
        let parsed = match serde_json::from_str::<KiroDbConversation>(&json_str) {
            Ok(p) => p,
            Err(_) => continue,
        };

        let context_window = parsed
            .model_info
            .as_ref()
            .and_then(|info| info.context_window_tokens)
            .unwrap_or(0);
        let model_id = parsed
            .model_info
            .as_ref()
            .and_then(|info| info.model_id.as_deref())
            .filter(|m| !m.trim().is_empty())
            .unwrap_or(UNKNOWN_MODEL)
            .to_string();
        let workspace_key = normalize_workspace_key(&cwd);
        let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

        let history = parsed.history.unwrap_or_default();
        for (index, turn) in history.into_iter().enumerate() {
            let Some(meta) = turn.request_metadata else {
                continue;
            };

            let ctx_pct = meta.context_usage_percentage.unwrap_or(0.0);
            let response_size = meta.response_size.unwrap_or(0);

            let input = if context_window > 0 && ctx_pct > 0.0 {
                ((context_window as f64) * ctx_pct / 100.0) as i64
            } else {
                0
            };
            let output = estimate_tokens(response_size);

            if input + output == 0 {
                continue;
            }

            let duration_ms = duration_between_ms(
                meta.request_start_timestamp_ms,
                meta.stream_end_timestamp_ms,
            );
            let timestamp = meta
                .request_start_timestamp_ms
                .or(meta.stream_end_timestamp_ms)
                .unwrap_or(0);

            let mut message = UnifiedMessage::new_with_dedup(
                CLIENT_ID,
                model_id.clone(),
                PROVIDER_ID,
                conversation_id.clone(),
                timestamp,
                TokenBreakdown {
                    input,
                    output,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.0,
                Some(format!("{}:{}", conversation_id, index)),
            );
            message.message_count = 1;
            message.duration_ms = duration_ms;
            message.is_turn_start = true;
            message.set_workspace(workspace_key.clone(), workspace_label.clone());
            messages.push(message);
        }
    }

    messages
}

#[derive(Debug, Deserialize)]
struct KiroDbConversation {
    history: Option<Vec<KiroDbTurn>>,
    model_info: Option<KiroModelInfo>,
}

#[derive(Debug, Deserialize)]
struct KiroDbTurn {
    request_metadata: Option<KiroDbRequestMetadata>,
}

#[derive(Debug, Deserialize)]
struct KiroDbRequestMetadata {
    context_usage_percentage: Option<f64>,
    response_size: Option<usize>,
    request_start_timestamp_ms: Option<i64>,
    stream_end_timestamp_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_session_files(
        dir: &TempDir,
        stem: &str,
        json: &str,
        jsonl: &str,
    ) -> std::path::PathBuf {
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
        assert_eq!(messages[0].duration_ms, Some(580));
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

    #[test]
    fn test_parse_kiro_skips_malformed_jsonl_lines() {
        let dir = TempDir::new().unwrap();
        let json = r#"{"session_id":"session-3","cwd":"/tmp/project","session_state":{"rts_model_state":{"model_info":{"model_id":"claude-sonnet-4-5"}},"conversation_metadata":{"user_turn_metadatas":[{"input_token_count":0,"output_token_count":0,"turn_duration":100,"end_timestamp":1770983427,"total_request_count":2,"message_ids":["prompt-3","assistant-3"]}]}}}"#;
        let jsonl = r#"{"version":"v1","kind":"Prompt","data":{"message_id":"prompt-3","content":[{"kind":"text","data":"hello world"}],"meta":{"timestamp":1770983426.420942}}}
not valid json at all
{"version":"v1","kind":"AssistantMessage","data":{"message_id":"assistant-3","content":[{"kind":"text","data":"response text"}]}}"#;
        let path = create_session_files(&dir, "session-3", json, jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].tokens.input > 0 || messages[0].tokens.output > 0);
    }

    #[test]
    fn test_parse_kiro_sqlite_sets_duration_from_request_metadata() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("data.sqlite3");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE conversations_v2 (key TEXT, conversation_id TEXT, value TEXT)",
            [],
        )
        .unwrap();
        let value = r#"{
            "model_info": {
                "model_id": "auto",
                "context_window_tokens": 1000
            },
            "history": [{
                "request_metadata": {
                    "context_usage_percentage": 10,
                    "response_size": 40,
                    "request_start_timestamp_ms": 1770983426000,
                    "stream_end_timestamp_ms": 1770983427500
                }
            }]
        }"#;
        conn.execute(
            "INSERT INTO conversations_v2 (key, conversation_id, value) VALUES (?1, ?2, ?3)",
            (&"/tmp/project", &"conv-1", &value),
        )
        .unwrap();
        drop(conn);

        let messages = parse_kiro_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "auto");
        assert_eq!(messages[0].timestamp, 1770983426000);
        assert_eq!(messages[0].duration_ms, Some(1500));
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 10);
    }

    #[test]
    fn test_parse_kiro_global_storage_chat_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-a/execution.chat",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            r#"{
                "model": "auto",
                "messages": [
                    {"role": "user", "content": "hello world"},
                    {"role": "assistant", "content": "response text"}
                ]
            }"#,
        )
        .unwrap();

        let messages = parse_kiro_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "kiro");
        assert_eq!(messages[0].model_id, "auto");
        assert!(messages[0].tokens.input > 0);
        assert!(messages[0].tokens.output > 0);
        // (4a) Workspace attribution: the `<workspace>` segment after
        // `kiro.kiroagent/` flows through the same workspace helpers.
        assert_eq!(messages[0].workspace_key, Some("workspace-a".to_string()));
        assert_eq!(messages[0].workspace_label, Some("workspace-a".to_string()));
        assert_eq!(
            messages[0].dedup_key,
            Some("workspace-a/execution:globalstorage".to_string())
        );
    }

    #[test]
    fn test_parse_kiro_global_storage_dedup_keys_differ_across_workspaces() {
        let dir = TempDir::new().unwrap();
        let payload = r#"{
                "model": "auto",
                "messages": [
                    {"role": "user", "content": "hello world"},
                    {"role": "assistant", "content": "response text"}
                ]
            }"#;

        // Two `execution.chat` snapshots under DIFFERENT workspaces.
        let path_a = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-a/execution.chat",
        );
        let path_b = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-b/execution.chat",
        );
        fs::create_dir_all(path_a.parent().unwrap()).unwrap();
        fs::create_dir_all(path_b.parent().unwrap()).unwrap();
        fs::write(&path_a, payload).unwrap();
        fs::write(&path_b, payload).unwrap();

        let messages_a = parse_kiro_file(&path_a);
        let messages_b = parse_kiro_file(&path_b);

        assert_eq!(messages_a.len(), 1);
        assert_eq!(messages_b.len(), 1);
        assert_ne!(messages_a[0].dedup_key, messages_b[0].dedup_key);
        assert_eq!(
            messages_a[0].dedup_key,
            Some("workspace-a/execution:globalstorage".to_string())
        );
        assert_eq!(
            messages_b[0].dedup_key,
            Some("workspace-b/execution:globalstorage".to_string())
        );
    }

    #[test]
    fn test_parse_kiro_global_storage_ignores_unknown_roles() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-a/execution.chat",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            r#"{
                "model": "auto",
                "messages": [
                    {"role": "mystery", "content": "mystery text"},
                    {"role": "assistant", "content": "response text"}
                ]
            }"#,
        )
        .unwrap();

        let messages = parse_kiro_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.output, 4);
    }
}
