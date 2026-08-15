//! Import historical usage from third-party aggregate exports.
//!
//! Supports the clawdboard.ai account export ("daily aggregates") and
//! `ccusage`'s own `daily --json` output (the `cc.json` that viberank and
//! similar dashboards ingest). The importer normalizes either into tokscale's
//! native [`GraphResult`], which the CLI then writes out as standard tokscale
//! JSON (identical in shape to `tokscale graph`) for review, archival, or a
//! future server-supported backfill.
//!
//! Motivation: `tokscale submit` computes totals from *raw* local session
//! files. Once those files are gone (Claude Code deletes transcripts after
//! `cleanupPeriodDays`, default 30), earlier months can never be re-scanned,
//! even though a competing dashboard may still hold the aggregates. This
//! importer recovers that history into tokscale's format.
//!
//! ccusage shapes, and what the importer assumes about each:
//!
//! - plain `ccusage daily --json` — `date`, no `metadata`, no reasoning. Every
//!   model is attributed by family, since nothing names the agents.
//! - the unified/agent-aware report — `period`, `agent: "all"`, and
//!   `metadata.agents` naming the participants. Reasoning, when present, is
//!   nested under `metadata`, not beside the other token fields.
//! - `ccusage-codex` — spells cost `costUSD` and puts `reasoningOutputTokens`
//!   beside the token fields (see ccusage#831).
//!
//! Across all three, `totalTokens` is `input + output + cacheCreation +
//! cacheRead`; reasoning is never a term of its own in it. See
//! [`ccusage_token_breakdown`] for why that matters when mapping into
//! tokscale's additive buckets.
//!
//! IMPORTANT — upload boundary: importing only *normalizes* data to a file. It
//! does not submit anything to the leaderboard. Backfilled aggregates are not
//! independently verifiable the way locally-scanned sessions are, so uploading
//! them requires server-side support for tagging backfilled submissions
//! distinctly from live CLI usage (so the two are not ranked identically).
//! See <https://github.com/junhoyeo/tokscale/issues/888>.

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet};
use tokscale_core::{
    calculate_intensities, generate_graph_result, ClientContribution, ClientId, DailyContribution,
    DailyTotals, GraphResult, TokenBreakdown,
};

/// Import source formats understood by [`parse_export`].
pub const SUPPORTED_FORMATS: &[&str] = &["clawdboard", "ccusage"];

/// Result of a successful import.
pub struct ImportOutcome {
    /// Normalized usage, ready to serialize as tokscale JSON.
    pub graph: GraphResult,
    /// Client ids present in the export that tokscale does not recognize.
    /// The leaderboard rejects unknown clients, so these are surfaced to the
    /// caller as a warning rather than silently dropped or silently kept.
    pub unknown_clients: Vec<String>,
    /// Number of negative token/cost values that were clamped to zero.
    pub negative_values_clamped: usize,
    /// Number of per-model rows with `cost > 0` but every token field `0`.
    /// The server rejects submissions shaped like this ("Cost submitted
    /// without tokens"), so these are surfaced as a warning rather than
    /// silently dropped — this importer does not upload, so the row is kept
    /// as-is for the caller to inspect. Cursor's legacy `premium-tool-call`
    /// rows are exempt (see [`is_cursor_legacy_tokenless`]), matching the
    /// server's own carve-out.
    pub suspect_cost_rows: usize,
    /// Number of daily aggregate rows dated after today. The submit
    /// endpoint rejects dates too far in the future (see
    /// `submission.ts`'s 2-day buffer), so these are surfaced as a warning.
    pub future_dated_rows: usize,
    /// Number of `totalCost` strings that failed to parse as a valid
    /// float (e.g. `"$1.25"`) and were treated as `0.0`.
    pub unparseable_cost_rows: usize,
    /// Number of non-finite (`NaN`/`Infinity`) cost values sanitized to
    /// `0.0`. Non-finite floats serialize to JSON `null`, which the submit
    /// endpoint rejects.
    pub non_finite_cost_rows: usize,
    /// Number of daily aggregate rows with no `modelBreakdowns` and more
    /// than one entry in `modelsUsed`: all usage in the row is attributed
    /// to the first model, since there is no per-model split to use.
    pub multi_model_fallback_rows: usize,
    /// Human-readable warnings for rows where `modelBreakdowns` are present
    /// but their summed tokens/cost diverge from the aggregate-level
    /// totals beyond a small tolerance — a sign of partial breakdown data.
    pub breakdown_reconciliation_warnings: Vec<String>,
}

/// Parse an export of the given `format` into normalized tokscale data.
pub fn parse_export(format: &str, json: &str) -> Result<ImportOutcome> {
    match format {
        "clawdboard" => parse_clawdboard_export(json),
        "ccusage" => parse_ccusage_export(json),
        other => bail!(
            "unsupported import format '{}' (supported: {})",
            other,
            SUPPORTED_FORMATS.join(", ")
        ),
    }
}

