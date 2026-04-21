//! Crush session parser
//!
//! Crush persists usage in a per-project SQLite database (`crush.db`).
//! The database exposes reliable session-level cost, but not reliable
//! per-message token accounting for import.

use super::utils::open_readonly_sqlite;
use super::UnifiedMessage;
use crate::TokenBreakdown;
use chrono::{Local, TimeZone};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

const CRUSH_MODEL_ID: &str = "session-total";
const CRUSH_PROVIDER_ID: &str = "crush";

#[derive(Debug)]
struct CrushSession {
    id: String,
    cost: f64,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DayBucket {
    timestamp_ms: i64,
    message_count: i32,
}

/// Parse root Crush sessions from a `crush.db` file.
///
/// Crush stores reliable cost at the root-session level, but does not expose a
/// stable per-message token breakdown. Tokscale v1 therefore preserves cost
/// and assistant-message counts without fabricating token precision:
/// - assistant messages are grouped by local day
/// - session cost is allocated across those days proportionally
/// - token fields remain zero
pub fn parse_crush_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite(db_path) else {
        return Vec::new();
    };

    let root_sessions = load_root_sessions(&conn);
    if root_sessions.is_empty() {
        return Vec::new();
    }

    let assistant_buckets = load_assistant_buckets(&conn);
    let db_namespace = db_path.to_string_lossy().to_string();
    let mut messages = Vec::new();

    for session in root_sessions {
        let session_key = format!("{}:{}", db_namespace, session.id);

        if let Some(day_buckets) = assistant_buckets.get(&session.id) {
            let total_assistant_messages: i32 =
                day_buckets.iter().map(|bucket| bucket.message_count).sum();
            let safe_cost = session.cost.max(0.0);
            let mut allocated_cost = 0.0;

            for (index, bucket) in day_buckets.iter().enumerate() {
                let bucket_cost = if index + 1 == day_buckets.len() {
                    (safe_cost - allocated_cost).max(0.0)
                } else {
                    safe_cost * f64::from(bucket.message_count)
                        / f64::from(total_assistant_messages)
                };
                allocated_cost += bucket_cost;

                let mut message = UnifiedMessage::new(
                    "crush",
                    CRUSH_MODEL_ID,
                    CRUSH_PROVIDER_ID,
                    session_key.clone(),
                    bucket.timestamp_ms,
                    TokenBreakdown::default(),
                    bucket_cost,
                );
                message.message_count = bucket.message_count.max(0);
                messages.push(message);
            }

            continue;
        }

        if session.cost <= 0.0 {
            continue;
        }

        let Some(timestamp_ms) =
            fallback_session_timestamp_ms(session.updated_at, session.created_at)
        else {
            continue;
        };

        let mut message = UnifiedMessage::new(
            "crush",
            CRUSH_MODEL_ID,
            CRUSH_PROVIDER_ID,
            session_key,
            timestamp_ms,
            TokenBreakdown::default(),
            session.cost.max(0.0),
        );
        message.message_count = 0;
        messages.push(message);
    }

    messages.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    messages
}

fn load_root_sessions(conn: &Connection) -> Vec<CrushSession> {
    let query = r#"
        SELECT id, cost, created_at, updated_at
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
        Ok(CrushSession {
            id: row.get(0)?,
            cost: row.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
            created_at: row.get::<_, Option<i64>>(2)?.unwrap_or(0),
            updated_at: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
        })
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };

    rows.flatten().collect()
}

