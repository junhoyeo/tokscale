//! Cursor IDE session parser
//!
//! Parses usage files cached locally at ~/.config/tokscale/cursor-cache/.
//! The active account's cache is `usage.json` (or legacy `usage.csv`);
//! additional accounts use `usage.<account>.json` (or legacy `usage.<account>.csv`).
//!
//! JSON (preferred) comes from the dashboard `get-filtered-usage-events`
//! endpoint. Each event carries a real `conversationId`, so sessions are keyed
//! by the Cursor session UUID instead of a synthetic timestamp bucket.
//!
//! CSV (legacy) formats, still parsed for caches written before the JSON switch:
//! - v1 (old): Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost,Cost to you
//! - v2 (new): Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
//! - v3 (latest): Date,Cloud Agent ID,Automation ID,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost

use super::{timestamp_to_date_with_timezone, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use std::path::Path;

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::sync::Mutex;

#[cfg(test)]
static PARSE_CURSOR_FILE_CALLS: Mutex<Option<HashMap<PathBuf, usize>>> = Mutex::new(None);

/// Keeps a path-scoped parser counter registered for the lifetime of a test.
///
/// The guard removes only its own root on drop, preventing stale registrations
/// from accumulating or a later test from accidentally observing old counts.
#[cfg(test)]
pub(crate) struct ParseCursorFileCounterGuard {
    root: PathBuf,
}

#[cfg(test)]
impl Drop for ParseCursorFileCounterGuard {
    fn drop(&mut self) {
        let Ok(mut counters) = PARSE_CURSOR_FILE_CALLS.lock() else {
            return;
        };
        if let Some(counters) = counters.as_mut() {
            counters.remove(&self.root);
        }
        if matches!(counters.as_ref(), Some(counters) if counters.is_empty()) {
            *counters = None;
        }
    }
}

#[cfg(test)]
pub(crate) fn register_parse_cursor_file_counter(root: &Path) -> ParseCursorFileCounterGuard {
    let root = root.to_path_buf();
    let inserted = {
        let mut counters = PARSE_CURSOR_FILE_CALLS.lock().unwrap();
        let counters = counters.get_or_insert_with(HashMap::new);
        if counters.contains_key(&root) {
            false
        } else {
            counters.insert(root.clone(), 0);
            true
        }
    };
    assert!(
        inserted,
        "a Cursor parser counter is already registered for this root"
    );
    ParseCursorFileCounterGuard { root }
}

#[cfg(test)]
pub(crate) fn parse_cursor_file_call_count(root: &Path) -> usize {
    PARSE_CURSOR_FILE_CALLS
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|counters| counters.get(root).copied())
        .unwrap_or(0)
}

#[cfg(test)]
fn record_parse_cursor_file_call(path: &Path) {
    let mut counters = PARSE_CURSOR_FILE_CALLS.lock().unwrap();
    if let Some(counters) = counters.as_mut() {
        for (root, count) in counters {
            if path.starts_with(root) {
                *count += 1;
            }
        }
    }
}

fn account_id_from_cursor_cache_path(path: &Path) -> String {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("usage.csv");

    if file_name == "usage.csv" || file_name == "usage.json" {
        return "active".to_string();
    }

    if let Some(stem) = file_name
        .strip_prefix("usage.")
        .and_then(|s| s.strip_suffix(".csv").or_else(|| s.strip_suffix(".json")))
    {
        // Keep it simple/ASCII. The CLI already sanitizes file names.
        let cleaned = stem
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>();
        if cleaned.is_empty() {
            return "unknown".to_string();
        }
        return cleaned;
    }

    "unknown".to_string()
}

/// Provider inference from model name
fn infer_provider(model: &str) -> &'static str {
    // Delimiter-aware inference so a model id that only contains a provider
    // family as a substring isn't misattributed (which would apply the wrong
    // pricing). Both the JSON and CSV lanes flow through here.
    provider_identity::inferred_provider_from_model_delimited(model).unwrap_or("cursor")
}

/// One row of `usageEventsDisplay`. Only the fields tokscale consumes are
/// modeled; unknown fields are ignored so upstream additions don't break parsing.
#[derive(Debug, Deserialize)]
struct CursorUsageEvent {
    #[serde(rename = "conversationId", default)]
    conversation_id: Option<String>,
    /// Unix milliseconds. Cursor sends this as a string, but a number is
    /// tolerated too via [`parse_ms_timestamp`].
    #[serde(default)]
    timestamp: Option<serde_json::Value>,
    #[serde(default)]
    model: Option<String>,
    /// Authoritative amount billed, in cents. Cursor may send it as an integer,
    /// a float, or a numeric string, so it is coerced leniently.
    #[serde(
        rename = "chargedCents",
        default,
        deserialize_with = "de_opt_f64_lenient"
    )]
    charged_cents: Option<f64>,
    #[serde(rename = "tokenUsage", default)]
    token_usage: Option<CursorTokenUsage>,
}