// ---------------------------------------------------------------------------
// clawdboard export schema (only the subset we consume)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawdboardExport {
    #[serde(default)]
    daily_aggregates: Vec<ClawdboardDailyAggregate>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawdboardDailyAggregate {
    date: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_creation_tokens: i64,
    #[serde(default)]
    cache_read_tokens: i64,
    /// clawdboard serializes the aggregate-level cost as a string (e.g.
    /// "0.5859"). Per-model `cost` in `modelBreakdowns` is a plain number.
    #[serde(default)]
    total_cost: Option<String>,
    #[serde(default)]
    models_used: Vec<String>,
    #[serde(default)]
    model_breakdowns: Vec<ClawdboardModelBreakdown>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClawdboardModelBreakdown {
    model_name: String,
    #[serde(default)]
    cost: f64,
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_tokens: i64,
    #[serde(default)]
    cache_creation_tokens: i64,
}

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

/// Map a clawdboard `source` id to a canonical tokscale client id.
fn normalize_client_id(source: &str) -> String {
    match source.trim().to_lowercase().as_str() {
        "claude-code" | "claude_code" | "claudecode" => "claude".to_string(),
        "codex-cli" | "codex_cli" | "ccusage-codex" => "codex".to_string(),
        // `model_family` answers in canonical client ids, and `resolve_client`
        // matches an agent to a model by comparing the two as strings. A CLI
        // suffix that survives normalization therefore never matches its own
        // models and drops the whole agent's usage into `unknown`.
        "gemini-cli" | "gemini_cli" => "gemini".to_string(),
        other => other.to_string(),
    }
}

/// Accumulates per-(client, model) rows within a single day.
#[derive(Default)]
struct DayBuilder {
    clients: BTreeMap<String, ClientContribution>,
}

/// Parse a clawdboard account export into normalized tokscale data.
///
/// Grouping: one [`DailyContribution`] per calendar date; within a day, one
/// [`ClientContribution`] per (client, model), summed across every aggregate
/// row that shares that date (clawdboard splits rows by machine).
pub fn parse_clawdboard_export(json: &str) -> Result<ImportOutcome> {
    let export: ClawdboardExport =
        serde_json::from_str(json).context("failed to parse clawdboard export JSON")?;

    if export.daily_aggregates.is_empty() {
        bail!("clawdboard export contains no dailyAggregates to import");
    }

    let mut days: BTreeMap<String, DayBuilder> = BTreeMap::new();
    let mut unknown: BTreeSet<String> = BTreeSet::new();
    let mut negative_values_clamped = 0usize;
    let mut suspect_cost_rows = 0usize;
    let mut future_dated_rows = 0usize;
    let mut unparseable_cost_rows = 0usize;
    let mut non_finite_cost_rows = 0usize;
    let mut multi_model_fallback_rows = 0usize;
    let mut breakdown_reconciliation_warnings: Vec<String> = Vec::new();
    let today = chrono::Utc::now().date_naive();

    for agg in &export.daily_aggregates {
        let parsed_date = parse_calendar_date(&agg.date)?;
        if parsed_date > today {
            future_dated_rows += 1;
        }

        let client = agg
            .source
            .as_deref()
            .map(normalize_client_id)
            .unwrap_or_else(|| "unknown".to_string());
        if ClientId::from_str(&client).is_none() {
            unknown.insert(client.clone());
        }

        let day = days.entry(agg.date.clone()).or_default();

        if agg.model_breakdowns.is_empty() {
            // No per-model breakdown: synthesize a single row from the
            // aggregate totals so no usage is lost.
            let model = agg
                .models_used
                .first()
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            if agg.models_used.len() > 1 {
                // All usage in this row is attributed to `model` alone;
                // there is no per-model split to divide it by.
                multi_model_fallback_rows += 1;
            }
            let raw_cost = parse_cost_string(agg.total_cost.as_deref(), &mut unparseable_cost_rows);
            let raw_cost = sanitize_cost(raw_cost, &mut non_finite_cost_rows);
            let cost = clamp_f64(raw_cost, &mut negative_values_clamped);
            let tokens = TokenBreakdown {
                input: clamp_i64(agg.input_tokens, &mut negative_values_clamped),
                output: clamp_i64(agg.output_tokens, &mut negative_values_clamped),
                cache_read: clamp_i64(agg.cache_read_tokens, &mut negative_values_clamped),
                cache_write: clamp_i64(agg.cache_creation_tokens, &mut negative_values_clamped),
                reasoning: 0,
            };
            if cost > 0.0 && tokens.total() == 0 && !is_cursor_legacy_tokenless(&client, &model) {
                suspect_cost_rows += 1;
            }
            add_row(day, &client, &model, tokens, cost);
        } else {
            let mut mb_input = 0i64;
            let mut mb_output = 0i64;
            let mut mb_cache_read = 0i64;
            let mut mb_cache_write = 0i64;
            let mut mb_cost = 0.0f64;

            for mb in &agg.model_breakdowns {
                let tokens = TokenBreakdown {
                    input: clamp_i64(mb.input_tokens, &mut negative_values_clamped),
                    output: clamp_i64(mb.output_tokens, &mut negative_values_clamped),
                    cache_read: clamp_i64(mb.cache_read_tokens, &mut negative_values_clamped),
                    cache_write: clamp_i64(mb.cache_creation_tokens, &mut negative_values_clamped),
                    reasoning: 0,
                };
                let raw_cost = sanitize_cost(mb.cost, &mut non_finite_cost_rows);
                let cost = clamp_f64(raw_cost, &mut negative_values_clamped);
                if cost > 0.0
                    && tokens.total() == 0
                    && !is_cursor_legacy_tokenless(&client, &mb.model_name)
                {
                    suspect_cost_rows += 1;
                }

                mb_input = mb_input.saturating_add(tokens.input);
                mb_output = mb_output.saturating_add(tokens.output);
                mb_cache_read = mb_cache_read.saturating_add(tokens.cache_read);
                mb_cache_write = mb_cache_write.saturating_add(tokens.cache_write);
                mb_cost += cost;

                add_row(day, &client, &mb.model_name, tokens, cost);
            }

            // Reconciliation: only compare against aggregate-level totals
            // when the export actually populated them — clawdboard rows
            // sometimes carry only `modelBreakdowns` with no duplicated
            // aggregate scalars, which is not a mismatch.
            let agg_tokens_present = agg.input_tokens != 0
                || agg.output_tokens != 0
                || agg.cache_read_tokens != 0
                || agg.cache_creation_tokens != 0;
            if agg_tokens_present {
                let agg_total = agg
                    .input_tokens
                    .max(0)
                    .saturating_add(agg.output_tokens.max(0))
                    .saturating_add(agg.cache_read_tokens.max(0))
                    .saturating_add(agg.cache_creation_tokens.max(0));
                let mb_total = mb_input
                    .saturating_add(mb_output)
                    .saturating_add(mb_cache_read)
                    .saturating_add(mb_cache_write);
                if tokens_diverge(mb_total, agg_total) {
                    breakdown_reconciliation_warnings.push(format!(
                        "{} {}: modelBreakdowns sum to {} token(s) but aggregate totals report {}",
                        agg.date, client, mb_total, agg_total
                    ));
                }
            }
            if let Some(raw) = agg.total_cost.as_deref() {
                let agg_cost = parse_cost_string(Some(raw), &mut unparseable_cost_rows);
                let agg_cost = sanitize_cost(agg_cost, &mut non_finite_cost_rows);
                if costs_diverge(mb_cost, agg_cost) {
                    breakdown_reconciliation_warnings.push(format!(
                        "{} {}: modelBreakdowns sum to cost {:.4} but aggregate totalCost reports {:.4}",
                        agg.date, client, mb_cost, agg_cost
                    ));
                }
            }
        }
    }

    // BTreeMap iterates dates in sorted order already; the explicit sort keeps
    // the invariant obvious and independent of the map type.
    let mut contributions: Vec<DailyContribution> = days
        .into_iter()
        .map(|(date, builder)| finalize_day(date, builder))
        .collect();
    contributions.sort_by(|a, b| a.date.cmp(&b.date));
    calculate_intensities(&mut contributions);

    // `processing_time_ms = 0`: this data was imported, not scanned.
    let graph = generate_graph_result(contributions, 0);

    Ok(ImportOutcome {
        graph,
        unknown_clients: unknown.into_iter().collect(),
        negative_values_clamped,
        suspect_cost_rows,
        future_dated_rows,
        unparseable_cost_rows,
        non_finite_cost_rows,
        multi_model_fallback_rows,
        breakdown_reconciliation_warnings,
    })
}

// ---------------------------------------------------------------------------
// ccusage export schema (`ccusage daily --json`, a.k.a. cc.json)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct CcusageExport {
    #[serde(default)]
    daily: Vec<CcusageDaily>,
}

/// One calendar day of the unified report.
///
/// Unlike clawdboard, a row is *not* scoped to one client: the unified report
/// merges every detected agent for that date into a single row and records the
/// participants in `metadata.agents`. The per-client split has to be recovered
/// from `modelBreakdowns` — see [`resolve_client`].
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CcusageDaily {
    /// The unified report emits `period`; the per-agent reports (and older
    /// ccusage releases) emit `date`. Accept either rather than forcing users
    /// to know which subcommand produced their file.
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_creation_tokens: i64,
    #[serde(default)]
    cache_read_tokens: i64,
    #[serde(default)]
    reasoning_output_tokens: i64,
    /// `input + output + cacheCreation + cacheRead`, as ccusage computes it.
    /// Reasoning is never a term in this sum — see [`ccusage_token_breakdown`].
    /// Used only as a cross-check; the buckets remain the source of truth.
    #[serde(default)]
    total_tokens: Option<i64>,
    /// Numeric in ccusage, but dashboards that re-serialize cc.json sometimes
    /// stringify it. [`CostValue`] accepts both.
    #[serde(default)]
    total_cost: Option<CostValue>,
    /// `ccusage-codex` spells the same field `costUSD` (see ccusage#831).
    /// Spelled out explicitly: `rename_all = "camelCase"` would derive
    /// `costUsd`, which matches nothing.
    #[serde(default, rename = "costUSD", alias = "costUsd")]
    cost_usd: Option<CostValue>,
    #[serde(default)]
    models_used: Vec<String>,
    #[serde(default)]
    model_breakdowns: Vec<CcusageModelBreakdown>,
    #[serde(default)]
    metadata: Option<CcusageMetadata>,
}

impl CcusageDaily {
    /// `totalCost`, or `ccusage-codex`'s `costUSD` spelling of it.
    fn declared_cost(&self) -> Option<&CostValue> {
        self.total_cost.as_ref().or(self.cost_usd.as_ref())
    }

