//! Parallel aggregation of session data
//!
//! Uses rayon for parallel map-reduce operations.

use crate::sessions::UnifiedMessage;
use crate::{
    DailyContribution, DailyTotals, DataSummary, GraphMeta, GraphResult, SourceContribution,
    TokenBreakdown, YearSummary,
};
use rayon::prelude::*;
use std::collections::HashMap;

/// Aggregate messages into daily contributions
pub fn aggregate_by_date(messages: Vec<UnifiedMessage>) -> Vec<DailyContribution> {
    if messages.is_empty() {
        return Vec::new();
    }

    // Parallel aggregation using fold/reduce pattern
    let daily_map: HashMap<String, DayAccumulator> = messages
        .into_par_iter()
        .fold(
            HashMap::new,
            |mut acc: HashMap<String, DayAccumulator>, msg| {
                let entry = acc.entry(msg.date.clone()).or_default();
                entry.add_message(&msg);
                acc
            },
        )
        .reduce(HashMap::new, |mut a, b| {
            for (date, acc) in b {
                a.entry(date).or_default().merge(acc);
            }
            a
        });

    // Convert to sorted vector
    let mut contributions: Vec<DailyContribution> = daily_map
        .into_iter()
        .map(|(date, acc)| acc.into_contribution(date))
        .collect();

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
    let active_days = contributions.iter().filter(|c| c.totals.cost > 0.0).count() as i32;
    let max_cost = contributions
        .iter()
        .map(|c| c.totals.cost)
        .fold(0.0, f64::max);

    // Collect unique sources and models
    let mut sources_set = std::collections::HashSet::new();
    let mut models_set = std::collections::HashSet::new();

    for c in contributions {
        for s in &c.sources {
            sources_set.insert(s.source.clone());
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
        sources: sources_set.into_iter().collect(),
        models: models_set.into_iter().collect(),
    }
}

/// Calculate year summaries
pub fn calculate_years(contributions: &[DailyContribution]) -> Vec<YearSummary> {
    let mut years_map: HashMap<String, YearAccumulator> = HashMap::new();

    for c in contributions {
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

    let mut years: Vec<YearSummary> = years_map
        .into_iter()
        .map(|(year, acc)| YearSummary {
            year,
            total_tokens: acc.tokens,
            total_cost: acc.cost,
            range_start: acc.start,
            range_end: acc.end,
        })
        .collect();

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

#[derive(Default)]
struct DayAccumulator {
    totals: DailyTotals,
    token_breakdown: TokenBreakdown,
    sources: HashMap<String, SourceContribution>,
}

impl DayAccumulator {
    fn add_message(&mut self, msg: &UnifiedMessage) {
        // Update totals
        let total_tokens = msg.tokens.input
            + msg.tokens.output
            + msg.tokens.cache_read
            + msg.tokens.cache_write
            + msg.tokens.reasoning;

        self.totals.tokens += total_tokens;
        self.totals.cost += msg.cost;
        self.totals.messages += 1;

        // Update token breakdown
        self.token_breakdown.input += msg.tokens.input;
        self.token_breakdown.output += msg.tokens.output;
        self.token_breakdown.cache_read += msg.tokens.cache_read;
        self.token_breakdown.cache_write += msg.tokens.cache_write;
        self.token_breakdown.reasoning += msg.tokens.reasoning;

        // Update source contribution
        let key = format!("{}:{}", msg.source, msg.model_id);
        let source = self
            .sources
            .entry(key)
            .or_insert_with(|| SourceContribution {
                source: msg.source.clone(),
                model_id: msg.model_id.clone(),
                provider_id: msg.provider_id.clone(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                messages: 0,
            });

        source.tokens.input += msg.tokens.input;
        source.tokens.output += msg.tokens.output;
        source.tokens.cache_read += msg.tokens.cache_read;
        source.tokens.cache_write += msg.tokens.cache_write;
        source.tokens.reasoning += msg.tokens.reasoning;
        source.cost += msg.cost;
        source.messages += 1;
    }

    fn merge(&mut self, other: DayAccumulator) {
        self.totals.tokens += other.totals.tokens;
        self.totals.cost += other.totals.cost;
        self.totals.messages += other.totals.messages;

        self.token_breakdown.input += other.token_breakdown.input;
        self.token_breakdown.output += other.token_breakdown.output;
        self.token_breakdown.cache_read += other.token_breakdown.cache_read;
        self.token_breakdown.cache_write += other.token_breakdown.cache_write;
        self.token_breakdown.reasoning += other.token_breakdown.reasoning;

        for (key, source) in other.sources {
            let entry = self
                .sources
                .entry(key)
                .or_insert_with(|| SourceContribution {
                    source: source.source.clone(),
                    model_id: source.model_id.clone(),
                    provider_id: source.provider_id.clone(),
                    tokens: TokenBreakdown::default(),
                    cost: 0.0,
                    messages: 0,
                });

            entry.tokens.input += source.tokens.input;
            entry.tokens.output += source.tokens.output;
            entry.tokens.cache_read += source.tokens.cache_read;
            entry.tokens.cache_write += source.tokens.cache_write;
            entry.tokens.reasoning += source.tokens.reasoning;
            entry.cost += source.cost;
            entry.messages += source.messages;
        }
    }

    fn into_contribution(self, date: String) -> DailyContribution {
        DailyContribution {
            date,
            totals: self.totals,
            intensity: 0, // Will be calculated later
            token_breakdown: self.token_breakdown,
            sources: self.sources.into_values().collect(),
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
