//! Aside browser's flat, per-response JSONL usage records.

use super::utils::{
    file_modified_timestamp_ms, for_each_json_line_with_bytes, parse_timestamp_value,
};
use super::UnifiedMessage;
use crate::{provider_identity::inferred_provider_from_model, TokenBreakdown};
use serde::Deserialize;
use std::ops::ControlFlow;
use std::path::Path;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    role: String,
    timestamp: Option<serde_json::Value>,
    model: Option<String>,
    provider: Option<String>,
    response_id: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Usage {
    input: Option<i64>,
    output: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
    reasoning: Option<i64>,
    cost: Option<Cost>,
}

#[derive(Deserialize)]
struct Cost {
    total: Option<f64>,
}

/// Parse usage metadata only; message content and account credentials are ignored.
pub fn parse_aside_file(path: &Path) -> Vec<UnifiedMessage> {
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let session = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    let account = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    let session_id = format!("{account}/{session}");
    let mut messages = Vec::new();
    for_each_json_line_with_bytes(path, &mut |line| {
        if !line.valid_utf8 {
            return ControlFlow::Continue(());
        }
        let Ok(record) = serde_json::from_str::<Record>(line.trimmed) else {
            return ControlFlow::Continue(());
        };
        if record.role != "assistant" {
            return ControlFlow::Continue(());
        }
        let Some(usage) = record.usage else {
            return ControlFlow::Continue(());
        };
        let output = usage.output.unwrap_or(0).max(0);
        // Aside output includes reasoning; Tokscale's buckets are disjoint.
        let reasoning = usage.reasoning.unwrap_or(0).clamp(0, output);
        let tokens = TokenBreakdown {
            input: usage.input.unwrap_or(0).max(0),
            output: output - reasoning,
            cache_read: usage.cache_read.unwrap_or(0).max(0),
            cache_write: usage.cache_write.unwrap_or(0).max(0),
            reasoning,
            ..Default::default()
        };
        if tokens.total() == 0 {
            return ControlFlow::Continue(());
        }
        let model = record
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown");
        let provider = record
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|s| {
                !s.is_empty()
                    && !s.eq_ignore_ascii_case("aside")
                    && !s.eq_ignore_ascii_case("unknown")
            })
            .unwrap_or_else(|| inferred_provider_from_model(model).unwrap_or("unknown"));
        let timestamp = record
            .timestamp
            .as_ref()
            .and_then(parse_timestamp_value)
            .unwrap_or(fallback_timestamp);
        let cost = usage
            .cost
            .and_then(|cost| cost.total)
            .filter(|cost| cost.is_finite() && *cost > 0.0)
            .unwrap_or(0.0);
        let mut message = UnifiedMessage::new(
            "aside",
            model,
            provider,
            &session_id,
            timestamp,
            tokens,
            cost,
        );
        if cost > 0.0 {
            message.mark_provider_reported_cost();
        }
        // Length framing prevents delimiter collisions; no ID means no dedup.
        message.dedup_key = record
            .response_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| format!("aside:response:{}:{provider}:{id}", provider.len()));
        messages.push(message);
        ControlFlow::Continue(())
    });
    messages
}
