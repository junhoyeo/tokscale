//! Meept session parser
//!
//! Parses per-call LLM usage rows from Meept's metrics SQLite database:
//! - `~/.meept/metrics.db`
//! - `$MEEPT_HOME/metrics.db` when `MEEPT_HOME` relocation is in play (the
//!   registry entry above covers the default `HOME/.meept` layout)
//!
//! Meept is a Go agent daemon; its metrics store (`internal/metrics`) writes
//! one `llm_calls` row per provider call. The schema this parser relies on is
//! pinned by meept's own integration test
//! (`internal/metrics/tokscale_ingest_test.go`, "Contract D" in the meept
//! plan tree `docs/plans/20260906-tokscale-ingest/`): if that test changes,
//! this parser changes with it.
//!
//! Column → breakdown mapping:
//!   tokens_sent            → input
//!   tokens_received        → output
//!   tokens_cached          → cache_read
//!   cache_creation_tokens  → cache_write
//!   reasoning_tokens       → reasoning
//!
//! Rows are per call, append-only, with a monotonic `id`. Error rows
//! (`error = 1`) carry no usage by construction, but the filter stays
//! explicit so a schema change that starts recording partial usage on
//! failures cannot silently inflate totals. Dedup keys off the row id —
//! two calls in the same millisecond to the same model are distinct work.

use super::utils::{open_readonly_sqlite, resolved_provider, sqlite_for_each_row_on};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use rusqlite::Connection;
use std::path::Path;
use tracing::warn;

/// One `llm_calls` row, decoded. Column order matches [`MEEPT_PROJECTION`].
struct MeeptUsageRow {
    id: i64,
    timestamp: String,
    provider: String,
    model_id: String,
    agent_id: String,
    session_id: String,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
}

/// The exact read surface meept's Contract D pins. `error = 0` excludes
/// failure rows (they carry no usage); the SUMs tolerate NULL on every
/// token column so a pre-migration database that somehow kept old rows
/// degrades to zeros instead of failing the whole parse.
const MEEPT_PROJECTION: &str = r#"
        SELECT
            id,
            timestamp,
            provider,
            model_id,
            agent_id,
            session_id,
            tokens_sent,
            tokens_received,
            tokens_cached,
            cache_creation_tokens,
            reasoning_tokens
        FROM llm_calls
        WHERE error = 0
          AND model_id IS NOT NULL
          AND TRIM(model_id) != ''
"#;

fn decode_usage_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeeptUsageRow> {
    Ok(MeeptUsageRow {
        id: row.get(0)?,
        timestamp: row.get(1)?,
        provider: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        model_id: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        agent_id: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        session_id: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        input: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
        output: row.get::<_, Option<i64>>(7)?.unwrap_or(0),
        cache_read: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
        cache_write: row.get::<_, Option<i64>>(9)?.unwrap_or(0),
        reasoning: row.get::<_, Option<i64>>(10)?.unwrap_or(0),
    })
}

/// Meept writes `timestamp` as ISO-8601 UTC via SQLite's
/// `strftime('%Y-%m-%dT%H:%M:%SZ','now')` default. Parse the full value
/// fallibly; on any unexpected shape fall back to the epoch so the row still
/// aggregates under "unknown time" rather than being dropped (or silently
/// re-dated by clamping).
fn parse_meept_timestamp(timestamp: &str) -> i64 {
    let trimmed = timestamp.trim();
    // char-bounded, not byte-bounded: a malformed value with multi-byte
    // characters passes `len() >= 19` with fewer than 19 chars, and the
    // slicing below must never panic. Fallible parse of the whole prefix —
    // no unwrap_or/clamp repair that would report a wrong-but-valid instant.
    if trimmed.chars().count() >= 19 {
        let chars: Vec<char> = trimmed.chars().collect();
        let parse_field = |range: std::ops::Range<usize>| -> Option<i64> {
            chars[range].iter().collect::<String>().parse().ok()
        };
        if let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
            parse_field(0..4),
            parse_field(5..7),
            parse_field(8..10),
            parse_field(11..13),
            parse_field(14..16),
            parse_field(17..19),
        ) {
            // Reject rather than clamp: month 13 or hour 99 means the value
            // is not what meept writes, and shifting it to a nearby valid
            // instant would misdate real usage.
            let in_range = (1..=12).contains(&month)
                && (1..=31).contains(&day)
                && hour <= 23
                && minute <= 59
                && second <= 59;
            if in_range {
                if let Some(date) =
                    chrono::NaiveDate::from_ymd_opt(year.max(1) as i32, month as u32, day as u32)
                {
                    if let Some(datetime) =
                        date.and_hms_opt(hour as u32, minute as u32, second as u32)
                    {
                        return datetime.and_utc().timestamp_millis();
                    }
                }
            }
        }
    }
    0
}

