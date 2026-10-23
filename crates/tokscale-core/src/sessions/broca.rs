//! Broca metering export parser.
//!
//! Broca writes one immutable usage fact per completed run to the
//! `export_facts` table in its live `run-index.db` SQLite database.

use super::utils::open_readonly_sqlite;
use super::UnifiedMessage;
use crate::TokenBreakdown;
#[cfg(test)]
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::path::Path;
use tracing::{debug, warn};

#[derive(Debug, Deserialize)]
struct BrocaSegment {
    fact_id: String,
    session: BrocaSession,
    provider: String,
    model: String,
    usage: BrocaUsage,
    occurred_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct BrocaSession {
    project_root: String,
    session: String,
}

#[derive(Debug, Deserialize)]
struct BrocaUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    cached_input_tokens: i64,
    #[serde(default)]
    cache_write_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    reasoning_tokens: i64,
}

fn build_message(segment: BrocaSegment, timestamp: i64) -> UnifiedMessage {
    let output_total = segment.usage.output_tokens.max(0);
    // Broca normalizes reasoning as a subset of output across providers, while
    // Tokscale's token buckets are disjoint. Split the subset out so totals do
    // not count reasoning twice.
    let reasoning = segment.usage.reasoning_tokens.max(0).min(output_total);
    let mut message = UnifiedMessage::new_with_dedup(
        "broca",
        segment.model,
        segment.provider,
        segment.session.session,
        timestamp,
        TokenBreakdown {
            // Broca's input field is already fresh, non-cached input. Unlike
            // inclusive provider counters, cached input must not be subtracted.
            input: segment.usage.input_tokens.max(0),
            output: output_total.saturating_sub(reasoning),
            cache_read: segment.usage.cached_input_tokens.max(0),
            cache_write: segment.usage.cache_write_tokens.max(0),
            // The metering fact reports one cache-write counter without a TTL
            // split, so every write is billed at the default 5-minute rate.
            cache_write_1h: 0,
            reasoning,
        },
        0.0,
        Some(segment.fact_id),
    );
    message.workspace_key = Some(segment.session.project_root.clone());
    message.workspace_label = Some(segment.session.project_root);
    message.message_count = 1;
    message
}

