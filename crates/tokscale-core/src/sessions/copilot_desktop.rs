//! GitHub Copilot Desktop SQLite parser.
//!
//! The macOS desktop app stores aggregate token totals in `~/.copilot/data.db`
//! and per-session event metadata in `~/.copilot/session-state/{session_id}`.

use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{provider_identity::inferred_provider_from_model, TokenBreakdown};
use chrono::{DateTime, NaiveDateTime};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::Path;
use tracing::warn;

#[derive(Debug)]
struct CopilotDesktopSessionRow {
    id: String,
    model: Option<String>,
    total_input_tokens: i64,
    total_output_tokens: i64,
    total_cached_tokens: i64,
    total_reasoning_tokens: i64,
    created_at: Option<String>,
}

#[derive(Debug, Default)]
struct SessionStateMetadata {
    model: Option<String>,
    cwd: Option<String>,
}

pub fn parse_copilot_desktop_db(db_path: &Path) -> Vec<UnifiedMessage> {
    let conn = match Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => conn,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open Copilot Desktop database"
            );
            return Vec::new();
        }
    };

    let mut stmt = match conn.prepare(
        r#"
        SELECT
            id,
            title,
            model,
            total_input_tokens,
            total_output_tokens,
            total_cached_tokens,
            total_reasoning_tokens,
            total_nano_aiu,
            created_at
        FROM sessions
        WHERE total_input_tokens > 0
           OR total_output_tokens > 0
           OR total_cached_tokens > 0
           OR total_reasoning_tokens > 0
        "#,
    ) {
        Ok(stmt) => stmt,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to prepare Copilot Desktop sessions query"
            );
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok(CopilotDesktopSessionRow {
            id: row.get(0)?,
            model: row.get(2)?,
            total_input_tokens: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
            total_output_tokens: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
            total_cached_tokens: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
            total_reasoning_tokens: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
            created_at: row.get(8)?,
        })
    }) {
        Ok(rows) => rows,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to execute Copilot Desktop sessions query"
            );
            return Vec::new();
        }
    };

    rows.filter_map(|row| match row {
        Ok(row) => Some(session_row_to_message(db_path, row)),
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to decode Copilot Desktop session row"
            );
            None
        }
    })
    .collect()
}

fn session_row_to_message(db_path: &Path, row: CopilotDesktopSessionRow) -> UnifiedMessage {
    let metadata = read_session_state_metadata(db_path, &row.id);
    let model_id = metadata
        .model
        .as_deref()
        .or(row.model.as_deref())
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or("auto")
        .to_string();
    let provider_id = inferred_provider_from_model(&model_id)
        .unwrap_or("github-copilot")
        .to_string();

    let timestamp_ms = row
        .created_at
        .as_deref()
        .and_then(parse_iso8601_timestamp_ms)
        .unwrap_or_else(|| {
            warn!(
                session_id = %row.id,
                created_at = ?row.created_at,
                "Copilot Desktop session has unparseable created_at; defaulting to 0"
            );
            0
        });

    let mut message = UnifiedMessage::new_with_dedup(
        "copilot",
        model_id,
        provider_id,
        row.id.clone(),
        timestamp_ms,
        TokenBreakdown {
            input: row.total_input_tokens.max(0),
            output: row.total_output_tokens.max(0),
            cache_read: row.total_cached_tokens.max(0),
            cache_write: 0,
            reasoning: row.total_reasoning_tokens.max(0),
        },
        0.0,
        Some(format!("copilot-desktop:{}", row.id)),
    );

    if let Some(workspace_key) = metadata.cwd.as_deref().and_then(normalize_workspace_key) {
        let workspace_label = workspace_label_from_key(&workspace_key);
        message.set_workspace(Some(workspace_key), workspace_label);
    }

    message
}

fn read_session_state_metadata(db_path: &Path, session_id: &str) -> SessionStateMetadata {
    let Some(copilot_root) = db_path.parent() else {
        return SessionStateMetadata::default();
    };
    let events_path = copilot_root
        .join("session-state")
        .join(session_id)
        .join("events.jsonl");

    read_events_metadata(&events_path)
}

