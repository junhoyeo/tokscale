//! Gemini CLI session parser
//!
//! Parses JSON and JSONL session files from ~/.gemini/tmp/* supporting legacy
//! `session-*.json` files, UUID-named files in `chats/`, and current
//! `session-*.jsonl` chat recordings.

use super::utils::{
    extract_i64, extract_string, file_modified_timestamp_ms, parse_timestamp_value,
    read_file_or_none,
};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
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
    pub tokens: Option<Value>,
    pub model: Option<String>,
}

fn first_i64(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|k| value.get(k).and_then(|v| v.as_i64()))
}

fn deserialize_tokens(value: &Value) -> Option<GeminiTokens> {
    Some(GeminiTokens {
        input: first_i64(
            value,
            &[
                "input",
                "prompt",
                "input_tokens",
                "prompt_tokens",
                "promptTokenCount",
            ],
        ),
        output: first_i64(
            value,
            &[
                "output",
                "candidates",
                "output_tokens",
                "completion_tokens",
                "candidatesTokenCount",
            ],
        ),
        cached: first_i64(
            value,
            &["cached", "cached_tokens", "cachedContentTokenCount"],
        ),
        thoughts: first_i64(value, &["thoughts", "reasoning", "thoughts_tokens"]),
        tool: first_i64(value, &["tool", "tool_tokens"]),
        total: first_i64(value, &["total", "totalTokenCount", "total_tokens"]),
    })
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct GeminiTokens {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cached: Option<i64>,
    pub thoughts: Option<i64>,
    pub tool: Option<i64>,
    pub total: Option<i64>,
}

pub(crate) struct GeminiParseResult {
    pub messages: Vec<UnifiedMessage>,
    pub cacheable: bool,
}

/// Parse a Gemini session file.
pub fn parse_gemini_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_gemini_file_with_cache_status(path).messages
}

pub(crate) fn parse_gemini_file_with_cache_status(path: &Path) -> GeminiParseResult {
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
        // Enforce the expected subdirectory pattern: tmp/<some_id>/chats/<file>
        let comps: Vec<&OsStr> = path.components().map(|c| c.as_os_str()).collect();
        let mut ok = false;
        'outer: for i in 0..comps.len().saturating_sub(1) {
            if comps[i] == "tmp" {
                // After "tmp", expect exactly 3 components: <some_id>, "chats", and the filename.
                let after_tmp = &comps[i + 1..];
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
            return GeminiParseResult {
                messages: Vec::new(),
                cacheable: true,
            };
        }
    }

    let Some(data) = read_file_or_none(path) else {
        return GeminiParseResult {
            messages: Vec::new(),
            cacheable: true,
        };
    };

    let mut bytes = data.clone();
    if let Ok(session) = simd_json::from_slice::<GeminiSession>(&mut bytes) {
        return GeminiParseResult {
            messages: parse_gemini_session(session, fallback_timestamp),
            cacheable: true,
        };
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
            return GeminiParseResult {
                messages,
                cacheable: true,
            };
        }
    }

    parse_gemini_headless_jsonl(path, fallback_timestamp)
}