    /// Reasoning tokens for the row as a whole.
    ///
    /// The unified report nests this under `metadata`; `ccusage-codex` puts it
    /// beside the other token fields. Accept both — reading only the flat
    /// spelling silently scores every real unified export as zero-reasoning.
    fn row_reasoning_tokens(&self) -> i64 {
        let nested = self
            .metadata
            .as_ref()
            .map(|m| m.reasoning_output_tokens)
            .unwrap_or(0);
        self.reasoning_output_tokens.max(nested)
    }

    /// Whether the row carries no usage at all. ccusage emits a row for every
    /// day in the range, including days with nothing on them.
    fn is_empty_day(&self) -> bool {
        self.model_breakdowns.is_empty()
            && self.models_used.is_empty()
            && self.input_tokens <= 0
            && self.output_tokens <= 0
            && self.cache_creation_tokens <= 0
            && self.cache_read_tokens <= 0
            && self.row_reasoning_tokens() <= 0
            && self
                .declared_cost()
                .map(|c| !c.is_positive())
                .unwrap_or(true)
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CcusageMetadata {
    #[serde(default)]
    agents: Vec<String>,
    /// Where the unified report actually records reasoning output.
    #[serde(default)]
    reasoning_output_tokens: i64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CcusageModelBreakdown {
    model_name: String,
    #[serde(default)]
    cost: f64,
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_tokens: i64,
    #[serde(default)]
    cache_creation_tokens: i64,
    #[serde(default)]
    reasoning_output_tokens: i64,
}

/// A cost field that may arrive as a JSON number or as a stringified number.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum CostValue {
    Number(f64),
    Text(String),
}

impl CostValue {
    fn resolve(&self, unparseable_count: &mut usize) -> f64 {
        match self {
            CostValue::Number(v) => *v,
            CostValue::Text(s) => parse_cost_string(Some(s), unparseable_count),
        }
    }

    /// Whether this cost is a real charge, for emptiness checks only. Parse
    /// failures are deliberately not counted here — a row rejected as empty is
    /// never reported on, so counting it would inflate the warning.
    fn is_positive(&self) -> bool {
        let mut ignored = 0usize;
        self.resolve(&mut ignored) > 0.0
    }
}

/// Build a tokscale [`TokenBreakdown`] from one ccusage row or model breakdown.
///
/// ccusage's own `totalTokens` is `input + output + cacheCreation + cacheRead`;
/// reasoning is never a term in it (verified against real exports and against
/// ccusage's documented examples). tokscale's buckets are additive and
/// `TokenBreakdown::total()` sums `reasoning` as a fifth term, so mapping
/// `reasoningOutputTokens` straight through reports a day *larger* than the
/// file it came from. Reasoning is an OpenAI output-detail subset, so the
/// additive `output` bucket keeps only the non-reasoning remainder — the same
/// correction `dsh.rs`, `grok.rs`, `senpi.rs`, `zcode.rs` and `reasonix.rs`
/// apply to this shape.
fn ccusage_token_breakdown(
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
    clamped: &mut usize,
) -> TokenBreakdown {
    let output = clamp_i64(output, clamped);
    let reasoning = clamp_i64(reasoning, clamped);
    TokenBreakdown {
        input: clamp_i64(input, clamped),
        output: output.saturating_sub(reasoning),
        cache_read: clamp_i64(cache_read, clamped),
        cache_write: clamp_i64(cache_write, clamped),
        reasoning,
    }
}

/// Pick the client a model's usage belongs to.
///
/// `agents` is the ground truth for *which* clients ran that day, so the
/// resolver never invents one that is not listed. When the day had a single
/// agent every model is trivially its own; when several ran, the model family
/// decides. A model that matches no listed agent is left unattributed rather
/// than silently folded into the first one — misattributed usage is worse than
/// usage the caller is told about.
///
/// Known limit: this splits by *family*, not by which client actually made the
/// call, and a family can belong to more than one listed agent. On a day with
/// `["claude", "opencode"]`, Claude models all land on `claude` even though
/// OpenCode routinely drives them — the unified row does not record enough to
/// tell them apart. Splitting exactly needs ccusage's `--by-agent` output,
/// where each agent carries its own `modelBreakdowns`; supporting that shape
/// would remove the guess entirely and is the right follow-up here.
fn resolve_client(model: &str, agents: &[String]) -> Option<String> {
    if agents.len() == 1 {
        return Some(normalize_client_id(&agents[0]));
    }

    let family = model_family(model)?;
    let normalized: Vec<String> = agents.iter().map(|a| normalize_client_id(a)).collect();

    if normalized.is_empty() {
        return Some(family.to_string());
    }
    normalized.into_iter().find(|a| a == family)
}

/// Map a model id to the client that conventionally produces it in a ccusage
/// report. Mirrors the vendor families `inferred_provider_from_model` knows,
/// narrowed to the CLIs ccusage attributes usage to.
fn model_family(model: &str) -> Option<&'static str> {
    let lower = model.to_lowercase();
    let has_word = |w: &str| {
        lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|seg| seg == w)
    };

    if lower.contains("claude")
        || has_word("opus")
        || has_word("sonnet")
        || has_word("haiku")
        || has_word("fable")
    {
        return Some("claude");
    }
    if lower.contains("gpt") || lower.contains("codex") || has_word("o1") || has_word("o3") {
        return Some("codex");
    }
    if lower.contains("gemini") {
        return Some("gemini");
    }
    None
}