/// The per-event token breakdown. `cacheWriteTokens` is absent on many events.
/// Counts are coerced leniently so a single float (e.g. `10.0`) or numeric
/// string doesn't fail the whole row.
#[derive(Debug, Deserialize, Default)]
struct CursorTokenUsage {
    #[serde(
        rename = "inputTokens",
        default,
        deserialize_with = "de_opt_i64_lenient"
    )]
    input_tokens: Option<i64>,
    #[serde(
        rename = "outputTokens",
        default,
        deserialize_with = "de_opt_i64_lenient"
    )]
    output_tokens: Option<i64>,
    #[serde(
        rename = "cacheReadTokens",
        default,
        deserialize_with = "de_opt_i64_lenient"
    )]
    cache_read_tokens: Option<i64>,
    #[serde(
        rename = "cacheWriteTokens",
        default,
        deserialize_with = "de_opt_i64_lenient"
    )]
    cache_write_tokens: Option<i64>,
    /// The metered cost of this event's tokens, in cents. Cursor's
    /// `aiserver.v1.TokenUsage` carries it next to the token counts and the
    /// discount fields, so it is populated even when the event was plan-included
    /// and the wallet was debited nothing.
    #[serde(
        rename = "totalCents",
        default,
        deserialize_with = "de_opt_f64_lenient"
    )]
    total_cents: Option<f64>,
}

/// Coerce a JSON number/string into `i64`, tolerating floats and numeric
/// strings so one unexpected shape doesn't drop an otherwise valid row.
fn json_value_to_i64(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            trimmed
                .parse::<i64>()
                .ok()
                .or_else(|| trimmed.parse::<f64>().ok().map(|f| f as i64))
        }
        _ => None,
    }
}

/// Coerce a JSON number/string into `f64`, tolerating `$`/`,` in strings.
fn json_value_to_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.replace(['$', ','], "").trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn de_opt_i64_lenient<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.as_ref().and_then(json_value_to_i64))
}

fn de_opt_f64_lenient<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.as_ref().and_then(json_value_to_f64))
}

/// Parse a Unix-milliseconds timestamp that may arrive as a JSON string or
/// number. A string is tried as base-10 milliseconds first, then as an
/// ISO-8601 / RFC 3339 datetime so either shape the dashboard emits resolves.
fn parse_ms_timestamp(value: &serde_json::Value) -> i64 {
    match value {
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            trimmed
                .parse::<i64>()
                .ok()
                .or_else(|| parse_iso8601_to_ms(trimmed))
                .unwrap_or(0)
        }
        serde_json::Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        _ => 0,
    }
}

/// Parse an ISO-8601 / RFC 3339 datetime string into Unix milliseconds.
fn parse_iso8601_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Parse a cost string like "$0.50" or "0.50" as a finite, non-negative number.
///
/// `Some(0.0)` represents an explicit zero from Cursor. Missing, sentinel,
/// invalid, negative, and non-finite values return `None` so callers can distinguish
/// provider-reported zero cost from a row that still needs estimation.
fn parse_finite_cost(cost_str: &str) -> Option<f64> {
    let cleaned = cost_str.replace(['$', ','], "");
    let trimmed = cleaned.trim();

    trimmed
        .parse::<f64>()
        .ok()
        .filter(|cost| cost.is_finite() && *cost >= 0.0)
}

/// Keep a cents figure only when it is finite and non-negative.
///
/// Mirrors `parse_finite_cost` on the CSV lane so both Cursor sources refuse a
/// nonsense amount the same way, leaving the row unknown rather than stamping
/// it provider-reported and immune to repricing.
fn finite_non_negative_cents(cents: Option<f64>) -> Option<f64> {
    cents.filter(|value| value.is_finite() && *value >= 0.0)
}

/// Parse a cost string, defaulting missing or invalid values to zero.
#[cfg(test)]
fn parse_cost(cost_str: &str) -> f64 {
    parse_finite_cost(cost_str).unwrap_or(0.0)
}

/// Parse a Cursor usage cache file, dispatching on its extension.
///
/// `.json` files come from the dashboard usage-events endpoint and are keyed by
/// the real `conversationId`. `.csv` files are the legacy export format and keep
/// the synthetic per-day session id for backward compatibility.
pub fn parse_cursor_file(path: &Path) -> Vec<UnifiedMessage> {
    #[cfg(test)]
    record_parse_cursor_file_call(path);

    let is_json = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));

    if is_json {
        parse_cursor_events_json(path)
    } else {
        parse_cursor_csv_file(path)
    }
}

