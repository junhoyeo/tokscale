//! Hermes Agent session parser
//!
//! Parses aggregated session rows from Hermes Agent's SQLite state database:
//! - `~/.hermes/state.db`
//! - `$HERMES_HOME/state.db`

use super::utils::{
    open_readonly_sqlite, resolved_provider, sqlite_for_each_row_on, timestamp_secs_to_ms,
};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;
use tracing::warn;

const HERMES_AGENT_NAME: &str = "Hermes Agent";

/// Stands in for a NULL `billing_provider` inside a per-model dedup key. Angle
/// brackets keep it out of the slug space real provider ids live in.
const NULL_PROVIDER_KEY: &str = "<null>";

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
/// survives grouping — and a deterministic row when none does.
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

fn build_message(row: HermesUsageRow, dedup_key: String) -> UnifiedMessage {
    let provider = resolved_provider(row.billing_provider, &row.model_id, "hermes");
    let mut msg = UnifiedMessage::new_with_agent(
        "hermes",
        row.model_id,
        provider,
        row.session_id,
        timestamp_secs_to_ms(row.started_at),
        TokenBreakdown {
            input: row.input.max(0),
            output: row.output.max(0),
            cache_read: row.cache_read.max(0),
            cache_write: row.cache_write.max(0),
            reasoning: row.reasoning.max(0),
        },
        row.cost.max(0.0),
        Some(HERMES_AGENT_NAME.to_string()),
    );
    msg.message_count = row.message_count.max(0);
    msg.dedup_key = Some(dedup_key);
    msg
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

    let mut messages: Vec<UnifiedMessage> = Vec::new();
    // Sessions the per-model pass emitted at least one row for.
    let mut covered_sessions: HashSet<String> = HashSet::new();
    // Sessions whose `message_count` has already been handed to a row.
    let mut counted_sessions: HashSet<String> = HashSet::new();

    for mut row in per_model_rows.unwrap_or_default() {
        covered_sessions.insert(row.session_id.clone());
        // `sessions.message_count` is a per-session total, so exactly one row
        // per session may carry it. ORDER BY sorts the row matching
        // `sessions.model` first, so "the first row of the session" is that
        // primary model when it survives grouping, and a deterministic
        // stand-in when it does not — a NULL `sessions.model`, a rename
        // between the two tables, or a primary model whose rows were all zero
        // and got filtered out. Zeroing every later row keeps the per-session
        // sum equal to `sessions.message_count` instead of dropping it.
        if !counted_sessions.insert(row.session_id.clone()) {
            row.message_count = 0;
        }
        // SQLite groups NULL and '' separately, so one (session, model) can
        // reach this loop as two rows that differ only in a NULL vs empty
        // billing_provider. Rendering NULL as a sentinel keeps their keys
        // apart; folding NULL to "" would collide and the caller's dedup pass
        // would then drop the second row's tokens outright.
        let provider_key = row.billing_provider.as_deref().unwrap_or(NULL_PROVIDER_KEY);
        let dedup_key = format!(
            "hermes:{}:{}:{}",
            row.session_id, row.model_id, provider_key
        );
        messages.push(build_message(row, dedup_key));
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
    for row in query_usage_rows(db_path, &conn, SESSION_TOTALS_QUERY).unwrap_or_default() {
        if covered_sessions.contains(&row.session_id) {
            continue;
        }
        // This path emits at most one message per session, so it keeps the bare
        // session id as its dedup key; the per-model keys are namespaced and
        // cannot collide with it.
        let dedup_key = row.session_id.clone();
        messages.push(build_message(row, dedup_key));
    }

    messages
}