/// Parse a `ccusage daily --json` export into normalized tokscale data.
///
/// Grouping matches [`parse_clawdboard_export`]: one [`DailyContribution`] per
/// date, one [`ClientContribution`] per (client, model) within it.
pub fn parse_ccusage_export(json: &str) -> Result<ImportOutcome> {
    let export: CcusageExport =
        serde_json::from_str(json).context("failed to parse ccusage export JSON")?;

    if export.daily.is_empty() {
        bail!("ccusage export contains no daily rows to import");
    }

    let mut days: BTreeMap<String, DayBuilder> = BTreeMap::new();
    let mut unknown: BTreeSet<String> = BTreeSet::new();
    let mut negative_values_clamped = 0usize;
    let mut suspect_cost_rows = 0usize;
    let mut future_dated_rows = 0usize;
    let mut unparseable_cost_rows = 0usize;
    let mut non_finite_cost_rows = 0usize;
    let mut multi_model_fallback_rows = 0usize;
    let mut breakdown_reconciliation_warnings: Vec<String> = Vec::new();
    let today = chrono::Utc::now().date_naive();

    for row in &export.daily {
        let date = row
            .period
            .as_deref()
            .or(row.date.as_deref())
            .ok_or_else(|| {
                anyhow::anyhow!("ccusage daily row is missing both `period` and `date`")
            })?
            .to_string();
        let parsed_date = parse_calendar_date(&date)?;

        // ccusage emits a row for every day in the range, so a real export
        // routinely carries days with no usage at all. Attributing one would
        // mint a phantom `unknown` client from the empty model list and warn
        // that the import would be rejected — for a day that contributes
        // nothing. Drop it before it can reach attribution.
        if row.is_empty_day() {
            continue;
        }

        if parsed_date > today {
            future_dated_rows += 1;
        }

        let agents: Vec<String> = row
            .metadata
            .as_ref()
            .map(|m| m.agents.clone())
            .unwrap_or_default();
        let day = days.entry(date.clone()).or_default();

        if row.model_breakdowns.is_empty() {
            // No per-model split: attribute the aggregate to the first listed
            // model, mirroring the clawdboard fallback.
            let model = row
                .models_used
                .first()
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            if row.models_used.len() > 1 {
                multi_model_fallback_rows += 1;
            }
            let client = attribute(&model, &agents, &mut unknown);
            let raw_cost = row
                .declared_cost()
                .map(|c| c.resolve(&mut unparseable_cost_rows))
                .unwrap_or(0.0);
            let raw_cost = sanitize_cost(raw_cost, &mut non_finite_cost_rows);
            let cost = clamp_f64(raw_cost, &mut negative_values_clamped);
            let tokens = ccusage_token_breakdown(
                row.input_tokens,
                row.output_tokens,
                row.cache_read_tokens,
                row.cache_creation_tokens,
                row.row_reasoning_tokens(),
                &mut negative_values_clamped,
            );
            if cost > 0.0 && tokens.total() == 0 && !is_cursor_legacy_tokenless(&client, &model) {
                suspect_cost_rows += 1;
            }
            add_row(day, &client, &model, tokens, cost);
            continue;
        }

        let mut mb_total = 0i64;
        let mut mb_cost = 0.0f64;

        for mb in &row.model_breakdowns {
            let client = attribute(&mb.model_name, &agents, &mut unknown);
            let tokens = ccusage_token_breakdown(
                mb.input_tokens,
                mb.output_tokens,
                mb.cache_read_tokens,
                mb.cache_creation_tokens,
                mb.reasoning_output_tokens,
                &mut negative_values_clamped,
            );
            let raw_cost = sanitize_cost(mb.cost, &mut non_finite_cost_rows);
            let cost = clamp_f64(raw_cost, &mut negative_values_clamped);
            if cost > 0.0
                && tokens.total() == 0
                && !is_cursor_legacy_tokenless(&client, &mb.model_name)
            {
                suspect_cost_rows += 1;
            }

            // `total()` re-adds the reasoning that `ccusage_token_breakdown`
            // moved out of `output`, so this sum is back in ccusage's own
            // vocabulary and is directly comparable to the row's scalars.
            mb_total = mb_total.saturating_add(tokens.total());
            mb_cost += cost;

            add_row(day, &client, &mb.model_name, tokens, cost);
        }

        // ccusage always populates the aggregate scalars alongside the
        // breakdowns, so a divergence here means the file was edited or
        // truncated — worth surfacing before the numbers reach a leaderboard.
        let agg_total = row
            .input_tokens
            .max(0)
            .saturating_add(row.output_tokens.max(0))
            .saturating_add(row.cache_read_tokens.max(0))
            .saturating_add(row.cache_creation_tokens.max(0));
        if agg_total != 0 && tokens_diverge(mb_total, agg_total) {
            breakdown_reconciliation_warnings.push(format!(
                "{date}: modelBreakdowns sum to {mb_total} token(s) but aggregate totals report {agg_total}"
            ));
        }
        // The row's own `totalTokens` is the one number the file states about
        // itself, and it is the cross-check that catches a token bucket being
        // mapped through twice. Compare it against what tokscale will report.
        if let Some(declared) = row.total_tokens.filter(|t| *t > 0) {
            if tokens_diverge(mb_total, declared) {
                breakdown_reconciliation_warnings.push(format!(
                    "{date}: imported {mb_total} token(s) but the export's own totalTokens reports {declared}"
                ));
            }
        }
        if let Some(raw) = row.declared_cost() {
            let agg_cost = sanitize_cost(
                raw.resolve(&mut unparseable_cost_rows),
                &mut non_finite_cost_rows,
            );
            if costs_diverge(mb_cost, agg_cost) {
                breakdown_reconciliation_warnings.push(format!(
                    "{date}: modelBreakdowns sum to cost {mb_cost:.4} but aggregate totalCost reports {agg_cost:.4}"
                ));
            }
        }
    }

    let mut contributions: Vec<DailyContribution> = days
        .into_iter()
        .map(|(date, builder)| finalize_day(date, builder))
        .collect();
    contributions.sort_by(|a, b| a.date.cmp(&b.date));
    calculate_intensities(&mut contributions);

    let graph = generate_graph_result(contributions, 0);

    Ok(ImportOutcome {
        graph,
        unknown_clients: unknown.into_iter().collect(),
        negative_values_clamped,
        suspect_cost_rows,
        future_dated_rows,
        unparseable_cost_rows,
        non_finite_cost_rows,
        multi_model_fallback_rows,
        breakdown_reconciliation_warnings,
    })
}

/// Resolve a model to its client, recording anything unattributable so the
/// caller can warn. The leaderboard rejects unknown clients, so `"unknown"`
/// is a visible placeholder rather than a silent default.
fn attribute(model: &str, agents: &[String], unknown: &mut BTreeSet<String>) -> String {
    match resolve_client(model, agents) {
        Some(client) if ClientId::from_str(&client).is_some() => client,
        Some(client) => {
            unknown.insert(client.clone());
            client
        }
        None => {
            unknown.insert("unknown".to_string());
            "unknown".to_string()
        }
    }
}

/// Validate that `s` is both shaped like `YYYY-MM-DD` (matching the
/// server's `^\d{4}-\d{2}-\d{2}$` regex) and a real calendar date — the
/// shape check alone lets invalid dates like `2026-02-31` through.
fn parse_calendar_date(s: &str) -> Result<NaiveDate> {
    if !is_iso_date(s) {
        bail!("invalid date {:?} in export (expected YYYY-MM-DD)", s);
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("invalid calendar date {:?} in export (not a real date)", s))
}

/// Parse a clawdboard `totalCost` string, tracking how many values failed
/// to parse so the caller can warn once with a summary count instead of
/// silently treating malformed strings (e.g. `"$1.25"`) as zero.
fn parse_cost_string(raw: Option<&str>, unparseable_count: &mut usize) -> f64 {
    match raw {
        None => 0.0,
        Some(s) => match s.parse::<f64>() {
            Ok(v) => v,
            Err(_) => {
                *unparseable_count += 1;
                0.0
            }
        },
    }
}

/// Sanitize a non-finite (`NaN`/`Infinity`) cost value to zero, tracking
/// the count so the caller can warn once. Non-finite floats serialize to
/// JSON `null` via `serde_json`, which the submit endpoint rejects.
fn sanitize_cost(v: f64, non_finite_count: &mut usize) -> f64 {
    if v.is_finite() {
        v
    } else {
        *non_finite_count += 1;
        0.0
    }
}

/// Mirrors the server's Cursor legacy carve-out
/// (`CURSOR_LEGACY_TOKENLESS_MODELS` in `submission.ts`): Cursor's
/// pre-2025-05 usage exports include `premium-tool-call` rows that are
/// billed per tool invocation and carry no token attribution at all. These
/// legitimately have `cost > 0` with every token field `0`, so they must
/// not be flagged as suspect.
fn is_cursor_legacy_tokenless(client: &str, model: &str) -> bool {
    client == "cursor" && model == "premium-tool-call"
}

/// Tolerance for reconciling `modelBreakdowns` sums against aggregate-level
/// totals: small rounding differences between clawdboard's per-model and
/// aggregate exports are expected and not worth warning about.
const RECONCILE_RELATIVE_TOLERANCE: f64 = 0.01; // 1%
const RECONCILE_TOKEN_ABS_TOLERANCE: i64 = 2;
const RECONCILE_COST_ABS_TOLERANCE: f64 = 0.01;

fn tokens_diverge(actual: i64, expected: i64) -> bool {
    let diff = (actual - expected).abs();
    let rel_bound = ((expected.unsigned_abs() as f64) * RECONCILE_RELATIVE_TOLERANCE) as i64;
    diff > rel_bound.max(RECONCILE_TOKEN_ABS_TOLERANCE)
}

fn costs_diverge(actual: f64, expected: f64) -> bool {
    let diff = (actual - expected).abs();
    let rel_bound = expected.abs() * RECONCILE_RELATIVE_TOLERANCE;
    diff > rel_bound.max(RECONCILE_COST_ABS_TOLERANCE)
}

/// Clamp a token value to zero if negative, tracking the number of times
/// clamping actually changed a value so the caller can warn once with a
/// summary count rather than spamming a message per field.
fn clamp_i64(v: i64, negative_count: &mut usize) -> i64 {
    if v < 0 {
        *negative_count += 1;
        0
    } else {
        v
    }
}

/// `f64` counterpart of [`clamp_i64`], used for `cost`.
fn clamp_f64(v: f64, negative_count: &mut usize) -> f64 {
    if v < 0.0 {
        *negative_count += 1;
        0.0
    } else {
        v
    }
}

