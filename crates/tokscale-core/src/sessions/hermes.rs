//! Hermes Agent session parser
//!
//! Parses aggregated session rows from Hermes Agent's SQLite state database:
//! - `~/.hermes/state.db`
//! - `$HERMES_HOME/state.db`
//!
//! Hermes records usage only as per-session aggregates (`session_model_usage`
//! carries `first_seen`/`last_seen` but no per-call timestamps, and
//! `messages.token_count` is never populated), so a session that spans several
//! calendar days would otherwise land its whole lifetime on its start day
//! (junhoyeo/tokscale#993). The rotated agent logs beside the database DO carry
//! one line per API call with a timestamp and the call's token figures, so the
//! parser splits each row's buckets across the days its logged calls fall in,
//! grouped in the scan's bucket timezone. The database stays authoritative for
//! totals: the logs only say WHEN, never HOW MUCH. Only the portion the logs
//! actually account for is distributed — whatever they cannot account for
//! (rotated-away calls, or turns that never reach the log) stays on the
//! session's start day — and a row whose logs over-report its buckets keeps
//! its whole total on the start day.

use super::utils::{
    lossy_lines, open_readonly_sqlite, resolved_provider, sqlite_for_each_row_on,
    timestamp_secs_to_ms,
};
use super::UnifiedMessage;
use crate::bucket_tz::BucketTimezone;
use crate::TokenBreakdown;
use chrono::{Local, TimeZone};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use tracing::warn;

const HERMES_AGENT_NAME: &str = "Hermes Agent";

/// Stands in for a NULL `billing_provider` inside a per-model dedup key. Angle
/// brackets keep it out of the slug space real provider ids live in.
const NULL_PROVIDER_KEY: &str = "<null>";

/// Hermes renders a missing runtime provider as this value in `agent.log`,
/// while current `session_model_usage` rows persist it as an empty string.
const UNKNOWN_LOG_PROVIDER_KEY: &str = "unknown";

/// Marker of a main-loop per-call log line. The usage-unavailable variant
/// (`in=? out=? usage=unavailable`) carries no figures and is skipped.
const API_CALL_LOG_MARKER: &str = " agent.conversation_loop: API call #";

/// Per-model breakdown, available once Hermes started recording
/// `session_model_usage`.
///
/// Grouping keeps `billing_provider` because the composite primary key lets a
/// single (session, model) pair span several providers; collapsing them would
/// erase one provider from the breakdown and credit its tokens to the other.
/// Cost is resolved per row (actual when non-zero, otherwise estimated) before
/// the SUM, so a reconciled sibling row cannot discard estimate-only siblings.
/// `ORDER BY` sorts the row matching `sessions.model` first within each session
/// and is otherwise total, so the caller can hand `sessions.message_count` to
/// the first row it sees per session and get the primary model whenever one
/// survives grouping — and a deterministic row when none does. Task-variant
/// rows (vision, compression, title generation, …) SUM into the same row as the
/// main loop's, which is what keeps one log-evidence lookup per key
/// unambiguous.
const PER_MODEL_QUERY: &str = r#"
        SELECT
            smu.session_id,
            smu.model,
            smu.billing_provider,
            s.started_at,
            COALESCE(s.message_count, 0) AS message_count,
            SUM(smu.input_tokens)        AS input_tokens,
            SUM(smu.output_tokens)       AS output_tokens,
            SUM(smu.cache_read_tokens)   AS cache_read_tokens,
            SUM(smu.cache_write_tokens)  AS cache_write_tokens,
            SUM(smu.reasoning_tokens)    AS reasoning_tokens,
            SUM(COALESCE(NULLIF(smu.actual_cost_usd, 0), smu.estimated_cost_usd, 0)) AS cost_usd
        FROM session_model_usage smu
        JOIN sessions s ON s.id = smu.session_id
        WHERE smu.model IS NOT NULL
          AND TRIM(smu.model) != ''
        GROUP BY smu.session_id, smu.model, smu.billing_provider, s.started_at, s.message_count
        HAVING SUM(smu.input_tokens) > 0
            OR SUM(smu.output_tokens) > 0
            OR SUM(smu.cache_read_tokens) > 0
            OR SUM(smu.cache_write_tokens) > 0
            OR SUM(smu.reasoning_tokens) > 0
            OR SUM(COALESCE(NULLIF(smu.actual_cost_usd, 0), smu.estimated_cost_usd, 0)) > 0
        ORDER BY smu.session_id,
                 CASE WHEN smu.model = s.model THEN 0 ELSE 1 END,
                 smu.model,
                 smu.billing_provider
