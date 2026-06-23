//! Kimi CLI / Kimi Code session parser
//!
//! Parses wire.jsonl from both `kimi-cli` and `kimi-code`.
//!
//! ~/.kimi/sessions/[GROUP_ID]/[SESSION_UUID]/wire.jsonl
//!   Token data comes from StatusUpdate messages.
//!
//! ~/.kimi-code/sessions/[WORKSPACE]/[SESSION]/agents/[AGENT]/wire.jsonl
//!   Token data comes from usage.record lines.

use super::utils::file_modified_timestamp_ms;
use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Top-level wire.jsonl line: either metadata or a timestamped message
#[derive(Debug, Deserialize)]
struct WireLine {
    timestamp: Option<f64>,
    message: Option<WireMessage>,
    #[serde(rename = "type")]
    line_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireMessage {
    #[serde(rename = "type")]
    msg_type: String,
    payload: Option<StatusPayload>,
}

#[derive(Debug, Deserialize)]
struct StatusPayload {
    token_usage: Option<TokenUsage>,
    #[allow(dead_code)]
    message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenUsage {
    input_other: Option<i64>,
    output: Option<i64>,
    input_cache_read: Option<i64>,
    input_cache_creation: Option<i64>,
}

/// Default model name when config.json is not available
const DEFAULT_MODEL: &str = "kimi-for-coding";
const DEFAULT_PROVIDER: &str = "moonshot";

/// Read model name from ~/.kimi/config.json if available
fn read_model_from_config(wire_path: &Path) -> String {
    // Navigate from wire.jsonl up to ~/.kimi/config.json
    // wire.jsonl is at ~/.kimi/sessions/GROUP/UUID/wire.jsonl
    if let Some(sessions_dir) = wire_path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
    {
        if let Some(kimi_dir) = sessions_dir.parent() {
            let config_path = kimi_dir.join("config.json");
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(bytes) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(model) = bytes.get("model").and_then(|v| v.as_str()) {
                        if !model.is_empty() {
                            return model.to_string();
                        }
                    }
                }
            }
        }
    }
    DEFAULT_MODEL.to_string()
}

/// Extract session ID from the wire.jsonl path
/// Path format: ~/.kimi/sessions/GROUP_ID/SESSION_UUID/wire.jsonl
fn extract_session_id(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Check whether a wire.jsonl path belongs to kimi-code.
pub fn is_kimi_code_path(path: &Path) -> bool {
    // 1) Default installation path contains .kimi-code segment
    if path.components().any(|c| {
        c.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(".kimi-code")
    }) {
        return true;
    }

    // 2) Custom KIMI_CODE_HOME directory
    if let Ok(kimi_code_home) = std::env::var("KIMI_CODE_HOME") {
        if !kimi_code_home.is_empty() && path.starts_with(&kimi_code_home) {
            return true;
        }
    }

    false
}

/// Extract session ID from a kimi-code wire.jsonl path.
/// Path format: ~/.kimi-code/sessions/WORKSPACE/SESSION_UUID/agents/AGENT/wire.jsonl
fn extract_session_id_from_kimi_code_path(path: &Path) -> String {
    // Walk up: wire.jsonl -> AGENT -> agents -> SESSION_UUID -> ...
    path.parent() // AGENT
        .and_then(|p| p.parent()) // agents
        .and_then(|p| p.parent()) // SESSION_UUID
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Strip the "kimi-code/" prefix from model IDs emitted by kimi-code.
fn normalize_kimi_code_model(model: &str) -> String {
    model
        .strip_prefix("kimi-code/")
        .unwrap_or(model)
        .to_string()
}

/// Kimi Code wire.jsonl line structure.
#[derive(Debug, Deserialize)]
struct KimiCodeWireLine {
    #[serde(rename = "type")]
    line_type: String,
    model: Option<String>,
    usage: Option<KimiCodeUsage>,
    #[serde(rename = "usageScope")]
    usage_scope: Option<String>,
    time: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct KimiCodeUsage {
    #[serde(rename = "inputOther")]
    input_other: Option<i64>,
    output: Option<i64>,
    #[serde(rename = "inputCacheRead")]
    input_cache_read: Option<i64>,
    #[serde(rename = "inputCacheCreation")]
    input_cache_creation: Option<i64>,
}

/// Parse a Kimi Code wire.jsonl file.
pub fn parse_kimi_code_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let session_id = extract_session_id_from_kimi_code_path(path);
    let fallback_timestamp = file_modified_timestamp_ms(path);

    let reader = BufReader::new(file);
    let mut messages: Vec<UnifiedMessage> = Vec::new();

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
        let wire_line = match simd_json::from_slice::<KimiCodeWireLine>(&mut bytes) {
            Ok(wl) => wl,
            Err(_) => continue,
        };

        // Only process usage.record lines.
        // step.end also carries usage, but it duplicates the same usage.record
        // that was emitted in the same turn, so we ignore it to avoid double counting.
        if wire_line.line_type != "usage.record" {
            continue;
        }

        // Only count turn-scoped usage. kimi-code tags every usage.record with
        // usageScope: "turn" for per-step LLM calls made inside a user turn and
        // "session" for non-turn bookkeeping (e.g. context compaction), and its
        // own tooling treats a missing usageScope as session-scoped, so require
        // an explicit "turn" to avoid counting aggregate records.
        if wire_line.usage_scope.as_deref() != Some("turn") {
            continue;
        }

        let usage = match wire_line.usage {
            Some(u) => u,
            None => continue,
        };

        let input = usage.input_other.unwrap_or(0).max(0);
        let output = usage.output.unwrap_or(0).max(0);
        let cache_read = usage.input_cache_read.unwrap_or(0).max(0);
        let cache_write = usage.input_cache_creation.unwrap_or(0).max(0);

        // Skip entries with zero tokens
        if input + output + cache_read + cache_write == 0 {
            continue;
        }

        let model = wire_line
            .model
            .as_deref()
            .map(normalize_kimi_code_model)
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        let timestamp_ms = wire_line.time.unwrap_or(fallback_timestamp);

        messages.push(UnifiedMessage::new(
            "kimi",
            model,
            DEFAULT_PROVIDER,
            session_id.clone(),
            timestamp_ms,
            TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                reasoning: 0,
            },
            0.0,
        ));
    }

    messages
}

