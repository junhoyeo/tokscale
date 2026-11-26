//! Read Raven's per-call usage telemetry without opening session transcripts.

use std::{ops::ControlFlow, path::Path};

use serde::Deserialize;

use super::{utils::for_each_json_line_with_bytes, UnifiedMessage};
use crate::{provider_identity::inferred_provider_from_model_delimited, TokenBreakdown};

#[derive(Deserialize)]
struct UsageRow {
    schema_version: u32,
    ts: String,
    #[serde(rename = "_type")]
    kind: Option<String>,
    model: Option<String>,
    session_key: Option<String>,
    root_session_key: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cache_write_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    cost_usd: Option<f64>,
}

impl UsageRow {
    fn into_message(self) -> Option<UnifiedMessage> {
        if self.kind.is_some() || !matches!(self.schema_version, 1 | 2) {
            return None;
        }

        let timestamp = chrono::DateTime::parse_from_rfc3339(&self.ts)
            .ok()?
            .timestamp_millis();
        if timestamp <= 0 {
            return None;
        }
        let output = self.output_tokens.unwrap_or(0).max(0);
        let reasoning = self.reasoning_tokens.unwrap_or(0).max(0);
        // The v2 writer records reasoning inside completion tokens. The
        // reported v1 format has separate output and reasoning buckets.
        let reasoning = if self.schema_version == 2 && self.output_tokens.is_some() {
            reasoning.min(output)
        } else {
            reasoning
        };
        let tokens = TokenBreakdown {
            input: self.input_tokens.unwrap_or(0).max(0),
            output: if self.schema_version == 2 {
                output.saturating_sub(reasoning).max(0)
            } else {
                output
            },
            cache_read: self.cache_read_tokens.unwrap_or(0).max(0),
            cache_write: self.cache_write_tokens.unwrap_or(0).max(0),
            reasoning,
            ..Default::default()
        };
        let cost = self
            .cost_usd
            .filter(|cost| self.schema_version == 2 && cost.is_finite() && *cost >= 0.0);
        if tokens.total() == 0 && !cost.is_some_and(|cost| cost > 0.0) {
            return None;
        }

        let model = self
            .model
            .as_deref()
            .filter(|model| !model.is_empty())
            .unwrap_or("unknown");
        let provider = inferred_provider_from_model_delimited(model).unwrap_or("unknown");
        let root = self
            .root_session_key
            .as_deref()
            .filter(|key| !key.is_empty());
        let session = self
            .session_key
            .as_deref()
            .filter(|key| !key.is_empty())
            .or(root)
            .unwrap_or("unknown");
        let mut message = UnifiedMessage::new(
            "raven",
            model,
            provider,
            session,
            timestamp,
            tokens,
            cost.unwrap_or(0.0),
        );
        message.parent_session_id = root.filter(|root| *root != session).map(Into::into);
        if cost.is_some() {
            message.mark_provider_reported_cost();
        }
        Some(message)
    }
}

pub fn parse_raven_file(path: &Path) -> Vec<UnifiedMessage> {
    let mut messages = Vec::new();
    for_each_json_line_with_bytes(path, &mut |line| {
        if !line.valid_utf8 {
            return ControlFlow::Continue(());
        }
        if let Some(message) = serde_json::from_str::<UsageRow>(line.trimmed)
            .ok()
            .and_then(UsageRow::into_message)
        {
            messages.push(message);
        }
        ControlFlow::Continue(())
    });
    messages
}
