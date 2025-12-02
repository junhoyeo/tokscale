//! Token Tracker Core - Native Rust module for high-performance session parsing
//!
//! This module provides parallel file scanning, SIMD JSON parsing, and efficient
//! aggregation of token usage data from multiple AI coding assistant sessions.

#![deny(clippy::all)]

use napi_derive::napi;

mod aggregator;
mod parser;
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

    // Get home directory
    let home_dir = options
        .home_dir
        .clone()
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_else(|| "/".to_string());

    // Get sources to scan
    let sources = options.sources.clone().unwrap_or_else(|| {
        vec![
            "opencode".to_string(),
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
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
    pub total_files: i32,
}

/// Scan for session files (for debugging/testing)
#[napi]
pub fn scan_sessions(home_dir: Option<String>, sources: Option<Vec<String>>) -> ScanStats {
    let home = home_dir
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_else(|| "/".to_string());

    let srcs = sources.unwrap_or_else(|| {
        vec![
            "opencode".to_string(),
            "claude".to_string(),
            "codex".to_string(),
            "gemini".to_string(),
        ]
    });

    let result = scanner::scan_all_sources(&home, &srcs);

    ScanStats {
        opencode_files: result.opencode_files.len() as i32,
        claude_files: result.claude_files.len() as i32,
        codex_files: result.codex_files.len() as i32,
        gemini_files: result.gemini_files.len() as i32,
        total_files: result.total_files() as i32,
    }
}
