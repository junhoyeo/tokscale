//! Amp (Sourcegraph) session parser
//!
//! Parses JSON files from ~/.local/share/amp/threads/

use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::path::Path;

/// Amp usage event from usageLedger
#[derive(Debug, Deserialize)]
pub struct AmpUsageEvent {
    pub timestamp: Option<String>,
    pub model: Option<String>,
    pub credits: Option<f64>,
    pub tokens: Option<AmpTokens>,
    #[serde(rename = "operationType")]
    pub _operation_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AmpTokens {
    pub input: Option<i64>,
    pub output: Option<i64>,
    #[serde(rename = "cacheReadInputTokens")]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(rename = "cacheCreationInputTokens")]
    pub cache_creation_input_tokens: Option<i64>,
}

/// Amp message usage (per-message, more detailed)
#[derive(Debug, Deserialize)]
pub struct AmpMessageUsage {
    pub model: Option<String>,
    #[serde(rename = "inputTokens")]
    pub input_tokens: Option<i64>,
    #[serde(rename = "outputTokens")]
    pub output_tokens: Option<i64>,
    #[serde(rename = "cacheReadInputTokens")]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(rename = "cacheCreationInputTokens")]
    pub cache_creation_input_tokens: Option<i64>,
    pub credits: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct AmpMessage {
    pub role: Option<String>,
    #[serde(rename = "messageId")]
    pub message_id: Option<i64>,
    pub usage: Option<AmpMessageUsage>,
}

#[derive(Debug, Deserialize)]
pub struct AmpUsageLedger {
    pub events: Option<Vec<AmpUsageEvent>>,
}

#[derive(Debug, Deserialize)]
pub struct AmpThread {
    pub id: Option<String>,
    pub created: Option<i64>,
    pub messages: Option<Vec<AmpMessage>>,
    #[serde(rename = "usageLedger")]
    pub usage_ledger: Option<AmpUsageLedger>,
}

/// Get provider from model name
fn get_provider_from_model(model: &str) -> &'static str {
    let model_lower = model.to_lowercase();
    if model_lower.contains("claude")
        || model_lower.contains("opus")
        || model_lower.contains("sonnet")
        || model_lower.contains("haiku")
    {
        return "anthropic";
    }
    if model_lower.contains("gpt") || model_lower.contains("o1") || model_lower.contains("o3") {
        return "openai";
    }
    if model_lower.contains("gemini") {
        return "google";
    }
    if model_lower.contains("grok") {
        return "xai";
    }
    "anthropic" // Default for Amp
}

/// Parse an Amp thread JSON file
pub fn parse_amp_file(path: &Path) -> Vec<UnifiedMessage> {
    let content = match std::fs::read(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Get file mtime as last-resort timestamp fallback
    let file_mtime_ms = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let mut bytes = content;
    let thread: AmpThread = match simd_json::from_slice(&mut bytes) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let thread_id = thread.id.clone().unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    });

    // Extract thread created time before consuming usage_ledger, for use as timestamp fallback
    let thread_created_ms = thread.created.unwrap_or(0);

    let mut messages = Vec::new();

    // Prefer usageLedger.events (cleaner aggregated data)
    if let Some(ledger) = thread.usage_ledger {
        if let Some(events) = ledger.events {
            for event in events {
                let model = match event.model {
                    Some(m) => m,
                    None => continue,
                };

                let timestamp = event
                    .timestamp
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
                    .map(|dt| dt.timestamp_millis())
                    .unwrap_or(0);

                // Use fallbacks when timestamp is missing: thread created time, then file mtime.
                // Never skip events solely because of a missing timestamp.
                let timestamp = if timestamp != 0 {
                    timestamp
                } else if thread_created_ms != 0 {
                    thread_created_ms
                } else {
                    file_mtime_ms
                };

                let tokens = event.tokens.unwrap_or(AmpTokens {
                    input: Some(0),
                    output: Some(0),
                    cache_read_input_tokens: Some(0),
                    cache_creation_input_tokens: Some(0),
                });

                messages.push(UnifiedMessage::new(
                    "amp",
                    &model,
                    get_provider_from_model(&model),
                    thread_id.clone(),
                    timestamp,
                    TokenBreakdown {
                        input: tokens.input.unwrap_or(0).max(0),
                        output: tokens.output.unwrap_or(0).max(0),
                        cache_read: tokens.cache_read_input_tokens.unwrap_or(0).max(0),
                        cache_write: tokens.cache_creation_input_tokens.unwrap_or(0).max(0),
                        reasoning: 0,
                    },
                    event.credits.unwrap_or(0.0).max(0.0),
                ));
            }
            if !messages.is_empty() {
                return messages;
            }
        }
    }

    // Fallback: parse from individual message usage
    let created = thread_created_ms;
    if let Some(thread_messages) = thread.messages {
        for msg in thread_messages {
            if msg.role.as_deref() != Some("assistant") {
                continue;
            }

            let usage = match msg.usage {
                Some(u) => u,
                None => continue,
            };

            let model = match usage.model {
                Some(m) => m,
                None => continue,
            };

            // Approximate timestamp from created + messageId offset
            let message_id = msg.message_id.unwrap_or(0);
            let timestamp = created + (message_id * 1000);

            messages.push(UnifiedMessage::new(
                "amp",
                &model,
                get_provider_from_model(&model),
                thread_id.clone(),
                timestamp,
                TokenBreakdown {
                    input: usage.input_tokens.unwrap_or(0).max(0),
                    output: usage.output_tokens.unwrap_or(0).max(0),
                    cache_read: usage.cache_read_input_tokens.unwrap_or(0).max(0),
                    cache_write: usage.cache_creation_input_tokens.unwrap_or(0).max(0),
                    reasoning: 0,
                },
                usage.credits.unwrap_or(0.0).max(0.0),
            ));
        }
    }

    messages
}
