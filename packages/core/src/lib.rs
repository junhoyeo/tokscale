//! Token Tracker Core - Native Rust module for high-performance session parsing
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
pub use pricing::PricingData;
pub use scanner::*;

/// Version of the native module
#[napi]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Simple health check to verify the native module is working
#[napi]
pub fn health_check() -> String {
    "token-tracker-core is healthy!".to_string()
}

/// Configuration options for graph generation
#[napi(object)]
#[derive(Debug, Clone)]
pub struct GraphOptions {
    /// Home directory path (defaults to user's home)
    pub home_dir: Option<String>,
    /// Sources to include: "opencode", "claude", "codex", "gemini"
    pub sources: Option<Vec<String>>,
    /// Start date filter (YYYY-MM-DD)
    pub since: Option<String>,
    /// End date filter (YYYY-MM-DD)
    pub until: Option<String>,
    /// Filter to specific year
    pub year: Option<String>,
    /// Number of parallel threads (defaults to CPU count)
    pub threads: Option<u32>,
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

/// Parsed message without cost (pricing applied in finalize step)
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
}

/// Options for finalizing report with pricing
#[napi(object)]
#[derive(Debug, Clone)]
pub struct FinalizeReportOptions {
    pub home_dir: Option<String>,
    pub local_messages: ParsedMessages,
    pub pricing: Vec<PricingEntry>,
    pub include_cursor: bool,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
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
// Main NAPI Export: generateGraph
// =============================================================================

use rayon::prelude::*;
use sessions::UnifiedMessage;
use std::time::Instant;

/// Get home directory from options or environment, returning error if not available
fn get_home_dir(home_dir_option: &Option<String>) -> napi::Result<String> {
    home_dir_option
        .clone()
        .or_else(|| std::env::var("HOME").ok())
        .ok_or_else(|| {
            napi::Error::from_reason(
                "HOME directory not specified and HOME environment variable not set",
            )
        })
}

/// Generate graph data from all session sources
///
/// This is the main entry point that orchestrates:
/// 1. Parallel file scanning
/// 2. Parallel session parsing
/// 3. Date filtering
/// 4. Parallel aggregation
#[napi]
pub fn generate_graph(options: GraphOptions) -> napi::Result<GraphResult> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    // Get sources to scan
    let sources = options.sources.clone().unwrap_or_else(|| {
        vec![
            "opencode".to_string(),
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
            "cursor".to_string(),
        ]
    });

    // Configure thread pool if specified
    if let Some(threads) = options.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads as usize)
            .build_global()
            .ok();
    }

    // 1. Parallel file scanning
    let scan_result = scanner::scan_all_sources(&home_dir, &sources);

    // 2. Parallel session parsing
    let mut all_messages: Vec<UnifiedMessage> = Vec::new();

    // Parse OpenCode files in parallel
    let opencode_messages: Vec<UnifiedMessage> = scan_result
        .opencode_files
        .par_iter()
        .filter_map(|path| sessions::opencode::parse_opencode_file(path))
        .collect();
    all_messages.extend(opencode_messages);

    // Parse Claude files in parallel
    let claude_messages: Vec<UnifiedMessage> = scan_result
        .claude_files
        .par_iter()
        .flat_map(|path| sessions::claudecode::parse_claude_file(path))
        .collect();
    all_messages.extend(claude_messages);

    // Parse Codex files in parallel
    let codex_messages: Vec<UnifiedMessage> = scan_result
        .codex_files
        .par_iter()
        .flat_map(|path| sessions::codex::parse_codex_file(path))
        .collect();
    all_messages.extend(codex_messages);

    // Parse Gemini files in parallel
    let gemini_messages: Vec<UnifiedMessage> = scan_result
        .gemini_files
        .par_iter()
        .flat_map(|path| sessions::gemini::parse_gemini_file(path))
        .collect();
    all_messages.extend(gemini_messages);

    // Parse Cursor files in parallel
    let cursor_messages: Vec<UnifiedMessage> = scan_result
        .cursor_files
        .par_iter()
        .flat_map(|path| sessions::cursor::parse_cursor_file(path))
        .collect();
    all_messages.extend(cursor_messages);

    // 3. Apply date filters
    let filtered_messages = filter_messages(all_messages, &options);

    // 4. Parallel aggregation
    let contributions = aggregator::aggregate_by_date(filtered_messages);

    // 5. Generate result
    let processing_time_ms = start.elapsed().as_millis() as u32;
    let result = aggregator::generate_graph_result(contributions, processing_time_ms);

    Ok(result)
}

