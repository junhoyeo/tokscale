//! Parallel aggregation of session data
//!
//! Uses rayon for parallel map-reduce operations.

use crate::sessions::UnifiedMessage;
use crate::{
    ClientContribution, DailyContribution, DailyTotals, DataSummary, GraphMeta, GraphResult,
    TokenBreakdown, YearSummary,
};
use rayon::prelude::*;
use std::collections::HashMap;

/// Aggregate messages into daily contributions
pub fn aggregate_by_date(messages: Vec<UnifiedMessage>) -> Vec<DailyContribution> {
    if messages.is_empty() {
        return Vec::new();
    }

    // Estimate unique days (typically 1-365) - use message count / 10 as heuristic
    let estimated_days = (messages.len() / 10).clamp(30, 400);

    // Parallel aggregation using fold/reduce pattern
    let daily_map: HashMap<String, DayAccumulator> = messages
        .into_par_iter()
        .fold(
            || HashMap::with_capacity(estimated_days),
            |mut acc: HashMap<String, DayAccumulator>, msg| {
                let entry = acc.entry(msg.date.clone()).or_default();
                entry.add_message(&msg);
                acc
            },
        )
        .reduce(
            || HashMap::with_capacity(estimated_days),
            |mut a, b| {
                for (date, acc) in b {
                    a.entry(date).or_default().merge(acc);
                }
                a
            },
        );

    // Convert to sorted vector with pre-allocated capacity
    let mut contributions: Vec<DailyContribution> = Vec::with_capacity(daily_map.len());
    contributions.extend(
        daily_map
            .into_iter()
            .map(|(date, acc)| acc.into_contribution(date)),
    );

    // Sort by date
    contributions.sort_by(|a, b| a.date.cmp(&b.date));

    // Calculate intensities based on max cost
    calculate_intensities(&mut contributions);

    contributions
}

/// Calculate summary statistics
pub fn calculate_summary(contributions: &[DailyContribution]) -> DataSummary {
    let total_tokens: i64 = contributions.iter().map(|c| c.totals.tokens).sum();
    let total_cost: f64 = contributions.iter().map(|c| c.totals.cost).sum();
    let active_days = contributions.iter().filter(|c| c.totals.tokens > 0).count() as i32;
    let max_cost = contributions
        .iter()
        .map(|c| c.totals.cost)
        .fold(0.0, f64::max);

    let mut clients_set = std::collections::HashSet::with_capacity(5);
    let mut models_set = std::collections::HashSet::with_capacity(20);

    for c in contributions {
        for s in &c.clients {
            clients_set.insert(s.client.clone());
            models_set.insert(s.model_id.clone());
        }
    }

    DataSummary {
        total_tokens,
        total_cost,
        total_days: contributions.len() as i32,
        active_days,
        average_per_day: if active_days > 0 {
            total_cost / active_days as f64
        } else {
            0.0
        },
        max_cost_in_single_day: max_cost,
        clients: {
            let mut v: Vec<_> = clients_set.into_iter().collect();
            v.sort();
            v
        },
        models: {
            let mut v: Vec<_> = models_set.into_iter().collect();
            v.sort();
            v
        },
    }
}

/// Calculate year summaries
pub fn calculate_years(contributions: &[DailyContribution]) -> Vec<YearSummary> {
    let mut years_map: HashMap<String, YearAccumulator> = HashMap::with_capacity(5);

    for c in contributions {
        // Guard against short/invalid date strings
        if c.date.len() < 4 {
            eprintln!(
                "Warning: Skipping contribution with invalid date '{}' ({} tokens, ${:.4} cost)",
                c.date, c.totals.tokens, c.totals.cost
            );
            continue;
        }
        let year = &c.date[0..4];
        let entry = years_map.entry(year.to_string()).or_default();
        entry.tokens += c.totals.tokens;
        entry.cost += c.totals.cost;

        if entry.start.is_empty() || c.date < entry.start {
            entry.start = c.date.clone();
        }
        if entry.end.is_empty() || c.date > entry.end {
            entry.end = c.date.clone();
        }
    }

    let mut years: Vec<YearSummary> = Vec::with_capacity(years_map.len());
    years.extend(years_map.into_iter().map(|(year, acc)| YearSummary {
        year,
        total_tokens: acc.tokens,
        total_cost: acc.cost,
        range_start: acc.start,
        range_end: acc.end,
    }));

    years.sort_by(|a, b| a.year.cmp(&b.year));
    years
}

