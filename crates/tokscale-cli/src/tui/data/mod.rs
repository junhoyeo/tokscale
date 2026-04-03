use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use tokio::runtime::{Handle, Runtime};

use tokscale_core::sessions::UnifiedMessage;
use tokscale_core::{
    normalize_model_for_grouping, parse_local_unified_messages, sessions, ClientId, GroupBy,
    LocalParseOptions,
};

#[derive(Debug, Clone, Default)]
pub struct TokenBreakdown {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

impl TokenBreakdown {
    pub fn total(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
            .saturating_add(self.reasoning)
    }
}

#[derive(Debug, Clone)]
pub struct ModelUsage {
    pub model: String,
    pub provider: String,
    pub client: String,
    pub workspace_key: Option<String>,
    pub workspace_label: Option<String>,
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub session_count: u32,
}

#[derive(Debug, Clone)]
pub struct AgentUsage {
    pub agent: String,
    pub clients: String,
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub message_count: u32,
}

#[derive(Debug, Clone)]
pub struct DailyModelInfo {
    pub client: String,
    pub display_name: String,
    pub color_key: String,
    pub tokens: TokenBreakdown,
    pub cost: f64,
}

#[derive(Debug, Clone)]
pub struct DailyUsage {
    pub date: NaiveDate,
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub models: BTreeMap<String, DailyModelInfo>,
}

#[derive(Debug, Clone)]
pub struct ContributionDay {
    pub date: NaiveDate,
    pub tokens: u64,
    pub cost: f64,
    pub intensity: f64,
}

#[derive(Debug, Clone)]
pub struct GraphData {
    pub weeks: Vec<Vec<Option<ContributionDay>>>,
}

#[derive(Debug, Clone, Default)]
pub struct UsageData {
    pub models: Vec<ModelUsage>,
    pub agents: Vec<AgentUsage>,
    pub daily: Vec<DailyUsage>,
    pub graph: Option<GraphData>,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub loading: bool,
    pub error: Option<String>,
    pub current_streak: u32,
    pub longest_streak: u32,
}

pub struct DataLoader {
    _sessions_path: Option<PathBuf>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
}

const UNKNOWN_WORKSPACE_LABEL: &str = "Unknown workspace";
const UNKNOWN_WORKSPACE_GROUP_KEY: &str = "\0unknown-workspace";

fn workspace_bucket(msg: &UnifiedMessage) -> (String, Option<String>, String) {
    match (&msg.workspace_key, &msg.workspace_label) {
        (Some(key), Some(label)) => (key.clone(), Some(key.clone()), label.clone()),
        (Some(key), None) => (
            key.clone(),
            Some(key.clone()),
            tokscale_core::sessions::workspace_label_from_key(key)
                .unwrap_or_else(|| UNKNOWN_WORKSPACE_LABEL.to_string()),
        ),
        _ => (
            UNKNOWN_WORKSPACE_GROUP_KEY.to_string(),
            None,
            UNKNOWN_WORKSPACE_LABEL.to_string(),
        ),
    }
}

fn workspace_model_display_label(workspace_label: &str, model: &str) -> String {
    format!("{workspace_label} / {model}")
}

fn workspace_model_daily_key(workspace_group_key: &str, model: &str) -> String {
    format!(
        "{}:{workspace_group_key}:{model}",
        workspace_group_key.len()
    )
}

impl DataLoader {
    pub fn new(sessions_path: Option<PathBuf>) -> Self {
        Self {
            _sessions_path: sessions_path,
            since: None,
            until: None,
            year: None,
        }
    }

    pub fn with_filters(
        sessions_path: Option<PathBuf>,
        since: Option<String>,
        until: Option<String>,
        year: Option<String>,
    ) -> Self {
        Self {
            _sessions_path: sessions_path,
            since,
            until,
            year,
        }
    }

    pub fn load(
        &self,
        enabled_clients: &[ClientId],
        group_by: &GroupBy,
        include_synthetic: bool,
    ) -> Result<UsageData> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?
            .to_string_lossy()
            .to_string();

        let mut sources: Vec<String> = enabled_clients
            .iter()
            .map(|client| client.as_str().to_string())
            .collect();
        if include_synthetic {
            sources.push("synthetic".to_string());
        }

        let opts = LocalParseOptions {
            home_dir: Some(home),
            use_env_roots: true,
            clients: Some(sources),
            since: self.since.clone(),
            until: self.until.clone(),
            year: self.year.clone(),
        };

        let messages = if Handle::try_current().is_ok() {
            std::thread::scope(|s| {
                s.spawn(|| {
                    let rt = Runtime::new().map_err(|e| e.to_string())?;
                    rt.block_on(parse_local_unified_messages(opts))
                })
                .join()
                .unwrap_or_else(|_| Err("data loader thread panicked".to_string()))
            })
        } else {
            Runtime::new()?.block_on(parse_local_unified_messages(opts))
        }
        .map_err(anyhow::Error::msg)?;

        self.aggregate_messages(messages, group_by)
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn load_with_pricing(
        &self,
        enabled_clients: &[ClientId],
        group_by: &GroupBy,
        include_synthetic: bool,
        pricing: &tokscale_core::pricing::PricingService,
    ) -> Result<UsageData> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?
            .to_string_lossy()
            .to_string();

        let mut sources: Vec<String> = enabled_clients
            .iter()
            .map(|client| client.as_str().to_string())
            .collect();
        if include_synthetic {
            sources.push("synthetic".to_string());
        }

        let opts = LocalParseOptions {
            home_dir: Some(home),
            clients: Some(sources),
            since: self.since.clone(),
            until: self.until.clone(),
            year: self.year.clone(),
            use_env_roots: false,
        };

        let messages = if Handle::try_current().is_ok() {
            std::thread::scope(|s| {
                s.spawn(|| {
                    let rt = Runtime::new().map_err(|e| e.to_string())?;
                    rt.block_on(tokscale_core::parse_local_unified_messages_with_pricing(
                        opts,
                        Some(pricing),
                    ))
                })
                .join()
                .unwrap_or_else(|_| Err("data loader thread panicked".to_string()))
            })
        } else {
            Runtime::new()?.block_on(tokscale_core::parse_local_unified_messages_with_pricing(
                opts,
                Some(pricing),
            ))
        }
        .map_err(anyhow::Error::msg)?;

