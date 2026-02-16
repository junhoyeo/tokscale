//! Claude Code session parser
//!
//! Parses JSONL files from ~/.claude/projects/

use super::utils::{
    extract_i64, extract_string, file_modified_timestamp_ms, parse_timestamp_value,
};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Claude Code entry structure (from JSONL files)
#[derive(Debug, Deserialize)]
pub struct ClaudeEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub timestamp: Option<String>,
    pub message: Option<ClaudeMessage>,
    /// Request ID for deduplication (used with message.id)
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClaudeMessage {
    pub model: Option<String>,
    pub usage: Option<ClaudeUsage>,
    /// Message ID for deduplication (used with requestId)
    pub id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClaudeUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
}

/// Parse a Claude Code JSONL file
pub fn parse_claude_file(path: &Path) -> Vec<UnifiedMessage> {
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let fallback_timestamp = file_modified_timestamp_ms(path);

    if path.extension().and_then(|s| s.to_str()) == Some("json") {
        let json_messages = parse_claude_headless_json(path, &session_id, fallback_timestamp);
        if !json_messages.is_empty() {
            return json_messages;
        }
    }

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut processed_hashes: HashSet<String> = HashSet::new();
    let mut headless_state = ClaudeHeadlessState::default();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut handled = false;
        let mut bytes = trimmed.as_bytes().to_vec();
        if let Ok(entry) = simd_json::from_slice::<ClaudeEntry>(&mut bytes) {
            // Only process assistant messages with usage data
            if entry.entry_type == "assistant" {
                let message = match entry.message {
                    Some(m) => m,
                    None => continue,
                };

                // Build dedup key for global deduplication (messageId:requestId composite)
                let dedup_key = match (&message.id, &entry.request_id) {
                    (Some(msg_id), Some(req_id)) => {
                        let hash = format!("{}:{}", msg_id, req_id);
                        if !processed_hashes.insert(hash.clone()) {
                            continue;
                        }
                        Some(hash)
                    }
                    _ => None,
                };

                let usage = match message.usage {
                    Some(u) => u,
                    None => continue,
                };

                let model = match message.model {
                    Some(m) => m,
                    None => continue,
                };

                let timestamp = entry
                    .timestamp
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
                    .map(|dt| dt.timestamp_millis())
                    .unwrap_or(fallback_timestamp);

                messages.push(UnifiedMessage::new_with_dedup(
                    "claude",
                    model,
                    "anthropic",
                    session_id.clone(),
                    timestamp,
                    TokenBreakdown {
                        input: usage.input_tokens.unwrap_or(0).max(0),
                        output: usage.output_tokens.unwrap_or(0).max(0),
                        cache_read: usage.cache_read_input_tokens.unwrap_or(0).max(0),
                        cache_write: usage.cache_creation_input_tokens.unwrap_or(0).max(0),
                        reasoning: 0,
                    },
                    0.0,
                    dedup_key,
                ));
                handled = true;
            }
        }

        if handled {
            continue;
        }

        if let Some(message) = process_claude_headless_line(
            trimmed,
            &session_id,
            &mut headless_state,
            fallback_timestamp,
        ) {
            messages.push(message);
        }
    }

    if let Some(message) =
        finalize_headless_state(&mut headless_state, &session_id, fallback_timestamp)
    {
        messages.push(message);
    }

    messages
}

#[derive(Default)]
struct ClaudeHeadlessState {
    model: Option<String>,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    timestamp_ms: Option<i64>,
}

fn parse_claude_headless_json(
    path: &Path,
    session_id: &str,
    fallback_timestamp: i64,
) -> Vec<UnifiedMessage> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut bytes = data;
    let value: Value = match simd_json::from_slice(&mut bytes) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut messages = Vec::new();
    if let Some(message) = extract_claude_headless_message(&value, session_id, fallback_timestamp) {
        messages.push(message);
    }

    messages
}

fn process_claude_headless_line(
    line: &str,
    session_id: &str,
    state: &mut ClaudeHeadlessState,
    fallback_timestamp: i64,
) -> Option<UnifiedMessage> {
    let mut bytes = line.as_bytes().to_vec();
    let value: Value = simd_json::from_slice(&mut bytes).ok()?;

    let event_type = value.get("type").and_then(|val| val.as_str()).unwrap_or("");
    let mut completed_message: Option<UnifiedMessage> = None;

    match event_type {
        "message_start" => {
            completed_message = finalize_headless_state(state, session_id, fallback_timestamp);

            state.model = extract_claude_model(&value);
            state.timestamp_ms = extract_claude_timestamp(&value).or(state.timestamp_ms);
            if let Some(usage) = value
                .get("message")
                .and_then(|msg| msg.get("usage"))
                .or_else(|| value.get("usage"))
            {
                update_claude_usage(state, usage);
            }
        }
        "message_delta" => {
            if let Some(usage) = value
                .get("usage")
                .or_else(|| value.get("delta").and_then(|delta| delta.get("usage")))
            {
                update_claude_usage(state, usage);
            }
        }
        "message_stop" => {
            completed_message = finalize_headless_state(state, session_id, fallback_timestamp);
        }
        _ => {
            if let Some(message) =
                extract_claude_headless_message(&value, session_id, fallback_timestamp)
            {
                completed_message = Some(message);
            }
        }
    }

    completed_message
}