"#;

/// Session-level totals credited to `sessions.model`. Covers Hermes builds that
/// predate `session_model_usage` and, on builds that have it, sessions with no
/// child rows in it; column order matches `PER_MODEL_QUERY`.
const SESSION_TOTALS_QUERY: &str = r#"
        SELECT
            id,
            model,
            billing_provider,
            started_at,
            message_count,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            reasoning_tokens,
            COALESCE(actual_cost_usd, estimated_cost_usd, 0) AS cost_usd
        FROM sessions
        WHERE model IS NOT NULL
          AND TRIM(model) != ''
          AND (
            COALESCE(input_tokens, 0) > 0 OR
            COALESCE(output_tokens, 0) > 0 OR
            COALESCE(cache_read_tokens, 0) > 0 OR
            COALESCE(cache_write_tokens, 0) > 0 OR
            COALESCE(reasoning_tokens, 0) > 0 OR
            COALESCE(actual_cost_usd, estimated_cost_usd, 0) > 0
          )
"#;

/// One usage row, decoded from either query — both project the same columns.
struct HermesUsageRow {
    session_id: String,
    model_id: String,
    billing_provider: Option<String>,
    started_at: f64,
    message_count: i32,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
    cost: f64,
}

impl HermesUsageRow {
    fn provider_key(&self) -> &str {
        self.billing_provider
            .as_deref()
            .unwrap_or(NULL_PROVIDER_KEY)
    }
}

fn decode_usage_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HermesUsageRow> {
    Ok(HermesUsageRow {
        session_id: row.get(0)?,
        model_id: row.get(1)?,
        billing_provider: row.get(2)?,
        started_at: row.get(3)?,
        message_count: row.get::<_, Option<i32>>(4)?.unwrap_or(0),
        input: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
        output: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
        cache_read: row.get::<_, Option<i64>>(7)?.unwrap_or(0),
        cache_write: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
        reasoning: row.get::<_, Option<i64>>(9)?.unwrap_or(0),
        cost: row.get::<_, Option<f64>>(10)?.unwrap_or(0.0),
    })
}

/// Runs one of the usage queries. Returns `None` when the query cannot run at
/// all (prepare or execute failure) so the caller can fall back, and `Some` —
/// possibly empty — when it ran and simply matched nothing.
fn query_usage_rows(db_path: &Path, conn: &Connection, query: &str) -> Option<Vec<HermesUsageRow>> {
    let mut rows = Vec::new();
    let scan = sqlite_for_each_row_on(conn, db_path, query, Some("Hermes session"), &mut |row| {
        rows.push(decode_usage_row(row)?);
        Ok(())
    });
    scan.ran().then_some(rows)
}

/// One `API call #N` line from a Hermes agent log: the timestamped evidence a
/// cross-day session's buckets are split over. `input` is the fresh prompt
/// share (`in` minus cache reads and writes), matching what Hermes stores in
/// `session_model_usage.input_tokens`.
struct LoggedCall {
    call_number: i64,
    timestamp_ms: i64,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
}

type LoggedCallFingerprint = (String, String, String, i64, i64, i64, i64, i64, i64);

