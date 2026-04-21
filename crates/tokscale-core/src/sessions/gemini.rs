//! Gemini CLI session parser
//!
//! Parses JSON session files from ~/.gemini/tmp/* supporting both legacy
//! `session-*.json` files and new UUID-named files in `chats/` directories.

use super::utils::{
    extract_i64, extract_string, file_modified_timestamp_ms, parse_timestamp_value,
    read_file_or_none,
};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Gemini session structure
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GeminiSession {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "projectHash")]
    pub project_hash: String,
    #[serde(rename = "startTime")]
    pub start_time: String,
    #[serde(rename = "lastUpdated")]
    pub last_updated: String,
    pub messages: Vec<GeminiMessage>,
}

/// Gemini message structure
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GeminiMessage {
    pub id: String,
    pub timestamp: Option<String>,
    #[serde(rename = "type")]
    pub message_type: String,
    pub tokens: Option<GeminiTokens>,
    pub model: Option<String>,
}

/// Gemini token structure
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GeminiTokens {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cached: Option<i64>,
    pub thoughts: Option<i64>,
    pub tool: Option<i64>,
    pub total: Option<i64>,
}

/// Parse a Gemini session file
pub fn parse_gemini_file(path: &Path) -> Vec<UnifiedMessage> {
    let fallback_timestamp = file_modified_timestamp_ms(path);

    if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
        return parse_gemini_headless_jsonl(path, fallback_timestamp);
    }

    // Filter to expected Gemini layouts only:
    // - Legacy: files starting with "session-"
    // - Modern: path structure .../.gemini/tmp/<some_id>/chats/<file>.json
    let file_name_os = path.file_name().unwrap_or_default();

    // Fast path: legacy files are always accepted
    if !file_name_os
        .to_str()
        .map(|s| s.starts_with("session-"))
        .unwrap_or(false)
    {
        use std::ffi::OsStr;
        // All scanned paths live under ~/.gemini/tmp (scanner root). Enforce the expected subdirectory pattern.
        let comps: Vec<&OsStr> = path.components().map(|c| c.as_os_str()).collect();
        let mut ok = false;
        // Look for ".gemini" immediately followed by "tmp" to avoid matching /tmp paths.
        'outer: for i in 0..comps.len().saturating_sub(2) {
            if comps[i] == ".gemini" && comps[i + 1] == "tmp" {
                // After "tmp", expect exactly 3 components: <some_id>, "chats", and the filename.
                let after_tmp = &comps[i + 2..];
                if after_tmp.len() == 3 {
                    let chats_dir = after_tmp[1];
                    let last = after_tmp[2];
                    if chats_dir == OsStr::new("chats") && last == file_name_os {
                        ok = true;
                        break 'outer;
                    }
                }
            }
        }
        if !ok {
            return Vec::new();
        }
    }

    let Some(data) = read_file_or_none(path) else {
        return Vec::new();
    };

    let mut bytes = data.clone();
    if let Ok(session) = simd_json::from_slice::<GeminiSession>(&mut bytes) {
        return parse_gemini_session(session, fallback_timestamp);
    }

    let mut bytes = data;
    if let Ok(value) = simd_json::from_slice::<Value>(&mut bytes) {
        let session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let messages = parse_gemini_headless_value(&value, &session_id, fallback_timestamp);
        if !messages.is_empty() {
            return messages;
        }
    }

    parse_gemini_headless_jsonl(path, fallback_timestamp)
}