fn load_assistant_buckets(conn: &Connection) -> HashMap<String, Vec<DayBucket>> {
    let query = r#"
        WITH RECURSIVE session_tree(root_session_id, session_id) AS (
            SELECT id, id
            FROM sessions
            WHERE parent_session_id IS NULL

            UNION ALL

            SELECT st.root_session_id, s.id
            FROM sessions s
            JOIN session_tree st ON s.parent_session_id = st.session_id
        )
        SELECT st.root_session_id, m.created_at
        FROM session_tree st
        JOIN messages m ON m.session_id = st.session_id
        WHERE m.role = 'assistant'
        ORDER BY st.root_session_id ASC, m.created_at ASC
    "#;

    let mut stmt = match conn.prepare(query) {
        Ok(stmt) => stmt,
        Err(_) => return HashMap::new(),
    };

    let rows = match stmt.query_map([], |row| {
        let session_id: String = row.get(0)?;
        let created_at: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
        Ok((session_id, created_at))
    }) {
        Ok(rows) => rows,
        Err(_) => return HashMap::new(),
    };

    let mut session_days: HashMap<String, BTreeMap<String, DayBucket>> = HashMap::new();

    for row in rows.flatten() {
        let (session_id, created_at) = row;
        let Some(timestamp_ms) = normalize_crush_timestamp_ms(created_at) else {
            continue;
        };
        let Some(local_day) = local_day_key(timestamp_ms) else {
            continue;
        };

        let day_map = session_days.entry(session_id).or_default();
        let bucket = day_map.entry(local_day).or_insert(DayBucket {
            timestamp_ms,
            message_count: 0,
        });
        bucket.timestamp_ms = bucket.timestamp_ms.min(timestamp_ms);
        bucket.message_count = bucket.message_count.saturating_add(1);
    }

    session_days
        .into_iter()
        .map(|(session_id, day_map)| (session_id, day_map.into_values().collect()))
        .collect()
}

fn normalize_crush_timestamp_ms(raw: i64) -> Option<i64> {
    if raw <= 0 {
        return None;
    }

    if raw >= 100_000_000_000 {
        Some(raw)
    } else {
        raw.checked_mul(1000)
    }
}

fn local_day_key(timestamp_ms: i64) -> Option<String> {
    match Local.timestamp_millis_opt(timestamp_ms) {
        chrono::LocalResult::Single(dt) => Some(dt.format("%Y-%m-%d").to_string()),
        _ => None,
    }
}

