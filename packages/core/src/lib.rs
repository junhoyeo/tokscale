//! Tokscale Core - Native Rust module for high-performance session parsing
//!
//! This module provides parallel file scanning, SIMD JSON parsing, and efficient
//! aggregation of token usage data from multiple AI coding assistant sessions.

#![deny(clippy::all)]

use napi_derive::napi;

mod aggregator;
mod parser;
mod pricing;
mod scanner;
mod sessions;

pub use aggregator::*;
pub use parser::*;
pub use scanner::*;

/// Version of the native module
#[napi]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Simple health check to verify the native module is working
#[napi]
pub fn health_check() -> String {
    "tokscale-core is healthy!".to_string()
}

/// Token breakdown by type
#[napi(object)]
#[derive(Debug, Clone, Default)]
pub struct TokenBreakdown {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
}

// =============================================================================
// Two-Phase Processing Types (for parallel execution optimization)
// =============================================================================

#[napi(object)]
#[derive(Debug, Clone)]
pub struct ParsedMessage {
    pub source: String,
    pub model_id: String,
    pub provider_id: String,
    pub session_id: String,
    pub timestamp: i64,
    pub date: String,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    pub agent: Option<String>,
}

/// Result of parsing local sources (excludes Cursor - it's network-synced)
#[napi(object)]
#[derive(Debug, Clone)]
pub struct ParsedMessages {
    pub messages: Vec<ParsedMessage>,
    pub opencode_count: i32,
    pub claude_count: i32,
    pub codex_count: i32,
    pub gemini_count: i32,
    pub amp_count: i32,
    pub droid_count: i32,
    pub openclaw_count: i32,
    pub pi_count: i32,
    pub processing_time_ms: u32,
}

/// Options for parsing local sources only (no Cursor)
#[napi(object)]
#[derive(Debug, Clone)]
pub struct LocalParseOptions {
    pub home_dir: Option<String>,
    pub sources: Option<Vec<String>>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
}

/// Options for finalizing report
#[napi(object)]
#[derive(Debug, Clone)]
pub struct FinalizeReportOptions {
    pub home_dir: Option<String>,
    pub local_messages: ParsedMessages,
    pub include_cursor: bool,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
}

/// Daily contribution totals
#[napi(object)]
#[derive(Debug, Clone, Default)]
pub struct DailyTotals {
    pub tokens: i64,
    pub cost: f64,
    pub messages: i32,
}

/// Source contribution for a specific day
#[napi(object)]
#[derive(Debug, Clone)]
pub struct SourceContribution {
    pub source: String,
    pub model_id: String,
    pub provider_id: String,
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub messages: i32,
}

/// Daily contribution data
#[napi(object)]
#[derive(Debug, Clone)]
pub struct DailyContribution {
    pub date: String,
    pub timestamp_ms: Option<i64>,
    pub totals: DailyTotals,
    pub intensity: u8,
    pub token_breakdown: TokenBreakdown,
    pub sources: Vec<SourceContribution>,
}

/// Year summary
#[napi(object)]
#[derive(Debug, Clone)]
pub struct YearSummary {
    pub year: String,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub range_start: String,
    pub range_end: String,
}

/// Data summary statistics
#[napi(object)]
#[derive(Debug, Clone)]
pub struct DataSummary {
    pub total_tokens: i64,
    pub total_cost: f64,
    pub total_days: i32,
    pub active_days: i32,
    pub average_per_day: f64,
    pub max_cost_in_single_day: f64,
    pub sources: Vec<String>,
    pub models: Vec<String>,
}

/// Metadata about the graph generation
#[napi(object)]
#[derive(Debug, Clone)]
pub struct GraphMeta {
    pub generated_at: String,
    pub version: String,
    pub date_range_start: String,
    pub date_range_end: String,
    pub processing_time_ms: u32,
}

/// Complete graph result
#[napi(object)]
#[derive(Debug, Clone)]
pub struct GraphResult {
    pub meta: GraphMeta,
    pub summary: DataSummary,
    pub years: Vec<YearSummary>,
    pub contributions: Vec<DailyContribution>,
}

// =============================================================================
// Shared Utilities
// =============================================================================

use rayon::prelude::*;
use sessions::UnifiedMessage;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn get_home_dir(home_dir_option: &Option<String>) -> napi::Result<String> {
    home_dir_option
        .clone()
        .or_else(|| std::env::var("HOME").ok())
        .or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().into_owned()))
        .ok_or_else(|| {
            napi::Error::from_reason(
                "HOME directory not specified and could not determine home directory",
            )
        })
}

// =============================================================================
// Pricing-aware APIs
// =============================================================================

/// Options for reports
#[napi(object)]
#[derive(Debug, Clone)]
pub struct ReportOptions {
    pub home_dir: Option<String>,
    pub sources: Option<Vec<String>>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
}

/// Model usage summary for reports
#[napi(object)]
#[derive(Debug, Clone)]
pub struct ModelUsage {
    pub source: String,
    pub model: String,
    pub provider: String,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    pub message_count: i32,
    pub cost: f64,
}

/// Monthly usage summary
#[napi(object)]
#[derive(Debug, Clone)]
pub struct MonthlyUsage {
    pub month: String,
    pub models: Vec<String>,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub message_count: i32,
    pub cost: f64,
}

/// Model report result
#[napi(object)]
#[derive(Debug, Clone)]
pub struct ModelReport {
    pub entries: Vec<ModelUsage>,
    pub total_input: i64,
    pub total_output: i64,
    pub total_cache_read: i64,
    pub total_cache_write: i64,
    pub total_messages: i32,
    pub total_cost: f64,
    pub processing_time_ms: u32,
}

