//! Codex CLI session parser
//!
//! Parses JSONL files from ~/.codex/sessions/
//! Note: This parser has stateful logic to track model and delta calculations.

use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Codex entry structure (from JSONL files)
#[derive(Debug, Deserialize)]
pub struct CodexEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub timestamp: Option<String>,
    pub payload: Option<CodexPayload>,
}

#[derive(Debug, Deserialize)]
pub struct CodexPayload {
    #[serde(rename = "type")]
    pub payload_type: Option<String>,
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub info: Option<CodexInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CodexInfo {
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub last_token_usage: Option<CodexTokenUsage>,
    pub total_token_usage: Option<CodexTokenUsage>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CodexTokenUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
}

/// Parse a Codex JSONL file with stateful tracking
pub fn parse_codex_file(path: &Path) -> Vec<UnifiedMessage> {
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

    // Stateful tracking
    let mut current_model: Option<String> = None;
    let mut previous_totals: Option<(i64, i64, i64)> = None; // (input, output, cached)

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
        let entry: CodexEntry = match simd_json::from_slice(&mut bytes) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let payload = match entry.payload {
            Some(p) => p,
            None => continue,
        };

        // Extract model from turn_context
        if entry.entry_type == "turn_context" {
            current_model = extract_model(&payload);
            continue;
        }

        // Process token_count events
        if entry.entry_type != "event_msg" {
            continue;
        }

        if payload.payload_type.as_deref() != Some("token_count") {
            continue;
        }

        // Try to extract model from payload
        if let Some(model) = extract_model(&payload) {
            current_model = Some(model);
        }

        let info = match payload.info {
            Some(i) => i,
            None => continue,
        };

        // Try to extract model from info
        if let Some(model) = info.model.clone().or(info.model_name.clone()) {
            current_model = Some(model);
        }

        let model = current_model
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        // Calculate delta tokens
        let (input, output, cached) = if let Some(last) = &info.last_token_usage {
            (
                last.input_tokens.unwrap_or(0),
                last.output_tokens.unwrap_or(0),
                last.cached_input_tokens
                    .or(last.cache_read_input_tokens)
                    .unwrap_or(0),
            )
        } else if let (Some(total), Some(prev)) = (&info.total_token_usage, &previous_totals) {
            let curr_input = total.input_tokens.unwrap_or(0);
            let curr_output = total.output_tokens.unwrap_or(0);
            let curr_cached = total
                .cached_input_tokens
                .or(total.cache_read_input_tokens)
                .unwrap_or(0);

            (
                (curr_input - prev.0).max(0),
                (curr_output - prev.1).max(0),
                (curr_cached - prev.2).max(0),
            )
        } else {
            continue;
        };

        // Update previous totals
        if let Some(total) = &info.total_token_usage {
            previous_totals = Some((
                total.input_tokens.unwrap_or(0),
                total.output_tokens.unwrap_or(0),
                total
                    .cached_input_tokens
                    .or(total.cache_read_input_tokens)
                    .unwrap_or(0),
            ));
        }

        // Skip empty deltas
        if input == 0 && output == 0 && cached == 0 {
            continue;
        }

        let timestamp = entry
            .timestamp
            .or(payload.payload_type.clone()) // fallback
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
            .map(|dt| dt.timestamp_millis())
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

        messages.push(UnifiedMessage::new(
            "codex",
            model,
            "openai",
            session_id.clone(),
            timestamp,
            TokenBreakdown {
                input,
                output,
                cache_read: cached,
                cache_write: 0,
                reasoning: 0,
            },
            0.0, // Cost calculated later
        ));
    }

    messages
}

fn extract_model(payload: &CodexPayload) -> Option<String> {
    payload
        .model
        .clone()
        .or(payload.model_name.clone())
        .or(payload.info.as_ref().and_then(|i| i.model.clone()))
        .or(payload.info.as_ref().and_then(|i| i.model_name.clone()))
        .filter(|m| !m.is_empty())
}
