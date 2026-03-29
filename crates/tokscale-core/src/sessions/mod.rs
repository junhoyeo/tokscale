//! Session parsers for different AI coding assistant formats
//!
//! Each client has its own parser that converts to a unified message format.

pub mod amp;
pub mod claudecode;
pub mod codex;
pub mod cursor;
pub mod droid;
pub mod gemini;
pub mod kilo;
pub mod kilocode;
pub mod kimi;
pub mod mux;
pub mod openclaw;
pub mod opencode;
pub mod pi;
pub mod qwen;
pub mod roocode;
pub mod synthetic;
pub(crate) mod utils;

use crate::TokenBreakdown;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UnifiedMessage {
    pub client: String,
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
    let trimmed = agent.trim();
    let stripped = strip_agent_prefix(trimmed);
    let agent_lower = stripped.to_lowercase();

    if agent_lower.contains("plan") {
        if agent_lower.contains("omo") || agent_lower.contains("sisyphus") {
            return "Planner-Sisyphus".to_string();
        }
        return titlecase_agent(stripped);
    }

    if agent_lower == "omo" || agent_lower == "sisyphus" {
        return "Sisyphus".to_string();
    }

    if agent_lower == "orchestrator-sisyphus" {
        return "Atlas".to_string();
    }

    titlecase_agent(stripped)
}

pub fn normalize_opencode_agent_name(agent: &str) -> String {
    let trimmed = agent.trim();
    let stripped = strip_agent_prefix(trimmed);
    let agent_lower = stripped.to_lowercase();

    if let Some(normalized) = normalize_oh_my_opencode_agent_name(&agent_lower) {
        return normalized;
    }

    normalize_agent_name(stripped)
}

fn normalize_oh_my_opencode_agent_name(agent_lower: &str) -> Option<String> {
    let normalized = match agent_lower {
        "sisyphus (ultraworker)" | "sisyphus" => "Sisyphus",
        "orchestrator-sisyphus" => "Atlas",
        "hephaestus (deep agent)" | "hephaestus" => "Hephaestus",
        "prometheus (plan builder)" | "prometheus (planner)" | "prometheus" => "Prometheus",
        "atlas (plan executor)" | "atlas" => "Atlas",
        "metis (plan consultant)" | "metis" => "Metis",
        "momus (plan critic)" | "momus (plan reviewer)" | "momus" => "Momus",
        "sisyphus-junior" => "Sisyphus-Junior",
        "planner-sisyphus" => "Planner-Sisyphus",
        _ => return None,
    };

    Some(normalized.to_string())
}

fn strip_agent_prefix(name: &str) -> &str {
    for prefix in &["astrape:", "oh-my-claudecode:", "oh-my-codex:"] {
        if name
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        {
            return &name[prefix.len()..];
        }
    }
    name
}

fn titlecase_word(word: &str) -> String {
    match word.to_lowercase().as_str() {
        "ui" => "UI".to_string(),
        "ux" => "UX".to_string(),
        "api" => "API".to_string(),
        _ => {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + &chars.collect::<String>()
                }
            }
        }
    }
}

fn titlecase_agent(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    name.split('-')
        .map(titlecase_word)
        .collect::<Vec<_>>()
        .join(" ")
}

