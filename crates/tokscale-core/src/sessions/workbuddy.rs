//! WorkBuddy session usage parser.
//!
//! WorkBuddy stores aggregate session usage in `~/.workbuddy/workbuddy.db`.
//! The `session_usage` table currently exposes a total `used` value, but does
//! not split input/output/cache tokens. Tokscale preserves that measured total
//! in the input bucket so reports can surface the usage without estimating.

use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use tracing::warn;

#[derive(Debug)]
struct WorkBuddyUsageRow {
    session_id: String,
    used: i64,
    updated_at: i64,
    model: Option<String>,
    cwd: Option<String>,
}

pub fn parse_workbuddy_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let conn = match Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => conn,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open WorkBuddy database"
            );
            return Vec::new();
        }
    };

    let mut stmt = match conn.prepare(
        r#"
        SELECT
            su.session_id,
            su.used,
            su.updated_at,
            s.model,
            s.cwd
        FROM session_usage su
        LEFT JOIN sessions s ON s.id = su.session_id
        WHERE su.used IS NOT NULL
          AND su.used > 0
          AND su.updated_at IS NOT NULL
          AND su.updated_at > 0
        "#,
    ) {
        Ok(stmt) => stmt,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to prepare WorkBuddy usage query"
            );
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok(WorkBuddyUsageRow {
            session_id: row.get(0)?,
            used: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
            updated_at: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
            model: row.get(3)?,
            cwd: row.get(4)?,
        })
    }) {
        Ok(rows) => rows,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to execute WorkBuddy usage query"
            );
            return Vec::new();
        }
    };

    rows.filter_map(|row| match row {
        Ok(row) => Some(usage_row_to_message(row)),
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to decode WorkBuddy usage row"
            );
            None
        }
    })
    .collect()
}

fn usage_row_to_message(row: WorkBuddyUsageRow) -> UnifiedMessage {
    let model_id = row
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or("auto")
        .to_string();
    let provider_id = provider_identity::inferred_provider_from_model(&model_id)
        .unwrap_or("workbuddy")
        .to_string();

    let mut message = UnifiedMessage::new(
        "workbuddy",
        model_id,
        provider_id,
        row.session_id.clone(),
        normalize_timestamp_ms(row.updated_at),
        TokenBreakdown {
            input: row.used.max(0),
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        },
        0.0,
    );
    // Include `updated_at` so distinct usage rows for the same session (e.g.
    // per-date or incremental writes) are not collapsed by the dedup key.
    message.dedup_key = Some(format!("workbuddy:{}:{}", row.session_id, row.updated_at));

    if let Some(workspace_key) = row.cwd.as_deref().and_then(normalize_workspace_key) {
        let workspace_label = workspace_label_from_key(&workspace_key);
        message.set_workspace(Some(workspace_key), workspace_label);
    }

    message
}

fn normalize_timestamp_ms(timestamp: i64) -> i64 {
    if timestamp > 10_000_000_000 {
        timestamp
    } else {
        timestamp.saturating_mul(1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn create_workbuddy_db(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                cwd TEXT,
                model TEXT
            );
            CREATE TABLE session_usage (
                session_id TEXT PRIMARY KEY,
                used INTEGER,
                size INTEGER,
                updated_at INTEGER,
                credit_json TEXT
            );
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn parse_workbuddy_sqlite_reads_session_usage() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("workbuddy.db");
        let conn = create_workbuddy_db(&db_path);
        conn.execute(
            "INSERT INTO sessions (id, cwd, model) VALUES (?1, ?2, ?3)",
            params!["session-1", "/Users/alice/project", "deepseek-v4-pro"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_usage (session_id, used, size, updated_at, credit_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["session-1", 1234, 1000000, 1_780_000_000_000_i64, "{}"],
        )
        .unwrap();
        drop(conn);

        let messages = parse_workbuddy_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "workbuddy");
        assert_eq!(message.model_id, "deepseek-v4-pro");
        assert_eq!(message.provider_id, "deepseek");
        assert_eq!(message.session_id, "session-1");
        assert_eq!(message.tokens.input, 1234);
        assert_eq!(message.tokens.output, 0);
        assert_eq!(message.message_count, 1);
        assert_eq!(message.workspace_label.as_deref(), Some("project"));
        assert_eq!(
            message.dedup_key.as_deref(),
            Some("workbuddy:session-1:1780000000000")
        );
    }

    #[test]
    fn parse_workbuddy_sqlite_skips_zero_usage() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("workbuddy.db");
        let conn = create_workbuddy_db(&db_path);
        conn.execute(
            "INSERT INTO session_usage (session_id, used, size, updated_at, credit_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["empty-session", 0, 1000000, 1_780_000_000_000_i64, "{}"],
        )
        .unwrap();
        drop(conn);

        assert!(parse_workbuddy_sqlite(&db_path).is_empty());
    }
}