/// Parse `2026-10-12 16:38:13,024 INFO [sid] agent.conversation_loop: API call
/// #69: model=m provider=p in=243703 out=1521 total=245224 latency=30.2s
/// cache=235008/243703 (96%) id=…`. Returns `None` for every other line —
/// including the usage-unavailable variant, whose `in=?` placeholders carry no
/// figures to allocate.
fn parse_logged_call(line: &str) -> Option<(String, String, String, LoggedCall)> {
    let (prefix, tail) = line.split_once(API_CALL_LOG_MARKER)?;
    let (call_number, fields) = tail.split_once(':')?;
    let call_number: i64 = call_number.trim().parse().ok()?;

    let mut model = None;
    let mut provider = None;
    let mut input = 0_i64;
    let mut output = 0_i64;
    let mut cache_read = 0_i64;
    let mut cache_write = 0_i64;
    for token in fields.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        match key {
            "model" => model = Some(value.to_string()),
            "provider" => provider = Some(value.to_string()),
            "in" => input = value.parse().ok()?,
            "out" => output = value.parse().ok()?,
            // `cache=<read>/<prompt>`: the log's `in` is the whole prompt;
            // cache reads and writes have separate stored buckets.
            "cache" => cache_read = value.split('/').next()?.parse().ok()?,
            "write" => cache_write = value.parse().ok()?,
            // total, latency, id, upstream: not needed for the split.
            _ => {}
        }
    }
    let (model, provider) = (model?, provider?);
    // The log's `in` is the whole prompt. The stored input bucket excludes
    // both cached and newly cache-written tokens, which have their own buckets.
    let input = (input - cache_read - cache_write).max(0);

    // Prefix shape: "YYYY-MM-DD HH:MM:SS,mmm INFO [session_id]".
    let session_id = prefix.rsplit_once(" [")?.1.strip_suffix(']')?;
    let (stamp, _) = prefix.split_once(" INFO ")?;
    // The logger writes local wall-clock time with millisecond precision; a
    // stamp without it still parses.
    let stamp = stamp.replacen(',', ".", 1);
    let naive = chrono::NaiveDateTime::parse_from_str(&stamp, "%Y-%m-%d %H:%M:%S%.3f")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(&stamp, "%Y-%m-%d %H:%M:%S"))
        .ok()?;
    // The logger writes local wall-clock time; a DST-ambiguous stamp takes the
    // earliest instant, matching how the day key is derived.
    let timestamp_ms = match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
            dt.timestamp_millis()
        }
        chrono::LocalResult::None => return None,
    };

    Some((
        session_id.to_string(),
        model,
        provider,
        LoggedCall {
            call_number,
            timestamp_ms,
            input,
            output,
            cache_read,
            cache_write,
        },
    ))
}

/// Rotated agent logs beside the database (`logs/agent.log*`). Deriving the
/// directory from the database path covers per-profile homes too: a profile's
/// `state.db` sits in its own directory with its own `logs/`.
fn hermes_log_paths(db_path: &Path) -> Vec<PathBuf> {
    let Some(parent) = db_path.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(parent.join("logs")) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("agent.log"))
        })
        .collect();
    paths.sort();
    paths
}

/// Index the per-call log lines for `sessions` only, keyed by
/// (session, model, provider). Sessions the database never recorded are
/// skipped: there is no row to allocate their tokens to, and indexing them
/// would grow with every log rotation. Rotation can re-emit the same line, so
/// its full call fingerprint is deduplicated first-wins. The call number alone
/// is not an identity: Hermes resets `session_api_calls` when a resumed session
/// gets a new agent process, so valid calls on different days can both be #1.
fn load_logged_calls(
    db_path: &Path,
    sessions: &HashSet<String>,
) -> HashMap<(String, String, String), Vec<LoggedCall>> {
    let mut index: HashMap<(String, String, String), Vec<LoggedCall>> = HashMap::new();
    let mut seen_calls: HashSet<LoggedCallFingerprint> = HashSet::new();
    for path in hermes_log_paths(db_path) {
        let Ok(file) = File::open(&path) else {
            continue;
        };
        for line in lossy_lines(BufReader::new(file)) {
            let Some((session_id, model, provider, call)) = parse_logged_call(&line) else {
                continue;
            };
            if !sessions.contains(&session_id) {
                continue;
            }
            let fingerprint = (
                session_id.clone(),
                model.clone(),
                provider.clone(),
                call.call_number,
                call.timestamp_ms,
                call.input,
                call.output,
                call.cache_read,
                call.cache_write,
            );
            if !seen_calls.insert(fingerprint) {
                continue;
            }
            index
                .entry((session_id, model, provider))
                .or_default()
                .push(call);
        }
    }
    index
}

/// One day's share of a usage row's buckets.
struct DayShare {
    day_key: String,
    timestamp_ms: i64,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
    cost: f64,
}