fn parse_gemini_session(session: GeminiSession, fallback_timestamp: i64) -> Vec<UnifiedMessage> {
    let mut messages = Vec::with_capacity(session.messages.len());
    let session_id = session.session_id.clone();

    for msg in session.messages {
        // Only process gemini messages with token data
        if msg.message_type != "gemini" {
            continue;
        }

        let tokens = match msg.tokens {
            Some(t) => t,
            None => continue,
        };

        let model = match msg.model {
            Some(m) => m,
            None => continue,
        };

        let timestamp = msg
            .timestamp
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(fallback_timestamp);
        let (input, cache_read) = normalize_gemini_session_input_and_cache(
            tokens.input.unwrap_or(0),
            tokens.cached.unwrap_or(0),
            tokens.output.unwrap_or(0),
            tokens.thoughts.unwrap_or(0),
            tokens.tool.unwrap_or(0),
            tokens.total,
        );

        messages.push(UnifiedMessage::new(
            "gemini",
            model,
            "google",
            session_id.clone(),
            timestamp,
            TokenBreakdown {
                input,
                output: tokens.output.unwrap_or(0).max(0),
                cache_read,
                cache_write: 0,
                reasoning: tokens.thoughts.unwrap_or(0).max(0),
            },
            0.0,
        ));
    }

    messages
}

fn parse_gemini_headless_jsonl(path: &Path, fallback_timestamp: i64) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let mut session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut current_model: Option<String> = None;
    let reader = BufReader::new(file);
    let mut messages = Vec::with_capacity(64);
    let mut buffer = Vec::with_capacity(4096);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        buffer.clear();
        buffer.extend_from_slice(trimmed.as_bytes());
        let value: Value = match simd_json::from_slice(&mut buffer) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let event_type = value.get("type").and_then(|val| val.as_str()).unwrap_or("");
        if event_type == "init" {
            if let Some(model) = extract_string(value.get("model")) {
                current_model = Some(model);
            }
            if let Some(id) =
                extract_string(value.get("session_id").or_else(|| value.get("sessionId")))
            {
                session_id = id;
            }
            continue;
        }

        let stats = value
            .get("stats")
            .or_else(|| value.get("result").and_then(|result| result.get("stats")));
        if let Some(stats) = stats {
            let timestamp = extract_timestamp_from_value(&value).unwrap_or(fallback_timestamp);
            messages.extend(build_messages_from_stats(
                stats,
                current_model.clone(),
                &session_id,
                timestamp,
            ));
        }
    }

    messages
}

fn parse_gemini_headless_value(
    value: &Value,
    session_id: &str,
    fallback_timestamp: i64,
) -> Vec<UnifiedMessage> {
    let stats = match value
        .get("stats")
        .or_else(|| value.get("result").and_then(|result| result.get("stats")))
    {
        Some(s) => s,
        None => return Vec::new(),
    };

    let model_hint = extract_string(value.get("model"));
    let timestamp = extract_timestamp_from_value(value).unwrap_or(fallback_timestamp);

    build_messages_from_stats(stats, model_hint, session_id, timestamp)
}

fn build_messages_from_stats(
    stats: &Value,
    model_hint: Option<String>,
    session_id: &str,
    timestamp: i64,
) -> Vec<UnifiedMessage> {
    let usages = extract_gemini_usages(stats, model_hint);
    usages
        .into_iter()
        .map(|usage| {
            let (input, cache_read) =
                normalize_gemini_headless_input_and_cache(usage.input, usage.cached);
            UnifiedMessage::new(
                "gemini",
                usage.model,
                "google",
                session_id.to_string(),
                timestamp,
                TokenBreakdown {
                    input,
                    output: usage.output.max(0),
                    cache_read,
                    cache_write: 0,
                    reasoning: usage.reasoning.max(0),
                },
                0.0,
            )
        })
        .collect()
}

fn subtract_cached_overlap(input: i64, cached: i64) -> (i64, i64) {
    let input = input.max(0);
    let cached = cached.max(0);
    let cached_portion = cached.min(input);
    (input.saturating_sub(cached_portion), cached)
}

fn normalize_gemini_headless_input_and_cache(input: i64, cached: i64) -> (i64, i64) {
    // Gemini usage_metadata promptTokenCount is cache-inclusive, while Tokscale
    // represents non-cached input and cache hits as separate buckets.
    subtract_cached_overlap(input, cached)
}