/// Generate complete graph result
pub fn generate_graph_result(
    contributions: Vec<DailyContribution>,
    processing_time_ms: u32,
) -> GraphResult {
    let summary = calculate_summary(&contributions);
    let years = calculate_years(&contributions);

    let date_range_start = contributions
        .first()
        .map(|c| c.date.clone())
        .unwrap_or_default();
    let date_range_end = contributions
        .last()
        .map(|c| c.date.clone())
        .unwrap_or_default();

    GraphResult {
        meta: GraphMeta {
            generated_at: chrono::Utc::now().to_rfc3339(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            date_range_start,
            date_range_end,
            processing_time_ms,
        },
        summary,
        years,
        contributions,
    }
}

// =============================================================================
// Internal helpers
// =============================================================================

struct DayAccumulator {
    totals: DailyTotals,
    token_breakdown: TokenBreakdown,
    clients: HashMap<String, ClientContribution>,
}

impl Default for DayAccumulator {
    fn default() -> Self {
        Self {
            totals: DailyTotals::default(),
            token_breakdown: TokenBreakdown::default(),
            clients: HashMap::with_capacity(8),
        }
    }
}

impl DayAccumulator {
    fn add_message(&mut self, msg: &UnifiedMessage) {
        let total_tokens = msg
            .tokens
            .input
            .saturating_add(msg.tokens.output)
            .saturating_add(msg.tokens.cache_read)
            .saturating_add(msg.tokens.cache_write)
            .saturating_add(msg.tokens.reasoning);

        self.totals.tokens = self.totals.tokens.saturating_add(total_tokens);
        self.totals.cost += msg.cost;
        self.totals.messages = self
            .totals
            .messages
            .saturating_add(msg.message_count.max(0));

        self.token_breakdown.input = self.token_breakdown.input.saturating_add(msg.tokens.input);
        self.token_breakdown.output = self
            .token_breakdown
            .output
            .saturating_add(msg.tokens.output);
        self.token_breakdown.cache_read = self
            .token_breakdown
            .cache_read
            .saturating_add(msg.tokens.cache_read);
        self.token_breakdown.cache_write = self
            .token_breakdown
            .cache_write
            .saturating_add(msg.tokens.cache_write);
        self.token_breakdown.reasoning = self
            .token_breakdown
            .reasoning
            .saturating_add(msg.tokens.reasoning);

        // Update client contribution
        let key = format!(
            "{}:{}",
            msg.client,
            crate::normalize_model_for_grouping(&msg.model_id)
        );
        let client_entry = self
            .clients
            .entry(key)
            .or_insert_with(|| ClientContribution {
                client: msg.client.clone(),
                model_id: crate::normalize_model_for_grouping(&msg.model_id),
                provider_id: msg.provider_id.clone(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                messages: 0,
            });

        // Merge provider_id if different provider contributes to same client+model
        if !client_entry
            .provider_id
            .split(", ")
            .any(|p| p == msg.provider_id)
        {
            client_entry.provider_id = format!("{}, {}", client_entry.provider_id, msg.provider_id);
        }

        client_entry.tokens.input = client_entry.tokens.input.saturating_add(msg.tokens.input);
        client_entry.tokens.output = client_entry.tokens.output.saturating_add(msg.tokens.output);
        client_entry.tokens.cache_read = client_entry
            .tokens
            .cache_read
            .saturating_add(msg.tokens.cache_read);
        client_entry.tokens.cache_write = client_entry
            .tokens
            .cache_write
            .saturating_add(msg.tokens.cache_write);
        client_entry.tokens.reasoning = client_entry
            .tokens
            .reasoning
            .saturating_add(msg.tokens.reasoning);
        client_entry.cost += msg.cost;
        client_entry.messages = client_entry
            .messages
            .saturating_add(msg.message_count.max(0));

        // Normalize provider order for deterministic output
        let mut providers: Vec<&str> = client_entry.provider_id.split(", ").collect();
        providers.sort_unstable();
        providers.dedup();
        client_entry.provider_id = providers.join(", ");
    }

    fn merge(&mut self, other: DayAccumulator) {
        self.totals.tokens = self.totals.tokens.saturating_add(other.totals.tokens);
        self.totals.cost += other.totals.cost;
        self.totals.messages = self.totals.messages.saturating_add(other.totals.messages);

        self.token_breakdown.input = self
            .token_breakdown
            .input
            .saturating_add(other.token_breakdown.input);
        self.token_breakdown.output = self
            .token_breakdown
            .output
            .saturating_add(other.token_breakdown.output);
        self.token_breakdown.cache_read = self
            .token_breakdown
            .cache_read
            .saturating_add(other.token_breakdown.cache_read);
        self.token_breakdown.cache_write = self
            .token_breakdown
            .cache_write
            .saturating_add(other.token_breakdown.cache_write);
        self.token_breakdown.reasoning = self
            .token_breakdown
            .reasoning
            .saturating_add(other.token_breakdown.reasoning);

        for (key, client_contrib) in other.clients {
            let entry = self
                .clients
                .entry(key)
                .or_insert_with(|| ClientContribution {
                    client: client_contrib.client.clone(),
                    model_id: client_contrib.model_id.clone(),
                    provider_id: client_contrib.provider_id.clone(),
                    tokens: TokenBreakdown::default(),
                    cost: 0.0,
                    messages: 0,
                });

            // Merge provider_ids from parallel reduction
            for provider in client_contrib.provider_id.split(", ") {
                if !entry.provider_id.split(", ").any(|p| p == provider) {
                    entry.provider_id = format!("{}, {}", entry.provider_id, provider);
                }
            }

            entry.tokens.input = entry
                .tokens
                .input
                .saturating_add(client_contrib.tokens.input);
            entry.tokens.output = entry
                .tokens
                .output
                .saturating_add(client_contrib.tokens.output);
            entry.tokens.cache_read = entry
                .tokens
                .cache_read
                .saturating_add(client_contrib.tokens.cache_read);
            entry.tokens.cache_write = entry
                .tokens
                .cache_write
                .saturating_add(client_contrib.tokens.cache_write);
            entry.tokens.reasoning = entry
                .tokens
                .reasoning
                .saturating_add(client_contrib.tokens.reasoning);
            entry.cost += client_contrib.cost;
            entry.messages = entry.messages.saturating_add(client_contrib.messages);
        }

        // Normalize provider order for deterministic output
        for entry in self.clients.values_mut() {
            let mut providers: Vec<&str> = entry.provider_id.split(", ").collect();
            providers.sort_unstable();
            providers.dedup();
            entry.provider_id = providers.join(", ");
        }
    }

    fn into_contribution(self, date: String) -> DailyContribution {
        let token_breakdown = TokenBreakdown {
            input: self.token_breakdown.input.max(0),
            output: self.token_breakdown.output.max(0),
            cache_read: self.token_breakdown.cache_read.max(0),
            cache_write: self.token_breakdown.cache_write.max(0),
            reasoning: self.token_breakdown.reasoning.max(0),
        };

        let clients: Vec<ClientContribution> = self
            .clients
            .into_values()
            .map(|mut s| {
                s.tokens.input = s.tokens.input.max(0);
                s.tokens.output = s.tokens.output.max(0);
                s.tokens.cache_read = s.tokens.cache_read.max(0);
                s.tokens.cache_write = s.tokens.cache_write.max(0);
                s.tokens.reasoning = s.tokens.reasoning.max(0);
                s.cost = s.cost.max(0.0);
                s
            })
            .collect();

        DailyContribution {
            date,
            totals: DailyTotals {
                tokens: self.totals.tokens.max(0),
                cost: self.totals.cost.max(0.0),
                messages: self.totals.messages.max(0),
            },
            intensity: 0,
            token_breakdown,
            clients,
        }
    }
}

#[derive(Default)]
struct YearAccumulator {
    tokens: i64,
    cost: f64,
    start: String,
    end: String,
}

fn calculate_intensities(contributions: &mut [DailyContribution]) {
    let max_cost = contributions
        .iter()
        .map(|c| c.totals.cost)
        .fold(0.0, f64::max);

    if max_cost == 0.0 {
        return;
    }

    for c in contributions.iter_mut() {
        let ratio = c.totals.cost / max_cost;
        c.intensity = if ratio >= 0.75 {
            4
        } else if ratio >= 0.5 {
            3
        } else if ratio >= 0.25 {
            2
        } else if ratio > 0.0 {
            1
        } else {
            0
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    // Helper function to create mock UnifiedMessage
    fn mock_unified_message(
        date: &str,
        tokens: i64,
        cost: f64,
        model: &str,
        client: &str,
    ) -> UnifiedMessage {
        // Parse date string to timestamp
        let datetime = format!("{}T00:00:00Z", date)
            .parse::<DateTime<Utc>>()
            .unwrap();
        let timestamp = datetime.timestamp_millis();

        UnifiedMessage {
            client: client.to_string(),
            model_id: model.to_string(),
            provider_id: "test-provider".to_string(),
            session_id: "test-session".to_string(),
            workspace_key: None,
            workspace_label: None,
            timestamp,
            date: date.to_string(),
            tokens: TokenBreakdown {
                input: tokens / 2,
                output: tokens / 2,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost,
            message_count: 1,
            agent: None,
            dedup_key: None,
            is_turn_start: false,
        }
    }

    #[test]
    fn test_aggregate_by_date_empty() {
        let messages = Vec::new();
        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_aggregate_by_date_single_message() {
        let messages = vec![mock_unified_message(
            "2024-01-01",
            1000,
            0.05,
            "claude-3-5-sonnet",
            "opencode",
        )];

        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date, "2024-01-01");
        assert_eq!(result[0].totals.tokens, 1000);
        assert_eq!(result[0].totals.cost, 0.05);
        assert_eq!(result[0].totals.messages, 1);
    }

    #[test]
    fn test_aggregate_by_date_multiple_dates() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-02", 2000, 0.10, "gpt-4", "claude"),
            mock_unified_message("2024-01-03", 1500, 0.08, "claude-3-5-sonnet", "opencode"),
        ];

        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 3);

        // Verify sorted by date
        assert_eq!(result[0].date, "2024-01-01");
        assert_eq!(result[1].date, "2024-01-02");
        assert_eq!(result[2].date, "2024-01-03");

        // Verify totals
        assert_eq!(result[0].totals.tokens, 1000);
        assert_eq!(result[1].totals.tokens, 2000);
        assert_eq!(result[2].totals.tokens, 1500);
    }

    #[test]
    fn test_aggregate_by_date_same_date_aggregation() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-01", 2000, 0.10, "gpt-4", "claude"),
            mock_unified_message("2024-01-01", 1500, 0.08, "claude-3-5-sonnet", "opencode"),
        ];

        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date, "2024-01-01");
        assert_eq!(result[0].totals.tokens, 4500);
        assert!((result[0].totals.cost - 0.23).abs() < 0.0001);
        assert_eq!(result[0].totals.messages, 3);
    }

    #[test]
    fn test_aggregate_by_date_token_breakdown() {
        let mut msg =
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode");
        msg.tokens = TokenBreakdown {
            input: 600,
            output: 300,
            cache_read: 50,
            cache_write: 40,
            reasoning: 10,
        };

        let result = aggregate_by_date(vec![msg]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].token_breakdown.input, 600);
        assert_eq!(result[0].token_breakdown.output, 300);
        assert_eq!(result[0].token_breakdown.cache_read, 50);
        assert_eq!(result[0].token_breakdown.cache_write, 40);
        assert_eq!(result[0].token_breakdown.reasoning, 10);
    }

    #[test]
    fn test_calculate_summary_empty() {
        let contributions = Vec::new();
        let summary = calculate_summary(&contributions);

        assert_eq!(summary.total_tokens, 0);
        assert_eq!(summary.total_cost, 0.0);
        assert_eq!(summary.total_days, 0);
        assert_eq!(summary.active_days, 0);
        assert_eq!(summary.average_per_day, 0.0);
        assert_eq!(summary.max_cost_in_single_day, 0.0);
    }

    #[test]
    fn test_calculate_summary_single_day() {
        let messages = vec![mock_unified_message(
            "2024-01-01",
            1000,
            0.05,
            "claude-3-5-sonnet",
            "opencode",
        )];
        let contributions = aggregate_by_date(messages);
        let summary = calculate_summary(&contributions);

        assert_eq!(summary.total_tokens, 1000);
        assert_eq!(summary.total_cost, 0.05);
        assert_eq!(summary.total_days, 1);
        assert_eq!(summary.active_days, 1);
        assert_eq!(summary.average_per_day, 0.05);
        assert_eq!(summary.max_cost_in_single_day, 0.05);
    }

    #[test]
    fn test_calculate_summary_multiple_days() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-02", 2000, 0.10, "gpt-4", "claude"),
            mock_unified_message("2024-01-03", 1500, 0.08, "claude-3-5-sonnet", "opencode"),
        ];
        let contributions = aggregate_by_date(messages);
        let summary = calculate_summary(&contributions);

        assert_eq!(summary.total_tokens, 4500);
        assert!((summary.total_cost - 0.23).abs() < 0.0001);
        assert_eq!(summary.total_days, 3);
        assert_eq!(summary.active_days, 3);
        assert!((summary.average_per_day - 0.23 / 3.0).abs() < 0.0001);
        assert!((summary.max_cost_in_single_day - 0.10).abs() < 0.0001);
    }

    #[test]
    fn test_calculate_summary_with_zero_token_days() {
        let contributions = vec![
            DailyContribution {
                date: "2024-01-01".to_string(),
                totals: DailyTotals {
                    tokens: 1000,
                    cost: 0.05,
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
            DailyContribution {
                date: "2024-01-02".to_string(),
                totals: DailyTotals {
                    tokens: 0,
                    cost: 0.0,
                    messages: 0,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
        ];

        let summary = calculate_summary(&contributions);
        assert_eq!(summary.total_days, 2);
        assert_eq!(summary.active_days, 1);
        assert!((summary.average_per_day - 0.05).abs() < 0.0001);
    }

    #[test]
    fn test_calculate_years_empty() {
        let contributions = Vec::new();
        let years = calculate_years(&contributions);
        assert_eq!(years.len(), 0);
    }

    #[test]
    fn test_calculate_years_single_year() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-06-15", 2000, 0.10, "gpt-4", "claude"),
            mock_unified_message("2024-12-31", 1500, 0.08, "claude-3-5-sonnet", "opencode"),
        ];
        let contributions = aggregate_by_date(messages);
        let years = calculate_years(&contributions);

        assert_eq!(years.len(), 1);
        assert_eq!(years[0].year, "2024");
        assert_eq!(years[0].total_tokens, 4500);
        assert!((years[0].total_cost - 0.23).abs() < 0.0001);
        assert_eq!(years[0].range_start, "2024-01-01");
        assert_eq!(years[0].range_end, "2024-12-31");
    }

    #[test]
    fn test_calculate_years_multiple_years() {
        let messages = vec![
            mock_unified_message("2023-12-31", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-01", 2000, 0.10, "gpt-4", "claude"),
            mock_unified_message("2024-06-15", 1500, 0.08, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2025-01-01", 3000, 0.15, "gpt-4", "claude"),
        ];
        let contributions = aggregate_by_date(messages);
        let years = calculate_years(&contributions);

        assert_eq!(years.len(), 3);

        // Verify sorted by year
        assert_eq!(years[0].year, "2023");
        assert_eq!(years[1].year, "2024");
        assert_eq!(years[2].year, "2025");

        // Verify 2024 aggregation
        assert_eq!(years[1].total_tokens, 3500);
        assert!((years[1].total_cost - 0.18).abs() < 0.0001);
        assert_eq!(years[1].range_start, "2024-01-01");
        assert_eq!(years[1].range_end, "2024-06-15");
    }

    #[test]
    fn test_calculate_years_year_boundary() {
        let messages = vec![
            mock_unified_message("2024-12-31", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2025-01-01", 2000, 0.10, "gpt-4", "claude"),
        ];
        let contributions = aggregate_by_date(messages);
        let years = calculate_years(&contributions);

        assert_eq!(years.len(), 2);
        assert_eq!(years[0].year, "2024");
        assert_eq!(years[0].total_tokens, 1000);
        assert_eq!(years[1].year, "2025");
        assert_eq!(years[1].total_tokens, 2000);
    }

    #[test]
    fn test_calculate_years_invalid_date() {
        let contributions = vec![DailyContribution {
            date: "abc".to_string(), // Invalid date (less than 4 chars)
            totals: DailyTotals {
                tokens: 1000,
                cost: 0.05,
                messages: 1,
            },
            intensity: 0,
            token_breakdown: TokenBreakdown::default(),
            clients: Vec::new(),
        }];

        let years = calculate_years(&contributions);
        assert_eq!(years.len(), 0); // Should skip invalid dates
    }

    #[test]
    fn test_generate_graph_result_empty() {
        let contributions = Vec::new();
        let result = generate_graph_result(contributions, 100);

        assert_eq!(result.contributions.len(), 0);
        assert_eq!(result.summary.total_tokens, 0);
        assert_eq!(result.years.len(), 0);
        assert_eq!(result.meta.processing_time_ms, 100);
        assert_eq!(result.meta.date_range_start, "");
        assert_eq!(result.meta.date_range_end, "");
    }

    #[test]
    fn test_generate_graph_result_with_data() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-02", 2000, 0.10, "gpt-4", "claude"),
        ];
        let contributions = aggregate_by_date(messages);
        let result = generate_graph_result(contributions, 150);

        assert_eq!(result.contributions.len(), 2);
        assert_eq!(result.summary.total_tokens, 3000);
        assert_eq!(result.years.len(), 1);
        assert_eq!(result.meta.processing_time_ms, 150);
        assert_eq!(result.meta.date_range_start, "2024-01-01");
        assert_eq!(result.meta.date_range_end, "2024-01-02");
        assert_eq!(result.meta.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_calculate_intensities_empty() {
        let mut contributions = Vec::new();
        calculate_intensities(&mut contributions);
        assert_eq!(contributions.len(), 0);
    }

    #[test]
    fn test_calculate_intensities_zero_cost() {
        let mut contributions = vec![
            DailyContribution {
                date: "2024-01-01".to_string(),
                totals: DailyTotals {
                    tokens: 1000,
                    cost: 0.0,
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
            DailyContribution {
                date: "2024-01-02".to_string(),
                totals: DailyTotals {
                    tokens: 2000,
                    cost: 0.0,
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
        ];

        calculate_intensities(&mut contributions);
        assert_eq!(contributions[0].intensity, 0);
        assert_eq!(contributions[1].intensity, 0);
    }

    #[test]
    fn test_calculate_intensities_levels() {
        let mut contributions = vec![
            DailyContribution {
                date: "2024-01-01".to_string(),
                totals: DailyTotals {
                    tokens: 1000,
                    cost: 1.0, // 100% of max
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
            DailyContribution {
                date: "2024-01-02".to_string(),
                totals: DailyTotals {
                    tokens: 800,
                    cost: 0.8, // 80% of max (>= 0.75)
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
            DailyContribution {
                date: "2024-01-03".to_string(),
                totals: DailyTotals {
                    tokens: 600,
                    cost: 0.6, // 60% of max (>= 0.5)
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
            DailyContribution {
                date: "2024-01-04".to_string(),
                totals: DailyTotals {
                    tokens: 300,
                    cost: 0.3, // 30% of max (>= 0.25)
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
            DailyContribution {
                date: "2024-01-05".to_string(),
                totals: DailyTotals {
                    tokens: 100,
                    cost: 0.1, // 10% of max (> 0.0)
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
        ];

        calculate_intensities(&mut contributions);

        assert_eq!(contributions[0].intensity, 4); // 100%
        assert_eq!(contributions[1].intensity, 4); // 80%
        assert_eq!(contributions[2].intensity, 3); // 60%
        assert_eq!(contributions[3].intensity, 2); // 30%
        assert_eq!(contributions[4].intensity, 1); // 10%
    }

    #[test]
    fn test_calculate_intensities_boundary_values() {
        let mut contributions = vec![
            DailyContribution {
                date: "2024-01-01".to_string(),
                totals: DailyTotals {
                    tokens: 1000,
                    cost: 1.0,
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
            DailyContribution {
                date: "2024-01-02".to_string(),
                totals: DailyTotals {
                    tokens: 750,
                    cost: 0.75, // Exactly 0.75 (should be level 4)
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
            DailyContribution {
                date: "2024-01-03".to_string(),
                totals: DailyTotals {
                    tokens: 500,
                    cost: 0.5, // Exactly 0.5 (should be level 3)
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
            DailyContribution {
                date: "2024-01-04".to_string(),
                totals: DailyTotals {
                    tokens: 250,
                    cost: 0.25, // Exactly 0.25 (should be level 2)
                    messages: 1,
                },
                intensity: 0,
                token_breakdown: TokenBreakdown::default(),
                clients: Vec::new(),
            },
        ];

        calculate_intensities(&mut contributions);

        assert_eq!(contributions[0].intensity, 4);
        assert_eq!(contributions[1].intensity, 4); // >= 0.75
        assert_eq!(contributions[2].intensity, 3); // >= 0.5
        assert_eq!(contributions[3].intensity, 2); // >= 0.25
    }

    #[test]
    fn test_aggregate_by_date_preserves_sources() {
        let messages = vec![
            mock_unified_message("2024-01-01", 1000, 0.05, "claude-3-5-sonnet", "opencode"),
            mock_unified_message("2024-01-01", 2000, 0.10, "gpt-4", "claude"),
        ];

        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].clients.len(), 2);

        // Verify both clients are present
        let client_names: Vec<&str> = result[0]
            .clients
            .iter()
            .map(|s| s.client.as_str())
            .collect();
        assert!(client_names.contains(&"opencode"));
        assert!(client_names.contains(&"claude"));
    }

    #[test]
    fn test_aggregate_by_date_large_dataset() {
        // Test with 100 messages across 10 days
        let mut messages = Vec::new();
        for day in 1..=10 {
            for _msg in 0..10 {
                let date = format!("2024-01-{:02}", day);
                messages.push(mock_unified_message(
                    &date,
                    1000,
                    0.05,
                    "claude-3-5-sonnet",
                    "opencode",
                ));
            }
        }

        let result = aggregate_by_date(messages);
        assert_eq!(result.len(), 10);

        // Each day should have 10 messages aggregated
        for contribution in &result {
            assert_eq!(contribution.totals.messages, 10);
            assert_eq!(contribution.totals.tokens, 10000);
            assert!((contribution.totals.cost - 0.5).abs() < 0.0001);
        }
    }
}