impl DayShare {
    fn tokens(&self) -> TokenBreakdown {
        // `reasoning_tokens` is a subset of `output_tokens` in Hermes, so the
        // additive buckets keep the same split the aggregate path applies.
        let reasoning = self.reasoning.clamp(0, self.output);
        TokenBreakdown {
            input: self.input.max(0),
            output: self.output.max(0) - reasoning,
            cache_read: self.cache_read.max(0),
            cache_write: self.cache_write.max(0),
            cache_write_1h: 0,
            reasoning,
        }
    }
}

/// Split `total` across `weights` proportionally, largest-remainder first, so
/// the parts sum back to `total` exactly. Ties break on weight then index to
/// stay deterministic. The products go through i128 because a cache-heavy
/// session can hold totals and weights large enough to overflow i64.
fn split_proportionally(total: i64, weights: &[i64], total_weight: i64) -> Vec<i64> {
    let total_wide = total as i128;
    let weight_wide = total_weight as i128;
    let mut parts: Vec<i64> = weights
        .iter()
        .map(|w| ((total_wide * (*w as i128)) / weight_wide) as i64)
        .collect();
    let mut remainder = total - parts.iter().sum::<i64>();
    if remainder == 0 {
        return parts;
    }
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by(|&a, &b| {
        let frac_a = (total_wide * (weights[a] as i128)) % weight_wide;
        let frac_b = (total_wide * (weights[b] as i128)) % weight_wide;
        frac_b
            .cmp(&frac_a)
            .then(weights[b].cmp(&weights[a]))
            .then(a.cmp(&b))
    });
    let step = remainder.signum();
    let mut i = 0;
    while remainder != 0 {
        let idx = order[i % order.len()];
        parts[idx] += step;
        remainder -= step;
        i += 1;
    }
    parts
}

