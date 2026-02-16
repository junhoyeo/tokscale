//! Codex CLI session parser
//!
//! Parses JSONL files from ~/.codex/sessions/
//! Note: This parser has stateful logic to track model and delta calculations.

use super::utils::{
    extract_i64, extract_string, file_modified_timestamp_ms, parse_timestamp_value,
};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
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
    pub source: Option<String>,
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

    let fallback_timestamp = file_modified_timestamp_ms(path);

    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    // Stateful tracking
    let mut current_model: Option<String> = None;
    let mut previous_totals: Option<(i64, i64, i64)> = None; // (input, output, cached)
    let mut session_is_headless = false;

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
        if let Ok(entry) = simd_json::from_slice::<CodexEntry>(&mut bytes) {
            if let Some(payload) = entry.payload {
                // Check session_meta for headless exec sessions
                if entry.entry_type == "session_meta" {
                    if payload.source.as_deref() == Some("exec") {
                        session_is_headless = true;
                    }
                }
                // Extract model from turn_context
                if entry.entry_type == "turn_context" {
                    current_model = extract_model(&payload);
                    handled = true;
                }

                // Process token_count events
                if entry.entry_type == "event_msg"
                    && payload.payload_type.as_deref() == Some("token_count")
                {
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
                    // Note: OpenAI's input_tokens INCLUDES cached tokens (they are a subset).
                    // We subtract cached from input to avoid double-counting when aggregating.
                    let (input, output, cached) = if let Some(last) = &info.last_token_usage {
                        let total_input = last.input_tokens.unwrap_or(0);
                        let cached = last
                            .cached_input_tokens
                            .or(last.cache_read_input_tokens)
                            .unwrap_or(0);
                        (
                            total_input.saturating_sub(cached),
                            last.output_tokens.unwrap_or(0),
                            cached,
                        )
                    } else if let (Some(total), Some(prev)) =
                        (&info.total_token_usage, &previous_totals)
                    {
                        let curr_input = total.input_tokens.unwrap_or(0);
                        let curr_output = total.output_tokens.unwrap_or(0);
                        let curr_cached = total
                            .cached_input_tokens
                            .or(total.cache_read_input_tokens)
                            .unwrap_or(0);

                        let delta_input = (curr_input - prev.0).max(0);
                        let delta_cached = (curr_cached - prev.2).max(0);
                        (
                            (delta_input - delta_cached).max(0),
                            (curr_output - prev.1).max(0),
                            delta_cached,
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
                        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
                        .map(|dt| dt.timestamp_millis())
                        .unwrap_or(fallback_timestamp);

                    let agent = if session_is_headless {
                        Some("headless".to_string())
                    } else {
                        None
                    };

                    messages.push(UnifiedMessage::new_with_agent(
                        "codex",
                        model,
                        "openai",
                        session_id.clone(),
                        timestamp,
                        TokenBreakdown {
                            input: input.max(0),
                            output: output.max(0),
                            cache_read: cached.max(0),
                            cache_write: 0,
                            reasoning: 0,
                        },
                        0.0,
                        agent,
                    ));
                    handled = true;
                }
            }

            // Mark session_meta as handled (even if payload was processed above)
            if entry.entry_type == "session_meta" {
                handled = true;
            }
        }

        if handled {
            continue;
        }

        if let Some(msg) =
            parse_codex_headless_line(trimmed, &session_id, &mut current_model, fallback_timestamp)
        {
            let mut msg = msg;
            if session_is_headless && msg.agent.is_none() {
                msg.agent = Some("headless".to_string());
            }
            messages.push(msg);
        }
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

struct CodexHeadlessUsage {
    input: i64,
    output: i64,
    cached: i64,
    model: Option<String>,
    timestamp_ms: Option<i64>,
}

fn parse_codex_headless_line(
    line: &str,
    session_id: &str,
    current_model: &mut Option<String>,
    fallback_timestamp: i64,
) -> Option<UnifiedMessage> {
    let mut bytes = line.as_bytes().to_vec();
    let value: Value = simd_json::from_slice(&mut bytes).ok()?;

    if let Some(model) = extract_model_from_value(&value) {
        *current_model = Some(model);
    }

    let usage = extract_headless_usage(&value)?;
    let model = usage
        .model
        .or_else(|| current_model.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let timestamp = usage.timestamp_ms.unwrap_or(fallback_timestamp);

    if usage.input == 0 && usage.output == 0 && usage.cached == 0 {
        return None;
    }

    Some(UnifiedMessage::new(
        "codex",
        model,
        "openai",
        session_id.to_string(),
        timestamp,
        TokenBreakdown {
            input: usage.input.max(0),
            output: usage.output.max(0),
            cache_read: usage.cached.max(0),
            cache_write: 0,
            reasoning: 0,
        },
        0.0,
    ))
}

fn extract_headless_usage(value: &Value) -> Option<CodexHeadlessUsage> {
    let usage = value
        .get("usage")
        .or_else(|| value.get("data").and_then(|data| data.get("usage")))
        .or_else(|| value.get("result").and_then(|data| data.get("usage")))
        .or_else(|| value.get("response").and_then(|data| data.get("usage")))?;

    let input_tokens = extract_i64(usage.get("input_tokens"))
        .or_else(|| extract_i64(usage.get("prompt_tokens")))
        .or_else(|| extract_i64(usage.get("input")))
        .unwrap_or(0);
    let output_tokens = extract_i64(usage.get("output_tokens"))
        .or_else(|| extract_i64(usage.get("completion_tokens")))
        .or_else(|| extract_i64(usage.get("output")))
        .unwrap_or(0);
    let cached_tokens = extract_i64(usage.get("cached_input_tokens"))
        .or_else(|| extract_i64(usage.get("cache_read_input_tokens")))
        .or_else(|| extract_i64(usage.get("cached_tokens")))
        .unwrap_or(0);

    let model = extract_model_from_value(value)
        .or_else(|| value.get("data").and_then(extract_model_from_value));
    let timestamp_ms = extract_timestamp_from_value(value);

    Some(CodexHeadlessUsage {
        input: input_tokens.saturating_sub(cached_tokens),
        output: output_tokens,
        cached: cached_tokens,
        model,
        timestamp_ms,
    })
}

fn extract_model_from_value(value: &Value) -> Option<String> {
    extract_string(value.get("model"))
        .or_else(|| extract_string(value.get("model_name")))
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| extract_string(data.get("model")))
        })
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| extract_string(data.get("model_name")))
        })
        .or_else(|| {
            value
                .get("response")
                .and_then(|data| extract_string(data.get("model")))
        })
}

fn extract_timestamp_from_value(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .or_else(|| value.get("time"))
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("data").and_then(|data| data.get("timestamp")))
        .and_then(parse_timestamp_value)
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
    fn test_headless_usage_line() {
        let content = r#"{"type":"turn.completed","model":"gpt-4o-mini","usage":{"input_tokens":120,"cached_input_tokens":20,"output_tokens":30}}"#;
        let file = create_test_file(content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o-mini");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
    }

    #[test]
    fn test_headless_usage_nested_data() {
        let content = r#"{"type":"result","data":{"model_name":"gpt-4o","usage":{"input_tokens":50,"cached_input_tokens":5,"output_tokens":12}}}"#;
        let file = create_test_file(content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o");
        assert_eq!(messages[0].tokens.input, 45);
        assert_eq!(messages[0].tokens.output, 12);
        assert_eq!(messages[0].tokens.cache_read, 5);
    }

    #[test]
    fn test_session_meta_exec_marks_headless() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"originator":"codex_exec","source":"exec"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#;
        let content = format!("{}\n{}", line1, line2);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent.as_deref(), Some("headless"));
    }
}
