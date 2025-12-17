//! Parallel aggregation of session data
//!
//! Uses rayon for parallel map-reduce operations.

use crate::sessions::UnifiedMessage;
use crate::{
    DailyContribution, DailyTotals, DataSummary, GraphMeta, GraphResult, SourceContribution,
    TokenBreakdown, YearSummary,
};
use napi_derive::napi;
use rayon::prelude::*;
use std::collections::HashMap;

/// Rate statistics for an interval bucket (tokens per minute)
#[napi(object)]
#[derive(Debug, Clone, Default)]
pub struct RateStats {
    pub avg_tokens_per_min: f64,
    pub max_tokens_per_min: f64,
    pub min_tokens_per_min: f64,
}

/// A time-bucketed aggregation of token usage (e.g., 15-minute intervals)
#[napi(object)]
#[derive(Debug, Clone)]
pub struct IntervalBucket {
    /// UTC timestamp of bucket start (milliseconds)
    pub start_ms: i64,
    /// UTC timestamp of bucket end (milliseconds)
    pub end_ms: i64,
    /// Token breakdown totals for this interval
    pub token_breakdown: TokenBreakdown,
    /// Number of messages in this interval
    pub messages: i32,
    /// Cost in microdollars (cost * 1_000_000) for precision
    pub cost_micros: i64,
    /// Optional rate statistics
    pub rate_stats: Option<RateStats>,
}

/// Aggregate messages into time interval buckets (e.g., 15-minute intervals)
pub fn aggregate_by_interval(
    messages: Vec<UnifiedMessage>,
    interval_ms: u64,
) -> Vec<IntervalBucket> {
    if messages.is_empty() {
        return Vec::new();
    }

    let interval_ms_i64 = interval_ms as i64;

    let (min_ts, max_ts) = messages
        .par_iter()
        .map(|m| m.timestamp)
        .fold(
            || (i64::MAX, i64::MIN),
            |(min, max), ts| (min.min(ts), max.max(ts)),
        )
        .reduce(
            || (i64::MAX, i64::MIN),
            |(min1, max1), (min2, max2)| (min1.min(min2), max1.max(max2)),
        );

    let first_bucket = (min_ts / interval_ms_i64) * interval_ms_i64;
    let last_bucket = (max_ts / interval_ms_i64) * interval_ms_i64;
    let bucket_count = ((last_bucket - first_bucket) / interval_ms_i64 + 1) as usize;

    let bucket_map: HashMap<i64, IntervalAccumulator> = messages
        .into_par_iter()
        .fold(
            || HashMap::<i64, IntervalAccumulator>::with_capacity(bucket_count),
            |mut acc, msg| {
                let bucket_start = (msg.timestamp / interval_ms_i64) * interval_ms_i64;
                acc.entry(bucket_start).or_default().add_message(&msg);
                acc
            },
        )
        .reduce(
            || HashMap::<i64, IntervalAccumulator>::with_capacity(bucket_count),
            |mut a, b| {
                for (bucket_start, acc) in b {
                    a.entry(bucket_start).or_default().merge(acc);
                }
                a
            },
        );

    let mut buckets: Vec<IntervalBucket> = Vec::with_capacity(bucket_count);
    let mut current = first_bucket;
    while current <= last_bucket {
        let bucket = match bucket_map.get(&current) {
            Some(acc) => acc.into_bucket(current, interval_ms_i64),
            None => IntervalBucket {
                start_ms: current,
                end_ms: current + interval_ms_i64,
                token_breakdown: TokenBreakdown::default(),
                messages: 0,
                cost_micros: 0,
                rate_stats: None,
            },
        };
        buckets.push(bucket);
        current += interval_ms_i64;
    }

    buckets
}

/// Aggregate messages into daily contributions
pub fn aggregate_by_date(messages: Vec<UnifiedMessage>) -> Vec<DailyContribution> {
    if messages.is_empty() {
        return Vec::new();
    }

    // Estimate unique days (typically 1-365) - use message count / 10 as heuristic
    let estimated_days = (messages.len() / 10).max(30).min(400);

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
    contributions.extend(daily_map.into_iter().map(|(date, acc)| acc.into_contribution(date)));

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

    let mut sources_set = std::collections::HashSet::with_capacity(5);
    let mut models_set = std::collections::HashSet::with_capacity(20);

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
    let mut years_map: HashMap<String, YearAccumulator> = HashMap::with_capacity(5);

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
    sources: HashMap<String, SourceContribution>,
}

impl Default for DayAccumulator {
    fn default() -> Self {
        Self {
            totals: DailyTotals::default(),
            token_breakdown: TokenBreakdown::default(),
            sources: HashMap::with_capacity(8),
        }
    }
}