/// Parse a Cursor usage events JSON file (dashboard `get-filtered-usage-events`).
///
/// Sessions are keyed by `conversationId` (the Cursor session UUID). Cost comes
/// from the metered `tokenUsage.totalCents`, falling back to `chargedCents`,
/// divided by 100 and marked provider-reported; an event carrying neither is
/// left with an unknown source so local pricing can estimate it. Rows are
/// parsed individually so one malformed entry is skipped rather than discarding
/// the whole cache. Events with no usable `conversationId` fall back to a
/// UTC-stable synthetic per-day id so their cost is never dropped from totals.
pub fn parse_cursor_events_json(path: &Path) -> Vec<UnifiedMessage> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let root: serde_json::Value = match serde_json::from_str(&content) {
        Ok(root) => root,
        Err(_) => return vec![],
    };

    let rows = root
        .get("usageEventsDisplay")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    let account_id = account_id_from_cursor_cache_path(path);
    let mut messages = Vec::with_capacity(rows.len());

    for row in rows {
        // Deserialize each row on its own so one malformed entry (an unexpected
        // type in a single field) skips just that row instead of discarding the
        // entire cache.
        let event: CursorUsageEvent = match serde_json::from_value(row) {
            Ok(event) => event,
            Err(_) => continue,
        };

        let model = event.model.unwrap_or_default();
        let model = model.trim();
        if model.is_empty() {
            continue;
        }

        let timestamp = event
            .timestamp
            .as_ref()
            .map(parse_ms_timestamp)
            .unwrap_or(0);
        if timestamp == 0 {
            continue;
        }

        // Prefer the real Cursor session UUID; fall back to the legacy synthetic
        // per-day id only when the event carries no usable conversation id, so
        // its cost still lands in the account's totals.
        let session_id = match event.conversation_id.as_deref().map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => format!(
                "cursor-{}-{}",
                account_id,
                timestamp_to_date_with_timezone(timestamp, &chrono::Utc)
            ),
        };

        let token_usage = event.token_usage.unwrap_or_default();

        // Cursor reports two different amounts. `tokenUsage.totalCents` is the
        // metered cost of the event's own tokens; `chargedCents` is what the
        // wallet was debited. They agree whenever the user pays, and diverge
        // exactly on plan-included / free-credit rows, where nothing is billed
        // but the usage still cost something. Prefer the metered figure so those
        // rows keep Cursor's own number instead of falling through to local
        // pricing, which refuses the router labels `auto`/`agent_review` (#1062)
        // and would drop them into the unpriced bucket at $0.00.
        let metered_cents = finite_non_negative_cents(token_usage.total_cents)
            .or_else(|| finite_non_negative_cents(event.charged_cents));
        let cost = metered_cents.map(|cents| cents / 100.0);

        let mut message = UnifiedMessage::new(
            "cursor",
            model,
            infer_provider(model),
            session_id,
            timestamp,
            TokenBreakdown {
                input: token_usage.input_tokens.unwrap_or(0).max(0),
                output: token_usage.output_tokens.unwrap_or(0).max(0),
                cache_read: token_usage.cache_read_tokens.unwrap_or(0).max(0),
                cache_write: token_usage.cache_write_tokens.unwrap_or(0).max(0),
                cache_write_1h: 0,
                reasoning: 0,
            },
            cost.unwrap_or(0.0).max(0.0),
        );
        if cost.is_some() {
            message.mark_provider_reported_cost();
        }
        messages.push(message);
    }

    messages
}

/// Parse a Cursor usage CSV file
///
/// Handles both formats:
/// - New: Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
/// - Old: Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost,Cost to you
fn parse_cursor_csv_file(path: &Path) -> Vec<UnifiedMessage> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut messages = Vec::with_capacity(128);
    let mut lines = content.lines();

    // Parse header line to determine column indices
    let header = match lines.next() {
        Some(h) => h,
        None => return vec![],
    };

    // Verify this is a valid Cursor CSV
    if !header.contains("Date") || !header.contains("Model") {
        return vec![];
    }

    // Detect format by checking for "Kind" column and column count
    let header_fields: Vec<&str> = parse_csv_line(header);
    let has_kind_column = header_fields.iter().any(|f| f.trim() == "Kind");
    let column_count = header_fields.len();

    // Column indices based on format
    let (
        model_idx,
        input_cache_write_idx,
        input_no_cache_idx,
        cache_read_idx,
        output_idx,
        cost_idx,
    ) = if has_kind_column && column_count >= 11 {
        // v3 format: Date,Cloud Agent ID,Automation ID,Kind,Model,...
        (4, 6, 7, 8, 9, 11)
    } else if has_kind_column {
        // v2 format: Date,Kind,Model,Max Mode,Input (w/ Cache Write),...
        (2, 4, 5, 6, 7, 9)
    } else {
        // v1 format: Date,Model,Input (w/ Cache Write),...
        (1, 2, 3, 4, 5, 7)
    };

    let account_id = account_id_from_cursor_cache_path(path);

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }

        // Parse CSV line (simple parsing, handles quoted fields)
        let fields: Vec<&str> = parse_csv_line(line);

        // Need at least enough columns for the format
        let min_fields = cost_idx + 1;
        if fields.len() < min_fields {
            continue;
        }

        let date_str = fields[0].trim().trim_matches('"');
        let model = fields[model_idx].trim().trim_matches('"');
        let input_with_cache_write: i64 = fields[input_cache_write_idx]
            .trim()
            .trim_matches('"')
            .parse()
            .unwrap_or(0);
        let input_without_cache_write: i64 = fields[input_no_cache_idx]
            .trim()
            .trim_matches('"')
            .parse()
            .unwrap_or(0);
        let cache_read: i64 = fields[cache_read_idx]
            .trim()
            .trim_matches('"')
            .parse()
            .unwrap_or(0);
        let output_tokens: i64 = fields[output_idx]
            .trim()
            .trim_matches('"')
            .parse()
            .unwrap_or(0);
        let cost_str = fields[cost_idx].trim().trim_matches('"');
        let cost = parse_finite_cost(cost_str);

        // Skip empty or errored entries
        if model.is_empty() {
            continue;
        }

        // Parse timestamp from date string
        let timestamp = parse_date_to_timestamp(date_str);
        if timestamp == 0 {
            continue;
        }

        // Cursor exports independent token buckets rather than cumulative totals.
        let mut message = UnifiedMessage::new(
            "cursor",
            model,
            infer_provider(model),
            format!("cursor-{}-{}", account_id, date_str),
            timestamp,
            TokenBreakdown {
                input: input_without_cache_write.max(0),
                output: output_tokens.max(0),
                cache_read: cache_read.max(0),
                cache_write: input_with_cache_write.max(0),
                cache_write_1h: 0,
                reasoning: 0,
            },
            cost.unwrap_or(0.0).max(0.0),
        );
        if cost.is_some() {
            message.mark_provider_reported_cost();
        }
        messages.push(message);
    }

    messages
}