fn fallback_session_timestamp_ms(updated_at: i64, created_at: i64) -> Option<i64> {
    normalize_crush_timestamp_ms(updated_at).or_else(|| normalize_crush_timestamp_ms(created_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
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

            CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                parts TEXT NOT NULL DEFAULT '[]',
                model TEXT,
                provider TEXT,
                is_summary_message INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT 0,
                finished_at INTEGER
            );
            "#,
        )
        .unwrap();
        db_path
    }

    fn insert_root_session(
        conn: &Connection,
        id: &str,
        message_count: i64,
        cost: f64,
        updated_at: i64,
        created_at: i64,
    ) {
        conn.execute(
            "INSERT INTO sessions (id, parent_session_id, title, message_count, cost, updated_at, created_at)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6)",
            params![id, "Root", message_count, cost, updated_at, created_at],
        )
        .unwrap();
    }

    fn insert_child_session(conn: &Connection, id: &str, parent_id: &str) {
        conn.execute(
            "INSERT INTO sessions (id, parent_session_id, title, message_count, cost, updated_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, parent_id, "Child", 2_i64, 99.0_f64, 1_742_342_001_i64, 1_742_300_100_i64],
        )
        .unwrap();
    }

    fn insert_message(
        conn: &Connection,
        id: &str,
        session_id: &str,
        role: &str,
        created_at: i64,
        is_summary_message: i64,
    ) {
        conn.execute(
            "INSERT INTO messages (id, session_id, role, parts, model, provider, is_summary_message, created_at, updated_at)
             VALUES (?1, ?2, ?3, '[]', 'gpt-5.4', 'crush', ?4, ?5, ?5)",
            params![id, session_id, role, is_summary_message, created_at],
        )
        .unwrap();
    }

    #[test]
    fn test_parse_crush_sqlite_allocates_cost_across_assistant_message_days() {
        let dir = TempDir::new().unwrap();
        let db_path = create_test_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        let day_one = 1_742_300_000_i64;
        let day_two = 1_742_386_400_i64;

        insert_root_session(&conn, "root-1", 5, 30.0, day_two, day_one);
        insert_child_session(&conn, "child-1", "root-1");
        insert_message(&conn, "msg-1", "root-1", "assistant", day_one, 0);
        insert_message(&conn, "msg-2", "root-1", "user", day_one + 10, 0);
        insert_message(&conn, "msg-3", "root-1", "assistant", day_two, 0);
        insert_message(&conn, "msg-4", "root-1", "assistant", day_two + 10, 1);
        insert_message(&conn, "msg-5", "child-1", "assistant", day_two + 20, 0);

        let messages = parse_crush_sqlite(&db_path);
        assert_eq!(messages.len(), 2);

        assert_eq!(messages[0].client, "crush");
        assert_eq!(messages[0].model_id, CRUSH_MODEL_ID);
        assert_eq!(messages[0].provider_id, CRUSH_PROVIDER_ID);
        assert_eq!(messages[0].timestamp, day_one * 1000);
        assert_eq!(messages[0].message_count, 1);
        assert!((messages[0].cost - 7.5).abs() < 1e-9);

        assert_eq!(messages[1].timestamp, day_two * 1000);
        assert_eq!(messages[1].message_count, 3);
        assert!((messages[1].cost - 22.5).abs() < 1e-9);
        assert!(
            (messages.iter().map(|msg| msg.cost).sum::<f64>() - 30.0).abs() < 1e-9,
            "allocated cost must sum back to the stored session total"
        );
        assert!(messages
            .iter()
            .all(|msg| msg.session_id.ends_with(":root-1")));
        assert!(messages.iter().all(|msg| msg.tokens.total() == 0));
    }

    #[test]
    fn test_parse_crush_sqlite_uses_updated_at_when_costed_session_has_no_assistant_messages() {
        let dir = TempDir::new().unwrap();
        let db_path = create_test_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_root_session(
            &conn,
            "root-1",
            3,
            4.5,
            1_742_342_000_i64,
            1_742_300_000_i64,
        );
        insert_message(&conn, "msg-1", "root-1", "user", 1_742_300_100_i64, 0);

        let messages = parse_crush_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].timestamp, 1_742_342_000_000_i64);
        assert_eq!(messages[0].message_count, 0);
        assert_eq!(messages[0].cost, 4.5);
    }

    #[test]
    fn test_parse_crush_sqlite_preserves_millisecond_timestamps() {
        let dir = TempDir::new().unwrap();
        let db_path = create_test_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        let created_at_ms = 1_742_300_000_123_i64;
        insert_root_session(&conn, "root-1", 1, 2.0, created_at_ms, created_at_ms);
        insert_message(&conn, "msg-1", "root-1", "assistant", created_at_ms, 0);

        let messages = parse_crush_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].timestamp, created_at_ms);
        assert_eq!(messages[0].message_count, 1);
        assert_eq!(messages[0].cost, 2.0);
    }

    #[test]
    fn test_parse_crush_sqlite_includes_child_session_assistant_messages() {
        let dir = TempDir::new().unwrap();
        let db_path = create_test_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        let day_one = 1_742_300_000_i64;
        let day_two = 1_742_386_400_i64;

        insert_root_session(&conn, "root-1", 4, 40.0, day_two, day_one);
        insert_child_session(&conn, "child-1", "root-1");
        insert_message(&conn, "msg-1", "root-1", "assistant", day_one, 0);
        insert_message(&conn, "msg-2", "child-1", "assistant", day_two, 0);

        let messages = parse_crush_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            2,
            "root-session cost should be distributed across assistant messages in descendant sessions too"
        );
        assert_eq!(messages[0].timestamp, day_one * 1000);
        assert_eq!(messages[0].message_count, 1);
        assert!((messages[0].cost - 20.0).abs() < 1e-9);

        assert_eq!(messages[1].timestamp, day_two * 1000);
        assert_eq!(messages[1].message_count, 1);
        assert!((messages[1].cost - 20.0).abs() < 1e-9);
        assert!(messages
            .iter()
            .all(|msg| msg.session_id.ends_with(":root-1")));
    }

    #[test]
    fn test_parse_crush_sqlite_returns_empty_for_missing_db() {
        let messages = parse_crush_sqlite(Path::new("/nonexistent/crush.db"));
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_crush_sqlite_skips_sessions_without_valid_timestamps() {
        let dir = TempDir::new().unwrap();
        let db_path = create_test_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_root_session(&conn, "root-1", 3, 4.5, 0, 0);

        let messages = parse_crush_sqlite(&db_path);
        assert!(messages.is_empty());
    }
}