impl DayAccumulator {
    fn add_message(&mut self, msg: &UnifiedMessage) {
        let total_tokens = msg.tokens.input
            .saturating_add(msg.tokens.output)
            .saturating_add(msg.tokens.cache_read)
            .saturating_add(msg.tokens.cache_write)
            .saturating_add(msg.tokens.reasoning);

        self.totals.tokens = self.totals.tokens.saturating_add(total_tokens);
        self.totals.cost += msg.cost;
        self.totals.messages = self.totals.messages.saturating_add(1);

        self.token_breakdown.input = self.token_breakdown.input.saturating_add(msg.tokens.input);
        self.token_breakdown.output = self.token_breakdown.output.saturating_add(msg.tokens.output);
        self.token_breakdown.cache_read = self.token_breakdown.cache_read.saturating_add(msg.tokens.cache_read);
        self.token_breakdown.cache_write = self.token_breakdown.cache_write.saturating_add(msg.tokens.cache_write);
        self.token_breakdown.reasoning = self.token_breakdown.reasoning.saturating_add(msg.tokens.reasoning);

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

        source.tokens.input = source.tokens.input.saturating_add(msg.tokens.input);
        source.tokens.output = source.tokens.output.saturating_add(msg.tokens.output);
        source.tokens.cache_read = source.tokens.cache_read.saturating_add(msg.tokens.cache_read);
        source.tokens.cache_write = source.tokens.cache_write.saturating_add(msg.tokens.cache_write);
        source.tokens.reasoning = source.tokens.reasoning.saturating_add(msg.tokens.reasoning);
        source.cost += msg.cost;
        source.messages = source.messages.saturating_add(1);
    }