/// Filter messages by date range options
fn filter_messages(messages: Vec<UnifiedMessage>, options: &GraphOptions) -> Vec<UnifiedMessage> {
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

/// Scan session files and return file counts per source
#[napi(object)]
pub struct ScanStats {
    pub opencode_files: i32,
    pub claude_files: i32,
    pub codex_files: i32,
    pub gemini_files: i32,
    pub cursor_files: i32,
    pub total_files: i32,
}

/// Scan for session files (for debugging/testing)
#[napi]
pub fn scan_sessions(home_dir: Option<String>, sources: Option<Vec<String>>) -> napi::Result<ScanStats> {
    let home = get_home_dir(&home_dir)?;

    let srcs = sources.unwrap_or_else(|| {
        vec![
            "opencode".to_string(),
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
            "cursor".to_string(),
        ]
    });

    let result = scanner::scan_all_sources(&home, &srcs);

    Ok(ScanStats {
        opencode_files: result.opencode_files.len() as i32,
        claude_files: result.claude_files.len() as i32,
        codex_files: result.codex_files.len() as i32,
        gemini_files: result.gemini_files.len() as i32,
        cursor_files: result.cursor_files.len() as i32,
        total_files: result.total_files() as i32,
    })
}

// =============================================================================
// Pricing-aware APIs
// =============================================================================

/// Pricing data for a single model (passed from TypeScript)
#[napi(object)]
#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    pub cache_read_input_token_cost: Option<f64>,
    pub cache_creation_input_token_cost: Option<f64>,
}

/// Entry in the pricing map
#[napi(object)]
#[derive(Debug, Clone)]
pub struct PricingEntry {
    pub model_id: String,
    pub pricing: ModelPricing,
}