        self.aggregate_messages(messages, group_by)
    }

    fn aggregate_messages(
        &self,
        messages: Vec<UnifiedMessage>,
        group_by: &GroupBy,
    ) -> Result<UsageData> {
        let mut model_map: HashMap<String, ModelUsage> = HashMap::new();
        let mut agent_map: HashMap<String, AgentUsage> = HashMap::new();
        let mut agent_clients: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut daily_map: HashMap<NaiveDate, DailyUsage> = HashMap::new();
        let mut model_session_ids: HashMap<String, HashSet<String>> = HashMap::new();

        for msg in &messages {
            let normalized_model = normalize_model_for_grouping(&msg.model_id);
            let (workspace_group_key, workspace_key, workspace_label) = workspace_bucket(msg);
            let key = match group_by {
                GroupBy::Model => normalized_model.clone(),
                GroupBy::ClientModel => format!("{}:{}", msg.client, normalized_model),
                GroupBy::ClientProviderModel => {
                    format!("{}:{}:{}", msg.client, msg.provider_id, normalized_model)
                }
                GroupBy::WorkspaceModel => {
                    format!("{}:{}", workspace_group_key, normalized_model)
                }
            };
            let merge_clients = matches!(group_by, GroupBy::Model | GroupBy::WorkspaceModel);

            let model_entry = model_map.entry(key.clone()).or_insert_with(|| ModelUsage {
                model: normalized_model.clone(),
                provider: msg.provider_id.clone(),
                client: msg.client.clone(),
                workspace_key: if *group_by == GroupBy::WorkspaceModel {
                    workspace_key.clone()
                } else {
                    None
                },
                workspace_label: if *group_by == GroupBy::WorkspaceModel {
                    Some(workspace_label.clone())
                } else {
                    None
                },
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                session_count: 0,
            });

            if merge_clients && !model_entry.client.split(", ").any(|s| s == msg.client) {
                model_entry.client = format!("{}, {}", model_entry.client, msg.client);
            }

            if *group_by != GroupBy::ClientProviderModel
                && !model_entry
                    .provider
                    .split(", ")
                    .any(|p| p == msg.provider_id)
            {
                model_entry.provider = format!("{}, {}", model_entry.provider, msg.provider_id);
            }

            model_entry.tokens.input = model_entry
                .tokens
                .input
                .saturating_add(msg.tokens.input.max(0) as u64);
            model_entry.tokens.output = model_entry
                .tokens
                .output
                .saturating_add(msg.tokens.output.max(0) as u64);
            model_entry.tokens.cache_read = model_entry
                .tokens
                .cache_read
                .saturating_add(msg.tokens.cache_read.max(0) as u64);
            model_entry.tokens.cache_write = model_entry
                .tokens
                .cache_write
                .saturating_add(msg.tokens.cache_write.max(0) as u64);
            model_entry.tokens.reasoning = model_entry
                .tokens
                .reasoning
                .saturating_add(msg.tokens.reasoning.max(0) as u64);
            let msg_cost = if msg.cost.is_finite() && msg.cost >= 0.0 {
                msg.cost
            } else {
                0.0
            };
            model_entry.cost += msg_cost;

            let session_key = format!("{}:{}", msg.client, msg.session_id);
            let model_sessions = model_session_ids.entry(key).or_default();
            if model_sessions.insert(session_key) {
                model_entry.session_count += 1;
            }

            if let Some(agent) = msg.agent.as_ref() {
                let normalized_agent = if msg.client == "opencode" {
                    sessions::normalize_opencode_agent_name(agent)
                } else {
                    sessions::normalize_agent_name(agent)
                };
                let agent_entry = agent_map
                    .entry(normalized_agent.clone())
                    .or_insert_with(|| AgentUsage {
                        agent: normalized_agent.clone(),
                        clients: String::new(),
                        tokens: TokenBreakdown::default(),
                        cost: 0.0,
                        message_count: 0,
                    });

                agent_entry.tokens.input = agent_entry
                    .tokens
                    .input
                    .saturating_add(msg.tokens.input.max(0) as u64);
                agent_entry.tokens.output = agent_entry
                    .tokens
                    .output
                    .saturating_add(msg.tokens.output.max(0) as u64);
                agent_entry.tokens.cache_read = agent_entry
                    .tokens
                    .cache_read
                    .saturating_add(msg.tokens.cache_read.max(0) as u64);
                agent_entry.tokens.cache_write = agent_entry
                    .tokens
                    .cache_write
                    .saturating_add(msg.tokens.cache_write.max(0) as u64);
                agent_entry.tokens.reasoning = agent_entry
                    .tokens
                    .reasoning
                    .saturating_add(msg.tokens.reasoning.max(0) as u64);
                agent_entry.cost += msg_cost;
                agent_entry.message_count = agent_entry
                    .message_count
                    .saturating_add(msg.message_count.max(0) as u32);

                agent_clients
                    .entry(normalized_agent)
                    .or_default()
                    .insert(msg.client.clone());
            }

            if let Some(date) = parse_date(&msg.date) {
                let daily_entry = daily_map.entry(date).or_insert_with(|| DailyUsage {
                    date,
                    tokens: TokenBreakdown::default(),
                    cost: 0.0,
                    models: BTreeMap::new(),
                });

                daily_entry.tokens.input = daily_entry
                    .tokens
                    .input
                    .saturating_add(msg.tokens.input.max(0) as u64);
                daily_entry.tokens.output = daily_entry
                    .tokens
                    .output
                    .saturating_add(msg.tokens.output.max(0) as u64);
                daily_entry.tokens.cache_read = daily_entry
                    .tokens
                    .cache_read
                    .saturating_add(msg.tokens.cache_read.max(0) as u64);
                daily_entry.tokens.cache_write = daily_entry
                    .tokens
                    .cache_write
                    .saturating_add(msg.tokens.cache_write.max(0) as u64);
                daily_entry.tokens.reasoning = daily_entry
                    .tokens
                    .reasoning
                    .saturating_add(msg.tokens.reasoning.max(0) as u64);
                let msg_cost = if msg.cost.is_finite() && msg.cost >= 0.0 {
                    msg.cost
                } else {
                    0.0
                };
                daily_entry.cost += msg_cost;

                let daily_model_key = if *group_by == GroupBy::WorkspaceModel {
                    workspace_model_daily_key(&workspace_group_key, &normalized_model)
                } else {
                    normalized_model.clone()
                };

                let model_info = daily_entry
                    .models
                    .entry(daily_model_key)
                    .or_insert_with(|| DailyModelInfo {
                        client: msg.client.clone(),
                        display_name: if *group_by == GroupBy::WorkspaceModel {
                            workspace_model_display_label(&workspace_label, &normalized_model)
                        } else {
                            normalized_model.clone()
                        },
                        color_key: normalized_model.clone(),
                        tokens: TokenBreakdown::default(),
                        cost: 0.0,
                    });

                model_info.tokens.input = model_info
                    .tokens
                    .input
                    .saturating_add(msg.tokens.input.max(0) as u64);
                model_info.tokens.output = model_info
                    .tokens
                    .output
                    .saturating_add(msg.tokens.output.max(0) as u64);
                model_info.tokens.cache_read = model_info
                    .tokens
                    .cache_read
                    .saturating_add(msg.tokens.cache_read.max(0) as u64);
                model_info.tokens.cache_write = model_info
                    .tokens
                    .cache_write
                    .saturating_add(msg.tokens.cache_write.max(0) as u64);
                model_info.tokens.reasoning = model_info
                    .tokens
                    .reasoning
                    .saturating_add(msg.tokens.reasoning.max(0) as u64);
                let model_msg_cost = if msg.cost.is_finite() && msg.cost >= 0.0 {
                    msg.cost
                } else {
                    0.0
                };
                model_info.cost += model_msg_cost;
            }
        }

        let mut models: Vec<ModelUsage> = model_map.into_values().collect();
        models.sort_by(|a, b| {
            b.cost
                .total_cmp(&a.cost)
                .then_with(|| a.model.cmp(&b.model))
                .then_with(|| a.provider.cmp(&b.provider))
                .then_with(|| a.client.cmp(&b.client))
        });

        for (agent, clients) in agent_clients {
            if let Some(agent_entry) = agent_map.get_mut(&agent) {
                agent_entry.clients = clients.into_iter().collect::<Vec<_>>().join(", ");
            }
        }

        let mut agents: Vec<AgentUsage> = agent_map.into_values().collect();
        agents.sort_by(|a, b| {
            b.cost
                .total_cmp(&a.cost)
                .then_with(|| b.tokens.total().cmp(&a.tokens.total()))
                .then_with(|| a.agent.cmp(&b.agent))
        });

        let mut daily: Vec<DailyUsage> = daily_map.into_values().collect();
        daily.sort_by(|a, b| b.date.cmp(&a.date));

        let total_tokens: u64 = models.iter().map(|m| m.tokens.total()).sum();
        let total_cost: f64 = models
            .iter()
            .map(|m| if m.cost.is_finite() { m.cost } else { 0.0 })
            .sum();

        let graph = build_contribution_graph(&daily);
        let (current_streak, longest_streak) = calculate_streaks(&daily);

        Ok(UsageData {
            models,
            agents,
            daily,
            graph: Some(graph),
            total_tokens,
            total_cost,
            loading: false,
            error: None,
            current_streak,
            longest_streak,
        })
    }
}

