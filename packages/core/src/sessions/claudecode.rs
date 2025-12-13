//! Claude Code session parser
//!
//! Parses JSONL files from ~/.claude/projects/

use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Claude Code entry structure (from JSONL files)
#[derive(Debug, Deserialize)]
pub struct ClaudeEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub timestamp: Option<String>,
    pub message: Option<ClaudeMessage>,
}

#[derive(Debug, Deserialize)]
pub struct ClaudeMessage {
    pub model: Option<String>,
    pub usage: Option<ClaudeUsage>,
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
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

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
        let entry: ClaudeEntry = match simd_json::from_slice(&mut bytes) {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Only process assistant messages with usage data
        if entry.entry_type != "assistant" {
            continue;
        }

        let message = match entry.message {
            Some(m) => m,
            None => continue,
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
            .unwrap_or(0);

        if timestamp == 0 {
            continue;
        }

        messages.push(UnifiedMessage::new(
            "claude",
            model,
            "anthropic",
            session_id.clone(),
            timestamp,
            TokenBreakdown {
                input: usage.input_tokens.unwrap_or(0),
                output: usage.output_tokens.unwrap_or(0),
                cache_read: usage.cache_read_input_tokens.unwrap_or(0),
                cache_write: usage.cache_creation_input_tokens.unwrap_or(0),
                reasoning: 0,
            },
            0.0, // Cost calculated later
        ));
    }

    messages
}