fn parse_gemini_session(session: GeminiSession, fallback_timestamp: i64) -> Vec<UnifiedMessage> {
    let mut messages = Vec::with_capacity(session.messages.len());
    let session_id = session.session_id.clone();

    for msg in session.messages {
        // Only process messages with token data
        let tokens = match msg.tokens.as_ref().and_then(deserialize_tokens) {
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
        messages.push(build_gemini_token_message(
            model,
            &session_id,
            timestamp,
            tokens,
        ));
    }

    messages
}

fn build_gemini_token_message(
    model: String,
    session_id: &str,
    timestamp: i64,
    tokens: GeminiTokens,
) -> UnifiedMessage {
    let (input, cache_read) = normalize_gemini_session_input_and_cache(
        tokens.input.unwrap_or(0),
        tokens.cached.unwrap_or(0),
        tokens.output.unwrap_or(0),
        tokens.thoughts.unwrap_or(0),
        tokens.tool.unwrap_or(0),
        tokens.total,
    );

    let tool = tokens.tool.unwrap_or(0).max(0);

    UnifiedMessage::new(
        "gemini",
        model,
        "google",
        session_id.to_string(),
        timestamp,
        TokenBreakdown {
            input: input.saturating_add(tool),
            output: tokens.output.unwrap_or(0).max(0),
            cache_read,
            cache_write: 0,
            reasoning: tokens.thoughts.unwrap_or(0).max(0),
        },
        0.0,
    )
}

fn parse_direct_gemini_token_message(
    value: &Value,
    model_hint: Option<String>,
    session_id: &str,
    fallback_timestamp: i64,
) -> Option<UnifiedMessage> {
    let model = extract_string(value.get("model")).or(model_hint)?;
    let tokens_value = value.get("tokens")?;
    let tokens = deserialize_tokens(tokens_value)?;
    let timestamp = extract_timestamp_from_value(value).unwrap_or(fallback_timestamp);

    Some(build_gemini_token_message(
        model, session_id, timestamp, tokens,
    ))
}

fn parse_gemini_headless_jsonl(path: &Path, fallback_timestamp: i64) -> GeminiParseResult {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => {
            return GeminiParseResult {
                messages: Vec::new(),
                cacheable: true,
            };
        }
    };

    let mut session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut current_model: Option<String> = None;
    let mut reader = BufReader::new(file);
    let mut messages = Vec::with_capacity(64);
    let mut direct_message_indices: HashMap<String, usize> = HashMap::new();
    let mut line_buffer = Vec::with_capacity(4096);
    let mut json_buffer = Vec::with_capacity(4096);
    let mut skipped_malformed_line = false;

    loop {
        line_buffer.clear();
        let bytes_read = match reader.read_until(b'\n', &mut line_buffer) {
            Ok(n) => n,
            Err(_) => {
                skipped_malformed_line = true;
                break;
            }
        };
        if bytes_read == 0 {
            break;
        }

        let trimmed = trim_ascii_bytes(&line_buffer);
        if trimmed.is_empty() {
            continue;
        }

        json_buffer.clear();
        json_buffer.extend_from_slice(trimmed);
        let value: Value = match simd_json::from_slice(&mut json_buffer) {
            Ok(v) => v,
            Err(_) => {
                skipped_malformed_line = true;
                continue;
            }
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

        if let Some(id) = extract_string(value.get("session_id").or_else(|| value.get("sessionId")))
        {
            session_id = id;
        }

        if event_type == "gemini" || value.get("tokens").is_some() {
            if let Some(model) = extract_string(value.get("model")) {
                current_model = Some(model);
            }

            if let Some(message) = parse_direct_gemini_token_message(
                &value,
                current_model.clone(),
                &session_id,
                fallback_timestamp,
            ) {
                if let Some(id) = extract_string(value.get("id")) {
                    if let Some(index) = direct_message_indices.get(&id).copied() {
                        messages[index] = message;
                    } else {
                        direct_message_indices.insert(id, messages.len());
                        messages.push(message);
                    }
                } else {
                    messages.push(message);
                }
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

    GeminiParseResult {
        messages,
        cacheable: !skipped_malformed_line,
    }
}

fn trim_ascii_bytes(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace());
    let Some(start) = start else {
        return &[];
    };

    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|idx| idx + 1)
        .unwrap_or(start);

    &bytes[start..end]
}

fn parse_gemini_headless_value(
    value: &Value,
    session_id: &str,
    fallback_timestamp: i64,
) -> Vec<UnifiedMessage> {
    if value.get("type").and_then(|val| val.as_str()) == Some("gemini")
        || value.get("tokens").is_some()
    {
        if let Some(message) =
            parse_direct_gemini_token_message(value, None, session_id, fallback_timestamp)
        {
            return vec![message];
        }
    }

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
            let (input, cache_read) = if usage.input_includes_cache {
                normalize_gemini_headless_input_and_cache(usage.input, usage.cached)
            } else {
                (usage.input.max(0), usage.cached.max(0))
            };
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
    input_includes_cache: bool,
}

fn extract_gemini_usages(stats: &Value, model_hint: Option<String>) -> Vec<GeminiHeadlessUsage> {
    if let Some(models) = stats.get("models").and_then(|val| val.as_object()) {
        let mut usages = Vec::new();
        for (model, data) in models {
            if let Some(usage) = extract_gemini_usage_from_value(model.clone(), data) {
                usages.push(usage);
            }
        }

        if !usages.is_empty() {
            return usages;
        }
    }

    extract_gemini_usage_from_value(model_hint.unwrap_or_else(|| "unknown".to_string()), stats)
        .into_iter()
        .collect()
}

fn extract_gemini_usage_from_value(model: String, value: &Value) -> Option<GeminiHeadlessUsage> {
    let has_tokens_wrapper = value.get("tokens").is_some();
    let tokens = value.get("tokens").unwrap_or(value);
    let prompt_input = extract_i64(tokens.get("prompt"))
        .or_else(|| extract_i64(tokens.get("input_tokens")))
        .or_else(|| extract_i64(tokens.get("prompt_tokens")));
    let net_input = extract_i64(tokens.get("input"));
    let wrapper_input = if has_tokens_wrapper { net_input } else { None };
    let input = prompt_input.or(wrapper_input).or(net_input).unwrap_or(0);
    let output = extract_i64(tokens.get("candidates"))
        .or_else(|| extract_i64(tokens.get("output")))
        .or_else(|| extract_i64(tokens.get("output_tokens")))
        .or_else(|| extract_i64(tokens.get("candidates_tokens")))
        .unwrap_or(0);
    let cached = extract_i64(tokens.get("cached"))
        .or_else(|| extract_i64(tokens.get("cached_tokens")))
        .unwrap_or(0);
    let reasoning = extract_i64(tokens.get("thoughts"))
        .or_else(|| extract_i64(tokens.get("thoughts_tokens")))
        .or_else(|| extract_i64(tokens.get("reasoning")))
        .or_else(|| extract_i64(tokens.get("reasoning_tokens")))
        .unwrap_or(0);

    if input == 0 && output == 0 && cached == 0 && reasoning == 0 {
        return None;
    }

    Some(GeminiHeadlessUsage {
        model,
        input,
        output,
        cached,
        reasoning,
        input_includes_cache: prompt_input.is_some()
            || wrapper_input.is_some()
            || net_input.is_none(),
    })
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
    fn test_parse_gemini_stream_jsonl_v0391_model_stats_without_tokens_wrapper() {
        let content = r#"{"type":"init","model":"gemini-2.5-pro","session_id":"session-1"}
{"type":"result","stats":{"total_tokens":32,"input_tokens":12,"output_tokens":20,"cached":5,"input":7,"models":{"gemini-2.5-pro":{"total_tokens":32,"input_tokens":12,"output_tokens":20,"cached":5,"input":7}}}}"#;
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
        assert_eq!(messages[0].tokens.total(), 32);
    }

    #[test]
    fn test_parse_gemini_stream_jsonl_v0391_flat_stats_uses_net_input_alias() {
        let content = r#"{"type":"init","model":"gemini-2.5-pro","session_id":"session-1"}
{"type":"result","stats":{"total_tokens":32,"output_tokens":20,"cached":5,"input":7}}"#;
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
        assert_eq!(messages[0].tokens.total(), 32);
    }

    #[test]
    fn test_parse_headless_stats_tokens_wrapper_preserves_cache_inclusive_input() {
        let json = r#"{"stats":{"models":{"gemini-2.5-pro":{"tokens":{"input":12,"output":20,"cached":5}}}}}"#;
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
        assert_eq!(messages[0].tokens.output, 20);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].tokens.total(), 32);
    }

    #[test]
    fn test_parse_gemini_stream_jsonl_direct_tokens() {
        let content = r#"{"sessionId":"gemini-session-1","projectHash":"abc123","startTime":"2026-05-01T00:00:00.000Z","lastUpdated":"2026-05-01T00:01:00.000Z"}
{"id":"msg-1","timestamp":"2026-05-01T00:01:00.000Z","type":"gemini","model":"gemini-3.1-pro-preview","tokens":{"input":14918,"output":60,"cached":0,"thoughts":863,"tool":7,"total":15848}}"#;
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join(".gemini/tmp/123/chats");
        std::fs::create_dir_all(&chats_dir).unwrap();
        let file_path = chats_dir.join("session-abc.jsonl");
        std::fs::write(&file_path, content).unwrap();

        let messages = parse_gemini_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "gemini-session-1");
        assert_eq!(messages[0].model_id, "gemini-3.1-pro-preview");
        assert_eq!(messages[0].provider_id, "google");
        assert_eq!(messages[0].tokens.input, 14925);
        assert_eq!(messages[0].tokens.output, 60);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.reasoning, 863);
        assert_eq!(messages[0].tokens.total(), 15848);
    }

    #[test]
    fn test_parse_gemini_stream_jsonl_replaces_duplicate_message_id() {
        let content = r#"{"type":"gemini","id":"msg-1","model":"gemini-3.1-pro-preview","tokens":{"input":10,"output":1,"cached":0,"thoughts":0,"tool":0,"total":11}}
{"type":"gemini","id":"msg-1","model":"gemini-3.1-pro-preview","tokens":{"input":20,"output":2,"cached":5,"thoughts":3,"tool":0,"total":25}}"#;
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join(".gemini/tmp/123/chats");
        std::fs::create_dir_all(&chats_dir).unwrap();
        let file_path = chats_dir.join("session-abc.jsonl");
        std::fs::write(&file_path, content).unwrap();

        let messages = parse_gemini_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-3.1-pro-preview");
        assert_eq!(messages[0].tokens.input, 15);
        assert_eq!(messages[0].tokens.output, 2);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].tokens.reasoning, 3);
        assert_eq!(messages[0].tokens.total(), 25);
    }

    #[test]
    fn test_parse_gemini_stream_jsonl_empty_file_returns_no_messages() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join(".gemini/tmp/123/chats");
        std::fs::create_dir_all(&chats_dir).unwrap();
        let file_path = chats_dir.join("empty.jsonl");
        std::fs::write(&file_path, b"").unwrap();

        let messages = parse_gemini_file(&file_path);

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_gemini_stream_jsonl_skips_corrupt_lines() {
        let content =
            b"{\"type\":\"init\",\"model\":\"gemini-2.5-pro\",\"session_id\":\"session-1\"}\n\
not-json\n\
{\"type\":\"result\",\"stats\":{\"input_tokens\":10,\"output_tokens\":20}}\n";
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join(".gemini/tmp/123/chats");
        std::fs::create_dir_all(&chats_dir).unwrap();
        let file_path = chats_dir.join("corrupt.jsonl");
        std::fs::write(&file_path, content).unwrap();

        let result = parse_gemini_file_with_cache_status(&file_path);

        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].session_id, "session-1");
        assert_eq!(result.messages[0].model_id, "gemini-2.5-pro");
        assert_eq!(result.messages[0].tokens.input, 10);
        assert_eq!(result.messages[0].tokens.output, 20);
        assert!(!result.cacheable);
    }

    #[test]
    fn test_parse_gemini_stream_jsonl_skips_truncated_final_line() {
        let content =
            b"{\"type\":\"init\",\"model\":\"gemini-2.5-pro\",\"session_id\":\"session-1\"}\n\
{\"type\":\"result\",\"stats\":{\"input_tokens\":10,\"output_tokens\":20}}\n\
{\"type\":\"result\",\"stats\":{\"input_tokens\":99";
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join(".gemini/tmp/123/chats");
        std::fs::create_dir_all(&chats_dir).unwrap();
        let file_path = chats_dir.join("truncated.jsonl");
        std::fs::write(&file_path, content).unwrap();

        let result = parse_gemini_file_with_cache_status(&file_path);

        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].model_id, "gemini-2.5-pro");
        assert_eq!(result.messages[0].tokens.input, 10);
        assert_eq!(result.messages[0].tokens.output, 20);
        assert!(!result.cacheable);
    }

    #[test]
    fn test_parse_gemini_stream_jsonl_mixed_valid_invalid_lines_preserves_duplicate_replacement() {
        let content = b"{\"type\":\"init\",\"model\":\"gemini-3.1-pro-preview\",\"session_id\":\"session-1\"}\n\
{\"type\":\"gemini\",\"id\":\"msg-1\",\"model\":\"gemini-3.1-pro-preview\",\"tokens\":{\"input\":10,\"output\":1,\"cached\":0,\"thoughts\":0,\"tool\":0,\"total\":11}}\n\
\xff\n\
{\"type\":\"gemini\",\"id\":\"msg-1\",\"model\":\"gemini-3.1-pro-preview\",\"tokens\":{\"input\":20,\"output\":2,\"cached\":5,\"thoughts\":3,\"tool\":0,\"total\":25}}\n\
{\"type\":\"result\",\"stats\":{\"input_tokens\":7,\"output_tokens\":8}}\n";
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join(".gemini/tmp/123/chats");
        std::fs::create_dir_all(&chats_dir).unwrap();
        let file_path = chats_dir.join("mixed.jsonl");
        std::fs::write(&file_path, content).unwrap();

        let messages = parse_gemini_file(&file_path);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].session_id, "session-1");
        assert_eq!(messages[0].model_id, "gemini-3.1-pro-preview");
        assert_eq!(messages[0].tokens.input, 15);
        assert_eq!(messages[0].tokens.output, 2);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].tokens.reasoning, 3);
        assert_eq!(messages[1].tokens.input, 7);
        assert_eq!(messages[1].tokens.output, 8);
    }

    #[test]
    fn test_parse_gemini_stream_jsonl_unreadable_file_returns_no_messages() {
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join(".gemini/tmp/123/chats");
        std::fs::create_dir_all(&chats_dir).unwrap();
        let file_path = chats_dir.join("missing.jsonl");

        let messages = parse_gemini_file(&file_path);

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_gemini_json_direct_tokens() {
        let json = r#"{"type":"gemini","model":"gemini-3.1-pro-preview","tokens":{"input":20,"output":2,"cached":5,"thoughts":3,"tool":4,"total":29}}"#;
        let file = tempfile::Builder::new()
            .prefix("session-")
            .suffix(".json")
            .tempfile()
            .unwrap();
        std::fs::write(file.path(), json).unwrap();

        let messages = parse_gemini_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-3.1-pro-preview");
        assert_eq!(messages[0].tokens.input, 19);
        assert_eq!(messages[0].tokens.output, 2);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].tokens.reasoning, 3);
        assert_eq!(messages[0].tokens.total(), 29);
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

    #[test]
    fn test_parse_gemini_tokens_with_camel_case_aliases() {
        let json = r#"{
            "sessionId": "ses_alias",
            "projectHash": "abc123",
            "startTime": "2025-06-15T12:00:00Z",
            "lastUpdated": "2025-06-15T12:30:00Z",
            "messages": [
                {
                    "id": "msg_1",
                    "timestamp": "2025-06-15T12:01:00Z",
                    "type": "gemini",
                    "model": "gemini-3-flash-preview",
                    "tokens": {
                        "promptTokenCount": 100,
                        "candidatesTokenCount": 50,
                        "cachedContentTokenCount": 20,
                        "totalTokenCount": 150
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
        assert_eq!(messages[0].model_id, "gemini-3-flash-preview");
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.total(), 150);
    }

    #[test]
    fn test_parse_gemini_tokens_with_snake_case_aliases() {
        let json = r#"{
            "sessionId": "ses_snake",
            "projectHash": "abc123",
            "startTime": "2025-06-15T12:00:00Z",
            "lastUpdated": "2025-06-15T12:30:00Z",
            "messages": [
                {
                    "id": "msg_1",
                    "timestamp": "2025-06-15T12:01:00Z",
                    "type": "gemini",
                    "model": "gemini-3-flash-preview",
                    "tokens": {
                        "prompt": 200,
                        "candidates": 80,
                        "cached_tokens": 30,
                        "reasoning": 10,
                        "tool_tokens": 5,
                        "total_tokens": 295
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
        assert_eq!(messages[0].tokens.input, 175);
        assert_eq!(messages[0].tokens.output, 80);
        assert_eq!(messages[0].tokens.cache_read, 30);
        assert_eq!(messages[0].tokens.reasoning, 10);
        assert_eq!(messages[0].tokens.total(), 295);
    }

    #[test]
    fn test_parse_gemini_session_non_gemini_type_with_tokens() {
        let json = r#"{
            "sessionId": "ses_nongemini",
            "projectHash": "abc123",
            "startTime": "2025-06-15T12:00:00Z",
            "lastUpdated": "2025-06-15T12:30:00Z",
            "messages": [
                {
                    "id": "msg_1",
                    "timestamp": "2025-06-15T12:01:00Z",
                    "type": "assistant",
                    "model": "gemini-3-flash-preview",
                    "tokens": {
                        "input": 150,
                        "output": 40,
                        "cached": 10,
                        "total": 190
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
        assert_eq!(messages[0].model_id, "gemini-3-flash-preview");
        assert_eq!(messages[0].tokens.input, 140);
        assert_eq!(messages[0].tokens.output, 40);
        assert_eq!(messages[0].tokens.cache_read, 10);
        assert_eq!(messages[0].tokens.total(), 190);
    }

    #[test]
    fn test_parse_gemini_valid_path_without_gemini_component() {
        let json = r#"{
            "sessionId": "ses_custom",
            "projectHash": "abc123",
            "startTime": "2025-06-15T12:00:00Z",
            "lastUpdated": "2025-06-15T12:30:00Z",
            "messages": [
                {
                    "id": "msg_1",
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
        let chats_dir = base.join("custom_home/tmp/abc123/chats");
        std::fs::create_dir_all(&chats_dir).unwrap();
        let file_path = chats_dir.join("session.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_gemini_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-2.0-flash");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 20);
    }

    #[test]
    fn test_parse_gemini_stream_jsonl_direct_tokens_without_gemini_prefix() {
        let content = r#"{"sessionId":"ses-nogem","projectHash":"abc123","startTime":"2026-05-01T00:00:00.000Z","lastUpdated":"2026-05-01T00:01:00.000Z"}
{"id":"msg-1","timestamp":"2026-05-01T00:01:00.000Z","type":"gemini","model":"gemini-3.1-pro-preview","tokens":{"input":500,"output":30,"cached":0,"thoughts":100,"tool":5,"total":635}}"#;
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("my_gemini/tmp/456/chats");
        std::fs::create_dir_all(&chats_dir).unwrap();
        let file_path = chats_dir.join("session.jsonl");
        std::fs::write(&file_path, content).unwrap();

        let messages = parse_gemini_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "ses-nogem");
        assert_eq!(messages[0].model_id, "gemini-3.1-pro-preview");
        assert_eq!(messages[0].tokens.input, 505);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.reasoning, 100);
        assert_eq!(messages[0].tokens.total(), 635);
    }

    #[test]
    fn test_parse_headless_jsonl_non_gemini_type_with_direct_tokens() {
        let content = r#"{"type":"init","model":"gemini-3-flash-preview","session_id":"session-tokens"}
{"type":"result","id":"msg-1","tokens":{"input":100,"output":25,"cached":10,"total":125}}"#;
        let dir = TempDir::new().unwrap();
        let chats_dir = dir.path().join("custom_root/tmp/789/chats");
        std::fs::create_dir_all(&chats_dir).unwrap();
        let file_path = chats_dir.join("session.jsonl");
        std::fs::write(&file_path, content).unwrap();

        let messages = parse_gemini_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-3-flash-preview");
        assert_eq!(messages[0].tokens.input, 90);
        assert_eq!(messages[0].tokens.output, 25);
        assert_eq!(messages[0].tokens.cache_read, 10);
        assert_eq!(messages[0].tokens.total(), 125);
    }

    #[test]
    fn test_parse_gemini_tokens_with_mixed_duplicate_fields() {
        let json = r#"{
            "sessionId": "ses_dup",
            "projectHash": "abc123",
            "startTime": "2025-06-15T12:00:00Z",
            "lastUpdated": "2025-06-15T12:30:00Z",
            "messages": [
                {
                    "id": "msg_1",
                    "timestamp": "2025-06-15T12:01:00Z",
                    "type": "gemini",
                    "model": "gemini-3-flash-preview",
                    "tokens": {
                        "input": 100,
                        "prompt": 200,
                        "output": 50,
                        "candidates": 60,
                        "cached": 5,
                        "total": 215
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
        assert_eq!(messages[0].model_id, "gemini-3-flash-preview");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.cache_read, 5);
    }
}
