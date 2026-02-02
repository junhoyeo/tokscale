//! Session parsers for different AI coding assistant formats
//!
//! Each source has its own parser that converts to a unified message format.

pub mod amp;
pub mod claudecode;
pub mod codex;
pub mod cursor;
pub mod droid;
pub mod gemini;
pub mod openclaw;
pub mod opencode;
pub(crate) mod utils;

use crate::TokenBreakdown;

#[derive(Debug, Clone)]
pub struct UnifiedMessage {
    pub source: String,
    pub model_id: String,
    pub provider_id: String,
    pub session_id: String,
    pub timestamp: i64,
    pub date: String,
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub agent: Option<String>,
    pub dedup_key: Option<String>,
}

pub fn normalize_agent_name(agent: &str) -> String {
    let agent_lower = agent.to_lowercase();

    if agent_lower.contains("plan") {
        if agent_lower.contains("omo") || agent_lower.contains("sisyphus") {
            return "Planner-Sisyphus".to_string();
        }
        return agent.to_string();
    }

    if agent_lower == "omo" || agent_lower == "sisyphus" {
        return "Sisyphus".to_string();
    }

    agent.to_string()
}

impl UnifiedMessage {
    pub fn new(
        source: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
    ) -> Self {
        Self::new_full(
            source,
            model_id,
            provider_id,
            session_id,
            timestamp,
            tokens,
            cost,
            None,
            None,
        )
    }

    pub fn new_with_agent(
        source: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
        agent: Option<String>,
    ) -> Self {
        Self::new_full(
            source,
            model_id,
            provider_id,
            session_id,
            timestamp,
            tokens,
            cost,
            agent,
            None,
        )
    }

    pub fn new_with_dedup(
        source: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
        dedup_key: Option<String>,
    ) -> Self {
        Self::new_full(
            source,
            model_id,
            provider_id,
            session_id,
            timestamp,
            tokens,
            cost,
            None,
            dedup_key,
        )
    }

    fn new_full(
        source: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
        agent: Option<String>,
        dedup_key: Option<String>,
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
            agent,
            dedup_key,
        }
    }
}

/// Convert Unix milliseconds timestamp to YYYY-MM-DD date string in local timezone
fn timestamp_to_date(timestamp_ms: i64) -> String {
    use chrono::{Local, TimeZone};

    let datetime = Local.timestamp_millis_opt(timestamp_ms);
    match datetime {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d").to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    fn expected_local_date(timestamp_ms: i64) -> String {
        Local
            .timestamp_millis_opt(timestamp_ms)
            .single()
            .map(|dt| dt.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    }

    #[test]
    fn test_timestamp_to_date() {
        // 2025-06-16 12:00:00 UTC (1750075200 seconds since epoch)
        let ts = 1750075200000_i64;
        let date = timestamp_to_date(ts);
        let expected = expected_local_date(ts);
        assert_eq!(date, expected);
    }

    #[test]
    fn test_timestamp_to_date_epoch() {
        let ts = 0_i64;
        let date = timestamp_to_date(ts);
        let expected = expected_local_date(ts);
        assert_eq!(date, expected);
    }

    #[test]
    fn test_timestamp_to_date_recent() {
        // 2024-12-01 00:00:00 UTC
        let ts = 1733011200000_i64;
        let date = timestamp_to_date(ts);
        let expected = expected_local_date(ts);
        assert_eq!(date, expected);
    }

    #[test]
    fn test_timestamp_to_date_format() {
        let ts = 1733011200000_i64;
        let date = timestamp_to_date(ts);
        assert!(
            date.len() == 10
                && date.chars().nth(4) == Some('-')
                && date.chars().nth(7) == Some('-'),
            "Date should be in YYYY-MM-DD format, got: {}",
            date
        );
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

        let ts = 1733011200000_i64;
        let msg = UnifiedMessage::new(
            "opencode",
            "claude-3-5-sonnet",
            "anthropic",
            "test-session-id",
            ts,
            tokens,
            0.05,
        );

        assert_eq!(msg.source, "opencode");
        assert_eq!(msg.model_id, "claude-3-5-sonnet");
        assert_eq!(msg.session_id, "test-session-id");
        assert_eq!(msg.date, expected_local_date(ts));
        assert_eq!(msg.cost, 0.05);
        assert_eq!(msg.agent, None);
    }

    #[test]
    fn test_normalize_agent_name() {
        assert_eq!(normalize_agent_name("OmO"), "Sisyphus");
        assert_eq!(normalize_agent_name("Sisyphus"), "Sisyphus");
        assert_eq!(normalize_agent_name("omo"), "Sisyphus");
        assert_eq!(normalize_agent_name("sisyphus"), "Sisyphus");

        assert_eq!(normalize_agent_name("OmO-Plan"), "Planner-Sisyphus");
        assert_eq!(normalize_agent_name("Planner-Sisyphus"), "Planner-Sisyphus");
        assert_eq!(normalize_agent_name("omo-plan"), "Planner-Sisyphus");

        assert_eq!(normalize_agent_name("explore"), "explore");
        assert_eq!(normalize_agent_name("CustomAgent"), "CustomAgent");
    }
}
