//! Kilo CLI session parser
//!
//! Parses messages from:
//! - SQLite database: ~/.local/share/kilo/kilo.db
//!
//! Kilo CLI uses a SQLite database similar to OpenCode.

use super::utils::{file_modified_timestamp_ms, open_readonly_sqlite};
use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct KiloMessage {
    #[serde(default)]
    pub id: Option<String>,
    pub session_id: Option<String>,
    pub role: String,
    #[serde(rename = "modelID", default)]
    pub model_id: Option<String>,
    #[serde(rename = "providerID", default)]
    pub provider_id: Option<String>,
    pub cost: Option<f64>,
    pub tokens: Option<KiloTokens>,
    pub time: Option<KiloTime>,
    pub agent: Option<String>,
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct KiloTokens {
    pub input: i64,
    pub output: i64,
    #[serde(default)]
    pub reasoning: Option<i64>,
    pub cache: KiloCache,
}

#[derive(Debug, Deserialize)]
pub struct KiloCache {
    pub read: i64,
    pub write: i64,
}

#[derive(Debug, Deserialize)]
pub struct KiloTime {
    pub created: f64,
    pub completed: Option<f64>,
}

pub fn parse_kilo_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let fallback_timestamp = file_modified_timestamp_ms(db_path);
    parse_kilo_sqlite_with_fallback(db_path, fallback_timestamp)
}