fn normalize_gemini_session_input_and_cache(
    input: i64,
    cached: i64,
    output: i64,
    reasoning: i64,
    tool: i64,
    total: Option<i64>,
) -> (i64, i64) {
    let input = input.max(0);
    let cached = cached.max(0);

    let Some(total) = total.map(|value| value.max(0)) else {
        return (input, cached);
    };

    let inclusive_total = input
        .saturating_add(output.max(0))
        .saturating_add(reasoning.max(0))
        .saturating_add(tool.max(0));
    let exclusive_total = inclusive_total.saturating_add(cached);

    if cached > 0 && total == inclusive_total && total != exclusive_total {
        return subtract_cached_overlap(input, cached);
    }

    (input, cached)
}

struct GeminiHeadlessUsage {
    model: String,
    input: i64,
    output: i64,
    cached: i64,
    reasoning: i64,
}

fn extract_gemini_usages(stats: &Value, model_hint: Option<String>) -> Vec<GeminiHeadlessUsage> {
    if let Some(models) = stats.get("models").and_then(|val| val.as_object()) {
        let mut usages = Vec::new();
        for (model, data) in models {
            let tokens = match data.get("tokens") {
                Some(t) => t,
                None => continue,
            };
            let input = extract_i64(tokens.get("prompt"))
                .or_else(|| extract_i64(tokens.get("input")))
                .or_else(|| extract_i64(tokens.get("input_tokens")))
                .unwrap_or(0);
            let output = extract_i64(tokens.get("candidates"))
                .or_else(|| extract_i64(tokens.get("output")))
                .or_else(|| extract_i64(tokens.get("output_tokens")))
                .unwrap_or(0);
            let cached = extract_i64(tokens.get("cached"))
                .or_else(|| extract_i64(tokens.get("cached_tokens")))
                .unwrap_or(0);
            let reasoning = extract_i64(tokens.get("thoughts"))
                .or_else(|| extract_i64(tokens.get("reasoning")))
                .unwrap_or(0);

            if input == 0 && output == 0 && cached == 0 && reasoning == 0 {
                continue;
            }

            usages.push(GeminiHeadlessUsage {
                model: model.clone(),
                input,
                output,
                cached,
                reasoning,
            });
        }

        if !usages.is_empty() {
            return usages;
        }
    }

    let input = extract_i64(stats.get("input_tokens"))
        .or_else(|| extract_i64(stats.get("prompt_tokens")))
        .unwrap_or(0);
    let output = extract_i64(stats.get("output_tokens"))
        .or_else(|| extract_i64(stats.get("candidates_tokens")))
        .unwrap_or(0);
    let cached = extract_i64(stats.get("cached_tokens")).unwrap_or(0);
    let reasoning = extract_i64(stats.get("thoughts_tokens"))
        .or_else(|| extract_i64(stats.get("reasoning_tokens")))
        .unwrap_or(0);

    if input == 0 && output == 0 && cached == 0 && reasoning == 0 {
        return Vec::new();
    }

    vec![GeminiHeadlessUsage {
        model: model_hint.unwrap_or_else(|| "unknown".to_string()),
        input,
        output,
        cached,
        reasoning,
    }]
}