/// Parse a Kimi CLI wire.jsonl file
pub fn parse_kimi_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let model = read_model_from_config(path);
    let session_id = extract_session_id(path);

    let reader = BufReader::new(file);
    let mut messages: Vec<UnifiedMessage> = Vec::new();
    let mut keyed_indices: HashMap<String, usize> = HashMap::new();

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
        let wire_line = match simd_json::from_slice::<WireLine>(&mut bytes) {
            Ok(wl) => wl,
            Err(_) => continue,
        };

        // Skip metadata lines (first line: {"type": "metadata", ...})
        if wire_line.line_type.as_deref() == Some("metadata") {
            continue;
        }

        let message = match wire_line.message {
            Some(m) => m,
            None => continue,
        };

        // Only process StatusUpdate messages
        if message.msg_type != "StatusUpdate" {
            continue;
        }

        let payload = match message.payload {
            Some(p) => p,
            None => continue,
        };

        let token_usage = match payload.token_usage {
            Some(u) => u,
            None => continue,
        };

        // Convert Unix seconds (float) to milliseconds, fallback to file mtime
        let timestamp_ms = wire_line
            .timestamp
            .map(|ts| (ts * 1000.0) as i64)
            .unwrap_or_else(|| file_modified_timestamp_ms(path));

        let input = token_usage.input_other.unwrap_or(0).max(0);
        let output = token_usage.output.unwrap_or(0).max(0);
        let cache_read = token_usage.input_cache_read.unwrap_or(0).max(0);
        let cache_write = token_usage.input_cache_creation.unwrap_or(0).max(0);

        // Skip entries with zero tokens
        if input + output + cache_read + cache_write == 0 {
            continue;
        }

        let dedup_key = payload.message_id;

        let message = UnifiedMessage::new_with_dedup(
            "kimi",
            model.clone(),
            DEFAULT_PROVIDER,
            session_id.clone(),
            timestamp_ms,
            TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                // Kimi wire protocol does not expose reasoning tokens; all reasoning included in output
                reasoning: 0,
            },
            0.0,
            dedup_key,
        );
        push_or_replace_status_update(&mut messages, &mut keyed_indices, message);
    }

    messages
}

fn should_replace_status_update(existing: &UnifiedMessage, candidate: &UnifiedMessage) -> bool {
    let existing_total = existing.tokens.total();
    let candidate_total = candidate.tokens.total();

    candidate_total > existing_total
        || (candidate_total == existing_total && candidate.timestamp >= existing.timestamp)
}

