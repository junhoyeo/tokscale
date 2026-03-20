//! Crush session parser
//!
//! Crush persists usage in a per-project SQLite database (`crush.db`).
//! The database exposes reliable session-level cost, but not reliable
//! cumulative per-message or per-model token accounting for import.

use super::UnifiedMessage;
use crate::TokenBreakdown;
use rusqlite::Connection;
use std::path::Path;

const CRUSH_MODEL_ID: &str = "session-total";
const CRUSH_PROVIDER_ID: &str = "crush";

/// Parse root Crush sessions from a `crush.db` file.
///
/// Crush stores `cost` as a session-level cumulative value, but its
/// `prompt_tokens` / `completion_tokens` session columns are rewritten across
/// steps and are not safe to import as cumulative totals. Tokscale v1 therefore
/// imports one synthetic record per root session, preserves the stored cost,
/// and leaves the token breakdown at zero instead of fabricating precision.
pub fn parse_crush_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let conn = match Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => conn,
        Err(_) => return Vec::new(),
    };

    let query = r#"
        SELECT id, message_count, cost, created_at
        FROM sessions
        WHERE parent_session_id IS NULL
          AND (COALESCE(message_count, 0) > 0 OR COALESCE(cost, 0) > 0)
        ORDER BY created_at ASC
    "#;

    let mut stmt = match conn.prepare(query) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };

    let rows = match stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let message_count: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
        let cost: f64 = row.get::<_, Option<f64>>(2)?.unwrap_or(0.0);
        let created_at_secs: i64 = row.get::<_, Option<i64>>(3)?.unwrap_or(0);
        Ok((id, message_count, cost, created_at_secs))
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };

    let db_namespace = db_path.to_string_lossy().to_string();
    let mut messages = Vec::new();

    for row in rows.flatten() {
        let (id, message_count, cost, created_at_secs) = row;
        if message_count <= 0 && cost <= 0.0 {
            continue;
        }

        let timestamp_ms = created_at_secs.saturating_mul(1000);
        let session_id = format!("{}:{}", db_namespace, id);

        messages.push(UnifiedMessage::new(
            "crush",
            CRUSH_MODEL_ID,
            CRUSH_PROVIDER_ID,
            session_id,
            timestamp_ms,
            TokenBreakdown::default(),
            cost.max(0.0),
        ));
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_db(dir: &TempDir) -> std::path::PathBuf {
        let db_path = dir.path().join("crush.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                parent_session_id TEXT,
                title TEXT,
                message_count INTEGER NOT NULL DEFAULT 0,
                prompt_tokens INTEGER NOT NULL DEFAULT 0,
                completion_tokens INTEGER NOT NULL DEFAULT 0,
                cost REAL NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )
        .unwrap();
        db_path
    }

    #[test]
    fn test_parse_crush_sqlite_imports_root_sessions_with_created_at_timestamp() {
        let dir = TempDir::new().unwrap();
        let db_path = create_test_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        conn.execute(
            "INSERT INTO sessions (id, parent_session_id, title, message_count, cost, updated_at, created_at)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["root-1", "Root", 7_i64, 12.5_f64, 1_742_342_000_i64, 1_742_300_000_i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, parent_session_id, title, message_count, cost, updated_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params!["child-1", "root-1", "Child", 3_i64, 99.0_f64, 1_742_342_001_i64, 1_742_300_100_i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, parent_session_id, title, message_count, cost, updated_at, created_at)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["empty-root", "Empty", 0_i64, 0.0_f64, 1_742_342_002_i64, 1_742_300_200_i64],
        )
        .unwrap();

        let messages = parse_crush_sqlite(&db_path);
        assert_eq!(messages.len(), 1);

        let message = &messages[0];
        assert_eq!(message.client, "crush");
        assert_eq!(message.model_id, CRUSH_MODEL_ID);
        assert_eq!(message.provider_id, CRUSH_PROVIDER_ID);
        assert_eq!(message.timestamp, 1_742_300_000_000_i64);
        assert_eq!(message.tokens.total(), 0);
        assert_eq!(message.cost, 12.5);
        assert!(message.session_id.ends_with(":root-1"));
    }

    #[test]
    fn test_parse_crush_sqlite_returns_empty_for_missing_db() {
        let messages = parse_crush_sqlite(Path::new("/nonexistent/crush.db"));
        assert!(messages.is_empty());
    }
}