pub fn parse_kilo_sqlite_with_fallback(
    db_path: &Path,
    fallback_timestamp: i64,
) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite(db_path) else {
        return Vec::new();
    };

    let query = r#"
        SELECT m.id, m.session_id, m.data
        FROM message m
        WHERE json_valid(m.data)
          AND json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
    "#;

    let mut stmt = match conn.prepare(query) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = match stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let session_id: String = row.get(1)?;
        let data_json: String = row.get(2)?;
        Ok((id, session_id, data_json))
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut messages = Vec::new();

    for row_result in rows {
        let (row_id, row_session_id, data_json) = match row_result {
            Ok(r) => r,
            Err(_) => continue,
        };

        let mut bytes = data_json.into_bytes();
        let msg: KiloMessage = match simd_json::from_slice(&mut bytes) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if msg.role != "assistant" {
            continue;
        }

        let tokens = match msg.tokens {
            Some(t) => t,
            None => continue,
        };

        let dedup_key = msg.id.or(Some(row_id));

        let model_id = match msg.model_id {
            Some(m) => m,
            None => continue,
        };

        let agent = msg.agent.or(msg.mode);
        let session_id = msg.session_id.unwrap_or(row_session_id);
        let timestamp = msg
            .time
            .map(|t| t.created as i64)
            .unwrap_or(fallback_timestamp);

        let provider = msg
            .provider_id
            .as_deref()
            .or_else(|| provider_identity::inferred_provider_from_model(&model_id))
            .unwrap_or("kilo")
            .to_string();

        let mut unified = UnifiedMessage::new_with_agent(
            "kilo",
            model_id,
            provider,
            session_id,
            timestamp,
            TokenBreakdown {
                input: tokens.input.max(0),
                output: tokens.output.max(0),
                cache_read: tokens.cache.read.max(0),
                cache_write: tokens.cache.write.max(0),
                reasoning: tokens.reasoning.unwrap_or(0).max(0),
            },
            msg.cost.unwrap_or(0.0).max(0.0),
            agent,
        );
        unified.dedup_key = dedup_key;

        messages.push(unified);
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use tempfile::TempDir;

    fn create_kilo_sqlite_db(dir: &TempDir) -> std::path::PathBuf {
        let db_path = dir.path().join("kilo.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                data TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
        db_path
    }

    fn insert_kilo_message(conn: &Connection, row_id: &str, session_id: &str, data_json: &str) {
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            params![row_id, session_id, data_json],
        )
        .unwrap();
    }

    #[test]
    fn test_parse_kilo_message_structure() {
        let json = r#"{
            "id": "msg-123",
            "session_id": "sess-456",
            "role": "assistant",
            "modelID": "minimax/m2.5",
            "providerID": "kilo",
            "cost": 0.15,
            "tokens": {
                "input": 1000,
                "output": 200,
                "cache": {"read": 500, "write": 100}
            },
            "time": {"created": 1700000000000}
        }"#;

        let mut bytes = json.as_bytes().to_vec();
        let msg: KiloMessage = simd_json::from_slice(&mut bytes).unwrap();
        assert_eq!(msg.role, "assistant");
        assert_eq!(msg.cost, Some(0.15));
        assert_eq!(msg.model_id, Some("minimax/m2.5".to_string()));
    }

    #[test]
    fn test_parse_kilo_sqlite_reads_assistant_rows() {
        let dir = TempDir::new().unwrap();
        let db_path = create_kilo_sqlite_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        let data_json = r#"{
            "id": "embedded-msg-1",
            "session_id": "sess-1",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.42,
            "agent": "architect",
            "tokens": {
                "input": 1200,
                "output": 300,
                "reasoning": 40,
                "cache": {"read": 75, "write": 25}
            },
            "time": {"created": 1700000000123.0}
        }"#;
        insert_kilo_message(&conn, "row-msg-1", "sess-1", data_json);
        drop(conn);

        let messages = parse_kilo_sqlite_with_fallback(&db_path, 42);
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(msg.client, "kilo");
        assert_eq!(msg.session_id, "sess-1");
        assert_eq!(msg.model_id, "claude-sonnet-4");
        assert_eq!(msg.provider_id, "anthropic");
        assert_eq!(msg.timestamp, 1_700_000_000_123);
        assert_eq!(msg.tokens.input, 1200);
        assert_eq!(msg.tokens.output, 300);
        assert_eq!(msg.tokens.reasoning, 40);
        assert_eq!(msg.tokens.cache_read, 75);
        assert_eq!(msg.tokens.cache_write, 25);
        assert_eq!(msg.cost, 0.42);
        assert_eq!(msg.agent.as_deref(), Some("architect"));
        assert_eq!(msg.dedup_key.as_deref(), Some("embedded-msg-1"));
    }

    #[test]
    fn test_parse_kilo_sqlite_skips_invalid_rows_and_clamps_values() {
        let dir = TempDir::new().unwrap();
        let db_path = create_kilo_sqlite_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_kilo_message(
            &conn,
            "row-user",
            "sess-user",
            r#"{
                "session_id": "sess-user",
                "role": "user",
                "modelID": "gpt-5.4",
                "tokens": {"input": 1, "output": 1, "cache": {"read": 0, "write": 0}}
            }"#,
        );
        insert_kilo_message(
            &conn,
            "row-no-tokens",
            "sess-no-tokens",
            r#"{
                "session_id": "sess-no-tokens",
                "role": "assistant",
                "modelID": "gpt-5.4"
            }"#,
        );
        insert_kilo_message(
            &conn,
            "row-no-model",
            "sess-no-model",
            r#"{
                "session_id": "sess-no-model",
                "role": "assistant",
                "tokens": {"input": 1, "output": 1, "cache": {"read": 0, "write": 0}}
            }"#,
        );
        insert_kilo_message(&conn, "row-invalid-json", "sess-invalid", "{not-json");
        insert_kilo_message(
            &conn,
            "row-valid",
            "sess-valid",
            r#"{
                "role": "assistant",
                "modelID": "gpt-5.4",
                "cost": -0.75,
                "mode": "debug",
                "tokens": {
                    "input": -100,
                    "output": -50,
                    "reasoning": -5,
                    "cache": {"read": -20, "write": -10}
                }
            }"#,
        );
        drop(conn);

        let messages = parse_kilo_sqlite_with_fallback(&db_path, 1_800_000_000_000);
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(msg.session_id, "sess-valid");
        assert_eq!(msg.model_id, "gpt-5.4");
        assert_eq!(msg.provider_id, "openai");
        assert_eq!(msg.timestamp, 1_800_000_000_000);
        assert_eq!(msg.tokens.input, 0);
        assert_eq!(msg.tokens.output, 0);
        assert_eq!(msg.tokens.reasoning, 0);
        assert_eq!(msg.tokens.cache_read, 0);
        assert_eq!(msg.tokens.cache_write, 0);
        assert_eq!(msg.cost, 0.0);
        assert_eq!(msg.agent.as_deref(), Some("debug"));
        assert_eq!(msg.dedup_key.as_deref(), Some("row-valid"));
    }

    #[test]
    fn test_parse_kilo_sqlite_returns_empty_for_missing_db() {
        let messages = parse_kilo_sqlite(std::path::Path::new("/nonexistent/kilo.db"));
        assert!(messages.is_empty());
    }
}