impl UnifiedMessage {
    pub fn new(
        client: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
    ) -> Self {
        Self::new_full(
            client,
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

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_agent(
        client: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
        agent: Option<String>,
    ) -> Self {
        Self::new_full(
            client,
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

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_dedup(
        client: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
        timestamp: i64,
        tokens: TokenBreakdown,
        cost: f64,
        dedup_key: Option<String>,
    ) -> Self {
        Self::new_full(
            client,
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

    #[allow(clippy::too_many_arguments)]
    fn new_full(
        client: impl Into<String>,
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
            client: client.into(),
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

    pub(crate) fn refresh_derived_fields(&mut self) {
        self.date = timestamp_to_date(self.timestamp);
    }

    pub(crate) fn set_timestamp(&mut self, timestamp: i64) {
        self.timestamp = timestamp;
        self.refresh_derived_fields();
    }
}

/// Convert Unix milliseconds to a local YYYY-MM-DD date string.
fn timestamp_to_date(timestamp_ms: i64) -> String {
    timestamp_to_date_with_timezone(timestamp_ms, &chrono::Local)
}

fn timestamp_to_date_with_timezone<Tz>(timestamp_ms: i64, timezone: &Tz) -> String
where
    Tz: chrono::TimeZone,
    Tz::Offset: std::fmt::Display,
{
    match timezone.timestamp_millis_opt(timestamp_ms) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d").to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn test_timestamp_to_date_with_positive_offset() {
        let kst = FixedOffset::east_opt(9 * 60 * 60).unwrap();
        let ts = 1772512200000_i64; // 2026-03-03T04:30:00Z
        let date = timestamp_to_date_with_timezone(ts, &kst);
        assert_eq!(date, "2026-03-03");
    }

    #[test]
    fn test_timestamp_to_date_with_negative_offset() {
        let pst = FixedOffset::west_opt(8 * 60 * 60).unwrap();
        let ts = 1772512200000_i64; // 2026-03-03T04:30:00Z
        let date = timestamp_to_date_with_timezone(ts, &pst);
        assert_eq!(date, "2026-03-02");
    }

    #[test]
    fn test_timestamp_to_date_invalid_timestamp() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let date = timestamp_to_date_with_timezone(i64::MAX, &utc);
        assert_eq!(date, "");
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

        assert_eq!(msg.client, "opencode");
        assert_eq!(msg.model_id, "claude-3-5-sonnet");
        assert_eq!(msg.session_id, "test-session-id");
        assert_eq!(msg.date, timestamp_to_date(1733011200000));
        assert_eq!(msg.cost, 0.05);
        assert_eq!(msg.agent, None);
    }

    #[test]
    fn test_normalize_agent_name() {
        assert_eq!(normalize_agent_name("OmO"), "Sisyphus");
        assert_eq!(normalize_agent_name("Sisyphus"), "Sisyphus");
        assert_eq!(normalize_agent_name("omo"), "Sisyphus");
        assert_eq!(normalize_agent_name("sisyphus"), "Sisyphus");
        assert_eq!(
            normalize_agent_name("Sisyphus (Ultraworker)"),
            "Sisyphus (Ultraworker)"
        );

        assert_eq!(
            normalize_opencode_agent_name("Sisyphus (Ultraworker)"),
            "Sisyphus"
        );
        assert_eq!(normalize_opencode_agent_name("hephaestus"), "Hephaestus");
        assert_eq!(normalize_opencode_agent_name("prometheus"), "Prometheus");
        assert_eq!(normalize_opencode_agent_name("atlas"), "Atlas");
        assert_eq!(normalize_opencode_agent_name("metis"), "Metis");
        assert_eq!(normalize_opencode_agent_name("momus"), "Momus");
        assert_eq!(
            normalize_opencode_agent_name("sisyphus-junior"),
            "Sisyphus-Junior"
        );
        assert_eq!(
            normalize_opencode_agent_name("planner-sisyphus"),
            "Planner-Sisyphus"
        );

        assert_eq!(
            normalize_opencode_agent_name("Hephaestus (Deep Agent)"),
            "Hephaestus"
        );
        assert_eq!(
            normalize_opencode_agent_name("Prometheus (Plan Builder)"),
            "Prometheus"
        );
        assert_eq!(
            normalize_opencode_agent_name("Prometheus (Planner)"),
            "Prometheus"
        );
        assert_eq!(
            normalize_opencode_agent_name("Atlas (Plan Executor)"),
            "Atlas"
        );
        assert_eq!(
            normalize_opencode_agent_name("Metis (Plan Consultant)"),
            "Metis"
        );
        assert_eq!(
            normalize_opencode_agent_name("Momus (Plan Critic)"),
            "Momus"
        );
        assert_eq!(
            normalize_opencode_agent_name("Momus (Plan Reviewer)"),
            "Momus"
        );

        assert_eq!(normalize_agent_name("OmO-Plan"), "Planner-Sisyphus");
        assert_eq!(normalize_agent_name("Planner-Sisyphus"), "Planner-Sisyphus");
        assert_eq!(normalize_agent_name("omo-plan"), "Planner-Sisyphus");

        assert_eq!(normalize_agent_name("orchestrator-sisyphus"), "Atlas");
        assert_eq!(
            normalize_opencode_agent_name("orchestrator-sisyphus"),
            "Atlas"
        );
        assert_eq!(normalize_agent_name("explore"), "Explore");
        assert_eq!(normalize_agent_name("CustomAgent"), "CustomAgent");

        assert_eq!(normalize_agent_name("executor"), "Executor");
        assert_eq!(
            normalize_agent_name("task-orchestrator"),
            "Task Orchestrator"
        );
        assert_eq!(normalize_agent_name("git-committer"), "Git Committer");
        assert_eq!(
            normalize_agent_name("frontend-ui-ux-engineer"),
            "Frontend UI UX Engineer"
        );
        assert_eq!(
            normalize_agent_name("astrape:executor-high"),
            "Executor High"
        );
        assert_eq!(
            normalize_agent_name("oh-my-claudecode:code-reviewer"),
            "Code Reviewer"
        );
        assert_eq!(normalize_agent_name("oh-my-codex:librarian"), "Librarian");
        assert_eq!(normalize_agent_name("astrape:executor"), "Executor");
        assert_eq!(normalize_agent_name("plan-reviewer"), "Plan Reviewer");
        assert_eq!(normalize_agent_name("astrape:planner"), "Planner");

        assert_eq!(
            normalize_opencode_agent_name("astrape:sisyphus"),
            "Sisyphus"
        );
        assert_eq!(
            normalize_opencode_agent_name("oh-my-claudecode:executor"),
            "Executor"
        );
    }
}