fn push_or_replace_status_update(
    messages: &mut Vec<UnifiedMessage>,
    keyed_indices: &mut HashMap<String, usize>,
    message: UnifiedMessage,
) {
    let dedup_key = message
        .dedup_key
        .as_ref()
        .filter(|key| !key.is_empty())
        .cloned();

    let Some(dedup_key) = dedup_key else {
        messages.push(message);
        return;
    };

    if let Some(index) = keyed_indices.get(&dedup_key).copied() {
        if should_replace_status_update(&messages[index], &message) {
            messages[index] = message;
        }
        return;
    }

    let index = messages.len();
    messages.push(message);
    keyed_indices.insert(dedup_key, index);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_parse_kimi_valid_status_update() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983426.420942, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 1562, "output": 2463, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "chatcmpl-xxx"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "kimi");
        assert_eq!(messages[0].model_id, "kimi-for-coding");
        assert_eq!(messages[0].provider_id, "moonshot");
        assert_eq!(messages[0].tokens.input, 1562);
        assert_eq!(messages[0].tokens.output, 2463);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.cache_write, 0);
        // Timestamp: 1770983426.420942 * 1000 = 1770983426420
        assert_eq!(messages[0].timestamp, 1770983426420);
    }

    #[test]
    fn test_parse_kimi_multi_turn() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983400.0, "message": {"type": "TurnBegin", "payload": {"user_input": "hello"}}}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 100, "output": 200, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-1"}}}
{"timestamp": 1770983420.0, "message": {"type": "TurnBegin", "payload": {"user_input": "world"}}}
{"timestamp": 1770983430.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 300, "output": 400, "input_cache_read": 50, "input_cache_creation": 0}, "message_id": "msg-2"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 200);
        assert_eq!(messages[1].tokens.input, 300);
        assert_eq!(messages[1].tokens.output, 400);
        assert_eq!(messages[1].tokens.cache_read, 50);
    }

    #[test]
    fn test_parse_kimi_skip_non_status_update() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983400.0, "message": {"type": "TurnBegin", "payload": {"user_input": "hello"}}}
{"timestamp": 1770983410.0, "message": {"type": "ContentPart", "payload": {"type": "text", "text": "response"}}}
{"timestamp": 1770983420.0, "message": {"type": "ToolCall", "payload": {"type": "function", "id": "tool_1", "function": {"name": "ReadFile", "arguments": "{}"}}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_kimi_empty_file() {
        let file = create_test_file("");

        let messages = parse_kimi_file(file.path());

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_kimi_tool_call_multi_step() {
        // Simulates a tool-call scenario with multiple StatusUpdate messages in one turn
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983400.0, "message": {"type": "TurnBegin", "payload": {"user_input": "read file"}}}
{"timestamp": 1770983405.0, "message": {"type": "StepBegin", "payload": {"n": 1}}}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 500, "output": 100, "input_cache_read": 200, "input_cache_creation": 0}, "message_id": "msg-step1"}}}
{"timestamp": 1770983415.0, "message": {"type": "ToolCall", "payload": {"type": "function", "id": "tool_1", "function": {"name": "ReadFile", "arguments": "{}"}}}}
{"timestamp": 1770983420.0, "message": {"type": "StepBegin", "payload": {"n": 2}}}
{"timestamp": 1770983425.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 800, "output": 300, "input_cache_read": 400, "input_cache_creation": 100}, "message_id": "msg-step2"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 2);
        // Step 1
        assert_eq!(messages[0].tokens.input, 500);
        assert_eq!(messages[0].tokens.output, 100);
        assert_eq!(messages[0].tokens.cache_read, 200);
        assert_eq!(messages[0].tokens.cache_write, 0);
        // Step 2
        assert_eq!(messages[1].tokens.input, 800);
        assert_eq!(messages[1].tokens.output, 300);
        assert_eq!(messages[1].tokens.cache_read, 400);
        assert_eq!(messages[1].tokens.cache_write, 100);
    }

    #[test]
    fn test_parse_kimi_with_cache_tokens() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1771123711.615454, "message": {"type": "StatusUpdate", "payload": {"context_usage": 0.024, "token_usage": {"input_other": 1508, "output": 205, "input_cache_read": 4864, "input_cache_creation": 0}, "message_id": "chatcmpl-2tNw2mhUNfdPMP0Jyie7gDhD"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 1508);
        assert_eq!(messages[0].tokens.output, 205);
        assert_eq!(messages[0].tokens.cache_read, 4864);
        assert_eq!(messages[0].tokens.cache_write, 0);
    }

    #[test]
    fn test_parse_kimi_deduplicates_repeated_status_updates_by_message_id() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 100, "output": 10, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-progressive"}}}
{"timestamp": 1770983420.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 120, "output": 30, "input_cache_read": 5, "input_cache_creation": 0}, "message_id": "msg-progressive"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("msg-progressive"));
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].timestamp, 1770983420000);
    }

    #[test]
    fn test_parse_kimi_keeps_distinct_and_missing_message_ids_separate() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 10, "output": 1, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-1"}}}
{"timestamp": 1770983420.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 20, "output": 2, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-2"}}}
{"timestamp": 1770983430.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 30, "output": 3, "input_cache_read": 0, "input_cache_creation": 0}}}}
{"timestamp": 1770983440.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 40, "output": 4, "input_cache_read": 0, "input_cache_creation": 0}}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("msg-1"));
        assert_eq!(messages[1].dedup_key.as_deref(), Some("msg-2"));
        assert!(messages[2].dedup_key.is_none());
        assert!(messages[3].dedup_key.is_none());
    }

    #[test]
    fn test_parse_kimi_skips_zero_token_entries() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 0, "output": 0, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-empty"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_kimi_malformed_lines() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
