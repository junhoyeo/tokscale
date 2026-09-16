//! GitHub Copilot CLI `session-store.db` parser.
//!
//! Copilot CLI writes per-turn usage to `~/.copilot/session-store.db` table
//! `assistant_usage_events`. This is a different schema from Desktop `data.db`.

use super::utils::sqlite_for_each_row;
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use chrono::{DateTime, NaiveDateTime};
use std::path::Path;
use tracing::warn;

const NANO_AIU_PER_USD: f64 = 1e11;

pub fn parse_copilot_session_store_db(db_path: &Path) -> Vec<UnifiedMessage> {
    let query = r#"
        SELECT
          e.id, e.session_id, e.turn_index, e.model, e.copilot_usage_model,
          e.input_tokens, e.output_tokens, e.cache_read_tokens, e.cache_write_tokens,
          e.reasoning_tokens, e.total_nano_aiu, e.duration_ms, e.created_at,
          s.cwd
        FROM assistant_usage_events e
        LEFT JOIN sessions s ON s.id = e.session_id
        WHERE COALESCE(e.input_tokens,0) > 0
           OR COALESCE(e.output_tokens,0) > 0
           OR COALESCE(e.cache_read_tokens,0) > 0
           OR COALESCE(e.cache_write_tokens,0) > 0
           OR COALESCE(e.reasoning_tokens,0) > 0
           OR COALESCE(e.total_nano_aiu,0) > 0
        "#;

    let mut messages = Vec::new();
    sqlite_for_each_row(
        db_path,
        query,
        Some("Copilot CLI session-store usage event"),
        &mut |row| {
            let event = UsageEvent {
                event_id: row.get(0)?,
                session_id: row.get(1)?,
                model: row.get(3)?,
                copilot_usage_model: row.get(4)?,
                input_tokens: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                output_tokens: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
                cache_read_tokens: row.get::<_, Option<i64>>(7)?.unwrap_or(0),
                cache_write_tokens: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
                reasoning_tokens: row.get::<_, Option<i64>>(9)?.unwrap_or(0),
                total_nano_aiu: row.get::<_, Option<i64>>(10)?.unwrap_or(0),
                duration_ms: row.get(11)?,
                created_at: row.get(12)?,
                cwd: row.get(13)?,
            };

            if let Some(message) = usage_event_to_message(event) {
                messages.push(message);
            }
            Ok(())
        },
    );

    messages
}

struct UsageEvent {
    event_id: i64,
    session_id: String,
    model: Option<String>,
    copilot_usage_model: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    reasoning_tokens: i64,
    total_nano_aiu: i64,
    duration_ms: Option<i64>,
    created_at: Option<String>,
    cwd: Option<String>,
}

fn usage_event_to_message(event: UsageEvent) -> Option<UnifiedMessage> {
    if event.session_id.trim().is_empty() {
        return None;
    }

    let cache_read = event.cache_read_tokens.max(0);
    let cache_write = event.cache_write_tokens.max(0);
    let tokens = TokenBreakdown {
        input: event
            .input_tokens
            .saturating_sub(cache_read)
            .saturating_sub(cache_write)
            .max(0),
        output: event.output_tokens.max(0),
        cache_read,
        cache_write,
        reasoning: event.reasoning_tokens.max(0),
    };

    let model_id = preferred_model(event.copilot_usage_model.as_deref(), event.model.as_deref());
    let provider_id = inferred_provider_from_model(&model_id)
        .unwrap_or("github-copilot")
        .to_string();
    let timestamp = event
        .created_at
        .as_deref()
        .and_then(parse_iso8601_timestamp_ms)
        .unwrap_or_else(|| {
            warn!(
                session_id = %event.session_id,
                event_id = event.event_id,
                created_at = ?event.created_at,
                "Copilot CLI session-store event has unparseable created_at; defaulting to 0"
            );
            0
        });
    let nano_aiu = event.total_nano_aiu.max(0);
    let cost = if nano_aiu > 0 {
        nano_aiu as f64 / NANO_AIU_PER_USD
    } else {
        0.0
    };
    let dedup_key = format!(
        "copilot-session-store:{}:{}",
        event.session_id, event.event_id
    );

    let mut message = UnifiedMessage::new_with_dedup(
        "copilot",
        model_id,
        provider_id,
        event.session_id,
        timestamp,
        tokens,
        cost,
        Some(dedup_key),
    );
    message.duration_ms = event.duration_ms;
    if nano_aiu > 0 {
        message.mark_provider_reported_cost();
    }
    if let Some(workspace_key) = event.cwd.as_deref().and_then(normalize_workspace_key) {
        let workspace_label = workspace_label_from_key(&workspace_key);
        message.set_workspace(Some(workspace_key), workspace_label);
    }
    Some(message)
}

fn preferred_model(copilot_usage_model: Option<&str>, model: Option<&str>) -> String {
    first_non_empty(copilot_usage_model)
        .or_else(|| first_non_empty(model))
        .unwrap_or("auto")
        .to_string()
}