    fn merge(&mut self, other: DayAccumulator) {
        self.totals.tokens = self.totals.tokens.saturating_add(other.totals.tokens);
        self.totals.cost += other.totals.cost;
        self.totals.messages = self.totals.messages.saturating_add(other.totals.messages);

        self.token_breakdown.input = self.token_breakdown.input.saturating_add(other.token_breakdown.input);
        self.token_breakdown.output = self.token_breakdown.output.saturating_add(other.token_breakdown.output);
        self.token_breakdown.cache_read = self.token_breakdown.cache_read.saturating_add(other.token_breakdown.cache_read);
        self.token_breakdown.cache_write = self.token_breakdown.cache_write.saturating_add(other.token_breakdown.cache_write);
        self.token_breakdown.reasoning = self.token_breakdown.reasoning.saturating_add(other.token_breakdown.reasoning);

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

            entry.tokens.input = entry.tokens.input.saturating_add(source.tokens.input);
            entry.tokens.output = entry.tokens.output.saturating_add(source.tokens.output);
            entry.tokens.cache_read = entry.tokens.cache_read.saturating_add(source.tokens.cache_read);
            entry.tokens.cache_write = entry.tokens.cache_write.saturating_add(source.tokens.cache_write);
            entry.tokens.reasoning = entry.tokens.reasoning.saturating_add(source.tokens.reasoning);
            entry.cost += source.cost;
            entry.messages = entry.messages.saturating_add(source.messages);
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

#[derive(Default)]
struct IntervalAccumulator {
    token_breakdown: TokenBreakdown,
    messages: i32,
    cost: f64,
    /// Track message timestamps and token counts for rate calculation: (timestamp_ms, total_tokens)
    message_data: Vec<(i64, i64)>,
}

impl IntervalAccumulator {
    fn add_message(&mut self, msg: &UnifiedMessage) {
        self.token_breakdown.input = self.token_breakdown.input.saturating_add(msg.tokens.input);
        self.token_breakdown.output = self.token_breakdown.output.saturating_add(msg.tokens.output);
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
        self.cost += msg.cost;
        self.messages += 1;

        let total_tokens = msg.tokens.input
            .saturating_add(msg.tokens.output)
            .saturating_add(msg.tokens.cache_read)
            .saturating_add(msg.tokens.cache_write)
            .saturating_add(msg.tokens.reasoning);
        self.message_data.push((msg.timestamp, total_tokens));
    }

    fn merge(&mut self, other: IntervalAccumulator) {
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
        self.cost += other.cost;
        self.messages += other.messages;
        self.message_data.extend(other.message_data);
    }

    fn into_bucket(&self, start_ms: i64, interval_ms: i64) -> IntervalBucket {
        let rate_stats = self.calculate_rate_stats(interval_ms);
        IntervalBucket {
            start_ms,
            end_ms: start_ms + interval_ms,
            token_breakdown: self.token_breakdown.clone(),
            messages: self.messages,
            cost_micros: (self.cost * 1_000_000.0) as i64,
            rate_stats,
        }
    }

    fn calculate_rate_stats(&self, interval_ms: i64) -> Option<RateStats> {
        if self.message_data.is_empty() {
            return None;
        }

        let total_tokens = self.token_breakdown.input
            + self.token_breakdown.output
            + self.token_breakdown.cache_read
            + self.token_breakdown.cache_write
            + self.token_breakdown.reasoning;

        let interval_minutes = interval_ms as f64 / 60_000.0;
        let avg_tokens_per_min = total_tokens as f64 / interval_minutes;

        if self.message_data.len() == 1 {
            return Some(RateStats {
                avg_tokens_per_min,
                max_tokens_per_min: avg_tokens_per_min,
                min_tokens_per_min: avg_tokens_per_min,
            });
        }

        let mut sorted = self.message_data.clone();
        sorted.sort_by_key(|(ts, _)| *ts);

        let mut max_rate: f64 = 0.0;
        let mut min_rate: f64 = f64::MAX;

        const MIN_DT_MS: i64 = 5_000;
        const MAX_DT_MS: i64 = 1_800_000;

        for i in 0..sorted.len() - 1 {
            let (ts1, _) = sorted[i];
            let (ts2, tokens2) = sorted[i + 1];

            let dt_ms = (ts2 - ts1).clamp(MIN_DT_MS, MAX_DT_MS);
            let dt_minutes = dt_ms as f64 / 60_000.0;
            let rate = tokens2 as f64 / dt_minutes;

            max_rate = max_rate.max(rate);
            min_rate = min_rate.min(rate);
        }

        if min_rate == f64::MAX {
            min_rate = avg_tokens_per_min;
        }

        Some(RateStats {
            avg_tokens_per_min,
            max_tokens_per_min: max_rate.max(avg_tokens_per_min),
            min_tokens_per_min: min_rate.min(avg_tokens_per_min),
        })
    }
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

    fn create_test_message(timestamp: i64, input: i64, output: i64, cost: f64) -> UnifiedMessage {
        UnifiedMessage {
            source: "test".to_string(),
            model_id: "test-model".to_string(),
            provider_id: "test-provider".to_string(),
            session_id: "test-session".to_string(),
            timestamp,
            date: "2024-01-01".to_string(),
            tokens: TokenBreakdown {
                input,
                output,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            cost,
        }
    }

    fn create_full_test_message(
        timestamp: i64,
        input: i64,
        output: i64,
        cache_read: i64,
        cache_write: i64,
        reasoning: i64,
        cost: f64,
    ) -> UnifiedMessage {
        UnifiedMessage {
            source: "test".to_string(),
            model_id: "test-model".to_string(),
            provider_id: "test-provider".to_string(),
            session_id: "test-session".to_string(),
            timestamp,
            date: "2024-01-01".to_string(),
            tokens: TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                reasoning,
            },
            cost,
        }
    }

    #[test]
    fn test_aggregate_by_interval_empty() {
        let result = aggregate_by_interval(vec![], 900_000);
        assert!(result.is_empty());
    }

    #[test]
    fn test_aggregate_by_interval_single_message() {
        let messages = vec![create_test_message(1000, 100, 50, 0.01)];
        let result = aggregate_by_interval(messages, 1000);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_ms, 1000);
        assert_eq!(result[0].end_ms, 2000);
        assert_eq!(result[0].messages, 1);
        assert_eq!(result[0].token_breakdown.input, 100);
        assert_eq!(result[0].token_breakdown.output, 50);
        assert_eq!(result[0].cost_micros, 10000);
    }

    #[test]
    fn test_aggregate_by_interval_fills_gaps() {
        let messages = vec![
            create_test_message(0, 100, 50, 0.01),
            create_test_message(3000, 200, 100, 0.02),
        ];
        let result = aggregate_by_interval(messages, 1000);

        assert_eq!(result.len(), 4);

        assert_eq!(result[0].start_ms, 0);
        assert_eq!(result[0].messages, 1);
        assert_eq!(result[0].token_breakdown.input, 100);

        assert_eq!(result[1].start_ms, 1000);
        assert_eq!(result[1].messages, 0);
        assert_eq!(result[1].token_breakdown.input, 0);

        assert_eq!(result[2].start_ms, 2000);
        assert_eq!(result[2].messages, 0);

        assert_eq!(result[3].start_ms, 3000);
        assert_eq!(result[3].messages, 1);
        assert_eq!(result[3].token_breakdown.input, 200);
    }

    #[test]
    fn test_aggregate_by_interval_multiple_in_same_bucket() {
        let messages = vec![
            create_test_message(100, 100, 50, 0.01),
            create_test_message(500, 200, 100, 0.02),
            create_test_message(900, 300, 150, 0.03),
        ];
        let result = aggregate_by_interval(messages, 1000);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].messages, 3);
        assert_eq!(result[0].token_breakdown.input, 600);
        assert_eq!(result[0].token_breakdown.output, 300);
        assert_eq!(result[0].cost_micros, 60000);
    }

    #[test]
    fn test_aggregate_by_interval_15_minutes() {
        let fifteen_minutes_ms: u64 = 15 * 60 * 1000;
        let messages = vec![
            create_test_message(0, 100, 50, 0.01),
            create_test_message((fifteen_minutes_ms * 2) as i64, 200, 100, 0.02),
        ];
        let result = aggregate_by_interval(messages, fifteen_minutes_ms);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].end_ms - result[0].start_ms, fifteen_minutes_ms as i64);
        assert_eq!(result[1].messages, 0);
    }

    #[test]
    fn test_rate_stats_empty_bucket() {
        let result = aggregate_by_interval(vec![], 60_000);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rate_stats_single_message() {
        let messages = vec![create_test_message(30_000, 100, 50, 0.01)];
        let result = aggregate_by_interval(messages, 60_000);

        assert_eq!(result.len(), 1);
        let rate_stats = result[0].rate_stats.as_ref().expect("rate_stats should be Some");

        let total_tokens = 100 + 50;
        let expected_avg = total_tokens as f64 / 1.0;
        assert!((rate_stats.avg_tokens_per_min - expected_avg).abs() < 0.001);
        assert!((rate_stats.max_tokens_per_min - expected_avg).abs() < 0.001);
        assert!((rate_stats.min_tokens_per_min - expected_avg).abs() < 0.001);
    }

    #[test]
    fn test_rate_stats_multiple_messages() {
        let messages = vec![
            create_test_message(0, 100, 0, 0.01),
            create_test_message(30_000, 200, 0, 0.02),
            create_test_message(60_000, 300, 0, 0.03),
        ];
        let result = aggregate_by_interval(messages, 120_000);

        assert_eq!(result.len(), 1);
        let rate_stats = result[0].rate_stats.as_ref().expect("rate_stats should be Some");

        let total_tokens = 100 + 200 + 300;
        let expected_avg = total_tokens as f64 / 2.0;
        assert!((rate_stats.avg_tokens_per_min - expected_avg).abs() < 0.001);

        assert!(rate_stats.max_tokens_per_min > 0.0);
        assert!(rate_stats.min_tokens_per_min > 0.0);
    }

    #[test]
    fn test_rate_stats_with_all_token_types() {
        let messages = vec![create_full_test_message(
            30_000, 100, 50, 25, 10, 15, 0.01,
        )];
        let result = aggregate_by_interval(messages, 60_000);

        assert_eq!(result.len(), 1);
        let rate_stats = result[0].rate_stats.as_ref().expect("rate_stats should be Some");

        let total_tokens = 100 + 50 + 25 + 10 + 15;
        let expected_avg = total_tokens as f64 / 1.0;
        assert!((rate_stats.avg_tokens_per_min - expected_avg).abs() < 0.001);
    }

    #[test]
    fn test_rate_stats_dt_clamp_min() {
        let messages = vec![
            create_test_message(0, 100, 0, 0.01),
            create_test_message(1000, 1000, 0, 0.02),
        ];
        let result = aggregate_by_interval(messages, 60_000);

        let rate_stats = result[0].rate_stats.as_ref().expect("rate_stats should be Some");
        let max_possible_rate = 1000.0 / (5.0 / 60.0);
        assert!(rate_stats.max_tokens_per_min <= max_possible_rate + 1.0);
    }

    #[test]
    fn test_rate_stats_dt_clamp_max_long_gap() {
        let messages = vec![
            create_test_message(0, 100, 0, 0.01),
            create_test_message(7_200_000, 600, 0, 0.02),
        ];
        let result = aggregate_by_interval(messages, 10_800_000);

        let rate_stats = result[0].rate_stats.as_ref().expect("rate_stats should be Some");

        let total_tokens = 700.0;
        let interval_minutes = 180.0;
        let expected_avg = total_tokens / interval_minutes;
        assert!((rate_stats.avg_tokens_per_min - expected_avg).abs() < 0.01);

        let unclamped_rate = 600.0 / 120.0;
        let clamped_rate = 600.0 / 30.0;

        assert!(rate_stats.max_tokens_per_min >= clamped_rate - 0.1);
        assert!(rate_stats.max_tokens_per_min > unclamped_rate);

        assert!(rate_stats.min_tokens_per_min <= expected_avg + 0.01);
    }

    #[test]
    fn test_rate_stats_zero_bucket_has_none() {
        let messages = vec![
            create_test_message(0, 100, 50, 0.01),
            create_test_message(120_000, 200, 100, 0.02),
        ];
        let result = aggregate_by_interval(messages, 60_000);

        assert_eq!(result.len(), 3);
        assert!(result[0].rate_stats.is_some());
        assert!(result[1].rate_stats.is_none());
        assert!(result[2].rate_stats.is_some());
    }
}