fn extract_timestamp_from_value(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .or_else(|| value.get("created_at"))
        .and_then(parse_timestamp_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_parse_gemini_structure() {
        let json = r#"{
            "sessionId": "ses_123",
            "projectHash": "abc123",
            "startTime": "2025-06-15T12:00:00Z",
            "lastUpdated": "2025-06-15T12:30:00Z",
            "messages": [
                {
                    "id": "msg_1",
                    "timestamp": "2025-06-15T12:00:00Z",
                    "type": "user"
                },
                {
                    "id": "msg_2",
                    "timestamp": "2025-06-15T12:01:00Z",
                    "type": "gemini",
                    "model": "gemini-2.0-flash",
                    "tokens": {
                        "input": 10,
                        "output": 20,
                        "cached": 5,
                        "thoughts": 0,
                        "tool": 0,
                        "total": 35
                    }
                }
            ]
        }"#;

        let mut bytes = json.as_bytes().to_vec();
        let session: GeminiSession = simd_json::from_slice(&mut bytes).unwrap();

        assert_eq!(session.messages.len(), 2);
        assert_eq!(
            session.messages[1].model,
            Some("gemini-2.0-flash".to_string())
        );
    }

    #[test]
    fn test_parse_gemini_with_array_content() {
        let json = r#"{
            "sessionId": "ses_123",
            "projectHash": "abc123",
            "startTime": "2025-06-15T12:00:00Z",
            "lastUpdated": "2025-06-15T12:30:00Z",
            "messages": [
                {
                    "id": "msg_1",
                    "timestamp": "2025-06-15T12:00:00Z",
                    "type": "user",
                    "content": [{"text": "Hello"}]
                },
                {
                    "id": "msg_2",
                    "timestamp": "2025-06-15T12:01:00Z",
                    "type": "gemini",
                    "content": "Hi there!",
                    "model": "gemini-2.0-flash",
                    "tokens": {
                        "input": 10,
                        "output": 20
                    }
                }
            ]
        }"#;

        // Create a path that matches the legacy prefix so it passes the 'is_in_chats' filter
        let file = tempfile::Builder::new()
            .prefix("session-")
            .suffix(".json")
            .tempfile()
            .unwrap();
        std::fs::write(file.path(), json).unwrap();

        let messages = parse_gemini_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-2.0-flash");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 20);
    }

    #[test]
    fn test_parse_gemini_session_normalizes_cached_input() {
        let json = r#"{
            "sessionId": "ses_123",
            "projectHash": "abc123",
            "startTime": "2025-06-15T12:00:00Z",
            "lastUpdated": "2025-06-15T12:30:00Z",
            "messages": [
                {
                    "id": "msg_2",
                    "timestamp": "2025-06-15T12:01:00Z",
                    "type": "gemini",
                    "model": "gemini-2.0-flash",
                    "tokens": {
                        "input": 15,
                        "output": 20,
                        "cached": 5,
                        "thoughts": 2,
                        "total": 37
                    }
                }
            ]
        }"#;

        let file = tempfile::Builder::new()
            .prefix("session-")
            .suffix(".json")
            .tempfile()
            .unwrap();
        std::fs::write(file.path(), json).unwrap();

        let messages = parse_gemini_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 20);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].tokens.reasoning, 2);
        assert_eq!(messages[0].tokens.total(), 37);
    }

    #[test]
    fn test_parse_gemini_session_preserves_already_net_input_when_total_matches() {
        let json = r#"{
            "sessionId": "ses_123",
            "projectHash": "abc123",
            "startTime": "2025-06-15T12:00:00Z",
            "lastUpdated": "2025-06-15T12:30:00Z",
            "messages": [
                {
                    "id": "msg_2",
                    "timestamp": "2025-06-15T12:01:00Z",
                    "type": "gemini",
                    "model": "gemini-2.0-flash",
                    "tokens": {
                        "input": 10,
                        "output": 20,
                        "cached": 5,
                        "thoughts": 2,
                        "total": 37
                    }
                }
            ]
        }"#;

        let file = tempfile::Builder::new()
            .prefix("session-")
            .suffix(".json")
            .tempfile()
            .unwrap();
        std::fs::write(file.path(), json).unwrap();

        let messages = parse_gemini_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 20);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].tokens.reasoning, 2);
        assert_eq!(messages[0].tokens.total(), 37);
    }

    #[test]
    fn test_parse_headless_json() {
        let json = r#"{"response":"Hi","stats":{"models":{"gemini-2.5-pro":{"tokens":{"prompt":12,"candidates":34,"cached":5,"thoughts":2}}}}}"#;
        // Use a legacy prefix to satisfy the path check
        let file = tempfile::Builder::new()
            .prefix("session-")
            .suffix(".json")
            .tempfile()
            .unwrap();
        std::fs::write(file.path(), json).unwrap();

        let messages = parse_gemini_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-2.5-pro");
        assert_eq!(messages[0].tokens.input, 7);
        assert_eq!(messages[0].tokens.output, 34);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].tokens.reasoning, 2);
        assert_eq!(messages[0].tokens.total(), 48);
    }

    #[test]
    fn test_parse_headless_stream_jsonl() {
        let content = r#"{"type":"init","model":"gemini-2.5-pro","session_id":"session-1"}
{"type":"result","stats":{"input_tokens":10,"output_tokens":20}}"#;
        let mut file = tempfile::Builder::new()
            .suffix(".jsonl")
            .tempfile()
            .unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();

        let messages = parse_gemini_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-2.5-pro");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 20);
    }

    #[test]
    fn test_parse_headless_stream_jsonl_normalizes_cached_input() {
        let content = r#"{"type":"init","model":"gemini-2.5-pro","session_id":"session-1"}
{"type":"result","stats":{"input_tokens":12,"output_tokens":20,"cached_tokens":5,"thoughts_tokens":3}}"#;
        let mut file = tempfile::Builder::new()
            .suffix(".jsonl")
            .tempfile()
            .unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();

        let messages = parse_gemini_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-2.5-pro");
        assert_eq!(messages[0].tokens.input, 7);
        assert_eq!(messages[0].tokens.output, 20);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].tokens.reasoning, 3);
        assert_eq!(messages[0].tokens.total(), 35);
    }

    #[test]
    fn test_parse_headless_json_clamps_cached_input_overlap() {
        let json = r#"{"response":"Hi","stats":{"models":{"gemini-2.5-pro":{"tokens":{"prompt":5,"candidates":2,"cached":10}}}}}"#;
        let file = tempfile::Builder::new()
            .prefix("session-")
            .suffix(".json")
            .tempfile()
            .unwrap();
        std::fs::write(file.path(), json).unwrap();

        let messages = parse_gemini_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 0);
        assert_eq!(messages[0].tokens.output, 2);
        assert_eq!(messages[0].tokens.cache_read, 10);
        assert_eq!(messages[0].tokens.total(), 12);
    }

    #[test]
    fn test_parse_gemini_valid_uuid_path() {
        let json = r#"{
            "sessionId": "ses_123",
            "projectHash": "abc123",
            "startTime": "2025-06-15T12:00:00Z",
            "lastUpdated": "2025-06-15T12:30:00Z",
            "messages": [
                {
                    "id": "msg_2",
                    "timestamp": "2025-06-15T12:01:00Z",
                    "type": "gemini",
                    "model": "gemini-2.0-flash",
                    "tokens": {
                        "input": 10,
                        "output": 20
                    }
                }
            ]
        }"#;

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let chats_dir = base.join(".gemini/tmp/abc123/chats");
        std::fs::create_dir_all(&chats_dir).unwrap();
        let file_path = chats_dir.join("uuid-file.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_gemini_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-2.0-flash");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 20);
    }

    #[test]
    fn test_parse_gemini_reject_nested_chats() {
        let json = r#"{
            "sessionId": "ses_123",
            "projectHash": "abc123",
            "startTime": "2025-06-15T12:00:00Z",
            "lastUpdated": "2025-06-15T12:30:00Z",
            "messages": [
                {
                    "id": "msg_2",
                    "timestamp": "2025-06-15T12:01:00Z",
                    "type": "gemini",
                    "content": [{"text": "test"}],
                    "model": "gemini-2.0-flash",
                    "tokens": {
                        "input": 10,
                        "output": 20
                    }
                }
            ]
        }"#;

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let nested_dir = base.join(".gemini/tmp/abc123/backup/chats");
        std::fs::create_dir_all(&nested_dir).unwrap();
        let file_path = nested_dir.join("nested.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_gemini_file(&file_path);

        assert_eq!(messages.len(), 0);
    }
}