fn first_non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn parse_iso8601_timestamp_ms(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp_millis())
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|timestamp| timestamp.and_utc().timestamp_millis())
        })
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|timestamp| timestamp.and_utc().timestamp_millis())
        })
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .map(|timestamp| timestamp.and_utc().timestamp_millis())
        })
        .or_else(|| {
            let numeric = value.parse::<i64>().ok()?;
            if numeric > 10_000_000_000 {
                Some(numeric)
            } else {
                Some(numeric.saturating_mul(1000))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::CostSource;
    use rusqlite::{params, Connection};
    use std::path::Path;

    fn create_session_store_db(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                cwd TEXT,
                repository TEXT,
                host_type TEXT,
                branch TEXT,
                summary TEXT,
                created_at TEXT,
                updated_at TEXT
            );
            CREATE TABLE assistant_usage_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT,
                turn_index INTEGER,
                model TEXT,
                copilot_usage_model TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_read_tokens INTEGER,
                cache_write_tokens INTEGER,
                reasoning_tokens INTEGER,
                total_nano_aiu INTEGER,
                duration_ms INTEGER,
                created_at TEXT
            );
            "#,
        )
        .unwrap();
        conn
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_usage_event(
        conn: &Connection,
        session_id: &str,
        model: Option<&str>,
        copilot_usage_model: Option<&str>,
        input: i64,
        output: i64,
        cache_read: i64,
        cache_write: i64,
        reasoning: i64,
        nano_aiu: i64,
        duration_ms: Option<i64>,
        created_at: &str,
    ) {
        conn.execute(
            r#"
            INSERT INTO assistant_usage_events (
                session_id, turn_index, model, copilot_usage_model,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                reasoning_tokens, total_nano_aiu, duration_ms, created_at
            ) VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                session_id,
                model,
                copilot_usage_model,
                input,
                output,
                cache_read,
                cache_write,
                reasoning,
                nano_aiu,
                duration_ms,
                created_at
            ],
        )
        .unwrap();
    }

    fn insert_session(conn: &Connection, id: &str, cwd: &str) {
        conn.execute(
            "INSERT INTO sessions (id, cwd, created_at) VALUES (?1, ?2, ?3)",
            params![id, cwd, "2026-07-01 12:34:56"],
        )
        .unwrap();
    }

    #[test]
    fn parse_copilot_session_store_db_returns_empty_for_missing_database() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.db");
        assert!(parse_copilot_session_store_db(&missing).is_empty());
    }

    #[test]
    fn parse_copilot_session_store_db_returns_empty_for_missing_table() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("session-store.db");
        Connection::open(&db_path).unwrap();
        assert!(parse_copilot_session_store_db(&db_path).is_empty());
    }

    #[test]
    fn parse_copilot_session_store_db_skips_zero_token_zero_nano_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("session-store.db");
        let conn = create_session_store_db(&db_path);
        insert_usage_event(
            &conn,
            "session-1",
            Some("gpt-5.4-mini"),
            None,
            0,
            0,
            0,
            0,
            0,
            0,
            None,
            "2026-07-01 12:34:56",
        );
        drop(conn);

        assert!(parse_copilot_session_store_db(&db_path).is_empty());
    }

    #[test]
    fn parse_copilot_session_store_db_subtracts_cache_read_and_write_from_input() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("session-store.db");
        let conn = create_session_store_db(&db_path);
        insert_usage_event(
            &conn,
            "session-1",
            Some("gpt-5.4-mini"),
            None,
            21_343,
            100,
            0,
            20_974,
            0,
            556_570_000,
            Some(1234),
            "2026-07-01 12:34:56",
        );
        drop(conn);

        let messages = parse_copilot_session_store_db(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 369);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.cache_write, 20_974);
        assert_eq!(messages[0].tokens.output, 100);
        assert!((messages[0].cost - 0.005_565_7).abs() < 1e-12);
        assert_eq!(messages[0].cost_source, CostSource::ProviderReported);
        assert_eq!(messages[0].duration_ms, Some(1234));
        assert_eq!(messages[0].message_count, 1);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("copilot-session-store:session-1:1")
        );
        assert_eq!(messages[0].timestamp, 1_782_909_296_000);
    }

    #[test]
    fn parse_copilot_session_store_db_leaves_unknown_cost_when_nano_aiu_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("session-store.db");
        let conn = create_session_store_db(&db_path);
        insert_usage_event(
            &conn,
            "session-1",
            Some("gpt-5.4-mini"),
            None,
            100,
            20,
            0,
            0,
            0,
            0,
            None,
            "2026-07-01 12:34:56",
        );
        drop(conn);

        let messages = parse_copilot_session_store_db(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 0.0);
        assert_eq!(messages[0].cost_source, CostSource::Unknown);
        assert_eq!(messages[0].tokens.input, 100);
    }

    #[test]
    fn parse_copilot_session_store_db_prefers_copilot_usage_model() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("session-store.db");
        let conn = create_session_store_db(&db_path);
        insert_usage_event(
            &conn,
            "session-1",
            Some("gpt-4o"),
            Some("claude-sonnet-4-5"),
            10,
            5,
            0,
            0,
            0,
            1,
            None,
            "2026-07-01 12:34:56",
        );
        drop(conn);

        let messages = parse_copilot_session_store_db(&db_path);
        assert_eq!(messages[0].model_id, "claude-sonnet-4-5");
        assert_eq!(messages[0].provider_id, "anthropic");
    }

    #[test]
    fn parse_copilot_session_store_db_sets_workspace_from_joined_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("session-store.db");
        let conn = create_session_store_db(&db_path);
        insert_session(&conn, "session-1", "/Users/dev/tokscale");
        insert_usage_event(
            &conn,
            "session-1",
            Some("gpt-5.4-mini"),
            None,
            10,
            5,
            0,
            0,
            0,
            1,
            None,
            "2026-07-01 12:34:56",
        );
        drop(conn);

        let messages = parse_copilot_session_store_db(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/dev/tokscale")
        );
        assert_eq!(messages[0].workspace_label.as_deref(), Some("tokscale"));
    }
}