/// Allocate one usage row's buckets across the days its logged calls fall in,
/// grouped in `bucket_timezone`.
///
/// Only the portion the logs actually account for is distributed: each logged
/// call's exact buckets stay on its day, and the remainder — rotated-away
/// calls, or turns that never reach the log — stays on the
/// session's start day, so partial evidence can never pull a lifetime total
/// onto one recent day.
///
/// `None` — the caller then keeps the whole row on the session's start day —
/// when the logs cannot speak for the row at all: no matching calls, a bucket
/// the logs over-report relative to the authoritative database row (rotated
/// or foreign lines), or no token weight to distribute by.
fn allocate_row_by_days(
    row: &HermesUsageRow,
    calls: &[LoggedCall],
    bucket_timezone: &BucketTimezone,
) -> Option<Vec<DayShare>> {
    if calls.is_empty() {
        return None;
    }
    let log_input: i64 = calls.iter().map(|c| c.input).sum();
    let log_output: i64 = calls.iter().map(|c| c.output).sum();
    let log_cache_read: i64 = calls.iter().map(|c| c.cache_read).sum();
    let log_cache_write: i64 = calls.iter().map(|c| c.cache_write).sum();
    if log_input > row.input
        || log_output > row.output
        || log_cache_read > row.cache_read
        || log_cache_write > row.cache_write
    {
        return None;
    }

    // Keep each logged bucket on the day that produced it. The database is
    // still authoritative for the row totals, but an accepted log line carries
    // exact per-call bucket counts, not merely an undifferentiated weight. Using
    // one combined token ratio for every bucket would move cache-read tokens to
    // output-heavy days (and output tokens to cache-heavy days).
    let mut shares: BTreeMap<String, DayShare> = BTreeMap::new();
    for call in calls {
        let day_key = bucket_timezone.day_key(call.timestamp_ms);
        if day_key.is_empty() {
            return None;
        }
        let day = shares.entry(day_key.clone()).or_insert(DayShare {
            day_key,
            timestamp_ms: call.timestamp_ms,
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            cost: 0.0,
        });
        day.timestamp_ms = day.timestamp_ms.min(call.timestamp_ms);
        day.input += call.input;
        day.output += call.output;
        day.cache_read += call.cache_read;
        day.cache_write += call.cache_write;
    }
    shares.retain(|_, share| {
        share.input > 0 || share.output > 0 || share.cache_read > 0 || share.cache_write > 0
    });
    if shares.is_empty() {
        return None;
    }

    // Whatever the logs could not account for stays on the start day, merged
    // into that day's share when the start day is itself a logged day. A row
    // the logs account for exactly gets no start-day share at all: inventing a
    // zero-token one would emit a phantom day.
    let start_ms = timestamp_secs_to_ms(row.started_at);
    let start_day_key = bucket_timezone.day_key(start_ms);
    if start_day_key.is_empty() {
        return None;
    }
    let residual_input = row.input - log_input;
    let residual_output = row.output - log_output;
    let residual_cache_read = row.cache_read - log_cache_read;
    let residual_cache_write = row.cache_write - log_cache_write;
    if residual_input != 0
        || residual_output != 0
        || residual_cache_read != 0
        || residual_cache_write != 0
    {
        let start_day = shares
            .entry(start_day_key.clone())
            .or_insert_with(|| DayShare {
                day_key: start_day_key.clone(),
                timestamp_ms: start_ms,
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
                cost: 0.0,
            });
        start_day.input += residual_input;
        start_day.output += residual_output;
        start_day.cache_read += residual_cache_read;
        start_day.cache_write += residual_cache_write;
    }

    // `reasoning_tokens` is a subset of `output_tokens`, so it follows the
    // emitted output shares; `DayShare::tokens` keeps the per-day relationship
    // intact even where largest-remainder rounding leaves a remainder.
    let output_weights: Vec<i64> = shares.values().map(|share| share.output.max(0)).collect();
    let total_output: i64 = output_weights.iter().sum();
    let reasoning = row.reasoning.clamp(0, total_output);
    let reasoning_parts = if total_output > 0 {
        split_proportionally(reasoning, &output_weights, total_output)
    } else {
        vec![0; shares.len()]
    };
    for (share, reasoning) in shares.values_mut().zip(reasoning_parts) {
        share.reasoning = reasoning;
    }

    // Cost rides the emitted tokens, so a day pays for what it is shown to
    // have spent; the last day absorbs the float residual. The weights are
    // i128 for the same reason `split_proportionally` uses it: a cache-heavy
    // session can hold buckets large enough to overflow an i64 sum.
    let token_weights: Vec<i128> = shares
        .values()
        .map(|share| {
            share.input.max(0) as i128
                + share.output.max(0) as i128
                + share.cache_read.max(0) as i128
                + share.cache_write.max(0) as i128
        })
        .collect();
    let total_tokens: i128 = token_weights.iter().sum();
    let row_cost = row.cost.max(0.0);
    let share_count = shares.len();
    if total_tokens > 0 {
        let mut cost_left = row_cost;
        for (i, share) in shares.values_mut().enumerate() {
            share.cost = if i + 1 == share_count {
                cost_left
            } else {
                let part = row_cost * (token_weights[i] as f64) / (total_tokens as f64);
                cost_left -= part;
                part
            };
        }
    } else if let Some(share) = shares.get_mut(&start_day_key) {
        // Degenerate row with cost but no tokens to weigh it by: keep the
        // cost with the start day rather than dropping it.
        share.cost = row_cost;
    }

    // Largest-remainder rounding can leave a positive-weight day with every
    // bucket rounded to zero; emitting it anyway would create an empty date
    // in daily aggregation. A share that carries nothing is dropped — the
    // row's totals are unaffected because it held none of them.
    Some(
        shares
            .into_values()
            .filter(|share| share.tokens().total() > 0 || share.cost != 0.0)
            .collect(),
    )
}

/// Whether this message's row was split across days from log evidence rather
/// than kept whole on the session's start day.
///
/// The caller uses it to pick the best-covered copy of a session that appears
/// in two databases: a split copy beats a no-logs fallback that would keep the
/// whole lifetime row on the start day, whatever the database order. The day
/// suffix is the signal — the fallback path's bare key never carries one.
pub fn message_is_day_split(message: &UnifiedMessage) -> bool {
    message
        .dedup_key
        .as_deref()
        .and_then(|key| key.rsplit_once(':'))
        .is_some_and(|(_, day)| {
            day.len() == 10 && chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok()
        })
}

