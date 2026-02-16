//! Gemini CLI session parser
//!
//! Parses JSON session files from ~/.gemini/tmp/*/chats/session-*.json

use super::utils::{
    extract_i64, extract_string, file_modified_timestamp_ms, parse_timestamp_value,
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
    pub content: Option<String>,
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

    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
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
    let mut messages = Vec::new();
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

        messages.push(UnifiedMessage::new(
            "gemini",
            model,
            "google",
            session_id.clone(),
            timestamp,
            TokenBreakdown {
                input: tokens.input.unwrap_or(0).max(0),
                output: tokens.output.unwrap_or(0).max(0),
                cache_read: tokens.cached.unwrap_or(0).max(0),
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
    let mut messages = Vec::new();

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
        let value: Value = match simd_json::from_slice(&mut bytes) {
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
            UnifiedMessage::new(
                "gemini",
                usage.model,
                "google",
                session_id.to_string(),
                timestamp,
                TokenBreakdown {
                    input: usage.input.max(0),
                    output: usage.output.max(0),
                    cache_read: usage.cached.max(0),
                    cache_write: 0,
                    reasoning: usage.reasoning.max(0),
                },
                0.0,
            )
        })
        .collect()
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
                    "type": "user",
                    "content": "Hello"
                },
                {
                    "id": "msg_2",
                    "timestamp": "2025-06-15T12:01:00Z",
                    "type": "gemini",
                    "content": "Hi there!",
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
    fn test_parse_headless_json() {
        let json = r#"{"response":"Hi","stats":{"models":{"gemini-2.5-pro":{"tokens":{"prompt":12,"candidates":34,"cached":5,"thoughts":2}}}}}"#;
        let file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        std::fs::write(file.path(), json).unwrap();

        let messages = parse_gemini_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-2.5-pro");
        assert_eq!(messages[0].tokens.input, 12);
        assert_eq!(messages[0].tokens.output, 34);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].tokens.reasoning, 2);
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
}