fn extract_claude_headless_message(
    value: &Value,
    session_id: &str,
    fallback_timestamp: i64,
) -> Option<UnifiedMessage> {
    let usage = value
        .get("usage")
        .or_else(|| value.get("message").and_then(|msg| msg.get("usage")))?;
    let model = extract_claude_model(value)?;
    let timestamp = extract_claude_timestamp(value).unwrap_or(fallback_timestamp);

    Some(UnifiedMessage::new(
        "claude",
        model,
        "anthropic",
        session_id.to_string(),
        timestamp,
        TokenBreakdown {
            input: extract_i64(usage.get("input_tokens")).unwrap_or(0).max(0),
            output: extract_i64(usage.get("output_tokens")).unwrap_or(0).max(0),
            cache_read: extract_i64(usage.get("cache_read_input_tokens"))
                .unwrap_or(0)
                .max(0),
            cache_write: extract_i64(usage.get("cache_creation_input_tokens"))
                .unwrap_or(0)
                .max(0),
            reasoning: 0,
        },
        0.0,
    ))
}

fn extract_claude_model(value: &Value) -> Option<String> {
    extract_string(value.get("model")).or_else(|| {
        value
            .get("message")
            .and_then(|msg| extract_string(msg.get("model")))
    })
}

fn extract_claude_timestamp(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("message").and_then(|msg| msg.get("created_at")))
        .and_then(parse_timestamp_value)
}

fn update_claude_usage(state: &mut ClaudeHeadlessState, usage: &Value) {
    if let Some(input) = extract_i64(usage.get("input_tokens")) {
        state.input = state.input.max(input);
    }
    if let Some(output) = extract_i64(usage.get("output_tokens")) {
        state.output = state.output.max(output);
    }
    if let Some(cache_read) = extract_i64(usage.get("cache_read_input_tokens")) {
        state.cache_read = state.cache_read.max(cache_read);
    }
    if let Some(cache_write) = extract_i64(usage.get("cache_creation_input_tokens")) {
        state.cache_write = state.cache_write.max(cache_write);
    }
}

fn finalize_headless_state(
    state: &mut ClaudeHeadlessState,
    session_id: &str,
    fallback_timestamp: i64,
) -> Option<UnifiedMessage> {
    let model = state.model.clone()?;
    let timestamp = state.timestamp_ms.unwrap_or(fallback_timestamp);
    if state.input == 0 && state.output == 0 && state.cache_read == 0 && state.cache_write == 0 {
        *state = ClaudeHeadlessState::default();
        return None;
    }

    let message = UnifiedMessage::new(
        "claude",
        model,
        "anthropic",
        session_id.to_string(),
        timestamp,
        TokenBreakdown {
            input: state.input.max(0),
            output: state.output.max(0),
            cache_read: state.cache_read.max(0),
            cache_write: state.cache_write.max(0),
            reasoning: 0,
        },
        0.0,
    );

    *state = ClaudeHeadlessState::default();
    Some(message)
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
    fn test_deduplication_skips_duplicate_entries() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:02.000Z","requestId":"req_002","message":{"id":"msg_002","model":"claude-3-5-sonnet","usage":{"input_tokens":200,"output_tokens":100}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(
            messages.len(),
            2,
            "Should deduplicate to 2 messages (first duplicate skipped)"
        );
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[1].tokens.input, 200);
    }

    #[test]
    fn test_deduplication_allows_same_message_different_request() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_002","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":150,"output_tokens":75}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(
            messages.len(),
            2,
            "Different requestId should not be deduplicated"
        );
    }

    #[test]
    fn test_entries_without_dedup_fields_still_processed() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","message":{"model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","message":{"model":"claude-3-5-sonnet","usage":{"input_tokens":200,"output_tokens":100}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(
            messages.len(),
            2,
            "Entries without messageId/requestId should still be processed"
        );
    }

    #[test]
    fn test_user_messages_ignored() {
        let content = r#"{"type":"user","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"Hello"}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1, "User messages should be ignored");
        assert_eq!(messages[0].tokens.input, 100);
    }

    #[test]
    fn test_token_breakdown_parsing() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":200,"cache_creation_input_tokens":100}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[0].tokens.output, 500);
        assert_eq!(messages[0].tokens.cache_read, 200);
        assert_eq!(messages[0].tokens.cache_write, 100);
        assert_eq!(messages[0].tokens.reasoning, 0);
    }

    #[test]
    fn test_headless_json_output() {
        let content = r#"{"type":"message","message":{"model":"claude-3-5-sonnet","usage":{"input_tokens":120,"output_tokens":60,"cache_read_input_tokens":10}}}"#;
        let file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        std::fs::write(file.path(), content).unwrap();

        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-3-5-sonnet");
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].tokens.output, 60);
        assert_eq!(messages[0].tokens.cache_read, 10);
    }

    #[test]
    fn test_headless_stream_output() {
        let content = r#"{"type":"message_start","timestamp":"2025-01-01T00:00:00Z","message":{"id":"msg_1","model":"claude-3-5-sonnet","usage":{"input_tokens":200,"cache_read_input_tokens":20,"cache_creation_input_tokens":5}}}
{"type":"message_delta","usage":{"output_tokens":80}}
{"type":"message_stop"}"#;
        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-3-5-sonnet");
        assert_eq!(messages[0].tokens.input, 200);
        assert_eq!(messages[0].tokens.output, 80);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.cache_write, 5);
    }
}