/// Options for reports with pricing
#[napi(object)]
#[derive(Debug, Clone)]
pub struct ReportOptions {
    /// Home directory path (defaults to user's home)
    pub home_dir: Option<String>,
    /// Sources to include: "opencode", "claude", "codex", "gemini"
    pub sources: Option<Vec<String>>,
    /// Pricing data for cost calculation
    pub pricing: Vec<PricingEntry>,
    /// Start date filter (YYYY-MM-DD)
    pub since: Option<String>,
    /// End date filter (YYYY-MM-DD)
    pub until: Option<String>,
    /// Filter to specific year
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

/// Convert pricing entries to internal PricingData
fn build_pricing_data(entries: &[PricingEntry]) -> PricingData {
    let mut pricing_data = PricingData::new();
    for entry in entries {
        pricing_data.add_model(
            entry.model_id.clone(),
            pricing::ModelPricing {
                input_cost_per_token: entry.pricing.input_cost_per_token,
                output_cost_per_token: entry.pricing.output_cost_per_token,
                cache_read_input_token_cost: entry
                    .pricing
                    .cache_read_input_token_cost
                    .unwrap_or(0.0),
                cache_creation_input_token_cost: entry
                    .pricing
                    .cache_creation_input_token_cost
                    .unwrap_or(0.0),
            },
        );
    }
    // Pre-compute sorted keys for fast lookups
    pricing_data.finalize();
    pricing_data
}

/// Parse all messages with pricing calculation
fn parse_all_messages_with_pricing(
    home_dir: &str,
    sources: &[String],
    pricing_data: &PricingData,
) -> Vec<UnifiedMessage> {
    let scan_result = scanner::scan_all_sources(home_dir, sources);
    let mut all_messages: Vec<UnifiedMessage> = Vec::new();

    // Parse OpenCode files in parallel
    let opencode_messages: Vec<UnifiedMessage> = scan_result
        .opencode_files
        .par_iter()
        .filter_map(|path| {
            let mut msg = sessions::opencode::parse_opencode_file(path)?;
            // Recalculate cost using pricing data
            msg.cost = pricing_data.calculate_cost(
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
    all_messages.extend(opencode_messages);

    // Parse Claude files in parallel
    let claude_messages: Vec<UnifiedMessage> = scan_result
        .claude_files
        .par_iter()
        .flat_map(|path| {
            sessions::claudecode::parse_claude_file(path)
                .into_iter()
                .map(|mut msg| {
                    msg.cost = pricing_data.calculate_cost(
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
                    msg.cost = pricing_data.calculate_cost(
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
                    msg.cost = pricing_data.calculate_cost(
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
                    let calculated_cost = pricing_data.calculate_cost(
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

    all_messages
}

/// Get model usage report with pricing calculation
#[napi]
pub fn get_model_report(options: ReportOptions) -> napi::Result<ModelReport> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let sources = options.sources.clone().unwrap_or_else(|| {
        vec![
            "opencode".to_string(),
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
            "cursor".to_string(),
        ]
    });

    let pricing_data = build_pricing_data(&options.pricing);
    let all_messages = parse_all_messages_with_pricing(&home_dir, &sources, &pricing_data);

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
pub fn get_monthly_report(options: ReportOptions) -> napi::Result<MonthlyReport> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let sources = options.sources.clone().unwrap_or_else(|| {
        vec![
            "opencode".to_string(),
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
            "cursor".to_string(),
        ]
    });

    let pricing_data = build_pricing_data(&options.pricing);
    let all_messages = parse_all_messages_with_pricing(&home_dir, &sources, &pricing_data);

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
pub fn generate_graph_with_pricing(options: ReportOptions) -> napi::Result<GraphResult> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let sources = options.sources.clone().unwrap_or_else(|| {
        vec![
            "opencode".to_string(),
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
            "cursor".to_string(),
        ]
    });

    let pricing_data = build_pricing_data(&options.pricing);
    let all_messages = parse_all_messages_with_pricing(&home_dir, &sources, &pricing_data);

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
        ]
    });

    // Filter out cursor if somehow included
    let local_sources: Vec<String> = sources.into_iter().filter(|s| s != "cursor").collect();

    let scan_result = scanner::scan_all_sources(&home_dir, &local_sources);

    let mut messages: Vec<ParsedMessage> = Vec::new();

    // Parse OpenCode files in parallel
    let opencode_msgs: Vec<ParsedMessage> = scan_result
        .opencode_files
        .par_iter()
        .filter_map(|path| {
            let msg = sessions::opencode::parse_opencode_file(path)?;
            Some(unified_to_parsed(&msg))
        })
        .collect();
    let opencode_count = opencode_msgs.len() as i32;
    messages.extend(opencode_msgs);

    // Parse Claude files in parallel
    let claude_msgs: Vec<ParsedMessage> = scan_result
        .claude_files
        .par_iter()
        .flat_map(|path| {
            sessions::claudecode::parse_claude_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
                .collect::<Vec<_>>()
        })
        .collect();
    let claude_count = claude_msgs.len() as i32;
    messages.extend(claude_msgs);

    // Parse Codex files in parallel
    let codex_msgs: Vec<ParsedMessage> = scan_result
        .codex_files
        .par_iter()
        .flat_map(|path| {
            sessions::codex::parse_codex_file(path)
                .into_iter()
                .map(|msg| unified_to_parsed(&msg))
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

    // Apply date filters
    let filtered = filter_parsed_messages(messages, &options);

    Ok(ParsedMessages {
        messages: filtered,
        opencode_count,
        claude_count,
        codex_count,
        gemini_count,
        processing_time_ms: start.elapsed().as_millis() as u32,
    })
}

/// Convert UnifiedMessage to ParsedMessage (drops cost)
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
    }
}

/// Filter parsed messages by date range
fn filter_parsed_messages(
    messages: Vec<ParsedMessage>,
    options: &LocalParseOptions,
) -> Vec<ParsedMessage> {
    let mut filtered = messages;

    if let Some(year) = &options.year {
        let year_prefix = format!("{}-", year);
        filtered.retain(|m| m.date.starts_with(&year_prefix));
    }

    if let Some(since) = &options.since {
        filtered.retain(|m| m.date.as_str() >= since.as_str());
    }

    if let Some(until) = &options.until {
        filtered.retain(|m| m.date.as_str() <= until.as_str());
    }

    filtered
}

/// Convert ParsedMessage back to UnifiedMessage with cost
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
    }
}

/// Finalize model report: apply pricing to local messages, add Cursor, aggregate
#[napi]
pub fn finalize_report(options: FinalizeReportOptions) -> napi::Result<ModelReport> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let pricing_data = build_pricing_data(&options.pricing);

    // Convert local messages and apply pricing
    let mut all_messages: Vec<UnifiedMessage> = options
        .local_messages
        .messages
        .iter()
        .map(|msg| {
            let cost = pricing_data.calculate_cost(
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
        let cursor_cache_dir = format!("{}/.config/token-tracker/cursor-cache", home_dir);
        let cursor_files = scanner::scan_directory(&cursor_cache_dir, "*.csv");

        let cursor_messages: Vec<UnifiedMessage> = cursor_files
            .par_iter()
            .flat_map(|path| {
                sessions::cursor::parse_cursor_file(path)
                    .into_iter()
                    .map(|mut msg| {
                        let csv_cost = msg.cost;
                        let calculated_cost = pricing_data.calculate_cost(
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
        if let Some(year) = &options.year {
            let year_prefix = format!("{}-", year);
            all_messages.retain(|m| m.date.starts_with(&year_prefix));
        }
        if let Some(since) = &options.since {
            all_messages.retain(|m| m.date.as_str() >= since.as_str());
        }
        if let Some(until) = &options.until {
            all_messages.retain(|m| m.date.as_str() <= until.as_str());
        }
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
    pub pricing: Vec<PricingEntry>,
    pub include_cursor: bool,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
}

/// Finalize monthly report with pricing
#[napi]
pub fn finalize_monthly_report(options: FinalizeMonthlyOptions) -> napi::Result<MonthlyReport> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let pricing_data = build_pricing_data(&options.pricing);

    // Convert local messages and apply pricing
    let mut all_messages: Vec<UnifiedMessage> = options
        .local_messages
        .messages
        .iter()
        .map(|msg| {
            let cost = pricing_data.calculate_cost(
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
        let cursor_cache_dir = format!("{}/.config/token-tracker/cursor-cache", home_dir);
        let cursor_files = scanner::scan_directory(&cursor_cache_dir, "*.csv");

        let cursor_messages: Vec<UnifiedMessage> = cursor_files
            .par_iter()
            .flat_map(|path| {
                sessions::cursor::parse_cursor_file(path)
                    .into_iter()
                    .map(|mut msg| {
                        let csv_cost = msg.cost;
                        let calculated_cost = pricing_data.calculate_cost(
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
    if let Some(year) = &options.year {
        let year_prefix = format!("{}-", year);
        all_messages.retain(|m| m.date.starts_with(&year_prefix));
    }
    if let Some(since) = &options.since {
        all_messages.retain(|m| m.date.as_str() >= since.as_str());
    }
    if let Some(until) = &options.until {
        all_messages.retain(|m| m.date.as_str() <= until.as_str());
    }

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
    pub pricing: Vec<PricingEntry>,
    pub include_cursor: bool,
    pub since: Option<String>,
    pub until: Option<String>,
    pub year: Option<String>,
}

/// Finalize graph with pricing
#[napi]
pub fn finalize_graph(options: FinalizeGraphOptions) -> napi::Result<GraphResult> {
    let start = Instant::now();

    let home_dir = get_home_dir(&options.home_dir)?;

    let pricing_data = build_pricing_data(&options.pricing);

    // Convert local messages and apply pricing
    let mut all_messages: Vec<UnifiedMessage> = options
        .local_messages
        .messages
        .iter()
        .map(|msg| {
            let cost = pricing_data.calculate_cost(
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
        let cursor_cache_dir = format!("{}/.config/token-tracker/cursor-cache", home_dir);
        let cursor_files = scanner::scan_directory(&cursor_cache_dir, "*.csv");

        let cursor_messages: Vec<UnifiedMessage> = cursor_files
            .par_iter()
            .flat_map(|path| {
                sessions::cursor::parse_cursor_file(path)
                    .into_iter()
                    .map(|mut msg| {
                        let csv_cost = msg.cost;
                        let calculated_cost = pricing_data.calculate_cost(
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
    if let Some(year) = &options.year {
        let year_prefix = format!("{}-", year);
        all_messages.retain(|m| m.date.starts_with(&year_prefix));
    }
    if let Some(since) = &options.since {
        all_messages.retain(|m| m.date.as_str() >= since.as_str());
    }
    if let Some(until) = &options.until {
        all_messages.retain(|m| m.date.as_str() <= until.as_str());
    }

    // Aggregate by date
    let contributions = aggregator::aggregate_by_date(all_messages);

    // Generate result
    let processing_time_ms = start.elapsed().as_millis() as u32;
    let result = aggregator::generate_graph_result(contributions, processing_time_ms);

    Ok(result)
}