fn add_row(day: &mut DayBuilder, client: &str, model: &str, tokens: TokenBreakdown, cost: f64) {
    let entry = day
        .clients
        .entry(format!("{client}\u{0}{model}"))
        .or_insert_with(|| ClientContribution {
            client: client.to_string(),
            model_id: model.to_string(),
            provider_id: String::new(),
            tokens: TokenBreakdown::default(),
            cost: 0.0,
            messages: 0,
        });
    entry.tokens += &tokens;
    entry.cost += cost;
}

/// Roll a day's per-client rows up into a [`DailyContribution`], deriving day
/// totals and the token breakdown *from* the client rows so the result is
/// internally consistent (the server validator requires client rows to sum to
/// day totals, and `tokenBreakdown` to equal day totals).
fn finalize_day(date: String, builder: DayBuilder) -> DailyContribution {
    let mut token_breakdown = TokenBreakdown::default();
    let mut cost = 0.0;
    let mut clients: Vec<ClientContribution> = Vec::with_capacity(builder.clients.len());

    for client in builder.clients.into_values() {
        token_breakdown += &client.tokens;
        cost += client.cost;
        clients.push(client);
    }

    // Deterministic output order.
    clients.sort_by(|a, b| {
        a.client
            .cmp(&b.client)
            .then_with(|| a.model_id.cmp(&b.model_id))
    });

    DailyContribution {
        date,
        totals: DailyTotals {
            tokens: token_breakdown.total(),
            cost,
            // clawdboard does not export per-model message counts; leaving this
            // at 0 keeps the day internally consistent (0 == sum of client 0s).
            messages: 0,
        },
        intensity: 0,
        token_breakdown,
        clients,
        active_time_ms: None,
    }
}

