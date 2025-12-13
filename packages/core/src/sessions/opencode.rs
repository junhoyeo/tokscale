//! OpenCode session parser
//!
//! Parses individual JSON files from ~/.local/share/opencode/storage/message/

use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::path::Path;

/// OpenCode message structure (from JSON files)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OpenCodeMessage {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub role: String,
    #[serde(rename = "modelID")]
    pub model_id: Option<String>,
    #[serde(rename = "providerID")]
    pub provider_id: Option<String>,
    pub cost: Option<f64>,
    pub tokens: Option<OpenCodeTokens>,
    pub time: OpenCodeTime,
}

#[derive(Debug, Deserialize)]
pub struct OpenCodeTokens {
    pub input: i64,
    pub output: i64,
    pub reasoning: Option<i64>,
    pub cache: OpenCodeCache,
}

#[derive(Debug, Deserialize)]
pub struct OpenCodeCache {
    pub read: i64,
    pub write: i64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OpenCodeTime {
    pub created: f64, // Unix timestamp in milliseconds (as float)
    pub completed: Option<f64>,
}

/// Parse an OpenCode message file
pub fn parse_opencode_file(path: &Path) -> Option<UnifiedMessage> {
    let data = std::fs::read(path).ok()?;
    let mut bytes = data;

    let msg: OpenCodeMessage = simd_json::from_slice(&mut bytes).ok()?;

    // Only process assistant messages with tokens
    if msg.role != "assistant" {
        return None;
    }

    let tokens = msg.tokens?;
    let model_id = msg.model_id?;

    Some(UnifiedMessage::new(
        "opencode",
        model_id,
        msg.provider_id.unwrap_or_else(|| "unknown".to_string()),
        msg.session_id.clone(),
        msg.time.created as i64,
        TokenBreakdown {
            input: tokens.input,
            output: tokens.output,
            cache_read: tokens.cache.read,
            cache_write: tokens.cache.write,
            reasoning: tokens.reasoning.unwrap_or(0),
        },
        msg.cost.unwrap_or(0.0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_opencode_structure() {
        let json = r#"{
            "id": "msg_123",
            "sessionID": "ses_456",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 100,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let mut bytes = json.as_bytes().to_vec();
        let msg: OpenCodeMessage = simd_json::from_slice(&mut bytes).unwrap();

        assert_eq!(msg.model_id, Some("claude-sonnet-4".to_string()));
        assert_eq!(msg.tokens.unwrap().input, 1000);
    }
}