fn read_events_metadata(events_path: &Path) -> SessionStateMetadata {
    let file = match std::fs::File::open(events_path) {
        Ok(file) => file,
        Err(_) => return SessionStateMetadata::default(),
    };

    let mut metadata = SessionStateMetadata::default();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let Some(event_type) = event.get("type").and_then(Value::as_str) else {
            continue;
        };

        match event_type {
            "session.start" if metadata.cwd.is_none() => {
                metadata.cwd = event
                    .pointer("/data/context/cwd")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|cwd| !cwd.is_empty())
                    .map(str::to_string);
            }
            "session.model_change" => {
                if let Some(model) = event
                    .pointer("/data/newModel")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|model| !model.is_empty() && model != &"auto")
                {
                    metadata.model = Some(model.to_string());
                }
            }
            _ => {}
        }
    }

    metadata
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
            let numeric = value.parse::<i64>().ok()?;
            // Distinguish seconds vs milliseconds: values < 10 billion are
            // assumed to be Unix seconds (common in SQLite), otherwise millis.
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
    use rusqlite::{params, Connection};
    use std::fs::{self, File};
    use std::io::Write;

    fn create_copilot_desktop_db(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT,
                title TEXT,
                session_type TEXT,
                mode TEXT,
                model TEXT,
                total_input_tokens INTEGER,
                total_output_tokens INTEGER,
                total_cached_tokens INTEGER,
                total_reasoning_tokens INTEGER,
                total_nano_aiu INTEGER,
                created_at TEXT,
                agent TEXT,
                provider_id TEXT
            );
            "#,
        )
        .unwrap();
        conn
    }

    fn insert_session(
        conn: &Connection,
        id: &str,
        model: &str,
        input: i64,
        output: i64,
        cached: i64,
        reasoning: i64,
    ) {
        conn.execute(
            r#"
            INSERT INTO sessions (
                id, title, session_type, mode, model,
                total_input_tokens, total_output_tokens, total_cached_tokens,
                total_reasoning_tokens, total_nano_aiu, created_at, agent, provider_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            params![
                id,
                "Test session",
                "chat",
                "agent",
                model,
                input,
                output,
                cached,
                reasoning,
                0_i64,
                "2026-07-01T12:34:56Z",
                "github.copilot.default",
                "github-copilot"
            ],
        )
        .unwrap();
    }

    fn write_events(root: &Path, session_id: &str, lines: &[&str]) {
        let events_dir = root.join("session-state").join(session_id);
        fs::create_dir_all(&events_dir).unwrap();
        let mut file = File::create(events_dir.join("events.jsonl")).unwrap();
        for line in lines {
            writeln!(file, "{}", line).unwrap();
        }
    }

    #[test]
    fn parse_copilot_desktop_db_reads_token_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 100, 50, 25, 10);
        drop(conn);

        let messages = parse_copilot_desktop_db(&db_path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "copilot");
        assert_eq!(message.model_id, "gpt-5.1-codex");
        assert_eq!(message.provider_id, "openai");
        assert_eq!(message.session_id, "session-1");
        assert_eq!(message.timestamp, 1_782_909_296_000);
        assert_eq!(message.tokens.input, 100);
        assert_eq!(message.tokens.output, 50);
        assert_eq!(message.tokens.cache_read, 25);
        assert_eq!(message.tokens.cache_write, 0);
        assert_eq!(message.tokens.reasoning, 10);
        assert_eq!(
            message.dedup_key.as_deref(),
            Some("copilot-desktop:session-1")
        );
    }

    #[test]
    fn parse_copilot_desktop_db_skips_zero_token_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "gpt-5.1-codex", 0, 0, 0, 0);
        drop(conn);

        assert!(parse_copilot_desktop_db(&db_path).is_empty());
    }

    #[test]
    fn parse_copilot_desktop_db_enriches_model_and_workspace_from_events() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "auto", 100, 50, 0, 0);
        drop(conn);
        write_events(
            dir.path(),
            "session-1",
            &[
                r#"{"type":"session.start","data":{"context":{"cwd":"/Users/alice/project"}}}"#,
                r#"{"type":"session.model_change","data":{"newModel":"claude-sonnet-4-5"}}"#,
            ],
        );

        let messages = parse_copilot_desktop_db(&db_path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.model_id, "claude-sonnet-4-5");
        assert_eq!(message.provider_id, "anthropic");
        assert_eq!(message.workspace_label.as_deref(), Some("project"));
    }

    #[test]
    fn parse_copilot_desktop_db_uses_github_copilot_provider_for_auto() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("data.db");
        let conn = create_copilot_desktop_db(&db_path);
        insert_session(&conn, "session-1", "auto", 100, 0, 0, 0);
        drop(conn);

        let messages = parse_copilot_desktop_db(&db_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "github-copilot");
    }
}