/// Simple CSV line parser that handles quoted fields
fn parse_csv_line(line: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let bytes = line.as_bytes();

    for (i, &byte) in bytes.iter().enumerate() {
        match byte {
            b'"' => in_quotes = !in_quotes,
            b',' if !in_quotes => {
                fields.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }

    // Add the last field
    if start <= line.len() {
        fields.push(&line[start..]);
    }

    fields
}

/// Parse a date string to Unix milliseconds timestamp
fn parse_date_to_timestamp(date_str: &str) -> i64 {
    use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};

    // Try ISO 8601 format with milliseconds: "2025-02-05T12:00:00.123Z"
    if let Ok(dt) = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S%.3fZ") {
        return Utc.from_utc_datetime(&dt).timestamp_millis();
    }

    // Try ISO 8601 format with time: "2025-02-05T12:00:00Z"
    if let Ok(dt) = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%SZ") {
        return Utc.from_utc_datetime(&dt).timestamp_millis();
    }

    // Try ISO 8601 format with milliseconds without Z: "2025-02-05T12:00:00.123"
    if let Ok(dt) = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S%.3f") {
        return Utc.from_utc_datetime(&dt).timestamp_millis();
    }

    // Try ISO 8601 format with time without Z: "2025-02-05T12:00:00"
    if let Ok(dt) = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S") {
        return Utc.from_utc_datetime(&dt).timestamp_millis();
    }

    // Date-only format: "2025-02-05" - use noon UTC (12:00:00Z)
    // Noon keeps the local date stable for all timezones from UTC-12 to UTC+14,
    // so filtering by local day boundaries won't shift the record to an adjacent day.
    if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        let dt = date.and_hms_opt(12, 0, 0).unwrap();
        return Utc.from_utc_datetime(&dt).timestamp_millis();
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_duplicate_parse_counter_registration_does_not_poison_sibling() {
        let root = tempfile::tempdir().unwrap();
        let sibling = tempfile::tempdir().unwrap();
        let _root_counter = register_parse_cursor_file_counter(root.path());

        let duplicate = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _duplicate_counter = register_parse_cursor_file_counter(root.path());
        }));
        assert!(duplicate.is_err());
        assert_eq!(parse_cursor_file_call_count(root.path()), 0);

        let _sibling_counter = register_parse_cursor_file_counter(sibling.path());
        record_parse_cursor_file_call(&sibling.path().join("usage.csv"));
        assert_eq!(parse_cursor_file_call_count(sibling.path()), 1);
    }

    #[test]
    fn test_infer_provider() {
        assert_eq!(infer_provider("claude-3-sonnet"), "anthropic");
        assert_eq!(infer_provider("gpt-4o"), "openai");
        assert_eq!(infer_provider("gemini-pro"), "google");
        assert_eq!(infer_provider("deepseek-coder"), "deepseek");
        assert_eq!(infer_provider("llama-3"), "meta");
        assert_eq!(infer_provider("unknown-model"), "cursor");
        // A family name appearing only as a substring (not at a token boundary)
        // must not be misattributed; delimiter-aware inference falls back.
        assert_eq!(infer_provider("supergptmodel"), "cursor");
    }

    #[test]
    fn test_parse_cost() {
        assert_eq!(parse_cost("$0.50"), 0.50);
        assert_eq!(parse_cost("0.50"), 0.50);
        assert_eq!(parse_cost("$1,234.56"), 1234.56);
        assert_eq!(parse_cost(""), 0.0);
        assert_eq!(parse_cost("NaN"), 0.0);
        assert_eq!(parse_cost("nan"), 0.0);
        assert_eq!(parse_cost("  "), 0.0);
        // v3 format values
        assert_eq!(parse_cost("Included"), 0.0);
        assert_eq!(parse_cost("-"), 0.0);
        assert_eq!(parse_finite_cost("$0.00"), Some(0.0));
        assert_eq!(parse_finite_cost("Included"), None);
        assert_eq!(parse_finite_cost("-"), None);
        assert_eq!(parse_finite_cost("inf"), None);
        assert_eq!(parse_finite_cost("-0.50"), None);
    }

    #[test]
    fn test_parse_csv_line() {
        let line = "2025-02-01,gpt-4o,10,5,0,15,30,$0.10,$0.10";
        let fields = parse_csv_line(line);
        assert_eq!(fields.len(), 9);
        assert_eq!(fields[0], "2025-02-01");
        assert_eq!(fields[1], "gpt-4o");
        assert_eq!(fields[8], "$0.10");
    }

    #[test]
    fn test_parse_date_to_timestamp() {
        // ISO with milliseconds and Z (new Cursor format)
        let ts = parse_date_to_timestamp("2025-11-13T18:36:05.846Z");
        assert!(ts > 0);

        // ISO with Z
        let ts = parse_date_to_timestamp("2025-02-05T12:00:00Z");
        assert!(ts > 0);

        // Date only
        let ts = parse_date_to_timestamp("2025-02-05");
        assert!(ts > 0);

        // Invalid
        let ts = parse_date_to_timestamp("invalid");
        assert_eq!(ts, 0);
    }

    #[test]
    fn test_parse_cursor_csv_sample_old_format() {
        let csv = "Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost,Cost to you
2025-02-01,gpt-4o,10,5,0,15,30,$0.10,$0.10
2025-02-02,gpt-4o-mini,0,0,0,5,5,$0.05,$0.05";

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.csv");
        std::fs::write(&file_path, csv).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 2);

        assert_eq!(messages[0].client, "cursor");
        assert_eq!(messages[0].model_id, "gpt-4o");
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].tokens.input, 5);
        assert_eq!(messages[0].tokens.output, 15);
        assert_eq!(messages[0].tokens.cache_write, 10);
        assert!((messages[0].cost - 0.10).abs() < 0.001);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );

        assert_eq!(messages[1].model_id, "gpt-4o-mini");
    }

    #[test]
    fn test_parse_cursor_csv_sample_new_format() {
        // Real format from Cursor API
        let csv = r#"Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
"2025-11-13T18:36:05.846Z","Included","auto","No","28342","775","105891","21282","156290","0.19"
"2025-11-13T13:35:04.658Z","On-Demand","gpt-5-codex","No","0","8263","66964","1612","76839","0.03""#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.csv");
        std::fs::write(&file_path, csv).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 2);

        // First message: auto model
        assert_eq!(messages[0].client, "cursor");
        assert_eq!(messages[0].model_id, "auto");
        assert_eq!(messages[0].provider_id, "cursor"); // unknown model -> cursor
        assert_eq!(messages[0].tokens.input, 775);
        assert_eq!(messages[0].tokens.output, 21282);
        assert_eq!(messages[0].tokens.cache_read, 105891);
        assert_eq!(messages[0].tokens.cache_write, 28342);
        assert!((messages[0].cost - 0.19).abs() < 0.001);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );

        // Second message: gpt-5-codex
        assert_eq!(messages[1].model_id, "gpt-5-codex");
        assert_eq!(messages[1].provider_id, "openai"); // gpt -> openai
        assert_eq!(messages[1].tokens.input, 8263);
        assert_eq!(messages[1].tokens.cache_read, 66964);
    }

    #[test]
    fn test_parse_cursor_csv_luna_style_total_tokens() {
        let csv = r#"Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
"2026-08-18T12:00:00.000Z","On-Demand","claude-sonnet-4","No","15000000","1000000","20000000","2000000","38000000","$9.50"
"2026-08-18T13:00:00.000Z","On-Demand","claude-sonnet-4","No","5000000","500000","8000000","660000","14160000","$3.54""#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.csv");
        std::fs::write(&file_path, csv).unwrap();

        let total_tokens: i64 = parse_cursor_file(&file_path)
            .iter()
            .map(|message| message.tokens.total())
            .sum();

        assert_eq!(total_tokens, 52_160_000);
    }

    #[test]
    fn test_parse_cursor_csv_sample_v3_format() {
        // v3 format includes Cloud Agent ID and Automation ID columns
        let csv = r#"Date,Cloud Agent ID,Automation ID,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
"2026-04-09T20:01:10.528Z","bc-a380fb49-e1a5-414e-817d-6a85b6cdc51c","cc30782e-26cc-4359-bc22-7567efe282be","Included","composer-2","Yes","0","343446","29045760","915201","30304407","Included"
"2026-04-09T18:02:13.576Z","bc-19a9b74b-2af3-46e2-9f61-3ba1cdac46c8","1a0df38f-1474-4dfe-896b-70b841d4a833","On-Demand","composer-2","Yes","0","43478","420864","7957","472299","0.11"
"2026-04-09T07:39:09.091Z","bc-49262501-0ee0-49f9-b856-a5b0466deddb","","Errored, No Charge","composer-2","Yes","0","104504","985600","3666","1093770","-""#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.csv");
        std::fs::write(&file_path, csv).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 3);

        // First message: "Included" cost should be 0
        assert_eq!(messages[0].client, "cursor");
        assert_eq!(messages[0].model_id, "composer-2");
        assert_eq!(messages[0].cost, 0.0);
        assert_eq!(messages[0].cost_source, super::super::CostSource::Unknown);
        assert_eq!(messages[0].tokens.cache_read, 29045760);

        // Second message: actual cost from "On-Demand"
        assert_eq!(messages[1].model_id, "composer-2");
        assert!((messages[1].cost - 0.11).abs() < 0.001);
        assert_eq!(
            messages[1].cost_source,
            super::super::CostSource::ProviderReported
        );

        // Third message: "-" cost should be 0 (Errored, No Charge)
        assert_eq!(messages[2].model_id, "composer-2");
        assert_eq!(messages[2].cost, 0.0);
        assert_eq!(messages[2].cost_source, super::super::CostSource::Unknown);
    }

    #[test]
    fn test_explicit_zero_cost_is_provider_reported() {
        let csv = r#"Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
"2026-08-18T12:00:00.000Z","On-Demand","gpt-5","No","10","5","20","3","38","$0.00""#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.csv");
        std::fs::write(&file_path, csv).unwrap();

        let messages = parse_cursor_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 0.0);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );
    }

    #[test]
    fn test_negative_cost_is_not_provider_reported() {
        let csv = r#"Date,Kind,Model,Max Mode,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Total Tokens,Cost
"2026-08-18T12:00:00.000Z","On-Demand","gpt-5","No","10","5","20","3","38","-$0.50""#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.csv");
        std::fs::write(&file_path, csv).unwrap();

        let messages = parse_cursor_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 0.0);
        assert_eq!(messages[0].cost_source, super::super::CostSource::Unknown);
    }

    #[test]
    fn test_parse_cursor_events_json_keys_by_conversation_id() {
        // Real shape from get-filtered-usage-events, aggregated by the CLI.
        let json = r#"{
            "totalUsageEventsCount": 2,
            "usageEventsDisplay": [
                {
                    "timestamp": "1788171994838",
                    "model": "cursor-grok-4.6-high",
                    "kind": "USAGE_EVENT_KIND_USAGE_BASED",
                    "chargedCents": 74.03765106201172,
                    "tokenUsage": {
                        "inputTokens": 22252,
                        "outputTokens": 4283,
                        "cacheReadTokens": 1379927,
                        "cacheWriteTokens": 512,
                        "totalCents": 74.03765106201172
                    },
                    "conversationId": "b92fdbf1-36d4-4d78-bd5b-afcb939eab16"
                },
                {
                    "timestamp": "1788171000000",
                    "model": "gpt-5-codex",
                    "kind": "USAGE_EVENT_KIND_USAGE_BASED",
                    "chargedCents": 3,
                    "tokenUsage": {
                        "inputTokens": 8263,
                        "outputTokens": 1612,
                        "cacheReadTokens": 66964,
                        "totalCents": 3
                    },
                    "conversationId": "b92fdbf1-36d4-4d78-bd5b-afcb939eab16"
                }
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 2);

        // Session id is the real conversation UUID, not a synthetic timestamp id.
        assert_eq!(
            messages[0].session_id,
            "b92fdbf1-36d4-4d78-bd5b-afcb939eab16"
        );
        assert_eq!(messages[0].client, "cursor");
        assert_eq!(messages[0].model_id, "cursor-grok-4.6-high");
        assert_eq!(messages[0].tokens.input, 22252);
        assert_eq!(messages[0].tokens.output, 4283);
        assert_eq!(messages[0].tokens.cache_read, 1379927);
        assert_eq!(messages[0].tokens.cache_write, 512);
        // cost == chargedCents / 100, provider-reported.
        assert!((messages[0].cost - 0.7403765106201172).abs() < 1e-9);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );
        assert!(messages[0].timestamp > 0);

        // cacheWriteTokens absent -> 0; integer chargedCents handled.
        assert_eq!(messages[1].tokens.cache_write, 0);
        assert!((messages[1].cost - 0.03).abs() < 1e-9);

        // Summed session cost equals sum(chargedCents)/100 (acceptance check).
        let session_cost: f64 = messages.iter().map(|m| m.cost).sum();
        assert!((session_cost - (74.03765106201172 + 3.0) / 100.0).abs() < 1e-9);
    }

    #[test]
    fn test_parse_cursor_events_json_keeps_rows_when_sync_marked_partial() {
        // A sync that runs out of time writes the pages it already collected
        // with `"partial": true`. That flag is cache metadata, not an event
        // field, and must not drop the rows.
        let json = r#"{
            "partial": true,
            "totalUsageEventsCount": 1,
            "usageEventsDisplay": [
                {
                    "timestamp": "1788171994838",
                    "model": "gpt-5",
                    "chargedCents": 10,
                    "tokenUsage": {"inputTokens": 4, "outputTokens": 6, "totalCents": 10},
                    "conversationId": "session-partial"
                }
            ]
        }"#;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "session-partial");
        assert_eq!(messages[0].tokens.input, 4);
        assert_eq!(messages[0].tokens.output, 6);
        assert!((messages[0].cost - 0.10).abs() < 1e-9);
    }

    #[test]
    fn test_parse_cursor_events_json_plan_included_row_uses_metered_total_cents() {
        // Plan-included / free-credit rows debit the wallet nothing
        // (`chargedCents: 0`) while `tokenUsage.totalCents` still carries what
        // the usage metered. Reading only the charge threw that number away and
        // handed the row to local pricing, which refuses the router label `auto`
        // (#1062) — so the row landed at $0.00/Unknown and its tokens fell into
        // the unpriced bucket. The metered figure must win.
        let json = r#"{
            "usageEventsDisplay": [
                {
                    "timestamp": "1788171994838",
                    "model": "auto",
                    "kind": "USAGE_EVENT_KIND_FREE_CREDIT",
                    "chargedCents": 0,
                    "tokenUsage": {"inputTokens": 10, "outputTokens": 5, "totalCents": 6.2},
                    "conversationId": "11111111-2222-3333-4444-555555555555"
                }
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 1);
        assert!((messages[0].cost - 0.062).abs() < 1e-9);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );
        // The row must never depend on pricing the label, which is refused.
        assert!(crate::pricing::lookup::is_routing_label(
            &messages[0].model_id
        ));
    }

    #[test]
    fn test_parse_cursor_events_json_prefers_total_cents_over_charged_cents() {
        // When the two diverge the metered cost is what tokscale reports, the
        // same quantity the CSV `Cost` column has always carried. `chargedCents`
        // describes the credit card, not the usage.
        let json = r#"{
            "usageEventsDisplay": [
                {
                    "timestamp": "1788171994838",
                    "model": "composer-2",
                    "chargedCents": 1,
                    "tokenUsage": {"inputTokens": 10, "totalCents": 25},
                    "conversationId": "11111111-2222-3333-4444-555555555555"
                }
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 1);
        assert!((messages[0].cost - 0.25).abs() < 1e-9);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );
    }

    #[test]
    fn test_parse_cursor_events_json_falls_back_to_charged_cents() {
        // An event whose `tokenUsage` omits `totalCents` still has an
        // authoritative amount in `chargedCents`; it must not go unpriced.
        let json = r#"{
            "usageEventsDisplay": [
                {
                    "timestamp": "1788171994838",
                    "model": "gpt-5",
                    "chargedCents": 12.5,
                    "tokenUsage": {"inputTokens": 10, "outputTokens": 5},
                    "conversationId": "11111111-2222-3333-4444-555555555555"
                }
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 1);
        assert!((messages[0].cost - 0.125).abs() < 1e-9);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );
    }

    #[test]
    fn test_parse_cursor_events_json_rejects_negative_and_non_finite_cents() {
        // A negative or non-numeric metered cost is nonsense, not an
        // authoritative zero. It falls back to the charge when that is usable
        // and otherwise stays unknown, mirroring `parse_finite_cost` on the CSV
        // lane, so a bad figure never becomes immune to repricing.
        let json = r#"{
            "usageEventsDisplay": [
                {
                    "timestamp": "1788171994838",
                    "model": "composer-2",
                    "chargedCents": 4,
                    "tokenUsage": {"inputTokens": 10, "totalCents": -3},
                    "conversationId": "11111111-2222-3333-4444-555555555555"
                },
                {
                    "timestamp": "1788171994839",
                    "model": "composer-2",
                    "chargedCents": -1,
                    "tokenUsage": {"inputTokens": 10, "totalCents": -3},
                    "conversationId": "22222222-3333-4444-5555-666666666666"
                },
                {
                    "timestamp": "1788171994840",
                    "model": "composer-2",
                    "tokenUsage": {"inputTokens": 10, "totalCents": "not-a-number"},
                    "conversationId": "33333333-4444-5555-6666-777777777777"
                }
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 3);

        // Negative metered cost -> usable charge wins.
        assert!((messages[0].cost - 0.04).abs() < 1e-9);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );

        // Both unusable -> unknown, so local pricing may still estimate.
        assert_eq!(messages[1].cost, 0.0);
        assert_eq!(messages[1].cost_source, super::super::CostSource::Unknown);
        assert_eq!(messages[2].cost, 0.0);
        assert_eq!(messages[2].cost_source, super::super::CostSource::Unknown);
    }

    #[test]
    fn test_parse_cursor_events_json_explicit_zero_total_cents_is_provider_reported() {
        // A genuine metered zero is a fact Cursor stated, not missing data. The
        // CSV lane already retains an explicit `$0.00` as provider-reported
        // (`test_explicit_zero_cost_is_provider_reported`); the JSON lane agrees.
        let json = r#"{
            "usageEventsDisplay": [
                {
                    "timestamp": "1788171994838",
                    "model": "composer-2",
                    "tokenUsage": {"inputTokens": 10, "totalCents": 0},
                    "conversationId": "11111111-2222-3333-4444-555555555555"
                }
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 0.0);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );
    }

    #[test]
    fn test_parse_cursor_events_json_missing_charged_cents_is_unknown() {
        // No chargedCents -> cost 0 with Unknown source so pricing can estimate.
        let json = r#"{
            "usageEventsDisplay": [
                {
                    "timestamp": "1788171994838",
                    "model": "composer-2",
                    "tokenUsage": {"inputTokens": 10},
                    "conversationId": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
                }
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 0.0);
        assert_eq!(messages[0].cost_source, super::super::CostSource::Unknown);
    }

    #[test]
    fn test_parse_cursor_events_json_missing_conversation_id_falls_back_to_synthetic() {
        // An event with no conversation id must not be dropped: its cost still
        // lands under a synthetic per-day id so account totals stay correct.
        let json = r#"{
            "usageEventsDisplay": [
                {
                    "timestamp": "1788171994838",
                    "model": "gpt-5",
                    "chargedCents": 12.5,
                    "tokenUsage": {"inputTokens": 10, "outputTokens": 5}
                }
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].session_id.starts_with("cursor-active-"));
        assert!((messages[0].cost - 0.125).abs() < 1e-9);
    }

    #[test]
    fn test_parse_cursor_events_json_skips_empty_model_and_bad_timestamp() {
        let json = r#"{
            "usageEventsDisplay": [
                {"timestamp": "1788171994838", "model": "", "conversationId": "x"},
                {"timestamp": "not-a-number", "model": "gpt-5", "conversationId": "y"},
                {"timestamp": "1788171994838", "model": "gpt-5", "chargedCents": 1, "conversationId": "z"}
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "z");
    }

    #[test]
    fn test_parse_cursor_events_json_secondary_account_synthetic_fallback_uses_account() {
        // Fallback synthetic id derives the account from the file name for a
        // per-account cache file.
        let json = r#"{
            "usageEventsDisplay": [
                {"timestamp": "1788171994838", "model": "gpt-5", "chargedCents": 1}
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.team-a.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].session_id.starts_with("cursor-team-a-"));
    }

    #[test]
    fn test_parse_cursor_events_json_skips_only_the_malformed_row() {
        // A single row with an unexpected shape (tokenUsage as a string) must be
        // skipped on its own rather than discarding every other event.
        let json = r#"{
            "usageEventsDisplay": [
                {"timestamp": "1788171994838", "model": "gpt-5", "chargedCents": 1, "tokenUsage": "oops", "conversationId": "bad"},
                {"timestamp": "1788171994838", "model": "gpt-5", "chargedCents": 1, "conversationId": "good"}
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "good");
    }

    #[test]
    fn test_parse_cursor_events_json_tolerates_float_token_counts() {
        // Token counts arriving as floats (or numeric strings) must not drop the
        // row; they are coerced to integers.
        let json = r#"{
            "usageEventsDisplay": [
                {
                    "timestamp": "1788171994838",
                    "model": "gpt-5",
                    "chargedCents": "12.5",
                    "tokenUsage": {"inputTokens": 10.0, "outputTokens": "5"},
                    "conversationId": "float-row"
                }
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 5);
        assert!((messages[0].cost - 0.125).abs() < 1e-9);
    }

    #[test]
    fn test_parse_cursor_events_json_accepts_iso8601_timestamp() {
        // The same instant, once as ISO-8601 and once as base-10 milliseconds,
        // must resolve to identical timestamps.
        let json = r#"{
            "usageEventsDisplay": [
                {"timestamp": "2026-08-18T12:00:00.000Z", "model": "gpt-5", "chargedCents": 1, "conversationId": "iso"},
                {"timestamp": "1787054400000", "model": "gpt-5", "chargedCents": 1, "conversationId": "ms"}
            ]
        }"#;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("usage.json");
        std::fs::write(&file_path, json).unwrap();

        let messages = parse_cursor_file(&file_path);
        assert_eq!(messages.len(), 2);
        assert!(messages[0].timestamp > 0);
        assert_eq!(messages[0].timestamp, messages[1].timestamp);
    }
}