/// Strict `YYYY-MM-DD` check (matches the server's `^\d{4}-\d{2}-\d{2}$`).
fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "exportedAt": "2026-07-14T17:45:44.315Z",
      "profile": { "name": "example", "githubUsername": "example" },
      "dailyAggregates": [
        {
          "date": "2026-05-11",
          "source": "codex",
          "machineId": "m1",
          "inputTokens": 157910,
          "outputTokens": 5224,
          "cacheCreationTokens": 0,
          "cacheReadTokens": 112640,
          "totalCost": "0.5859",
          "premiumRequests": 0,
          "modelsUsed": ["gpt-5.5"],
          "modelBreakdowns": [
            { "modelName": "gpt-5.5", "cost": 0.585882, "inputTokens": 157910,
              "outputTokens": 5224, "cacheReadTokens": 112640, "cacheCreationTokens": 0 }
          ]
        },
        {
          "date": "2026-05-11",
          "source": "claude",
          "machineId": "m2",
          "modelsUsed": ["claude-sonnet"],
          "modelBreakdowns": [
            { "modelName": "claude-sonnet", "cost": 1.0, "inputTokens": 100,
              "outputTokens": 200, "cacheReadTokens": 0, "cacheCreationTokens": 50 }
          ]
        },
        {
          "date": "2026-05-12",
          "source": "codex",
          "machineId": "m1",
          "modelsUsed": ["gpt-5.5"],
          "modelBreakdowns": [
            { "modelName": "gpt-5.5", "cost": 0.10, "inputTokens": 10,
              "outputTokens": 20, "cacheReadTokens": 5, "cacheCreationTokens": 0 }
          ]
        }
      ]
    }"#;

    #[test]
    fn extreme_token_counts_saturate_without_panicking() {
        // Two model breakdowns each at i64::MAX must saturate the per-day
        // reconciliation accumulators instead of overflowing (debug panic /
        // release wrap), and must not produce a spurious mismatch warning
        // when the aggregate totals are equally extreme.
        let max = i64::MAX;
        let sample = format!(
            r#"{{
              "exportedAt": "2026-07-14T17:45:44.315Z",
              "profile": {{ "name": "example", "githubUsername": "example" }},
              "dailyAggregates": [
                {{
                  "date": "2026-05-11",
                  "source": "codex",
                  "machineId": "m1",
                  "inputTokens": {max},
                  "outputTokens": {max},
                  "modelsUsed": ["gpt-5.5"],
                  "modelBreakdowns": [
                    {{ "modelName": "gpt-5.5", "cost": 1.0, "inputTokens": {max},
                      "outputTokens": 0, "cacheReadTokens": 0, "cacheCreationTokens": 0 }},
                    {{ "modelName": "gpt-5.5-mini", "cost": 1.0, "inputTokens": {max},
                      "outputTokens": 0, "cacheReadTokens": 0, "cacheCreationTokens": 0 }}
                  ]
                }}
              ]
            }}"#
        );

        let out = parse_clawdboard_export(&sample).unwrap();
        assert_eq!(out.graph.contributions.len(), 1);
        assert_eq!(out.graph.contributions[0].totals.tokens, i64::MAX);
        assert!(
            out.breakdown_reconciliation_warnings.is_empty(),
            "saturated totals on both sides must not be reported as divergent: {:?}",
            out.breakdown_reconciliation_warnings
        );
    }

    #[test]
    fn parses_dates_and_client_rows() {
        let out = parse_clawdboard_export(SAMPLE).unwrap();
        let g = &out.graph;
        assert_eq!(g.contributions.len(), 2, "two distinct dates");
        assert_eq!(g.meta.date_range_start, "2026-05-11");
        assert_eq!(g.meta.date_range_end, "2026-05-12");
        assert!(out.unknown_clients.is_empty(), "codex + claude are known");

        let day1 = &g.contributions[0];
        assert_eq!(day1.date, "2026-05-11");
        assert_eq!(day1.clients.len(), 2, "codex + claude on the same day");
    }

    #[test]
    fn days_are_internally_consistent() {
        // The server validator requires tokenBreakdown == day totals and the
        // client rows to sum to day totals; verify both hold by construction.
        let out = parse_clawdboard_export(SAMPLE).unwrap();
        for day in &out.graph.contributions {
            assert_eq!(day.totals.tokens, day.token_breakdown.total());

            let mut summed = TokenBreakdown::default();
            let mut cost = 0.0;
            for c in &day.clients {
                summed += &c.tokens;
                cost += c.cost;
            }
            assert_eq!(summed.total(), day.totals.tokens);
            assert!((cost - day.totals.cost).abs() < 1e-9);
            assert!(day.intensity <= 4);
        }
    }

    #[test]
    fn summary_tokens_match_contributions() {
        let out = parse_clawdboard_export(SAMPLE).unwrap();
        let g = &out.graph;
        let summed: i64 = g.contributions.iter().map(|c| c.totals.tokens).sum();
        assert_eq!(g.summary.total_tokens, summed);
        // day1 codex 157910+5224+112640=275774; day1 claude 100+200+50=350;
        // day2 codex 10+20+5=35
        assert_eq!(summed, 275774 + 350 + 35);
    }

    #[test]
    fn highest_cost_day_has_max_intensity() {
        let out = parse_clawdboard_export(SAMPLE).unwrap();
        // day1 cost (0.585882 + 1.0) is the max → intensity 4.
        assert_eq!(out.graph.contributions[0].intensity, 4);
    }

    #[test]
    fn unknown_clients_are_flagged() {
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"totally-not-a-client",
            "modelBreakdowns":[{"modelName":"x","cost":0.0,"inputTokens":1,"outputTokens":0,
            "cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert_eq!(
            out.unknown_clients,
            vec!["totally-not-a-client".to_string()]
        );
    }

    #[test]
    fn empty_export_is_an_error() {
        assert!(parse_clawdboard_export(r#"{"dailyAggregates":[]}"#).is_err());
        assert!(parse_clawdboard_export("not json").is_err());
    }

    #[test]
    fn bad_date_is_rejected() {
        let json = r#"{"dailyAggregates":[{"date":"2026-5-1","source":"codex",
            "modelBreakdowns":[{"modelName":"x","cost":0.0,"inputTokens":1,"outputTokens":0,
            "cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        assert!(parse_clawdboard_export(json).is_err());
    }

    #[test]
    fn falls_back_to_aggregate_totals_when_no_model_breakdowns() {
        // No `modelBreakdowns` at all: the aggregate-level token/cost fields
        // must be used directly instead of being silently dropped.
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "inputTokens":100,"outputTokens":50,"cacheReadTokens":10,
            "cacheCreationTokens":5,"totalCost":"1.25","modelsUsed":["gpt-5.5"]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        let day = &out.graph.contributions[0];
        assert_eq!(day.clients.len(), 1);
        let client = &day.clients[0];
        assert_eq!(client.model_id, "gpt-5.5");
        assert_eq!(client.tokens.input, 100);
        assert_eq!(client.tokens.output, 50);
        assert_eq!(client.tokens.cache_read, 10);
        assert_eq!(client.tokens.cache_write, 5);
        assert!((client.cost - 1.25).abs() < 1e-9);
    }

    #[test]
    fn empty_models_used_falls_back_to_unknown_model() {
        // No `modelBreakdowns` and no `modelsUsed`: the synthesized row's
        // model id must fall back to "unknown" rather than panicking or
        // being left empty.
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "inputTokens":10,"outputTokens":5,"totalCost":"0.01"}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        let day = &out.graph.contributions[0];
        assert_eq!(day.clients.len(), 1);
        assert_eq!(day.clients[0].model_id, "unknown");
    }

    #[test]
    fn sums_multiple_machine_rows_for_same_client_model_date() {
        // clawdboard splits rows by machineId; two rows sharing (client,
        // model, date) must be summed into a single client contribution.
        let json = r#"{"dailyAggregates":[
            {"date":"2026-05-11","source":"codex","machineId":"m1",
             "modelBreakdowns":[{"modelName":"gpt-5.5","cost":1.0,"inputTokens":10,
                "outputTokens":20,"cacheReadTokens":0,"cacheCreationTokens":0}]},
            {"date":"2026-05-11","source":"codex","machineId":"m2",
             "modelBreakdowns":[{"modelName":"gpt-5.5","cost":2.0,"inputTokens":30,
                "outputTokens":40,"cacheReadTokens":5,"cacheCreationTokens":0}]}
        ]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        let day = &out.graph.contributions[0];
        assert_eq!(
            day.clients.len(),
            1,
            "same (client, model) merges into one row"
        );
        let client = &day.clients[0];
        assert_eq!(client.tokens.input, 40);
        assert_eq!(client.tokens.output, 60);
        assert_eq!(client.tokens.cache_read, 5);
        assert!((client.cost - 3.0).abs() < 1e-9);
    }

    #[test]
    fn flags_cost_without_tokens_as_suspect() {
        // A modelBreakdown row with cost > 0 but all token fields 0 would
        // fail the server's "Cost submitted without tokens" check; it must
        // be surfaced as a warning (kept, not silently dropped).
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "modelBreakdowns":[{"modelName":"gpt-5.5","cost":0.5,"inputTokens":0,
            "outputTokens":0,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert_eq!(out.suspect_cost_rows, 1);
        // The row is kept, not dropped.
        assert_eq!(out.graph.contributions[0].clients.len(), 1);
        assert!((out.graph.contributions[0].clients[0].cost - 0.5).abs() < 1e-9);
    }

    #[test]
    fn clamps_negative_values_to_zero() {
        // Negative token/cost values (malformed or adversarial export data)
        // must be clamped to zero and counted so the caller can warn once.
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "modelBreakdowns":[{"modelName":"gpt-5.5","cost":-1.0,"inputTokens":-5,
            "outputTokens":10,"cacheReadTokens":-2,"cacheCreationTokens":0}]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        // input (-5), cacheRead (-2), cost (-1.0) → 3 clamped values.
        assert_eq!(out.negative_values_clamped, 3);
        let client = &out.graph.contributions[0].clients[0];
        assert_eq!(client.tokens.input, 0);
        assert_eq!(client.tokens.output, 10);
        assert_eq!(client.tokens.cache_read, 0);
        assert_eq!(client.cost, 0.0);
    }

    #[test]
    fn calendar_invalid_date_is_rejected() {
        // "2026-02-31" is shaped like YYYY-MM-DD but is not a real date
        // (February never has 31 days); the shape-only check previously let
        // this through.
        let json = r#"{"dailyAggregates":[{"date":"2026-02-31","source":"codex",
            "modelBreakdowns":[{"modelName":"x","cost":0.0,"inputTokens":1,"outputTokens":0,
            "cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        assert!(parse_clawdboard_export(json).is_err());
    }

    #[test]
    fn far_future_date_is_warned() {
        let json = r#"{"dailyAggregates":[{"date":"2099-01-01","source":"codex",
            "modelBreakdowns":[{"modelName":"x","cost":0.0,"inputTokens":1,"outputTokens":0,
            "cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert_eq!(out.future_dated_rows, 1);
        // The row is still kept; the submit endpoint rejects it, this
        // importer only warns.
        assert_eq!(out.graph.contributions[0].date, "2099-01-01");
    }

    #[test]
    fn reconciliation_warns_when_breakdown_sum_diverges_from_aggregate() {
        // modelBreakdowns sum to far less than the aggregate-level totals
        // report: a sign of a partial breakdown (silent usage loss if the
        // caller only trusts modelBreakdowns).
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "inputTokens":1000,"outputTokens":500,"cacheReadTokens":0,"cacheCreationTokens":0,
            "totalCost":"10.00","modelsUsed":["gpt-5.5"],
            "modelBreakdowns":[{"modelName":"gpt-5.5","cost":1.0,"inputTokens":100,
            "outputTokens":50,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert_eq!(
            out.breakdown_reconciliation_warnings.len(),
            2,
            "both token and cost mismatch"
        );
        assert!(out.breakdown_reconciliation_warnings[0].contains("token"));
        assert!(out.breakdown_reconciliation_warnings[1].contains("cost"));
    }

    #[test]
    fn reconciliation_is_silent_within_tolerance() {
        // Small rounding differences between aggregate and per-model totals
        // (as in real clawdboard exports) must not trigger a warning.
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "inputTokens":157910,"outputTokens":5224,"cacheReadTokens":112640,"cacheCreationTokens":0,
            "totalCost":"0.5859","modelsUsed":["gpt-5.5"],
            "modelBreakdowns":[{"modelName":"gpt-5.5","cost":0.585882,"inputTokens":157910,
            "outputTokens":5224,"cacheReadTokens":112640,"cacheCreationTokens":0}]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert!(out.breakdown_reconciliation_warnings.is_empty());
    }

    #[test]
    fn reconciliation_skipped_when_aggregate_totals_absent() {
        // clawdboard rows that only carry modelBreakdowns (no duplicated
        // aggregate-level scalars) must not be flagged as mismatched.
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "modelsUsed":["gpt-5.5"],
            "modelBreakdowns":[{"modelName":"gpt-5.5","cost":1.0,"inputTokens":100,
            "outputTokens":50,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert!(out.breakdown_reconciliation_warnings.is_empty());
    }

    #[test]
    fn multi_model_fallback_without_breakdowns_is_warned() {
        // No modelBreakdowns and multiple modelsUsed: all usage is
        // attributed to the first model only; the caller must be warned.
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "inputTokens":10,"outputTokens":5,"totalCost":"0.01",
            "modelsUsed":["gpt-5.5","gpt-5.5-mini"]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert_eq!(out.multi_model_fallback_rows, 1);
        assert_eq!(out.graph.contributions[0].clients[0].model_id, "gpt-5.5");
    }

    #[test]
    fn single_model_without_breakdowns_is_not_warned() {
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "inputTokens":10,"outputTokens":5,"totalCost":"0.01",
            "modelsUsed":["gpt-5.5"]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert_eq!(out.multi_model_fallback_rows, 0);
    }

    #[test]
    fn unparseable_cost_string_is_warned_and_treated_as_zero() {
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "inputTokens":10,"outputTokens":5,"totalCost":"$1.25",
            "modelsUsed":["gpt-5.5"]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert_eq!(out.unparseable_cost_rows, 1);
        assert_eq!(out.graph.contributions[0].clients[0].cost, 0.0);
    }

    #[test]
    fn non_finite_cost_is_sanitized_to_zero() {
        // "NaN"/"Infinity" parse successfully via f64::from_str but must not
        // survive to serialize as JSON null (which the endpoint rejects).
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "inputTokens":10,"outputTokens":5,"totalCost":"NaN",
            "modelsUsed":["gpt-5.5"]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert_eq!(out.non_finite_cost_rows, 1);
        assert_eq!(out.unparseable_cost_rows, 0, "NaN parses fine as a float");
        let cost = out.graph.contributions[0].clients[0].cost;
        assert_eq!(cost, 0.0);
        assert!(cost.is_finite());
    }

    #[test]
    fn cursor_legacy_premium_tool_call_is_exempt_from_suspect_warning() {
        // Mirrors submission.ts's CURSOR_LEGACY_TOKENLESS_MODELS carve-out:
        // Cursor's premium-tool-call rows legitimately have cost > 0 with
        // no token attribution and must not be flagged as suspect.
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"cursor",
            "modelBreakdowns":[{"modelName":"premium-tool-call","cost":0.5,"inputTokens":0,
            "outputTokens":0,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert_eq!(out.suspect_cost_rows, 0);
        assert!((out.graph.contributions[0].clients[0].cost - 0.5).abs() < 1e-9);
    }

    #[test]
    fn non_cursor_tokenless_cost_row_is_still_flagged() {
        // Sanity check: the exemption is specific to cursor +
        // premium-tool-call, not tokenless cost rows in general.
        let json = r#"{"dailyAggregates":[{"date":"2026-05-11","source":"codex",
            "modelBreakdowns":[{"modelName":"premium-tool-call","cost":0.5,"inputTokens":0,
            "outputTokens":0,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_clawdboard_export(json).unwrap();
        assert_eq!(out.suspect_cost_rows, 1);
    }

    // -----------------------------------------------------------------------
    // ccusage
    // -----------------------------------------------------------------------

    /// A `ccusage daily --json` row as the unified report emits it: `period`
    /// rather than `date`, and one row carrying both agents' models.
    const CCUSAGE_MIXED: &str = r#"{
      "daily": [
        {
          "agent": "all",
          "period": "2026-08-14",
          "inputTokens": 1100,
          "outputTokens": 2200,
          "cacheCreationTokens": 300,
          "cacheReadTokens": 4400,
          "totalCost": 30.0,
          "totalTokens": 8000,
          "metadata": { "agents": ["claude", "codex"] },
          "modelsUsed": ["claude-opus-5", "gpt-5.6-sol"],
          "modelBreakdowns": [
            { "modelName": "claude-opus-5", "cost": 20.0, "inputTokens": 100,
              "outputTokens": 200, "cacheReadTokens": 400, "cacheCreationTokens": 300 },
            { "modelName": "gpt-5.6-sol", "cost": 10.0, "inputTokens": 1000,
              "outputTokens": 2000, "cacheReadTokens": 4000, "cacheCreationTokens": 0,
              "reasoningOutputTokens": 700 }
          ]
        }
      ]
    }"#;

    fn client_rows(out: &ImportOutcome, day: usize) -> Vec<(String, String)> {
        out.graph.contributions[day]
            .clients
            .iter()
            .map(|c| (c.client.clone(), c.model_id.clone()))
            .collect()
    }

    #[test]
    fn ccusage_splits_a_shared_day_by_model_family() {
        // The point of the format: one row, two agents. Attributing the whole
        // row to a single client would move $10 of Codex spend onto Claude.
        let out = parse_ccusage_export(CCUSAGE_MIXED).unwrap();
        assert_eq!(
            client_rows(&out, 0),
            vec![
                ("claude".to_string(), "claude-opus-5".to_string()),
                ("codex".to_string(), "gpt-5.6-sol".to_string()),
            ]
        );

        let claude = &out.graph.contributions[0].clients[0];
        let codex = &out.graph.contributions[0].clients[1];
        assert!((claude.cost - 20.0).abs() < 1e-9);
        assert!((codex.cost - 10.0).abs() < 1e-9);
        assert!(out.unknown_clients.is_empty());
    }

    #[test]
    fn ccusage_preserves_reasoning_tokens() {
        // Codex reports reasoning output separately; dropping it understates
        // the day and breaks the server's token-sum check.
        let out = parse_ccusage_export(CCUSAGE_MIXED).unwrap();
        assert_eq!(out.graph.contributions[0].token_breakdown.reasoning, 700);
        assert_eq!(out.graph.contributions[0].clients[1].tokens.reasoning, 700);
    }

    #[test]
    fn ccusage_day_total_matches_the_export_declared_total() {
        // The anti-double-count guard. ccusage's `totalTokens` is
        // input+output+cacheCreation+cacheRead and never counts reasoning as a
        // term of its own, while tokscale's buckets are additive. Mapping
        // `reasoningOutputTokens` through without taking it out of `output`
        // reported 8700 for this row -- 700 more than the file says it is.
        let out = parse_ccusage_export(CCUSAGE_MIXED).unwrap();
        let day = &out.graph.contributions[0];
        assert_eq!(day.totals.tokens, 8000, "must match the row's totalTokens");
        // The reasoning is still tracked, just not counted twice.
        assert_eq!(day.token_breakdown.reasoning, 700);
        assert_eq!(day.token_breakdown.output, 2200 - 700);
        assert!(
            out.breakdown_reconciliation_warnings.is_empty(),
            "a self-consistent export must not warn: {:?}",
            out.breakdown_reconciliation_warnings
        );
    }

    #[test]
    fn ccusage_reads_reasoning_nested_under_metadata() {
        // The unified report records reasoning under `metadata`, not beside
        // the other token fields. Reading only the flat spelling scores every
        // real unified export as zero-reasoning.
        let json = r#"{"daily":[{"period":"2026-08-14","totalCost":1.0,
            "inputTokens":10,"outputTokens":100,"totalTokens":110,
            "metadata":{"agents":["codex"],"reasoningOutputTokens":40},
            "modelsUsed":["gpt-5.6-sol"]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        let day = &out.graph.contributions[0];
        assert_eq!(day.token_breakdown.reasoning, 40);
        assert_eq!(
            day.token_breakdown.output, 60,
            "100 output less 40 reasoning"
        );
        assert_eq!(day.totals.tokens, 110, "still the declared total");
    }

    #[test]
    fn ccusage_empty_day_row_is_skipped() {
        // ccusage emits a row per day in the range, so real exports carry
        // days with nothing on them. Attributing one mints a phantom
        // `unknown` client and warns that the import would be rejected.
        let json = r#"{"daily":[
            {"date":"2025-11-21","inputTokens":0,"outputTokens":0,
             "cacheCreationTokens":0,"cacheReadTokens":0,"totalTokens":0,
             "totalCost":0,"modelsUsed":[],"modelBreakdowns":[]},
            {"date":"2025-11-22","totalCost":1.0,"modelsUsed":["claude-opus-5"],
             "modelBreakdowns":[{"modelName":"claude-opus-5","cost":1.0,
             "inputTokens":10,"outputTokens":20}]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        assert!(
            out.unknown_clients.is_empty(),
            "an empty day must not mint a client: {:?}",
            out.unknown_clients
        );
        let dates: Vec<&str> = out
            .graph
            .contributions
            .iter()
            .map(|c| c.date.as_str())
            .collect();
        assert_eq!(dates, vec!["2025-11-22"]);
    }

    #[test]
    fn ccusage_gemini_cli_agent_resolves_to_its_own_models() {
        // `model_family` answers "gemini"; an agent spelled `gemini-cli` has
        // to normalize to the same id or its usage lands in `unknown`.
        let json = r#"{"daily":[{"period":"2026-08-14","totalCost":2.0,
            "metadata":{"agents":["claude-code","gemini-cli"]},
            "modelBreakdowns":[
              {"modelName":"gemini-3-pro","cost":1.0,"inputTokens":10,"outputTokens":20},
              {"modelName":"claude-opus-5","cost":1.0,"inputTokens":10,"outputTokens":20}]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        assert_eq!(
            client_rows(&out, 0),
            vec![
                ("claude".to_string(), "claude-opus-5".to_string()),
                ("gemini".to_string(), "gemini-3-pro".to_string()),
            ]
        );
        assert!(out.unknown_clients.is_empty());
    }

    #[test]
    fn ccusage_accepts_the_codex_cost_usd_spelling() {
        // `ccusage-codex` emits `costUSD` where `ccusage` emits `totalCost`
        // (ccusage#831). Reading only one spelling imports the day at $0.
        let json = r#"{"daily":[{"period":"2026-08-14","costUSD":3.5,
            "inputTokens":10,"outputTokens":20,
            "metadata":{"agents":["codex"]},"modelsUsed":["gpt-5.6-sol"]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        assert!((out.graph.contributions[0].totals.cost - 3.5).abs() < 1e-9);
    }

    #[test]
    fn ccusage_day_totals_match_client_rows() {
        let out = parse_ccusage_export(CCUSAGE_MIXED).unwrap();
        let day = &out.graph.contributions[0];
        let summed: i64 = day.clients.iter().map(|c| c.tokens.total()).sum();
        let cost: f64 = day.clients.iter().map(|c| c.cost).sum();
        assert_eq!(day.totals.tokens, summed);
        assert_eq!(day.token_breakdown.total(), summed);
        assert!((day.totals.cost - cost).abs() < 1e-9);
    }

    #[test]
    fn ccusage_accepts_date_as_well_as_period() {
        // Per-agent reports and older releases emit `date`.
        let json = r#"{"daily":[{"date":"2026-08-14","totalCost":1.0,
            "metadata":{"agents":["claude"]},
            "modelBreakdowns":[{"modelName":"claude-opus-5","cost":1.0,"inputTokens":10,
            "outputTokens":20,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        assert_eq!(out.graph.contributions[0].date, "2026-08-14");
    }

    #[test]
    fn ccusage_row_without_any_date_is_rejected() {
        let json = r#"{"daily":[{"totalCost":1.0,"modelsUsed":["claude-opus-5"]}]}"#;
        let err = match parse_ccusage_export(json) {
            Ok(_) => panic!("a row with neither `period` nor `date` must be rejected"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("period"), "unexpected error: {err}");
    }

    #[test]
    fn ccusage_single_agent_day_needs_no_model_inference() {
        // A day with one agent is unambiguous even for a model the family
        // matcher has never seen — the export already told us who ran.
        let json = r#"{"daily":[{"period":"2026-08-14","totalCost":1.0,
            "metadata":{"agents":["opencode"]},
            "modelBreakdowns":[{"modelName":"some-private-model","cost":1.0,"inputTokens":10,
            "outputTokens":20,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        assert_eq!(out.graph.contributions[0].clients[0].client, "opencode");
        assert!(out.unknown_clients.is_empty());
    }

    #[test]
    fn ccusage_unattributable_model_on_a_shared_day_is_flagged() {
        // Two agents ran and the model matches neither. Folding it into the
        // first agent would silently misattribute it, so it is surfaced.
        let json = r#"{"daily":[{"period":"2026-08-14","totalCost":1.0,
            "metadata":{"agents":["claude","codex"]},
            "modelBreakdowns":[{"modelName":"mystery-1","cost":1.0,"inputTokens":10,
            "outputTokens":20,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        assert_eq!(out.unknown_clients, vec!["unknown".to_string()]);
        assert_eq!(out.graph.contributions[0].clients[0].client, "unknown");
    }

    #[test]
    fn ccusage_infers_family_when_metadata_is_absent() {
        // Older exports carry no `metadata.agents` at all.
        let json = r#"{"daily":[{"period":"2026-08-14","totalCost":1.0,
            "modelBreakdowns":[{"modelName":"gpt-5.5","cost":1.0,"inputTokens":10,
            "outputTokens":20,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        assert_eq!(out.graph.contributions[0].clients[0].client, "codex");
    }

    #[test]
    fn ccusage_accepts_stringified_cost() {
        // Dashboards that round-trip cc.json sometimes serialize the
        // aggregate cost as a string.
        let json = r#"{"daily":[{"period":"2026-08-14","totalCost":"1.25",
            "inputTokens":10,"outputTokens":20,
            "metadata":{"agents":["claude"]},"modelsUsed":["claude-opus-5"]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        assert!((out.graph.contributions[0].totals.cost - 1.25).abs() < 1e-9);
        assert_eq!(out.unparseable_cost_rows, 0);
    }

    #[test]
    fn ccusage_reconciliation_warns_when_breakdowns_diverge() {
        let json = r#"{"daily":[{"period":"2026-08-14","inputTokens":10000,
            "outputTokens":0,"cacheReadTokens":0,"cacheCreationTokens":0,"totalCost":1.0,
            "metadata":{"agents":["claude"]},
            "modelBreakdowns":[{"modelName":"claude-opus-5","cost":1.0,"inputTokens":10,
            "outputTokens":0,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        assert_eq!(out.breakdown_reconciliation_warnings.len(), 1);
        assert!(out.breakdown_reconciliation_warnings[0].contains("2026-08-14"));
    }

    #[test]
    fn ccusage_empty_export_is_an_error() {
        assert!(parse_ccusage_export(r#"{"daily":[]}"#).is_err());
    }

    #[test]
    fn ccusage_negative_values_are_clamped() {
        let json = r#"{"daily":[{"period":"2026-08-14","totalCost":1.0,
            "metadata":{"agents":["claude"]},
            "modelBreakdowns":[{"modelName":"claude-opus-5","cost":-1.0,"inputTokens":-10,
            "outputTokens":20,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        assert_eq!(out.negative_values_clamped, 2);
        assert_eq!(out.graph.contributions[0].token_breakdown.input, 0);
        assert!(out.graph.contributions[0].totals.cost.abs() < 1e-9);
    }

    #[test]
    fn ccusage_days_are_sorted_and_intensities_assigned() {
        let json = r#"{"daily":[
            {"period":"2026-08-15","totalCost":1.0,"metadata":{"agents":["claude"]},
             "modelBreakdowns":[{"modelName":"claude-opus-5","cost":1.0,"inputTokens":10,
             "outputTokens":0,"cacheReadTokens":0,"cacheCreationTokens":0}]},
            {"period":"2026-08-13","totalCost":9.0,"metadata":{"agents":["claude"]},
             "modelBreakdowns":[{"modelName":"claude-opus-5","cost":9.0,"inputTokens":90,
             "outputTokens":0,"cacheReadTokens":0,"cacheCreationTokens":0}]}]}"#;
        let out = parse_ccusage_export(json).unwrap();
        let dates: Vec<&str> = out
            .graph
            .contributions
            .iter()
            .map(|c| c.date.as_str())
            .collect();
        assert_eq!(dates, vec!["2026-08-13", "2026-08-15"]);
        let top = out
            .graph
            .contributions
            .iter()
            .max_by(|a, b| a.intensity.cmp(&b.intensity))
            .unwrap();
        assert_eq!(top.date, "2026-08-13");
    }

    #[test]
    fn ccusage_parse_export_dispatches_by_format() {
        assert!(parse_export("ccusage", CCUSAGE_MIXED).is_ok());
        assert!(parse_export("clawdboard", SAMPLE).is_ok());
        assert!(parse_export("viberank", CCUSAGE_MIXED).is_err());
    }
}