fn parse_date(date_str: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()
}

fn build_contribution_graph(daily: &[DailyUsage]) -> GraphData {
    build_contribution_graph_for_today(daily, Local::now().date_naive())
}

fn build_contribution_graph_for_today(daily: &[DailyUsage], today: NaiveDate) -> GraphData {
    if daily.is_empty() {
        return GraphData { weeks: vec![] };
    }

    let days_to_sunday = today.weekday().num_days_from_sunday();
    let end_date = today;
    let start_date = end_date - chrono::Duration::days(364 + days_to_sunday as i64);

    let daily_map: HashMap<NaiveDate, &DailyUsage> = daily.iter().map(|d| (d.date, d)).collect();

    let max_cost = daily.iter().map(|d| d.cost).fold(0.0_f64, |a, b| a.max(b));

    let mut weeks: Vec<Vec<Option<ContributionDay>>> = Vec::new();
    let mut current_week: Vec<Option<ContributionDay>> = Vec::new();

    let mut current_date = start_date;
    while current_date <= end_date {
        let day = if let Some(usage) = daily_map.get(&current_date) {
            let raw_intensity = if max_cost > 0.0 {
                usage.cost / max_cost
            } else {
                0.0
            };
            let intensity = if raw_intensity.is_finite() {
                raw_intensity.clamp(0.0, 1.0)
            } else {
                0.0
            };
            Some(ContributionDay {
                date: current_date,
                tokens: usage.tokens.total(),
                cost: usage.cost,
                intensity,
            })
        } else {
            Some(ContributionDay {
                date: current_date,
                tokens: 0,
                cost: 0.0,
                intensity: 0.0,
            })
        };

        current_week.push(day);

        if current_date.weekday() == chrono::Weekday::Sat || current_date == end_date {
            weeks.push(current_week);
            current_week = Vec::new();
        }

        current_date += chrono::Duration::days(1);
    }

    GraphData { weeks }
}

fn calculate_streaks(daily: &[DailyUsage]) -> (u32, u32) {
    calculate_streaks_for_today(daily, Local::now().date_naive())
}