fn build_message(row: &HermesUsageRow, dedup_key: String) -> UnifiedMessage {
    let output = row.output.max(0);
    let reasoning = row.reasoning.max(0).min(output);
    message_from_parts(
        row,
        dedup_key,
        timestamp_secs_to_ms(row.started_at),
        TokenBreakdown {
            input: row.input.max(0),
            output: output - reasoning,
            cache_read: row.cache_read.max(0),
            cache_write: row.cache_write.max(0),
            cache_write_1h: 0,
            reasoning,
        },
        row.cost.max(0.0),
    )
}

fn message_from_parts(
    row: &HermesUsageRow,
    dedup_key: String,
    timestamp_ms: i64,
    tokens: TokenBreakdown,
    cost: f64,
) -> UnifiedMessage {
    let provider = resolved_provider(row.billing_provider.clone(), &row.model_id, "hermes");
    let mut msg = UnifiedMessage::new_with_agent(
        "hermes",
        row.model_id.clone(),
        provider,
        row.session_id.clone(),
        timestamp_ms,
        tokens,
        cost,
        Some(HERMES_AGENT_NAME.to_string()),
    );
    msg.message_count = row.message_count.max(0);
    msg.dedup_key = Some(dedup_key);
    msg
}

/// Emit one usage row: split across its logged days when the evidence allows,
/// otherwise as a single message on the session's start day. `message_count` is
/// the session total already resolved to "credit exactly once" by the caller;
/// only the first message emitted for the row may carry it.
fn push_row_messages(
    row: &HermesUsageRow,
    base_key: String,
    calls: Option<&[LoggedCall]>,
    message_count: i32,
    bucket_timezone: &BucketTimezone,
    out: &mut Vec<UnifiedMessage>,
) {
    let shares = calls.and_then(|calls| allocate_row_by_days(row, calls, bucket_timezone));
    match shares {
        Some(shares) => {
            let mut first = true;
            for share in shares {
                let mut msg = message_from_parts(
                    row,
                    // The day is part of the identity: two days of one group
                    // must not share a dedup key, or the caller's dedup pass
                    // drops every day but the first.
                    format!("{}:{}", base_key, share.day_key),
                    share.timestamp_ms,
                    share.tokens(),
                    share.cost,
                );
                msg.message_count = if first { message_count } else { 0 };
                first = false;
                out.push(msg);
            }
        }
        None => {
            let mut msg = build_message(row, base_key);
            msg.message_count = message_count;
            out.push(msg);
        }
    }
}

fn has_session_model_usage(db_path: &Path, conn: &Connection) -> bool {
    match conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'session_model_usage'",
        [],
        |row| row.get::<_, i64>(0),
    ) {
        Ok(_) => true,
        Err(rusqlite::Error::QueryReturnedNoRows) => false,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to probe Hermes session_model_usage table; using session totals"
            );
            false
        }
    }
}

pub fn parse_hermes_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    parse_hermes_sqlite_in(db_path, &BucketTimezone::Local)
}