pub fn parse_broca_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    // Do not use SQLite's immutable mode here. Broca writes in WAL mode, and
    // immutable readers ignore committed rows that still live in the WAL file.
    let conn = match open_readonly_sqlite(db_path) {
        Ok(conn) => conn,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open Broca run index"
            );
            return Vec::new();
        }
    };

    let mut stmt = match conn.prepare("SELECT segment_json FROM export_facts ORDER BY export_seq") {
        Ok(stmt) => stmt,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to prepare Broca export query"
            );
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
        Ok(rows) => rows,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to query Broca export facts"
            );
            return Vec::new();
        }
    };

    let mut messages = Vec::new();
    let mut malformed_rows = 0usize;
    let mut untimestamped_rows = 0usize;

    for row in rows {
        let Ok(segment_json) = row else {
            malformed_rows += 1;
            continue;
        };
        let Ok(segment) = serde_json::from_str::<BrocaSegment>(&segment_json) else {
            malformed_rows += 1;
            continue;
        };
        let Some(timestamp) = segment.occurred_at_ms.filter(|timestamp| *timestamp > 0) else {
            untimestamped_rows += 1;
            continue;
        };
        messages.push(build_message(segment, timestamp));
    }

    if malformed_rows > 0 {
        warn!(
            db_path = %db_path.display(),
            skipped_rows = malformed_rows,
            "Skipped malformed Broca export facts"
        );
    }
    if untimestamped_rows > 0 {
        debug!(
            db_path = %db_path.display(),
            skipped_rows = untimestamped_rows,
            "Skipped historical Broca export facts without timestamps"
        );
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::NamedTempFile;

    fn create_export_db(segments: &[serde_json::Value]) -> NamedTempFile {
        let file = NamedTempFile::new().unwrap();
        let conn = Connection::open(file.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE export_facts (\
                 export_seq INTEGER PRIMARY KEY AUTOINCREMENT,\
                 fact_id TEXT NOT NULL UNIQUE,\
                 session TEXT NOT NULL,\
                 run_id TEXT NOT NULL,\
                 segment_json TEXT NOT NULL\
             );",
        )
        .unwrap();

        for (index, segment) in segments.iter().enumerate() {
            conn.execute(
                "INSERT INTO export_facts (fact_id, session, run_id, segment_json) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    format!("row-{index}"),
                    format!("session-{index}"),
                    format!("run-{index}"),
                    segment.to_string()
                ],
            )
            .unwrap();
        }
        drop(conn);
        file
    }

    fn segment(timestamp: Option<i64>) -> serde_json::Value {
        serde_json::json!({
            "run_id": "run-1",
            "session": {
                "project_root": "/work/project",
                "harness": "runner",
                "session": "alfonso:main"
            },
            "segment_ordinal": 0,
            "fact_id": "fact-1",
            "provider": "anthropic",
            "model": "claude-opus-5",
            "usage": {
                "input_tokens": 100,
                "cached_input_tokens": 5000,
                "cache_write_tokens": 20,
                "output_tokens": 1000,
                "reasoning_tokens": 400
            },
            "occurred_at_ms": timestamp,
            "occurred_at_precision": "terminal",
            "terminal_reason": "completed"
        })
    }

    #[test]
    fn maps_disjoint_tokens_identity_and_workspace() {
        let file = create_export_db(&[segment(Some(1_786_181_660_799))]);

        let messages = parse_broca_sqlite(file.path());

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "broca");
        assert_eq!(message.provider_id, "anthropic");
        assert_eq!(message.model_id, "claude-opus-5");
        assert_eq!(message.session_id, "alfonso:main");
        assert_eq!(message.workspace_key.as_deref(), Some("/work/project"));
        assert_eq!(message.workspace_label.as_deref(), Some("/work/project"));
        assert_eq!(message.dedup_key.as_deref(), Some("fact-1"));
        assert_eq!(message.tokens.input, 100);
        assert_eq!(message.tokens.cache_read, 5000);
        assert_eq!(message.tokens.cache_write, 20);
        assert_eq!(message.tokens.output, 600);
        assert_eq!(message.tokens.reasoning, 400);
        assert_eq!(message.tokens.output + message.tokens.reasoning, 1000);
        assert_eq!(message.tokens.total(), 6120);
        assert_eq!(message.cost, 0.0);
        assert_eq!(message.message_count, 1);
    }

    #[test]
    fn reads_committed_facts_that_are_still_in_the_wal() {
        let file = NamedTempFile::new().unwrap();
        let conn = Connection::open(file.path()).unwrap();
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "wal");
        conn.execute_batch(
            "PRAGMA wal_autocheckpoint = 0;
             CREATE TABLE export_facts (
                 export_seq INTEGER PRIMARY KEY AUTOINCREMENT,
                 fact_id TEXT NOT NULL UNIQUE,
                 session TEXT NOT NULL,
                 run_id TEXT NOT NULL,
                 segment_json TEXT NOT NULL
             );",
        )
        .unwrap();
        let checkpoint_busy: i64 = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
            .unwrap();
        assert_eq!(checkpoint_busy, 0);

        conn.execute(
            "INSERT INTO export_facts (fact_id, session, run_id, segment_json) \
             VALUES (?1, ?2, ?3, ?4)",
            params![
                "wal-fact",
                "alfonso:main",
                "run-1",
                segment(Some(1_786_181_660_799)).to_string()
            ],
        )
        .unwrap();

        let wal_path = std::path::PathBuf::from(format!("{}-wal", file.path().display()));
        assert!(std::fs::metadata(&wal_path).unwrap().len() > 0);

        // This control demonstrates the failure mode: immutable readers ignore
        // the committed row because it exists only in the WAL sidecar.
        let immutable_uri = format!("file:{}?immutable=1", file.path().display());
        let immutable = Connection::open_with_flags(
            immutable_uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI,
        )
        .unwrap();
        let immutable_count: i64 = immutable
            .query_row("SELECT COUNT(*) FROM export_facts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(immutable_count, 0);

        let messages = parse_broca_sqlite(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("fact-1"));
        drop(conn);
    }

    #[test]
    fn skips_rows_without_a_usable_timestamp() {
        let file = create_export_db(&[segment(None), segment(Some(0))]);

        assert!(parse_broca_sqlite(file.path()).is_empty());
    }

    #[test]
    fn skips_malformed_export_facts_without_losing_valid_rows() {
        let file = create_export_db(&[
            serde_json::json!({"fact_id": "malformed"}),
            segment(Some(1_786_181_660_799)),
        ]);

        let messages = parse_broca_sqlite(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("fact-1"));
    }
}