fn calculate_streaks_for_today(daily: &[DailyUsage], today: NaiveDate) -> (u32, u32) {
    if daily.is_empty() {
        return (0, 0);
    }

    let dates: HashSet<NaiveDate> = daily.iter().map(|d| d.date).collect();

    let mut current_streak = 0u32;
    let mut check_date = today;

    while dates.contains(&check_date) {
        current_streak += 1;
        check_date -= chrono::Duration::days(1);
    }

    if current_streak == 0 {
        let yesterday = today - chrono::Duration::days(1);
        check_date = yesterday;
        while dates.contains(&check_date) {
            current_streak += 1;
            check_date -= chrono::Duration::days(1);
        }
    }

    let mut longest_streak = 0u32;
    let mut sorted_dates: Vec<NaiveDate> = dates.into_iter().collect();
    sorted_dates.sort();

    let mut streak = 0u32;
    let mut prev_date: Option<NaiveDate> = None;

    for date in sorted_dates {
        if let Some(prev) = prev_date {
            if date == prev + chrono::Duration::days(1) {
                streak += 1;
            } else {
                longest_streak = longest_streak.max(streak);
                streak = 1;
            }
        } else {
            streak = 1;
        }
        prev_date = Some(date);
    }
    longest_streak = longest_streak.max(streak);

    (current_streak, longest_streak)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::collections::HashMap;
    use std::env;
    use std::fs;
    use tempfile::TempDir;
    use tokio::runtime::{Handle, Runtime};
    use tokscale_core::parse_local_unified_messages_with_pricing;
    use tokscale_core::pricing::{ModelPricing, PricingService};
    use tokscale_core::TokenBreakdown as CoreTokenBreakdown;

    fn test_pricing_service() -> PricingService {
        let mut litellm = HashMap::new();
        litellm.insert(
            "claude-sonnet-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00001),
                output_cost_per_token: Some(0.00002),
                cache_read_input_token_cost: Some(0.000003),
                ..Default::default()
            },
        );
        litellm.insert(
            "claude-haiku-4".into(),
            ModelPricing {
                input_cost_per_token: Some(0.000004),
                output_cost_per_token: Some(0.000006),
                cache_read_input_token_cost: Some(0.000001),
                ..Default::default()
            },
        );
        litellm.insert(
            "accounts/fireworks/models/deepseek-v3-0324".into(),
            ModelPricing {
                input_cost_per_token: Some(0.01),
                output_cost_per_token: Some(0.03),
                ..Default::default()
            },
        );

        PricingService::new(litellm, HashMap::new())
    }

    fn load_with_pricing(
        loader: &DataLoader,
        enabled_clients: &[ClientId],
        group_by: &GroupBy,
        include_synthetic: bool,
        pricing: Option<&PricingService>,
    ) -> Result<UsageData> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?
            .to_string_lossy()
            .to_string();

        let mut sources: Vec<String> = enabled_clients
            .iter()
            .map(|client| client.as_str().to_string())
            .collect();
        if include_synthetic {
            sources.push("synthetic".to_string());
        }

        let opts = LocalParseOptions {
            home_dir: Some(home),
            use_env_roots: true,
            clients: Some(sources),
            since: loader.since.clone(),
            until: loader.until.clone(),
            year: loader.year.clone(),
        };

        let messages = if Handle::try_current().is_ok() {
            std::thread::scope(|s| {
                s.spawn(|| {
                    let rt = Runtime::new().map_err(|e| e.to_string())?;
                    rt.block_on(parse_local_unified_messages_with_pricing(opts, pricing))
                })
                .join()
                .unwrap_or_else(|_| Err("data loader thread panicked".to_string()))
            })
        } else {
            Runtime::new()?.block_on(parse_local_unified_messages_with_pricing(opts, pricing))
        }
        .map_err(anyhow::Error::msg)?;

        loader.aggregate_messages(messages, group_by)
    }

    fn expected_message_cost(
        pricing: &PricingService,
        model_id: &str,
        provider_id: &str,
        tokens: CoreTokenBreakdown,
    ) -> f64 {
        pricing.calculate_cost_with_provider(model_id, Some(provider_id), &tokens)
    }

    fn assert_cost_matches(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected cost {expected}, got {actual}"
        );
    }

    fn make_workspace_message(
        client: &str,
        model_id: &str,
        provider_id: &str,
        session_id: &str,
        cost: f64,
        workspace_key: Option<&str>,
        workspace_label: Option<&str>,
    ) -> UnifiedMessage {
        let mut msg = UnifiedMessage::new(
            client,
            model_id,
            provider_id,
            session_id,
            1_735_689_600_000,
            tokscale_core::TokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost,
        );
        msg.set_workspace(
            workspace_key.map(str::to_string),
            workspace_label.map(str::to_string),
        );
        msg
    }

    #[test]
    fn test_client_all() {
        let clients = ClientId::ALL;
        assert_eq!(clients.len(), 17);
        assert_eq!(clients[0], ClientId::OpenCode);
        assert_eq!(clients[1], ClientId::Claude);
        assert_eq!(clients[2], ClientId::Codex);
        assert_eq!(clients[3], ClientId::Cursor);
        assert_eq!(clients[4], ClientId::Gemini);
        assert_eq!(clients[5], ClientId::Amp);
        assert_eq!(clients[6], ClientId::Droid);
        assert_eq!(clients[7], ClientId::OpenClaw);
        assert_eq!(clients[8], ClientId::Pi);
        assert_eq!(clients[9], ClientId::Kimi);
        assert_eq!(clients[10], ClientId::Qwen);
        assert_eq!(clients[11], ClientId::RooCode);
        assert_eq!(clients[12], ClientId::KiloCode);
        assert_eq!(clients[13], ClientId::Mux);
        assert_eq!(clients[14], ClientId::Kilo);
        assert_eq!(clients[15], ClientId::Crush);
        assert_eq!(clients[16], ClientId::Hermes);
    }

    #[test]
    fn test_client_as_str() {
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::OpenCode),
            "OpenCode"
        );
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::Claude),
            "Claude"
        );
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::Codex),
            "Codex"
        );
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::Cursor),
            "Cursor"
        );
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::Gemini),
            "Gemini"
        );
        assert_eq!(crate::tui::client_ui::display_name(ClientId::Amp), "Amp");
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::Droid),
            "Droid"
        );
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::OpenClaw),
            "OpenClaw"
        );
        assert_eq!(crate::tui::client_ui::display_name(ClientId::Pi), "Pi");
        assert_eq!(crate::tui::client_ui::display_name(ClientId::Kimi), "Kimi");
        assert_eq!(crate::tui::client_ui::display_name(ClientId::Qwen), "Qwen");
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::RooCode),
            "Roo Code"
        );
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::KiloCode),
            "KiloCode"
        );
        assert_eq!(crate::tui::client_ui::display_name(ClientId::Mux), "Mux");
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::Kilo),
            "Kilo CLI"
        );
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::Crush),
            "Crush"
        );
        assert_eq!(
            crate::tui::client_ui::display_name(ClientId::Hermes),
            "Hermes Agent"
        );
    }

    #[test]
    fn test_client_key() {
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::OpenCode), '1');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Claude), '2');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Codex), '3');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Cursor), '4');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Gemini), '5');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Amp), '6');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Droid), '7');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::OpenClaw), '8');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Pi), '9');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Kimi), '0');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Qwen), 'w');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::RooCode), 'r');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::KiloCode), 'k');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Mux), 'x');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Kilo), 'l');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Crush), 'h');
        assert_eq!(crate::tui::client_ui::hotkey(ClientId::Hermes), 'e');
    }

    #[test]
    fn test_client_from_key() {
        assert_eq!(
            crate::tui::client_ui::from_hotkey('1'),
            Some(ClientId::OpenCode)
        );
        assert_eq!(
            crate::tui::client_ui::from_hotkey('2'),
            Some(ClientId::Claude)
        );
        assert_eq!(
            crate::tui::client_ui::from_hotkey('3'),
            Some(ClientId::Codex)
        );
        assert_eq!(
            crate::tui::client_ui::from_hotkey('4'),
            Some(ClientId::Cursor)
        );
        assert_eq!(
            crate::tui::client_ui::from_hotkey('5'),
            Some(ClientId::Gemini)
        );
        assert_eq!(crate::tui::client_ui::from_hotkey('6'), Some(ClientId::Amp));
        assert_eq!(
            crate::tui::client_ui::from_hotkey('7'),
            Some(ClientId::Droid)
        );
        assert_eq!(
            crate::tui::client_ui::from_hotkey('8'),
            Some(ClientId::OpenClaw)
        );
        assert_eq!(crate::tui::client_ui::from_hotkey('9'), Some(ClientId::Pi));
        assert_eq!(
            crate::tui::client_ui::from_hotkey('0'),
            Some(ClientId::Kimi)
        );
        assert_eq!(
            crate::tui::client_ui::from_hotkey('w'),
            Some(ClientId::Qwen)
        );
        assert_eq!(
            crate::tui::client_ui::from_hotkey('r'),
            Some(ClientId::RooCode)
        );
        assert_eq!(
            crate::tui::client_ui::from_hotkey('k'),
            Some(ClientId::KiloCode)
        );
        assert_eq!(
            crate::tui::client_ui::from_hotkey('l'),
            Some(ClientId::Kilo)
        );
        assert_eq!(crate::tui::client_ui::from_hotkey('x'), Some(ClientId::Mux));
        assert_eq!(
            crate::tui::client_ui::from_hotkey('h'),
            Some(ClientId::Crush)
        );
        assert_eq!(
            crate::tui::client_ui::from_hotkey('e'),
            Some(ClientId::Hermes)
        );
        assert_eq!(crate::tui::client_ui::from_hotkey('a'), None);
    }

    #[test]
    fn test_token_breakdown_total() {
        let breakdown = TokenBreakdown {
            input: 100,
            output: 200,
            cache_read: 50,
            cache_write: 25,
            reasoning: 10,
        };
        assert_eq!(breakdown.total(), 385);
    }

    #[test]
    fn test_token_breakdown_total_with_overflow() {
        let breakdown = TokenBreakdown {
            input: u64::MAX,
            output: 1,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        };
        // saturating_add should prevent overflow
        assert_eq!(breakdown.total(), u64::MAX);
    }

    #[test]
    fn test_token_breakdown_default() {
        let breakdown = TokenBreakdown::default();
        assert_eq!(breakdown.input, 0);
        assert_eq!(breakdown.output, 0);
        assert_eq!(breakdown.cache_read, 0);
        assert_eq!(breakdown.cache_write, 0);
        assert_eq!(breakdown.reasoning, 0);
        assert_eq!(breakdown.total(), 0);
    }

    #[test]
    fn test_data_loader_new() {
        let loader = DataLoader::new(None);
        assert!(loader._sessions_path.is_none());
        assert!(loader.since.is_none());
        assert!(loader.until.is_none());
        assert!(loader.year.is_none());
    }

    #[test]
    fn test_data_loader_with_filters() {
        let loader = DataLoader::with_filters(
            Some(PathBuf::from("/tmp/sessions")),
            Some("2024-01-01".to_string()),
            Some("2024-12-31".to_string()),
            Some("2024".to_string()),
        );

        assert_eq!(loader._sessions_path, Some(PathBuf::from("/tmp/sessions")));
        assert_eq!(loader.since, Some("2024-01-01".to_string()));
        assert_eq!(loader.until, Some("2024-12-31".to_string()));
        assert_eq!(loader.year, Some("2024".to_string()));
    }

    #[test]
    fn test_parse_date() {
        assert_eq!(
            parse_date("2024-01-15"),
            Some(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap())
        );
        assert_eq!(
            parse_date("2024-12-31"),
            Some(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap())
        );
        assert_eq!(parse_date("invalid"), None);
        assert_eq!(parse_date("2024-13-01"), None);
        assert_eq!(parse_date(""), None);
    }

    #[test]
    fn test_build_contribution_graph_uses_provided_today() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let graph = build_contribution_graph_for_today(&[], today);
        assert!(graph.weeks.is_empty());

        let daily = vec![DailyUsage {
            date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            tokens: TokenBreakdown::default(),
            cost: 0.0,
            models: BTreeMap::new(),
        }];
        let graph = build_contribution_graph_for_today(&daily, today);
        let last_day = graph
            .weeks
            .last()
            .and_then(|week| week.last())
            .and_then(|day| day.as_ref())
            .map(|day| day.date);
        assert_eq!(last_day, Some(today));
    }

    #[test]
    fn test_aggregate_messages_builds_agent_usage() {
        let loader = DataLoader::new(None);
        let messages = vec![
            UnifiedMessage::new_with_agent(
                "opencode",
                "claude-sonnet-4",
                "anthropic",
                "session-1",
                1_735_689_600_000,
                tokscale_core::TokenBreakdown {
                    input: 10,
                    output: 5,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                1.25,
                Some("builder".to_string()),
            ),
            UnifiedMessage::new_with_agent(
                "roocode",
                "claude-sonnet-4",
                "anthropic",
                "session-2",
                1_735_689_700_000,
                tokscale_core::TokenBreakdown {
                    input: 20,
                    output: 10,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                2.75,
                Some("builder".to_string()),
            ),
        ];

        let usage = loader
            .aggregate_messages(messages, &GroupBy::Model)
            .unwrap();

        assert_eq!(usage.agents.len(), 1);
        assert_eq!(usage.agents[0].agent, "Builder");
        assert_eq!(usage.agents[0].clients, "opencode, roocode");
        assert_eq!(usage.agents[0].message_count, 2);
        assert!((usage.agents[0].cost - 4.0).abs() < f64::EPSILON);
        assert_eq!(usage.agents[0].tokens.total(), 45);
    }

    #[test]
    fn test_aggregate_messages_groups_by_workspace_and_model() {
        let loader = DataLoader::new(None);
        let usage = loader
            .aggregate_messages(
                vec![
                    make_workspace_message(
                        "claude",
                        "claude-sonnet-4-5-20250929",
                        "anthropic",
                        "session-1",
                        1.25,
                        Some("/repo-a"),
                        Some("repo-a"),
                    ),
                    make_workspace_message(
                        "qwen",
                        "claude-sonnet-4-5-20250929",
                        "anthropic",
                        "session-2",
                        2.75,
                        Some("/repo-a"),
                        Some("repo-a"),
                    ),
                ],
                &GroupBy::WorkspaceModel,
            )
            .unwrap();

        assert_eq!(usage.models.len(), 1);
        assert_eq!(usage.models[0].workspace_key.as_deref(), Some("/repo-a"));
        assert_eq!(usage.models[0].workspace_label.as_deref(), Some("repo-a"));
        assert_eq!(usage.models[0].model, "claude-sonnet-4-5");
        assert_eq!(usage.models[0].client, "claude, qwen");
        assert_eq!(usage.models[0].session_count, 2);
        assert_eq!(usage.models[0].cost, 4.0);
    }

    #[test]
    fn test_aggregate_messages_workspace_grouping_keeps_unknown_bucket_visible() {
        let loader = DataLoader::new(None);
        let usage = loader
            .aggregate_messages(
                vec![
                    make_workspace_message(
                        "claude",
                        "claude-sonnet-4-5-20250929",
                        "anthropic",
                        "session-1",
                        1.0,
                        None,
                        None,
                    ),
                    make_workspace_message(
                        "claude",
                        "claude-sonnet-4-5-20250929",
                        "anthropic",
                        "session-2",
                        2.0,
                        None,
                        None,
                    ),
                ],
                &GroupBy::WorkspaceModel,
            )
            .unwrap();

        assert_eq!(usage.models.len(), 1);
        assert_eq!(usage.models[0].workspace_key, None);
        assert_eq!(
            usage.models[0].workspace_label.as_deref(),
            Some(UNKNOWN_WORKSPACE_LABEL)
        );
        assert_eq!(usage.models[0].session_count, 2);
        assert_eq!(usage.models[0].cost, 3.0);
    }

    #[test]
    fn test_aggregate_messages_workspace_grouping_keeps_real_unknown_workspace_separate() {
        let loader = DataLoader::new(None);
        let usage = loader
            .aggregate_messages(
                vec![
                    make_workspace_message(
                        "claude",
                        "claude-sonnet-4-5-20250929",
                        "anthropic",
                        "session-1",
                        1.0,
                        Some("unknown-workspace"),
                        Some("unknown-workspace"),
                    ),
                    make_workspace_message(
                        "claude",
                        "claude-sonnet-4-5-20250929",
                        "anthropic",
                        "session-2",
                        2.0,
                        None,
                        None,
                    ),
                ],
                &GroupBy::WorkspaceModel,
            )
            .unwrap();

        assert_eq!(usage.models.len(), 2);
        assert!(usage.models.iter().any(|model| {
            model.workspace_key.as_deref() == Some("unknown-workspace")
                && model.workspace_label.as_deref() == Some("unknown-workspace")
                && (model.cost - 1.0).abs() < f64::EPSILON
        }));
        assert!(usage.models.iter().any(|model| {
            model.workspace_key.is_none()
                && model.workspace_label.as_deref() == Some(UNKNOWN_WORKSPACE_LABEL)
                && (model.cost - 2.0).abs() < f64::EPSILON
        }));
    }

    #[test]
    fn test_aggregate_messages_workspace_grouping_splits_daily_models_by_workspace() {
        let loader = DataLoader::new(None);
        let usage = loader
            .aggregate_messages(
                vec![
                    make_workspace_message(
                        "claude",
                        "claude-sonnet-4-5-20250929",
                        "anthropic",
                        "session-1",
                        1.0,
                        Some("/repo-a"),
                        Some("repo-a"),
                    ),
                    make_workspace_message(
                        "claude",
                        "claude-sonnet-4-5-20250929",
                        "anthropic",
                        "session-2",
                        2.0,
                        Some("/repo-b"),
                        Some("repo-b"),
                    ),
                ],
                &GroupBy::WorkspaceModel,
            )
            .unwrap();

        assert_eq!(usage.daily.len(), 1);
        let daily_keys: Vec<_> = usage.daily[0].models.keys().cloned().collect();
        assert_eq!(daily_keys.len(), 2);
        assert_ne!(daily_keys[0], daily_keys[1]);
        let daily_display_names: Vec<_> = usage.daily[0]
            .models
            .values()
            .map(|info| info.display_name.clone())
            .collect();
        assert_eq!(
            daily_display_names,
            vec![
                "repo-a / claude-sonnet-4-5".to_string(),
                "repo-b / claude-sonnet-4-5".to_string()
            ]
        );
    }

    #[test]
    fn test_aggregate_messages_workspace_grouping_disambiguates_identical_labels() {
        let loader = DataLoader::new(None);
        let usage = loader
            .aggregate_messages(
                vec![
                    make_workspace_message(
                        "claude",
                        "claude-sonnet-4-5-20250929",
                        "anthropic",
                        "session-1",
                        1.0,
                        Some("/srv/team-a/demo"),
                        Some("demo"),
                    ),
                    make_workspace_message(
                        "claude",
                        "claude-sonnet-4-5-20250929",
                        "anthropic",
                        "session-2",
                        2.0,
                        Some("/srv/team-b/demo"),
                        Some("demo"),
                    ),
                ],
                &GroupBy::WorkspaceModel,
            )
            .unwrap();

        assert_eq!(usage.daily.len(), 1);
        assert_eq!(usage.daily[0].models.len(), 2);
        let display_names: Vec<_> = usage.daily[0]
            .models
            .values()
            .map(|info| info.display_name.clone())
            .collect();
        assert_eq!(
            display_names,
            vec![
                "demo / claude-sonnet-4-5".to_string(),
                "demo / claude-sonnet-4-5".to_string()
            ]
        );
    }

    #[test]
    fn test_aggregate_messages_merges_oh_my_opencode_agent_variants() {
        let loader = DataLoader::new(None);
        let messages = vec![
            UnifiedMessage::new_with_agent(
                "opencode",
                "claude-opus-4-6",
                "anthropic",
                "session-1",
                1_735_689_600_000,
                tokscale_core::TokenBreakdown {
                    input: 10,
                    output: 5,
                    cache_read: 100,
                    cache_write: 20,
                    reasoning: 0,
                },
                1.5,
                Some("Sisyphus".to_string()),
            ),
            UnifiedMessage::new_with_agent(
                "opencode",
                "claude-opus-4-6",
                "anthropic",
                "session-2",
                1_735_689_700_000,
                tokscale_core::TokenBreakdown {
                    input: 20,
                    output: 10,
                    cache_read: 200,
                    cache_write: 40,
                    reasoning: 0,
                },
                2.5,
                Some("Sisyphus (Ultraworker)".to_string()),
            ),
        ];

        let usage = loader
            .aggregate_messages(messages, &GroupBy::Model)
            .unwrap();

        assert_eq!(usage.agents.len(), 1);
        assert_eq!(usage.agents[0].agent, "Sisyphus");
        assert_eq!(usage.agents[0].clients, "opencode");
        assert_eq!(usage.agents[0].message_count, 2);
        assert!((usage.agents[0].cost - 4.0).abs() < f64::EPSILON);
        assert_eq!(usage.agents[0].tokens.total(), 405);
    }

    #[test]
    fn test_aggregate_messages_merges_opencode_agent_case_variants() {
        let loader = DataLoader::new(None);
        let messages = vec![
            UnifiedMessage::new_with_agent(
                "opencode",
                "claude-opus-4-6",
                "anthropic",
                "session-1",
                1_735_689_600_000,
                tokscale_core::TokenBreakdown {
                    input: 10,
                    output: 5,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                1.5,
                Some("Hephaestus".to_string()),
            ),
            UnifiedMessage::new_with_agent(
                "opencode",
                "claude-opus-4-6",
                "anthropic",
                "session-2",
                1_735_689_700_000,
                tokscale_core::TokenBreakdown {
                    input: 20,
                    output: 10,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                2.5,
                Some("hephaestus".to_string()),
            ),
        ];

        let usage = loader
            .aggregate_messages(messages, &GroupBy::Model)
            .unwrap();

        assert_eq!(usage.agents.len(), 1);
        assert_eq!(usage.agents[0].agent, "Hephaestus");
        assert_eq!(usage.agents[0].clients, "opencode");
        assert_eq!(usage.agents[0].message_count, 2);
        assert!((usage.agents[0].cost - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_aggregate_messages_does_not_merge_omo_variants_for_non_opencode_clients() {
        let loader = DataLoader::new(None);
        let messages = vec![
            UnifiedMessage::new_with_agent(
                "claude",
                "claude-opus-4-6",
                "anthropic",
                "session-1",
                1_735_689_600_000,
                tokscale_core::TokenBreakdown {
                    input: 10,
                    output: 5,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                1.5,
                Some("Sisyphus".to_string()),
            ),
            UnifiedMessage::new_with_agent(
                "claude",
                "claude-opus-4-6",
                "anthropic",
                "session-2",
                1_735_689_700_000,
                tokscale_core::TokenBreakdown {
                    input: 20,
                    output: 10,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                2.5,
                Some("Sisyphus (Ultraworker)".to_string()),
            ),
        ];

        let usage = loader
            .aggregate_messages(messages, &GroupBy::Model)
            .unwrap();

        assert_eq!(usage.agents.len(), 2);
        assert!(usage.agents.iter().any(|agent| agent.agent == "Sisyphus"));
        assert!(usage
            .agents
            .iter()
            .any(|agent| agent.agent == "Sisyphus (Ultraworker)"));
    }

    #[test]
    #[serial]
    fn test_data_loader_loads_agent_usage_from_roocode_files() {
        let temp_dir = TempDir::new().unwrap();
        let previous_home = env::var_os("HOME");
        let task_root = temp_dir
            .path()
            .join(".config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks");

        let architect_dir = task_root.join("task-architect");
        fs::create_dir_all(&architect_dir).unwrap();
        fs::write(
            architect_dir.join("ui_messages.json"),
            r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-03-07T16:00:00Z",
    "text": "{\"cost\":8.4,\"tokensIn\":420000,\"tokensOut\":120000,\"cacheReads\":32000,\"cacheWrites\":0,\"apiProtocol\":\"anthropic\"}"
  },
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-03-07T16:05:00Z",
    "text": "{\"cost\":3.1,\"tokensIn\":90000,\"tokensOut\":60000,\"cacheReads\":12000,\"cacheWrites\":0,\"apiProtocol\":\"anthropic\"}"
  }
]"#,
        )
        .unwrap();
        fs::write(
            architect_dir.join("api_conversation_history.json"),
            r#"before
<environment_details>
<model>claude-sonnet-4</model>
<slug>architect</slug>
<name>Architect</name>
</environment_details>
after"#,
        )
        .unwrap();

        let reviewer_dir = task_root.join("task-reviewer");
        fs::create_dir_all(&reviewer_dir).unwrap();
        fs::write(
            reviewer_dir.join("ui_messages.json"),
            r#"[
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-03-07T17:00:00Z",
    "text": "{\"cost\":1.8,\"tokensIn\":70000,\"tokensOut\":26000,\"cacheReads\":8000,\"cacheWrites\":0,\"apiProtocol\":\"anthropic\"}"
  },
  {
    "type": "say",
    "say": "api_req_started",
    "ts": "2026-03-07T17:09:00Z",
    "text": "{\"cost\":0.9,\"tokensIn\":22000,\"tokensOut\":18000,\"cacheReads\":3000,\"cacheWrites\":0,\"apiProtocol\":\"anthropic\"}"
  }
]"#,
        )
        .unwrap();
        fs::write(
            reviewer_dir.join("api_conversation_history.json"),
            r#"before
<environment_details>
<model>claude-haiku-4</model>
<slug>reviewer</slug>
<name>Reviewer</name>
</environment_details>
after"#,
        )
        .unwrap();

        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let pricing = test_pricing_service();
        let loader = DataLoader::new(None);
        let usage = load_with_pricing(
            &loader,
            &[ClientId::RooCode],
            &GroupBy::Model,
            false,
            Some(&pricing),
        )
        .unwrap();

        let architect_expected = expected_message_cost(
            &pricing,
            "claude-sonnet-4",
            "anthropic",
            CoreTokenBreakdown {
                input: 420_000,
                output: 120_000,
                cache_read: 32_000,
                cache_write: 0,
                reasoning: 0,
            },
        ) + expected_message_cost(
            &pricing,
            "claude-sonnet-4",
            "anthropic",
            CoreTokenBreakdown {
                input: 90_000,
                output: 60_000,
                cache_read: 12_000,
                cache_write: 0,
                reasoning: 0,
            },
        );
        let reviewer_expected = expected_message_cost(
            &pricing,
            "claude-haiku-4",
            "anthropic",
            CoreTokenBreakdown {
                input: 70_000,
                output: 26_000,
                cache_read: 8_000,
                cache_write: 0,
                reasoning: 0,
            },
        ) + expected_message_cost(
            &pricing,
            "claude-haiku-4",
            "anthropic",
            CoreTokenBreakdown {
                input: 22_000,
                output: 18_000,
                cache_read: 3_000,
                cache_write: 0,
                reasoning: 0,
            },
        );

        assert_eq!(usage.agents.len(), 2);
        assert_eq!(usage.agents[0].agent, "Architect");
        assert_eq!(usage.agents[0].clients, "roocode");
        assert_eq!(usage.agents[0].message_count, 2);
        assert_cost_matches(usage.agents[0].cost, architect_expected);
        assert_eq!(usage.agents[0].tokens.total(), 734_000);

        assert_eq!(usage.agents[1].agent, "Reviewer");
        assert_eq!(usage.agents[1].message_count, 2);
        assert_cost_matches(usage.agents[1].cost, reviewer_expected);
        assert_eq!(usage.agents[1].tokens.total(), 147_000);

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    #[test]
    #[serial]
    fn test_data_loader_keeps_synthetic_gateway_messages_under_original_client() {
        let temp_dir = TempDir::new().unwrap();
        let previous_home = env::var_os("HOME");
        let message_dir = temp_dir
            .path()
            .join(".local/share/opencode/storage/message/project-1");
        fs::create_dir_all(&message_dir).unwrap();
        fs::write(
            message_dir.join("msg_001.json"),
            r#"{"id":"msg-1","sessionID":"session-1","role":"assistant","modelID":"accounts/fireworks/models/deepseek-v3-0324","providerID":"fireworks","cost":0.25,"tokens":{"input":10,"output":5,"reasoning":0,"cache":{"read":0,"write":0}},"time":{"created":1733011200000}}"#,
        )
        .unwrap();

        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let pricing = test_pricing_service();
        let loader = DataLoader::new(None);
        let usage = load_with_pricing(
            &loader,
            &[ClientId::OpenCode],
            &GroupBy::ClientProviderModel,
            true,
            Some(&pricing),
        )
        .unwrap();

        let expected_cost = expected_message_cost(
            &pricing,
            "accounts/fireworks/models/deepseek-v3-0324",
            "fireworks",
            CoreTokenBreakdown {
                input: 10,
                output: 5,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
        );

        assert_eq!(usage.models.len(), 1);
        assert_eq!(usage.models[0].client, "opencode");
        assert_eq!(usage.models[0].provider, "fireworks");
        assert_eq!(usage.models[0].model, "deepseek-v3-0324");
        assert_eq!(usage.models[0].tokens.total(), 15);
        assert_cost_matches(usage.models[0].cost, expected_cost);

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    #[test]
    fn test_calculate_streaks_uses_provided_today() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 3).unwrap();
        let daily = vec![
            DailyUsage {
                date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                models: BTreeMap::new(),
            },
            DailyUsage {
                date: NaiveDate::from_ymd_opt(2026, 3, 3).unwrap(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                models: BTreeMap::new(),
            },
        ];
        let (current, longest) = calculate_streaks_for_today(&daily, today);
        assert_eq!(current, 2);
        assert_eq!(longest, 2);
    }
}
