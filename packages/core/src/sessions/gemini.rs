//! Gemini CLI session parser
//!
//! Parses JSON session files from ~/.gemini/tmp/*/chats/session-*.json

use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
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
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut bytes = data;
    let session: GeminiSession = match simd_json::from_slice(&mut bytes) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

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
            .unwrap_or(0);

        if timestamp == 0 {
            continue;
        }

        messages.push(UnifiedMessage::new(
            "gemini",
            model,
            "google",
            session_id.clone(),
            timestamp,
            TokenBreakdown {
                input: tokens.input.unwrap_or(0),
                output: tokens.output.unwrap_or(0),
                cache_read: tokens.cached.unwrap_or(0),
                cache_write: 0,
                reasoning: tokens.thoughts.unwrap_or(0),
            },
            0.0, // Cost calculated later
        ));
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
