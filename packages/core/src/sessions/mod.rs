//! Session parsers for different AI coding assistant formats
//!
//! Each source has its own parser that converts to a unified message format.

pub mod claudecode;
pub mod codex;
pub mod cursor;
pub mod gemini;
pub mod opencode;

use crate::TokenBreakdown;

/// Unified message format across all sources
#[derive(Debug, Clone)]
pub struct UnifiedMessage {
    pub source: String,
    pub model_id: String,
    pub provider_id: String,
    pub session_id: String,
    pub timestamp: i64, // Unix milliseconds
    pub date: String,   // YYYY-MM-DD
    pub tokens: TokenBreakdown,
    pub cost: f64,
}

impl UnifiedMessage {
    /// Create a new unified message
    pub fn new(
        source: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
    ) -> Self {
        let date = timestamp_to_date(timestamp);
        Self {
            source: source.into(),
            model_id: model_id.into(),
            provider_id: provider_id.into(),
            session_id: session_id.into(),
            timestamp,
            date,
            tokens,
            cost,
        }
    }
}

/// Convert Unix milliseconds timestamp to YYYY-MM-DD date string
fn timestamp_to_date(timestamp_ms: i64) -> String {
    use chrono::{TimeZone, Utc};

    let datetime = Utc.timestamp_millis_opt(timestamp_ms);
    match datetime {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d").to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timestamp_to_date() {
        // 2025-06-16 12:00:00 UTC (1750075200 seconds since epoch)
        let ts = 1750075200000_i64;
        let date = timestamp_to_date(ts);
        assert_eq!(date, "2025-06-16");
    }

    #[test]
    fn test_timestamp_to_date_epoch() {
        // Unix epoch: 1970-01-01
        let ts = 0_i64;
        let date = timestamp_to_date(ts);
        assert_eq!(date, "1970-01-01");
    }

    #[test]
    fn test_timestamp_to_date_recent() {
        // 2024-12-01 00:00:00 UTC
        let ts = 1733011200000_i64;
        let date = timestamp_to_date(ts);
        assert_eq!(date, "2024-12-01");
    }

    #[test]
    fn test_unified_message_creation() {
        let tokens = TokenBreakdown {
            input: 100,
            output: 50,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        };

        let msg = UnifiedMessage::new(
            "opencode",
            "claude-3-5-sonnet",
            "anthropic",
            "test-session-id",
            1733011200000,
            tokens,
            0.05,
        );

        assert_eq!(msg.source, "opencode");
        assert_eq!(msg.model_id, "claude-3-5-sonnet");
        assert_eq!(msg.session_id, "test-session-id");
        assert_eq!(msg.date, "2024-12-01");
        assert_eq!(msg.cost, 0.05);
    }
}
