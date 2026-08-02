//! Import historical usage from third-party aggregate exports.
//!
//! Currently supports the clawdboard.ai account export ("daily aggregates").
//! The importer normalizes that export into tokscale's native [`GraphResult`],
//! which the CLI then writes out as standard tokscale JSON (identical in shape
//! to `tokscale graph`) for review, archival, or a future server-supported
//! backfill.
//!
//! Motivation: `tokscale submit` computes totals from *raw* local session
//! files. Once those files are gone (Claude Code deletes transcripts after
//! `cleanupPeriodDays`, default 30), earlier months can never be re-scanned,
//! even though a competing dashboard may still hold the aggregates. This
//! importer recovers that history into tokscale's format.
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
pub const SUPPORTED_FORMATS: &[&str] = &["clawdboard"];

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
        "codex-cli" => "codex".to_string(),
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
    entry.tokens.input = entry.tokens.input.saturating_add(tokens.input);
    entry.tokens.output = entry.tokens.output.saturating_add(tokens.output);
    entry.tokens.cache_read = entry.tokens.cache_read.saturating_add(tokens.cache_read);
    entry.tokens.cache_write = entry.tokens.cache_write.saturating_add(tokens.cache_write);
    entry.tokens.reasoning = entry.tokens.reasoning.saturating_add(tokens.reasoning);
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
        token_breakdown.input = token_breakdown.input.saturating_add(client.tokens.input);
        token_breakdown.output = token_breakdown.output.saturating_add(client.tokens.output);
        token_breakdown.cache_read = token_breakdown
            .cache_read
            .saturating_add(client.tokens.cache_read);
        token_breakdown.cache_write = token_breakdown
            .cache_write
            .saturating_add(client.tokens.cache_write);
        token_breakdown.reasoning = token_breakdown
            .reasoning
            .saturating_add(client.tokens.reasoning);
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
                summed.input += c.tokens.input;
                summed.output += c.tokens.output;
                summed.cache_read += c.tokens.cache_read;
                summed.cache_write += c.tokens.cache_write;
                summed.reasoning += c.tokens.reasoning;
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
}