fn build_message(row: MeeptUsageRow) -> UnifiedMessage {
    let provider = resolved_provider(
        if row.provider.is_empty() {
            None
        } else {
            Some(row.provider.clone())
        },
        &row.model_id,
        "meept",
    );
    let mut msg = UnifiedMessage::new_with_agent(
        "meept",
        row.model_id,
        provider,
        row.session_id,
        parse_meept_timestamp(&row.timestamp),
        TokenBreakdown {
            input: row.input.max(0),
            output: row.output.max(0),
            cache_read: row.cache_read.max(0),
            cache_write: row.cache_write.max(0),
            reasoning: row.reasoning.max(0),
        },
        // Meept does not persist per-call cost; pricing is resolved by the
        // caller from the model id.
        0.0,
        if row.agent_id.is_empty() {
            None
        } else {
            Some(row.agent_id)
        },
    );
    msg.dedup_key = Some(format!("meept:{}", row.id));
    msg
}

pub fn parse_meept_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let conn = match open_readonly_sqlite(db_path) {
        Ok(c) => c,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open Meept metrics database"
            );
            return Vec::new();
        }
    };
    parse_meept_sqlite_on(&conn, db_path)
}

fn parse_meept_sqlite_on(conn: &Connection, db_path: &Path) -> Vec<UnifiedMessage> {
    // A metrics database created before meept's session/reasoning migration
    // lacks the new columns entirely. Probe once so the expected case stays
    // quiet instead of logging a prepare failure per install.
    let has_schema = match conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('llm_calls')
         WHERE name IN ('session_id', 'reasoning_tokens', 'cache_creation_tokens')",
        [],
        |row| row.get::<_, i64>(0),
    ) {
        Ok(count) => count == 3,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to probe Meept llm_calls schema"
            );
            false
        }
    };
    if !has_schema {
        return Vec::new();
    }

    let mut rows = Vec::new();
    let scan = sqlite_for_each_row_on(
        conn,
        db_path,
        MEEPT_PROJECTION,
        Some("Meept llm_calls row"),
        &mut |row| {
            rows.push(decode_usage_row(row)?);
            Ok(())
        },
    );
    // Require a COMPLETE scan: an Incomplete result is a prefix of the rows,
    // indistinguishable from a small database, so converting it to messages
    // would report silently undercounted totals.
    if !scan.completed() {
        return Vec::new();
    }

    rows.into_iter().map(build_message).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Builds an in-memory SQLite database with the exact schema meept's
    /// Contract D pins, seeds fixture rows, and runs the parser against it.
    fn seed_db(
        rows: &[(&str, &str, &str, &str, &str, i64, i64, i64, i64, i64, i64)],
    ) -> Vec<UnifiedMessage> {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE llm_calls (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                provider        TEXT NOT NULL DEFAULT '',
                model_id        TEXT NOT NULL DEFAULT '',
                agent_id        TEXT NOT NULL DEFAULT '',
                session_id      TEXT NOT NULL DEFAULT '',
                tokens_sent     INTEGER NOT NULL DEFAULT 0,
                tokens_received INTEGER NOT NULL DEFAULT 0,
                tokens_cached   INTEGER NOT NULL DEFAULT 0,
                reasoning_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                error           INTEGER NOT NULL DEFAULT 0,
                error_message   TEXT NOT NULL DEFAULT '',
                latency_ms      INTEGER NOT NULL DEFAULT 0,
                duration        REAL NOT NULL DEFAULT 0
            );
            "#,
        )
        .expect("create llm_calls");
        for (ts, provider, model, agent, session, sent, recv, cached, cache_w, reasoning, error) in
            rows
        {
            conn.execute(
                "INSERT INTO llm_calls (timestamp, provider, model_id, agent_id, session_id,
                     tokens_sent, tokens_received, tokens_cached, cache_creation_tokens,
                     reasoning_tokens, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    ts, provider, model, agent, session, sent, recv, cached, cache_w, reasoning,
                    error
                ],
            )
            .expect("insert fixture row");
        }
        parse_meept_sqlite_on(&conn, Path::new(":memory:"))
    }

    #[test]
    fn test_parses_contract_d_fixture() {
        let messages = seed_db(&[
            // convA/claude: 1000 in, 500 out, 200 cached, 150 cache-write, 100 reasoning
            (
                "2026-09-06T12:00:00Z",
                "anthropic",
                "claude-test",
                "coder",
                "convA",
                1000,
                500,
                200,
                150,
                100,
                0,
            ),
            // convA/claude: second call
            (
                "2026-09-06T12:01:00Z",
                "anthropic",
                "claude-test",
                "coder",
                "convA",
                800,
                300,
                0,
                0,
                0,
                0,
            ),
            // convA/other
            (
                "2026-09-06T12:02:00Z",
                "anthropic",
                "other-test",
                "coder",
                "convA",
                100,
                40,
                0,
                0,
                0,
                0,
            ),
            // convB/gpt
            (
                "2026-09-06T12:03:00Z",
                "openai",
                "gpt-test",
                "reviewer",
                "convB",
                50,
                25,
                0,
                0,
                0,
                0,
            ),
            // error row: excluded
            (
                "2026-09-06T12:04:00Z",
                "openai",
                "gpt-test",
                "reviewer",
                "convB",
                0,
                0,
                0,
                0,
                0,
                1,
            ),
        ]);
        assert_eq!(
            messages.len(),
            4,
            "error row excluded: {:?}",
            messages.len()
        );

        let total = |m: &[UnifiedMessage]| -> TokenBreakdown {
            m.iter().fold(TokenBreakdown::default(), |mut acc, msg| {
                acc.add_assign_saturating(&msg.tokens);
                acc
            })
        };
        let t = total(&messages);
        assert_eq!(t.input, 1950);
        assert_eq!(t.output, 865);
        assert_eq!(t.cache_read, 200);
        assert_eq!(t.cache_write, 150);
        assert_eq!(t.reasoning, 100);
    }

    #[test]
    fn test_dedup_keys_are_row_scoped() {
        let messages = seed_db(&[
            (
                "2026-09-06T12:00:00Z",
                "anthropic",
                "claude-test",
                "coder",
                "convA",
                10,
                5,
                0,
                0,
                0,
                0,
            ),
            (
                "2026-09-06T12:00:00Z",
                "anthropic",
                "claude-test",
                "coder",
                "convA",
                10,
                5,
                0,
                0,
                0,
                0,
            ),
        ]);
        assert_eq!(messages.len(), 2, "identical calls are distinct rows");
        let keys: HashSet<String> = messages
            .iter()
            .filter_map(|m| m.dedup_key.clone())
            .collect();
        assert_eq!(keys.len(), 2, "dedup keys derive from row id: {:?}", keys);
    }

    #[test]
    fn test_agent_and_session_flow_through() {
        let messages = seed_db(&[(
            "2026-09-06T12:00:00Z",
            "anthropic",
            "claude-test",
            "coder",
            "convA",
            10,
            5,
            0,
            0,
            0,
            0,
        )]);
        let msg = &messages[0];
        assert_eq!(msg.agent.as_deref(), Some("coder"));
        assert_eq!(msg.session_id, "convA");
        assert_eq!(msg.client, "meept");
    }

    #[test]
    fn test_timestamp_parsing() {
        assert_eq!(
            parse_meept_timestamp("2026-09-06T12:00:00Z"),
            chrono::NaiveDate::from_ymd_opt(2026, 9, 6)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_millis()
        );
        assert_eq!(parse_meept_timestamp("garbage"), 0);
        assert_eq!(parse_meept_timestamp(""), 0);
        // Multi-byte characters pass a byte-length check with fewer than 19
        // chars; the parser must fall back to 0, never panic on slicing.
        assert_eq!(parse_meept_timestamp("日本語のタイムスタンプです"), 0);
        // Out-of-range fields are rejected (epoch fallback), not clamped
        // into a nearby valid instant that would misdate real usage.
        assert_eq!(parse_meept_timestamp("2026-13-40T25:99:99Z"), 0);
        assert_eq!(parse_meept_timestamp("2026-00-00T00:00:00Z"), 0);
    }

    #[test]
    fn test_missing_columns_yield_no_messages() {
        // A database whose llm_calls predates the meept migration: parser
        // must return empty instead of erroring per install.
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE llm_calls (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                provider TEXT NOT NULL DEFAULT '',
                model_id TEXT NOT NULL DEFAULT '',
                tokens_sent INTEGER NOT NULL DEFAULT 0
            );",
        )
        .expect("create legacy table");
        let messages = parse_meept_sqlite_on(&conn, Path::new(":memory:"));
        assert!(messages.is_empty());
    }
}