not valid json at all
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 100, "output": 200, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-1"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 100);
    }

    // -------------------------------------------------------------------------
    // Kimi Code tests
    // -------------------------------------------------------------------------

    fn create_kimi_code_test_file(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        // Build a fake kimi-code path so extract_session_id_from_kimi_code_path works:
        //   .../.kimi-code/sessions/ws/session-uuid/agents/main/wire.jsonl
        let fake_path = dir
            .path()
            .join(".kimi-code")
            .join("sessions")
            .join("test-ws")
            .join("sess-abc-123")
            .join("agents")
            .join("main")
            .join("wire.jsonl");
        std::fs::create_dir_all(fake_path.parent().unwrap()).unwrap();
        std::fs::write(&fake_path, content).unwrap();
        (dir, fake_path)
    }

    #[test]
    fn test_parse_kimi_code_valid_usage_record() {
        let content = r#"{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":5102,"output":172,"inputCacheRead":13312,"inputCacheCreation":0},"usageScope":"turn","time":1780319377014}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "kimi");
        assert_eq!(messages[0].model_id, "kimi-for-coding");
        assert_eq!(messages[0].provider_id, "moonshot");
        assert!(!messages[0].session_id.is_empty());
        assert_ne!(messages[0].session_id, "unknown");
        assert_eq!(messages[0].tokens.input, 5102);
        assert_eq!(messages[0].tokens.output, 172);
        assert_eq!(messages[0].tokens.cache_read, 13312);
        assert_eq!(messages[0].tokens.cache_write, 0);
        assert_eq!(messages[0].timestamp, 1780319377014);
    }

    #[test]
    fn test_parse_kimi_code_skip_non_usage_record() {
        let content = r#"{"type":"context.append_loop_event","event":{"type":"tool.call","name":"Read"},"time":1780319377000}
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":100,"output":50,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319378000}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].timestamp, 1780319378000);
    }

    #[test]
    fn test_parse_kimi_code_model_normalization() {
        let content = r#"{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":1,"output":1,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377014}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);
        assert_eq!(messages[0].model_id, "kimi-for-coding");
    }

    #[test]
    fn test_parse_kimi_code_session_id_extraction() {
        assert_eq!(
            extract_session_id_from_kimi_code_path(std::path::Path::new(
                "/home/user/.kimi-code/sessions/workspace/session-uuid/agents/main/wire.jsonl"
            )),
            "session-uuid"
        );
        assert_eq!(
            extract_session_id_from_kimi_code_path(std::path::Path::new(
                "C:/Users/Alice/.kimi-code/sessions/workspace/sess-123/agents/coder/wire.jsonl"
            )),
            "sess-123"
        );
        assert_eq!(
            extract_session_id_from_kimi_code_path(std::path::Path::new("wire.jsonl")),
            "unknown"
        );
    }

    #[test]
    fn test_parse_kimi_code_only_counts_turn_scoped_usage() {
        // "session"-scoped records are non-turn bookkeeping (e.g. compaction)
        // and records without usageScope are treated as session-scoped by
        // kimi-code itself; neither should be counted.
        let content = r#"{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":999,"output":999,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"session","time":1780319377000}
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":888,"output":888,"inputCacheRead":0,"inputCacheCreation":0},"time":1780319377005}
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":100,"output":50,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377010}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].timestamp, 1780319377010);
    }

    #[test]
    fn test_parse_kimi_code_zero_tokens_skipped() {
        let content = r#"{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":0,"output":0,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377014}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_is_kimi_code_path() {
        assert!(is_kimi_code_path(std::path::Path::new(
            "/home/user/.kimi-code/sessions/workspace/sess/agents/main/wire.jsonl"
        )));
        assert!(!is_kimi_code_path(std::path::Path::new(
            "/home/user/.kimi/sessions/group/uuid/wire.jsonl"
        )));
    }

    #[test]
    #[serial_test::serial]
    fn test_is_kimi_code_path_with_custom_home() {
        std::env::set_var("KIMI_CODE_HOME", "/data/kimi");
        assert!(is_kimi_code_path(std::path::Path::new(
            "/data/kimi/sessions/ws/sess/agents/main/wire.jsonl"
        )));
        assert!(!is_kimi_code_path(std::path::Path::new(
            "/home/user/.kimi/sessions/group/uuid/wire.jsonl"
        )));
        std::env::remove_var("KIMI_CODE_HOME");
    }
}