/// [`parse_hermes_sqlite`], keyed by the scan's bucketing timezone.
///
/// The day split is decided here, at parse time, so it has to group the logged
/// calls in the same zone the report buckets by. Grouping in the host zone
/// first would let the post-parse `rebucket_days` pass relabel the shares but
/// never recreate a day the grouping collapsed.
pub fn parse_hermes_sqlite_in(
    db_path: &Path,
    bucket_timezone: &BucketTimezone,
) -> Vec<UnifiedMessage> {
    let conn = match open_readonly_sqlite(db_path) {
        Ok(c) => c,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open Hermes state database"
            );
            return Vec::new();
        }
    };

    // Hermes builds that predate `session_model_usage` cannot run the per-model
    // query at all; probing first keeps that expected case quiet instead of
    // logging a prepare failure for every such install. When the table does
    // exist but the query still fails to prepare — schema drift, e.g. an older
    // `session_model_usage` without `reasoning_tokens` — `query_usage_rows`
    // returns None and the session totals below carry the whole database, so
    // drift degrades to coarser data instead of losing all of it.
    let per_model_rows = if has_session_model_usage(db_path, &conn) {
        query_usage_rows(db_path, &conn, PER_MODEL_QUERY)
    } else {
        None
    };
    let per_model_rows = per_model_rows.unwrap_or_default();
    let session_total_rows =
        query_usage_rows(db_path, &conn, SESSION_TOTALS_QUERY).unwrap_or_default();

    let covered_sessions: HashSet<String> = per_model_rows
        .iter()
        .map(|row| row.session_id.clone())
        .collect();
    // Current Hermes schemas persist a missing provider as `''`, whereas old
    // nullable schemas can expose NULL. If both variants exist for one group,
    // give the log's `provider=unknown` evidence to the current empty-string
    // row only so it is never applied twice.
    let empty_provider_groups: HashSet<(String, String)> = per_model_rows
        .iter()
        .filter(|row| row.billing_provider.as_deref() == Some(""))
        .map(|row| (row.session_id.clone(), row.model_id.clone()))
        .collect();
    let logged_calls = load_logged_calls(
        db_path,
        &per_model_rows
            .iter()
            .chain(session_total_rows.iter())
            .map(|row| row.session_id.clone())
            .collect(),
    );
    let mut messages: Vec<UnifiedMessage> = Vec::new();
    // Sessions whose `message_count` has already been handed to a row.
    let mut counted_sessions: HashSet<String> = HashSet::new();

    for row in &per_model_rows {
        // `sessions.message_count` is a per-session total, so exactly one row
        // per session may carry it. ORDER BY sorts the row matching
        // `sessions.model` first, so "the first row of the session" is that
        // primary model when it survives grouping, and a deterministic
        // stand-in when it does not — a NULL `sessions.model`, a rename
        // between the two tables, or a primary model whose rows were all zero
        // and got filtered out. Zeroing every later row keeps the per-session
        // sum equal to `sessions.message_count` instead of dropping it.
        let message_count = if counted_sessions.insert(row.session_id.clone()) {
            row.message_count
        } else {
            0
        };
        // SQLite groups NULL and '' separately, so one (session, model) can
        // reach this loop as two rows that differ only in a NULL vs empty
        // billing_provider. Rendering NULL as a sentinel keeps their keys
        // apart; folding NULL to "" would collide and the caller's dedup pass
        // would then drop the second row's tokens outright.
        let base_key = format!(
            "hermes:{}:{}:{}",
            row.session_id,
            row.model_id,
            row.provider_key()
        );
        let null_provider_is_shadowed = row.billing_provider.is_none()
            && empty_provider_groups.contains(&(row.session_id.clone(), row.model_id.clone()));
        let log_provider_key = match row.billing_provider.as_deref() {
            Some("") => UNKNOWN_LOG_PROVIDER_KEY,
            None if !null_provider_is_shadowed => UNKNOWN_LOG_PROVIDER_KEY,
            _ => row.provider_key(),
        };
        let calls = logged_calls
            .get(&(
                row.session_id.clone(),
                row.model_id.clone(),
                log_provider_key.to_string(),
            ))
            .map(Vec::as_slice);
        push_row_messages(
            row,
            base_key,
            calls,
            message_count,
            bucket_timezone,
            &mut messages,
        );
    }

    // Reconcile per session, not per table: a Hermes upgrade can create
    // `session_model_usage` without backfilling older sessions, and those
    // sessions still carry their totals on `sessions`. Emit totals only for
    // sessions the per-model pass did not cover, which makes the two result
    // sets disjoint by construction — they cannot double count.
    //
    // Deliberately no "top up" of a session whose smu rows only partially cover
    // its session totals: if a session has ANY smu row we trust smu, because
    // adding the difference would double count the models smu did record. Do
    // not "fix" this into a per-column reconciliation.
    for row in &session_total_rows {
        if covered_sessions.contains(&row.session_id) {
            continue;
        }
        let message_count = if counted_sessions.insert(row.session_id.clone()) {
            row.message_count
        } else {
            0
        };
        // This path emits at most one row per session, so it keeps the bare
        // session id as its dedup key; the per-model keys are namespaced and
        // cannot collide with it.
        let log_provider_key = match row.billing_provider.as_deref() {
            None | Some("") => UNKNOWN_LOG_PROVIDER_KEY,
            Some(provider) => provider,
        };
        let calls = logged_calls
            .get(&(
                row.session_id.clone(),
                row.model_id.clone(),
                log_provider_key.to_string(),
            ))
            .map(Vec::as_slice);
        push_row_messages(
            row,
            row.session_id.clone(),
            calls,
            message_count,
            bucket_timezone,
            &mut messages,
        );
    }

    messages
}
