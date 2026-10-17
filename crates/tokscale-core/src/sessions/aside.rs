//! Aside browser's flat, per-response JSONL usage records.

use super::utils::{
    file_modified_timestamp_ms, for_each_json_line_with_bytes, parse_timestamp_value,
};
use super::{BillingIdentityOrigin, UnifiedMessage};
use crate::{provider_identity::inferred_provider_from_model_delimited, TokenBreakdown};
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
    /// Subset of `cache_write` billed at the 1-hour cache TTL rate (2x input)
    /// rather than the 5-minute rate (1.25x input). Aside writes this key on
    /// Anthropic-API responses.
    cache_write_1h: Option<i64>,
    reasoning: Option<i64>,
    cost: Option<Cost>,
}

#[derive(Deserialize)]
struct Cost {
    total: Option<f64>,
}

/// The vendor an Aside `provider` names, or `None` when it names a route.
///
/// `provider` records the ROUTE Aside took, not always the vendor that served
/// the call. Real transcripts hold `ai-gateway` for a user-configured gateway,
/// `claude-code` for a Claude Code subscription, and `aside` for Aside's own
/// gateway — none of which is a provider the pricing datasets key on. The
/// public format reference this parser was written against maps the same ids
/// and blanks the rest for exactly that reason.
///
/// Only the route ids above are rewritten; any other value is left to the
/// caller to pass through. A vendor id Aside starts writing later is a
/// stronger claim than inference from a model name, so it must not be
/// discarded by an allowlist that has not heard of it yet.
///
/// `claude-code` resolves to `anthropic` because a Claude Code subscription is
/// an Anthropic route — the vendor is known, only the billing relationship
/// differs. `ai-gateway` resolves to nothing: it is a generic proxy whose
/// upstream is configured per user, so the model id is the only evidence left.
fn route_provider_vendor(raw: &str) -> Option<Option<&'static str>> {
    match raw.to_ascii_lowercase().as_str() {
        "claude-code" => Some(Some("anthropic")),
        // Aside's own gateway and an unqualified routing label name no vendor.
        "ai-gateway" | "aside" | "unknown" => Some(None),
        _ => None,
    }
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
        let cache_write = usage.cache_write.unwrap_or(0).max(0);
        let tokens = TokenBreakdown {
            input: usage.input.unwrap_or(0).max(0),
            output: output - reasoning,
            cache_read: usage.cache_read.unwrap_or(0).max(0),
            cache_write,
            // `cache_write_1h` is a subset of `cache_write`, never an extra
            // bucket: `TokenBreakdown::total()` does not add it, and pricing
            // bills it at 2x input instead of 1.25x. Clamping to `cache_write`
            // restores that invariant on a source that contradicts itself.
            cache_write_1h: usage.cache_write_1h.unwrap_or(0).clamp(0, cache_write),
            reasoning,
        };
        let cost = usage
            .cost
            .and_then(|cost| cost.total)
            .filter(|cost| cost.is_finite() && *cost > 0.0)
            .unwrap_or(0.0);
        if tokens.total() == 0 && cost == 0.0 {
            return ControlFlow::Continue(());
        }
        let model = record
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown");
        // Explicit vendor ids stay authoritative; only the route ids Aside is
        // known to write are rewritten, and a route that names no vendor falls
        // back to the delimiter-aware model-family inference.
        let provider = record
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map_or(Some(None), |raw| {
                route_provider_vendor(raw).or(Some(Some(raw)))
            })
            .flatten()
            .or_else(|| inferred_provider_from_model_delimited(model))
            .unwrap_or("unknown");
        let timestamp = record
            .timestamp
            .as_ref()
            .and_then(parse_timestamp_value)
            .unwrap_or(fallback_timestamp);
        let mut message = UnifiedMessage::new(
            "aside",
            model,
            provider,
            &session_id,
            timestamp,
            tokens,
            cost,
        )
        .with_billing(
            record.provider.as_deref(),
            BillingIdentityOrigin::ExplicitRecord,
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

#[cfg(test)]
mod billing_tests {
    use super::*;

    #[test]
    fn aside_keeps_explicit_billing_route_separate_from_inferred_vendor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.jsonl");
        let rows: Vec<_> = [
            Some("ai-gateway"),
            Some("claude-code"),
            Some("aside"),
            Some("anthropic"),
            None,
        ]
        .into_iter()
        .map(|provider| {
            serde_json::json!({
                "role":"assistant", "model":"claude-sonnet-4", "provider":provider,
                "usage":{"input":100}, "timestamp":"2026-10-16T00:00:00Z"
            })
            .to_string()
        })
        .collect();
        std::fs::write(&path, rows.join("\n")).unwrap();
        let messages = parse_aside_file(&path);
        assert_eq!(messages.len(), 5);
        for (message, expected) in messages.iter().zip([
            Some("ai-gateway"),
            Some("claude-code"),
            Some("aside"),
            Some("anthropic"),
            None,
        ]) {
            assert_eq!(message.provider_id, "anthropic");
            assert_eq!(message.billing_identity.id(), expected);
        }
    }
}