/// Monthly report result
#[napi(object)]
#[derive(Debug, Clone)]
pub struct MonthlyReport {
    pub entries: Vec<MonthlyUsage>,
    pub total_cost: f64,
    pub processing_time_ms: u32,
}

fn parse_all_messages_with_pricing(
    home_dir: &str,
    sources: &[String],
    pricing: &pricing::PricingService,
) -> Vec<UnifiedMessage> {
    let scan_result = scanner::scan_all_sources(home_dir, sources);
    let mut all_messages: Vec<UnifiedMessage> = Vec::new();

    // Parse OpenCode: read both SQLite (1.2+) and legacy JSON, deduplicate by message ID
    let mut opencode_seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    if let Some(db_path) = &scan_result.opencode_db {
        let sqlite_messages: Vec<UnifiedMessage> = sessions::opencode::parse_opencode_sqlite(db_path)
            .into_iter()
            .map(|mut msg| {
                msg.cost = pricing.calculate_cost(
                    &msg.model_id,
                    msg.tokens.input,
                    msg.tokens.output,
                    msg.tokens.cache_read,
                    msg.tokens.cache_write,
                    msg.tokens.reasoning,
                );
                if let Some(ref key) = msg.dedup_key {
                    opencode_seen.insert(key.clone());
                }
                msg
            })
            .collect();
        all_messages.extend(sqlite_messages);
    }

    let opencode_messages: Vec<UnifiedMessage> = scan_result
        .opencode_files
        .par_iter()
        .filter_map(|path| {
            let mut msg = sessions::opencode::parse_opencode_file(path)?;
            msg.cost = pricing.calculate_cost(
                &msg.model_id,
                msg.tokens.input,
                msg.tokens.output,
                msg.tokens.cache_read,
                msg.tokens.cache_write,
                msg.tokens.reasoning,
            );
            Some(msg)
        })
        .collect();
    all_messages.extend(
        opencode_messages
            .into_iter()
            .filter(|msg| {
                msg.dedup_key.as_ref().map_or(true, |key| opencode_seen.insert(key.clone()))
            }),
    );

    // Parse Claude files in parallel
    let claude_messages: Vec<UnifiedMessage> = scan_result
        .claude_files
        .par_iter()
        .flat_map(|path| {
            sessions::claudecode::parse_claude_file(path)
                .into_iter()
                .map(|mut msg| {
                    msg.cost = pricing.calculate_cost(
                        &msg.model_id,
                        msg.tokens.input,
                        msg.tokens.output,
                        msg.tokens.cache_read,
                        msg.tokens.cache_write,
                        msg.tokens.reasoning,
                    );
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(claude_messages);

    // Parse Codex files in parallel
    let codex_messages: Vec<UnifiedMessage> = scan_result
        .codex_files
        .par_iter()
        .flat_map(|path| {
            sessions::codex::parse_codex_file(path)
                .into_iter()
                .map(|mut msg| {
                    msg.cost = pricing.calculate_cost(
                        &msg.model_id,
                        msg.tokens.input,
                        msg.tokens.output,
                        msg.tokens.cache_read,
                        msg.tokens.cache_write,
                        msg.tokens.reasoning,
                    );
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(codex_messages);

    // Parse Gemini files in parallel
    let gemini_messages: Vec<UnifiedMessage> = scan_result
        .gemini_files
        .par_iter()
        .flat_map(|path| {
            sessions::gemini::parse_gemini_file(path)
                .into_iter()
                .map(|mut msg| {
                    // Gemini: thoughts count as output for billing
                    msg.cost = pricing.calculate_cost(
                        &msg.model_id,
                        msg.tokens.input,
                        msg.tokens.output + msg.tokens.reasoning,
                        0, // Gemini cached tokens are free
                        0,
                        0,
                    );
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(gemini_messages);

    // Parse Cursor files in parallel
    // Calculate cost using our pricing data for consistency with other providers
    // Fall back to CSV cost only if no pricing is found
    let cursor_messages: Vec<UnifiedMessage> = scan_result
        .cursor_files
        .par_iter()
        .flat_map(|path| {
            sessions::cursor::parse_cursor_file(path)
                .into_iter()
                .map(|mut msg| {
                    let csv_cost = msg.cost; // Store original CSV cost
                    let calculated_cost = pricing.calculate_cost(
                        &msg.model_id,
                        msg.tokens.input,
                        msg.tokens.output,
                        msg.tokens.cache_read,
                        msg.tokens.cache_write,
                        msg.tokens.reasoning,
                    );
                    // Use calculated cost if available, otherwise keep CSV cost
                    msg.cost = if calculated_cost > 0.0 {
                        calculated_cost
                    } else {
                        csv_cost
                    };
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(cursor_messages);

    // Parse Amp files in parallel
    // Calculate cost using our pricing data for consistency with other providers
    // Fall back to credits (original cost) only if no pricing is found
    let amp_messages: Vec<UnifiedMessage> = scan_result
        .amp_files
        .par_iter()
        .flat_map(|path| {
            sessions::amp::parse_amp_file(path)
                .into_iter()
                .map(|mut msg| {
                    let credits = msg.cost; // Store original credits value
                    let calculated_cost = pricing.calculate_cost(
                        &msg.model_id,
                        msg.tokens.input,
                        msg.tokens.output,
                        msg.tokens.cache_read,
                        msg.tokens.cache_write,
                        msg.tokens.reasoning,
                    );
                    // Use calculated cost if available, otherwise keep credits
                    msg.cost = if calculated_cost > 0.0 {
                        calculated_cost
                    } else {
                        credits
                    };
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(amp_messages);

    // Parse Droid files in parallel
    let droid_messages: Vec<UnifiedMessage> = scan_result
        .droid_files
        .par_iter()
        .flat_map(|path| {
            sessions::droid::parse_droid_file(path)
                .into_iter()
                .map(|mut msg| {
                    msg.cost = pricing.calculate_cost(
                        &msg.model_id,
                        msg.tokens.input,
                        msg.tokens.output,
                        msg.tokens.cache_read,
                        msg.tokens.cache_write,
                        msg.tokens.reasoning,
                    );
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(droid_messages);

    // Parse OpenClaw transcript JSONL files
    let openclaw_messages: Vec<UnifiedMessage> = scan_result
        .openclaw_files
        .par_iter()
        .flat_map(|path| {
            sessions::openclaw::parse_openclaw_transcript(path)
                .into_iter()
                .map(|mut msg| {
                    msg.cost = pricing.calculate_cost(
                        &msg.model_id,
                        msg.tokens.input,
                        msg.tokens.output,
                        msg.tokens.cache_read,
                        msg.tokens.cache_write,
                        msg.tokens.reasoning,
                    );
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(openclaw_messages);

    // Parse Pi JSONL files
    let pi_messages: Vec<UnifiedMessage> = scan_result
        .pi_files
        .par_iter()
        .flat_map(|path| {
            sessions::pi::parse_pi_file(path)
                .into_iter()
                .map(|mut msg| {
                    msg.cost = pricing.calculate_cost(
                        &msg.model_id,
                        msg.tokens.input,
                        msg.tokens.output,
                        msg.tokens.cache_read,
                        msg.tokens.cache_write,
                        msg.tokens.reasoning,
                    );
                    msg
                })
                .collect::<Vec<_>>()
        })
        .collect();
    all_messages.extend(pi_messages);

    all_messages
}

/// Get model usage report with pricing calculation
#[napi]
pub async fn get_model_report(options: ReportOptions) -> napi::Result<ModelReport> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let sources = options.sources.clone().unwrap_or_else(|| {
        vec![
            "opencode".to_string(),
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
            "cursor".to_string(),
            "amp".to_string(),
            "droid".to_string(),
            "openclaw".to_string(),
            "pi".to_string(),
        ]
    });

    let pricing = pricing::PricingService::get_or_init()
        .await
        .map_err(|e| napi::Error::from_reason(e))?;
    let all_messages = parse_all_messages_with_pricing(&home_dir, &sources, &pricing);

    // Apply date filters
    let filtered = filter_messages_for_report(all_messages, &options);

    // Aggregate by model
    let mut model_map: std::collections::HashMap<String, ModelUsage> =
        std::collections::HashMap::new();

    for msg in filtered {
        let key = format!("{}:{}:{}", msg.source, msg.provider_id, msg.model_id);
        let entry = model_map.entry(key).or_insert_with(|| ModelUsage {
            source: msg.source.clone(),
            model: msg.model_id.clone(),
            provider: msg.provider_id.clone(),
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 0,
            cost: 0.0,
        });

        entry.input += msg.tokens.input;
        entry.output += msg.tokens.output;
        entry.cache_read += msg.tokens.cache_read;
        entry.cache_write += msg.tokens.cache_write;
        entry.reasoning += msg.tokens.reasoning;
        entry.message_count += 1;
        entry.cost += msg.cost;
    }

    let mut entries: Vec<ModelUsage> = model_map.into_values().collect();
    // Sort by cost descending (NaN values sorted to the end)
    entries.sort_by(|a, b| {
        // Handle NaN: treat as smallest value so they sort to the end in descending order
        match (a.cost.is_nan(), b.cost.is_nan()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater, // NaN sorts after valid values
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => b
                .cost
                .partial_cmp(&a.cost)
                .unwrap_or(std::cmp::Ordering::Equal),
        }
    });

    let total_input: i64 = entries.iter().map(|e| e.input).sum();
    let total_output: i64 = entries.iter().map(|e| e.output).sum();
    let total_cache_read: i64 = entries.iter().map(|e| e.cache_read).sum();
    let total_cache_write: i64 = entries.iter().map(|e| e.cache_write).sum();
    let total_messages: i32 = entries.iter().map(|e| e.message_count).sum();
    let total_cost: f64 = entries.iter().map(|e| e.cost).sum();

    Ok(ModelReport {
        entries,
        total_input,
        total_output,
        total_cache_read,
        total_cache_write,
        total_messages,
        total_cost,
        processing_time_ms: start.elapsed().as_millis() as u32,
    })
}

/// Helper struct for aggregating monthly data (avoids clippy::type_complexity)
#[derive(Default)]
struct MonthAggregator {
    models: std::collections::HashSet<String>,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    message_count: i32,
    cost: f64,
}

/// Get monthly usage report with pricing calculation
#[napi]
pub async fn get_monthly_report(options: ReportOptions) -> napi::Result<MonthlyReport> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let sources = options.sources.clone().unwrap_or_else(|| {
        vec![
            "opencode".to_string(),
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
            "cursor".to_string(),
            "amp".to_string(),
            "droid".to_string(),
            "openclaw".to_string(),
            "pi".to_string(),
        ]
    });

    let pricing = pricing::PricingService::get_or_init()
        .await
        .map_err(|e| napi::Error::from_reason(e))?;
    let all_messages = parse_all_messages_with_pricing(&home_dir, &sources, &pricing);

    // Apply date filters
    let filtered = filter_messages_for_report(all_messages, &options);

    // Aggregate by month
    let mut month_map: std::collections::HashMap<String, MonthAggregator> =
        std::collections::HashMap::new();

    for msg in filtered {
        // Extract month from date (YYYY-MM-DD -> YYYY-MM)
        let month = if msg.date.len() >= 7 {
            msg.date[..7].to_string()
        } else {
            continue;
        };

        let entry = month_map.entry(month).or_default();

        entry.models.insert(msg.model_id.clone());
        entry.input += msg.tokens.input;
        entry.output += msg.tokens.output;
        entry.cache_read += msg.tokens.cache_read;
        entry.cache_write += msg.tokens.cache_write;
        entry.message_count += 1;
        entry.cost += msg.cost;
    }

    let mut entries: Vec<MonthlyUsage> = month_map
        .into_iter()
        .map(|(month, agg)| MonthlyUsage {
            month,
            models: agg.models.into_iter().collect(),
            input: agg.input,
            output: agg.output,
            cache_read: agg.cache_read,
            cache_write: agg.cache_write,
            message_count: agg.message_count,
            cost: agg.cost,
        })
        .collect();

    // Sort by month ascending
    entries.sort_by(|a, b| a.month.cmp(&b.month));

    let total_cost: f64 = entries.iter().map(|e| e.cost).sum();

    Ok(MonthlyReport {
        entries,
        total_cost,
        processing_time_ms: start.elapsed().as_millis() as u32,
    })
}

/// Generate graph data with pricing calculation
#[napi]
pub async fn generate_graph_with_pricing(options: ReportOptions) -> napi::Result<GraphResult> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let sources = options.sources.clone().unwrap_or_else(|| {
        vec![
            "opencode".to_string(),
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
            "cursor".to_string(),
            "amp".to_string(),
            "droid".to_string(),
            "openclaw".to_string(),
            "pi".to_string(),
        ]
    });

    let pricing = pricing::PricingService::get_or_init()
        .await
        .map_err(|e| napi::Error::from_reason(e))?;
    let all_messages = parse_all_messages_with_pricing(&home_dir, &sources, &pricing);

    // Apply date filters
    let filtered = filter_messages_for_report(all_messages, &options);

    // Aggregate by date
    let contributions = aggregator::aggregate_by_date(filtered);

    // Generate result
    let processing_time_ms = start.elapsed().as_millis() as u32;
    let result = aggregator::generate_graph_result(contributions, processing_time_ms);

    Ok(result)
}

/// Filter messages by date range (for reports)
fn filter_messages_for_report(
    messages: Vec<UnifiedMessage>,
    options: &ReportOptions,
) -> Vec<UnifiedMessage> {
    let mut filtered = messages;

    // Filter by year
    if let Some(year) = &options.year {
        let year_prefix = format!("{}-", year);
        filtered.retain(|m| m.date.starts_with(&year_prefix));
    }

    // Filter by since date
    if let Some(since) = &options.since {
        filtered.retain(|m| m.date.as_str() >= since.as_str());
    }

    // Filter by until date
    if let Some(until) = &options.until {
        filtered.retain(|m| m.date.as_str() <= until.as_str());
    }

    filtered
}

fn is_headless_path(path: &Path, headless_roots: &[PathBuf]) -> bool {
    headless_roots.iter().any(|root| path.starts_with(root))
}

fn apply_headless_agent(message: &mut UnifiedMessage, is_headless: bool) {
    if is_headless && message.agent.is_none() {
        message.agent = Some("headless".to_string());
    }
}

// =============================================================================
// Two-Phase Processing Functions (for parallel execution optimization)
// =============================================================================

/// Parse local sources only (OpenCode, Claude, Codex, Gemini - NO Cursor)
/// This can run in parallel with network operations (Cursor sync, pricing fetch)
#[napi]
pub fn parse_local_sources(options: LocalParseOptions) -> napi::Result<ParsedMessages> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    // Default to local sources only (no cursor)
    let sources = options.sources.clone().unwrap_or_else(|| {
        vec![
            "opencode".to_string(),
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
            "amp".to_string(),
            "droid".to_string(),
            "openclaw".to_string(),
            "pi".to_string(),
        ]
    });

    // Filter out cursor if somehow included
    let local_sources: Vec<String> = sources.into_iter().filter(|s| s != "cursor").collect();

    let scan_result = scanner::scan_all_sources(&home_dir, &local_sources);
    let headless_roots = scanner::headless_roots(&home_dir);

    let mut messages: Vec<ParsedMessage> = Vec::new();

    // Parse OpenCode: read both SQLite (1.2+) and legacy JSON, deduplicate by message ID
    let opencode_count: i32 = {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut count: i32 = 0;

        if let Some(db_path) = &scan_result.opencode_db {
            let sqlite_msgs: Vec<(String, ParsedMessage)> =
                sessions::opencode::parse_opencode_sqlite(db_path)
                    .into_iter()
                    .map(|msg| {
                        let key = msg.dedup_key.clone().unwrap_or_default();
                        (key, unified_to_parsed(&msg))
                    })
                    .collect();
            count += sqlite_msgs.len() as i32;
            for (key, parsed) in sqlite_msgs {
                if !key.is_empty() {
                    seen.insert(key);
                }
                messages.push(parsed);
            }
        }

        let json_msgs: Vec<(String, ParsedMessage)> = scan_result
            .opencode_files
            .par_iter()
            .filter_map(|path| {
                let msg = sessions::opencode::parse_opencode_file(path)?;
                let key = msg.dedup_key.clone().unwrap_or_default();
                Some((key, unified_to_parsed(&msg)))
            })
            .collect();
        let deduped: Vec<ParsedMessage> = json_msgs
            .into_iter()
            .filter(|(key, _)| key.is_empty() || seen.insert(key.clone()))
            .map(|(_, msg)| msg)
            .collect();
        count += deduped.len() as i32;
        messages.extend(deduped);

        count
    };

    // Parse Claude files in parallel, then deduplicate globally
    let claude_msgs_raw: Vec<(String, ParsedMessage)> = scan_result
        .claude_files
        .par_iter()
        .flat_map(|path| {
            sessions::claudecode::parse_claude_file(path)
                .into_iter()
                .map(|msg| {
                    let dedup_key = msg.dedup_key.clone().unwrap_or_default();
                    (dedup_key, unified_to_parsed(&msg))
                })
                .collect::<Vec<_>>()
        })
        .collect();

    // Global deduplication across all Claude files
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let claude_msgs: Vec<ParsedMessage> = claude_msgs_raw
        .into_iter()
        .filter(|(key, _)| key.is_empty() || seen_keys.insert(key.clone()))
        .map(|(_, msg)| msg)
        .collect();
    let claude_count = claude_msgs.len() as i32;
    messages.extend(claude_msgs);

    // Parse Codex files in parallel
    let codex_msgs: Vec<ParsedMessage> = scan_result
        .codex_files
        .par_iter()
        .flat_map(|path| {
            let is_headless = is_headless_path(path, &headless_roots);
            sessions::codex::parse_codex_file(path)
                .into_iter()
                .map(|mut msg| {
                    apply_headless_agent(&mut msg, is_headless);
                    unified_to_parsed(&msg)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let codex_count = codex_msgs.len() as i32;
    messages.extend(codex_msgs);

    // Parse Gemini files in parallel
    let gemini_msgs: Vec<ParsedMessage> = scan_result
        .gemini_files
        .par_iter()
        .flat_map(|path| {
            sessions::gemini::parse_gemini_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let gemini_count = gemini_msgs.len() as i32;
    messages.extend(gemini_msgs);

    // Parse Amp files in parallel
    let amp_msgs: Vec<ParsedMessage> = scan_result
        .amp_files
        .par_iter()
        .flat_map(|path| {
            sessions::amp::parse_amp_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let amp_count = amp_msgs.len() as i32;
    messages.extend(amp_msgs);

    // Parse Droid files in parallel
    let droid_msgs: Vec<ParsedMessage> = scan_result
        .droid_files
        .par_iter()
        .flat_map(|path| {
            sessions::droid::parse_droid_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let droid_count = droid_msgs.len() as i32;
    messages.extend(droid_msgs);

    // Parse OpenClaw transcript JSONL files
    let openclaw_msgs: Vec<ParsedMessage> = scan_result
        .openclaw_files
        .par_iter()
        .flat_map(|path| {
            sessions::openclaw::parse_openclaw_transcript(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let openclaw_count = openclaw_msgs.len() as i32;
    messages.extend(openclaw_msgs);

    // Parse Pi files in parallel
    let pi_msgs: Vec<ParsedMessage> = scan_result
        .pi_files
        .par_iter()
        .flat_map(|path| {
            sessions::pi::parse_pi_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let pi_count = pi_msgs.len() as i32;
    messages.extend(pi_msgs);

    // Apply date filters
    let filtered = filter_parsed_messages(messages, &options);

    Ok(ParsedMessages {
        messages: filtered,
        opencode_count,
        claude_count,
        codex_count,
        gemini_count,
        amp_count,
        droid_count,
        openclaw_count,
        pi_count,
        processing_time_ms: start.elapsed().as_millis() as u32,
    })
}

fn unified_to_parsed(msg: &UnifiedMessage) -> ParsedMessage {
    ParsedMessage {
        source: msg.source.clone(),
        model_id: msg.model_id.clone(),
        provider_id: msg.provider_id.clone(),
        session_id: msg.session_id.clone(),
        timestamp: msg.timestamp,
        date: msg.date.clone(),
        input: msg.tokens.input,
        output: msg.tokens.output,
        cache_read: msg.tokens.cache_read,
        cache_write: msg.tokens.cache_write,
        reasoning: msg.tokens.reasoning,
        agent: msg.agent.clone(),
    }
}

fn apply_date_filters<T>(
    items: &mut Vec<T>,
    since_ts: Option<i64>,
    until_ts: Option<i64>,
    since: &Option<String>,
    until: &Option<String>,
    year: &Option<String>,
    get_timestamp: impl Fn(&T) -> i64,
    get_date: impl Fn(&T) -> &str,
) {
    if since_ts.is_some() || until_ts.is_some() {
        if let Some(since_ts) = since_ts {
            items.retain(|item| get_timestamp(item) >= since_ts);
        }
        if let Some(until_ts) = until_ts {
            items.retain(|item| get_timestamp(item) < until_ts);
        }
    } else {
        if let Some(year) = year {
            let year_prefix = format!("{}-", year);
            items.retain(|item| get_date(item).starts_with(&year_prefix));
        }
        if let Some(since) = since {
            items.retain(|item| get_date(item) >= since.as_str());
        }
        if let Some(until) = until {
            items.retain(|item| get_date(item) <= until.as_str());
        }
    }
}

/// Filter parsed messages by date range or timestamp range
fn filter_parsed_messages(
    messages: Vec<ParsedMessage>,
    options: &LocalParseOptions,
) -> Vec<ParsedMessage> {
    let mut filtered = messages;

    apply_date_filters(
        &mut filtered,
        options.since_ts,
        options.until_ts,
        &options.since,
        &options.until,
        &options.year,
        |m| m.timestamp,
        |m| m.date.as_str(),
    );

    filtered
}

fn parsed_to_unified(msg: &ParsedMessage, cost: f64) -> UnifiedMessage {
    UnifiedMessage {
        source: msg.source.clone(),
        model_id: msg.model_id.clone(),
        provider_id: msg.provider_id.clone(),
        session_id: msg.session_id.clone(),
        timestamp: msg.timestamp,
        date: msg.date.clone(),
        tokens: TokenBreakdown {
            input: msg.input,
            output: msg.output,
            cache_read: msg.cache_read,
            cache_write: msg.cache_write,
            reasoning: msg.reasoning,
        },
        cost,
        agent: msg.agent.clone(),
        dedup_key: None,
    }
}

/// Finalize model report: apply pricing to local messages, add Cursor, aggregate
#[napi]
pub async fn finalize_report(options: FinalizeReportOptions) -> napi::Result<ModelReport> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let pricing = pricing::PricingService::get_or_init()
        .await
        .map_err(|e| napi::Error::from_reason(e))?;

    // Convert local messages and apply pricing
    let mut all_messages: Vec<UnifiedMessage> = options
        .local_messages
        .messages
        .iter()
        .map(|msg| {
            let cost = pricing.calculate_cost(
                &msg.model_id,
                msg.input,
                msg.output,
                msg.cache_read,
                msg.cache_write,
                msg.reasoning,
            );
            parsed_to_unified(msg, cost)
        })
        .collect();

    // Add Cursor messages if enabled
    if options.include_cursor {
        let cursor_cache_dir = format!("{}/.config/tokscale/cursor-cache", home_dir);
        let cursor_files = scanner::scan_directory(&cursor_cache_dir, "usage*.csv");

        let cursor_messages: Vec<UnifiedMessage> = cursor_files
            .par_iter()
            .flat_map(|path| {
                sessions::cursor::parse_cursor_file(path)
                    .into_iter()
                    .map(|mut msg| {
                        let csv_cost = msg.cost;
                        let calculated_cost = pricing.calculate_cost(
                            &msg.model_id,
                            msg.tokens.input,
                            msg.tokens.output,
                            msg.tokens.cache_read,
                            msg.tokens.cache_write,
                            msg.tokens.reasoning,
                        );
                        msg.cost = if calculated_cost > 0.0 {
                            calculated_cost
                        } else {
                            csv_cost
                        };
                        msg
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        all_messages.extend(cursor_messages);
    }

    // Apply date filters to cursor messages (local already filtered)
    if options.include_cursor {
        apply_date_filters(
            &mut all_messages,
            options.since_ts,
            options.until_ts,
            &options.since,
            &options.until,
            &options.year,
            |m| m.timestamp,
            |m| m.date.as_str(),
        );
    }

    // Aggregate by model
    let mut model_map: std::collections::HashMap<String, ModelUsage> =
        std::collections::HashMap::new();

    for msg in all_messages {
        let key = format!("{}:{}:{}", msg.source, msg.provider_id, msg.model_id);
        let entry = model_map.entry(key).or_insert_with(|| ModelUsage {
            source: msg.source.clone(),
            model: msg.model_id.clone(),
            provider: msg.provider_id.clone(),
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 0,
            cost: 0.0,
        });

        entry.input += msg.tokens.input;
        entry.output += msg.tokens.output;
        entry.cache_read += msg.tokens.cache_read;
        entry.cache_write += msg.tokens.cache_write;
        entry.reasoning += msg.tokens.reasoning;
        entry.message_count += 1;
        entry.cost += msg.cost;
    }

    let mut entries: Vec<ModelUsage> = model_map.into_values().collect();
    entries.sort_by(|a, b| match (a.cost.is_nan(), b.cost.is_nan()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => b
            .cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal),
    });

    let total_input: i64 = entries.iter().map(|e| e.input).sum();
    let total_output: i64 = entries.iter().map(|e| e.output).sum();
    let total_cache_read: i64 = entries.iter().map(|e| e.cache_read).sum();
    let total_cache_write: i64 = entries.iter().map(|e| e.cache_write).sum();
    let total_messages: i32 = entries.iter().map(|e| e.message_count).sum();
    let total_cost: f64 = entries.iter().map(|e| e.cost).sum();

    Ok(ModelReport {
        entries,
        total_input,
        total_output,
        total_cache_read,
        total_cache_write,
        total_messages,
        total_cost,
        processing_time_ms: start.elapsed().as_millis() as u32,
    })
}

/// Options for finalizing monthly report
#[napi(object)]
#[derive(Debug, Clone)]
pub struct FinalizeMonthlyOptions {
    pub home_dir: Option<String>,
    pub local_messages: ParsedMessages,
    pub include_cursor: bool,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
}

/// Finalize monthly report
#[napi]
pub async fn finalize_monthly_report(options: FinalizeMonthlyOptions) -> napi::Result<MonthlyReport> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let pricing = pricing::PricingService::get_or_init()
        .await
        .map_err(|e| napi::Error::from_reason(e))?;

    // Convert local messages and apply pricing
    let mut all_messages: Vec<UnifiedMessage> = options
        .local_messages
        .messages
        .iter()
        .map(|msg| {
            let cost = pricing.calculate_cost(
                &msg.model_id,
                msg.input,
                msg.output,
                msg.cache_read,
                msg.cache_write,
                msg.reasoning,
            );
            parsed_to_unified(msg, cost)
        })
        .collect();

    // Add Cursor messages if enabled
    if options.include_cursor {
        let cursor_cache_dir = format!("{}/.config/tokscale/cursor-cache", home_dir);
        let cursor_files = scanner::scan_directory(&cursor_cache_dir, "usage*.csv");

        let cursor_messages: Vec<UnifiedMessage> = cursor_files
            .par_iter()
            .flat_map(|path| {
                sessions::cursor::parse_cursor_file(path)
                    .into_iter()
                    .map(|mut msg| {
                        let csv_cost = msg.cost;
                        let calculated_cost = pricing.calculate_cost(
                            &msg.model_id,
                            msg.tokens.input,
                            msg.tokens.output,
                            msg.tokens.cache_read,
                            msg.tokens.cache_write,
                            msg.tokens.reasoning,
                        );
                        msg.cost = if calculated_cost > 0.0 {
                            calculated_cost
                        } else {
                            csv_cost
                        };
                        msg
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        all_messages.extend(cursor_messages);
    }

    // Apply date filters
    apply_date_filters(
        &mut all_messages,
        options.since_ts,
        options.until_ts,
        &options.since,
        &options.until,
        &options.year,
        |m| m.timestamp,
        |m| m.date.as_str(),
    );

    // Aggregate by month
    let mut month_map: std::collections::HashMap<String, MonthAggregator> =
        std::collections::HashMap::new();

    for msg in all_messages {
        let month = if msg.date.len() >= 7 {
            msg.date[..7].to_string()
        } else {
            continue;
        };

        let entry = month_map.entry(month).or_default();
        entry.models.insert(msg.model_id.clone());
        entry.input += msg.tokens.input;
        entry.output += msg.tokens.output;
        entry.cache_read += msg.tokens.cache_read;
        entry.cache_write += msg.tokens.cache_write;
        entry.message_count += 1;
        entry.cost += msg.cost;
    }

    let mut entries: Vec<MonthlyUsage> = month_map
        .into_iter()
        .map(|(month, agg)| MonthlyUsage {
            month,
            models: agg.models.into_iter().collect(),
            input: agg.input,
            output: agg.output,
            cache_read: agg.cache_read,
            cache_write: agg.cache_write,
            message_count: agg.message_count,
            cost: agg.cost,
        })
        .collect();

    entries.sort_by(|a, b| a.month.cmp(&b.month));
    let total_cost: f64 = entries.iter().map(|e| e.cost).sum();

    Ok(MonthlyReport {
        entries,
        total_cost,
        processing_time_ms: start.elapsed().as_millis() as u32,
    })
}

/// Options for finalizing graph
#[napi(object)]
#[derive(Debug, Clone)]
pub struct FinalizeGraphOptions {
    pub home_dir: Option<String>,
    pub local_messages: ParsedMessages,
    pub include_cursor: bool,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
}

/// Finalize graph
#[napi]
pub async fn finalize_graph(options: FinalizeGraphOptions) -> napi::Result<GraphResult> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let pricing = pricing::PricingService::get_or_init()
        .await
        .map_err(|e| napi::Error::from_reason(e))?;

    // Convert local messages and apply pricing
    let mut all_messages: Vec<UnifiedMessage> = options
        .local_messages
        .messages
        .iter()
        .map(|msg| {
            let cost = pricing.calculate_cost(
                &msg.model_id,
                msg.input,
                msg.output,
                msg.cache_read,
                msg.cache_write,
                msg.reasoning,
            );
            parsed_to_unified(msg, cost)
        })
        .collect();

    // Add Cursor messages if enabled
    if options.include_cursor {
        let cursor_cache_dir = format!("{}/.config/tokscale/cursor-cache", home_dir);
        let cursor_files = scanner::scan_directory(&cursor_cache_dir, "usage*.csv");

        let cursor_messages: Vec<UnifiedMessage> = cursor_files
            .par_iter()
            .flat_map(|path| {
                sessions::cursor::parse_cursor_file(path)
                    .into_iter()
                    .map(|mut msg| {
                        let csv_cost = msg.cost;
                        let calculated_cost = pricing.calculate_cost(
                            &msg.model_id,
                            msg.tokens.input,
                            msg.tokens.output,
                            msg.tokens.cache_read,
                            msg.tokens.cache_write,
                            msg.tokens.reasoning,
                        );
                        msg.cost = if calculated_cost > 0.0 {
                            calculated_cost
                        } else {
                            csv_cost
                        };
                        msg
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        all_messages.extend(cursor_messages);
    }

    // Apply date filters
    apply_date_filters(
        &mut all_messages,
        options.since_ts,
        options.until_ts,
        &options.since,
        &options.until,
        &options.year,
        |m| m.timestamp,
        |m| m.date.as_str(),
    );

    // Aggregate by date
    let contributions = aggregator::aggregate_by_date(all_messages);

    // Generate result
    let processing_time_ms = start.elapsed().as_millis() as u32;
    let result = aggregator::generate_graph_result(contributions, processing_time_ms);

    Ok(result)
}

/// Combined result for report and graph (single pricing lookup)
#[napi(object)]
pub struct ReportAndGraph {
    pub report: ModelReport,
    pub graph: GraphResult,
}

/// Finalize both report and graph in a single call with shared pricing
/// This ensures consistent costs between report and graph data
#[napi]
pub async fn finalize_report_and_graph(options: FinalizeReportOptions) -> napi::Result<ReportAndGraph> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    // Single pricing lookup - shared by both report and graph
    let pricing = pricing::PricingService::get_or_init()
        .await
        .map_err(|e| napi::Error::from_reason(e))?;

    // Convert local messages and apply pricing (once)
    let mut all_messages: Vec<UnifiedMessage> = options
        .local_messages
        .messages
        .iter()
        .map(|msg| {
            let cost = pricing.calculate_cost(
                &msg.model_id,
                msg.input,
                msg.output,
                msg.cache_read,
                msg.cache_write,
                msg.reasoning,
            );
            parsed_to_unified(msg, cost)
        })
        .collect();

    // Add Cursor messages if enabled
    if options.include_cursor {
        let cursor_cache_dir = format!("{}/.config/tokscale/cursor-cache", home_dir);
        let cursor_files = scanner::scan_directory(&cursor_cache_dir, "usage*.csv");

        let cursor_messages: Vec<UnifiedMessage> = cursor_files
            .par_iter()
            .flat_map(|path| {
                sessions::cursor::parse_cursor_file(path)
                    .into_iter()
                    .map(|mut msg| {
                        let csv_cost = msg.cost;
                        let calculated_cost = pricing.calculate_cost(
                            &msg.model_id,
                            msg.tokens.input,
                            msg.tokens.output,
                            msg.tokens.cache_read,
                            msg.tokens.cache_write,
                            msg.tokens.reasoning,
                        );
                        msg.cost = if calculated_cost > 0.0 {
                            calculated_cost
                        } else {
                            csv_cost
                        };
                        msg
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        all_messages.extend(cursor_messages);
    }

    // Apply date filters
    apply_date_filters(
        &mut all_messages,
        options.since_ts,
        options.until_ts,
        &options.since,
        &options.until,
        &options.year,
        |m| m.timestamp,
        |m| m.date.as_str(),
    );

    // Clone messages for graph aggregation (report consumes for model aggregation)
    let messages_for_graph = all_messages.clone();

    // --- Generate Report ---
    let mut model_map: std::collections::HashMap<String, ModelUsage> =
        std::collections::HashMap::new();

    for msg in all_messages {
        let key = format!("{}:{}:{}", msg.source, msg.provider_id, msg.model_id);
        let entry = model_map.entry(key).or_insert_with(|| ModelUsage {
            source: msg.source.clone(),
            model: msg.model_id.clone(),
            provider: msg.provider_id.clone(),
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 0,
            cost: 0.0,
        });

        entry.input += msg.tokens.input;
        entry.output += msg.tokens.output;
        entry.cache_read += msg.tokens.cache_read;
        entry.cache_write += msg.tokens.cache_write;
        entry.reasoning += msg.tokens.reasoning;
        entry.message_count += 1;
        entry.cost += msg.cost;
    }

    let mut entries: Vec<ModelUsage> = model_map.into_values().collect();
    entries.sort_by(|a, b| match (a.cost.is_nan(), b.cost.is_nan()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => b
            .cost
            .partial_cmp(&a.cost)
            .unwrap_or(std::cmp::Ordering::Equal),
    });

    let total_input: i64 = entries.iter().map(|e| e.input).sum();
    let total_output: i64 = entries.iter().map(|e| e.output).sum();
    let total_cache_read: i64 = entries.iter().map(|e| e.cache_read).sum();
    let total_cache_write: i64 = entries.iter().map(|e| e.cache_write).sum();
    let total_messages: i32 = entries.iter().map(|e| e.message_count).sum();
    let total_cost: f64 = entries.iter().map(|e| e.cost).sum();

    let report = ModelReport {
        entries,
        total_input,
        total_output,
        total_cache_read,
        total_cache_write,
        total_messages,
        total_cost,
        processing_time_ms: start.elapsed().as_millis() as u32,
    };

    // --- Generate Graph ---
    let contributions = aggregator::aggregate_by_date(messages_for_graph);
    let graph = aggregator::generate_graph_result(contributions, start.elapsed().as_millis() as u32);

    Ok(ReportAndGraph { report, graph })
}

// =============================================================================
// New Pricing API (Rust-native pricing fetching)
// =============================================================================

#[napi(object)]
pub struct NativePricing {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    pub cache_read_input_token_cost: Option<f64>,
    pub cache_creation_input_token_cost: Option<f64>,
}

#[napi(object)]
pub struct PricingLookupResult {
    pub model_id: String,
    pub matched_key: String,
    pub source: String,
    pub pricing: NativePricing,
}

#[napi]
pub async fn lookup_pricing(model_id: String, provider: Option<String>) -> napi::Result<PricingLookupResult> {
    let service = pricing::PricingService::get_or_init()
        .await
        .map_err(|e| napi::Error::from_reason(e))?;

    let force_source = provider.as_deref();
    
    match service.lookup_with_source(&model_id, force_source) {
        Some(result) => Ok(PricingLookupResult {
            model_id,
            matched_key: result.matched_key,
            source: result.source,
            pricing: NativePricing {
                input_cost_per_token: result.pricing.input_cost_per_token.unwrap_or(0.0),
                output_cost_per_token: result.pricing.output_cost_per_token.unwrap_or(0.0),
                cache_read_input_token_cost: result.pricing.cache_read_input_token_cost,
                cache_creation_input_token_cost: result.pricing.cache_creation_input_token_cost,
            },
        }),
        None => Err(napi::Error::from_reason(format!(
            "Model not found: {}{}",
            model_id,
            force_source.map(|s| format!(" (forced source: {})", s)).unwrap_or_default()
        ))),
    }
}
