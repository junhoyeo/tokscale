//! Devin session parser
//!
//! Parses local session data from:
//! - Devin CLI SQLite database (`~/.local/share/devin/cli/sessions.db`)
//! - Devin Desktop NDJSON event streams (`~/Library/Application Support/Devin/User/acp-events/*.ndjson`)

use super::utils::{file_modified_timestamp_ms, open_readonly_sqlite};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;

// ---------------------------------------------------------------------------
// Devin CLI (SQLite)
// ---------------------------------------------------------------------------

/// `sessions.model` can be set to `"adaptive"`, which is a Devin routing mode
/// rather than a real model id. Exclude it from the session-model fallback so
/// rows missing `generation_model` are skipped instead of reported under a
/// fictitious model.
fn is_devin_routing_mode(s: &str) -> bool {
    matches!(s, "adaptive")
}

#[derive(Debug, Deserialize)]
struct DevinChatMessage {
    role: String,
    #[serde(default)]
    metadata: Option<DevinNodeMetadata>,
}

#[derive(Debug, Deserialize, Default)]
struct DevinNodeMetadata {
    #[serde(default)]
    num_tokens: Option<i64>,
    #[serde(default)]
    metrics: Option<DevinMetrics>,
    #[serde(default)]
    generation_model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DevinMetrics {
    #[serde(default)]
    input_tokens: Option<i64>,
    #[serde(default)]
    output_tokens: Option<i64>,
    #[serde(default)]
    cache_read_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_tokens: Option<i64>,
    #[serde(default)]
    total_time_ms: Option<i64>,
}

pub fn parse_devin_cli_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let fallback_timestamp = file_modified_timestamp_ms(db_path);
    let Some(conn) = open_readonly_sqlite(db_path) else {
        return Vec::new();
    };

    // Token usage metrics live inside the `chat_message` JSON blob under
    // `$.metadata.metrics`, NOT in the separate `metadata` SQL column (which is
    // always NULL in real Devin CLI databases). The per-message model is
    // `$.metadata.generation_model`; `sessions.model` is only a fallback because
    // it can be "adaptive" (a routing mode, not a real model id).
    //
    // message_nodes.created_at is stored as Unix seconds; convert to ms.
    let query = r#"
        SELECT
            m.row_id,
            m.session_id,
            m.chat_message,
            m.created_at * 1000 AS created_at_ms,
            s.model,
            s.working_directory
        FROM message_nodes m
        JOIN sessions s ON m.session_id = s.id
        WHERE json_extract(m.chat_message, '$.role') = 'assistant'
    "#;

    let mut stmt = match conn.prepare(query) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut messages = Vec::new();

    for row_result in rows {
        let (row_id, session_id, chat_json, created_at_ms, session_model, workspace) =
            match row_result {
                Ok(r) => r,
                Err(_) => continue,
            };

        // Confirm role == assistant (the SQL filter should already guarantee this,
        // but parsing lets us skip corrupt rows cleanly).
        let chat_msg: DevinChatMessage = match serde_json::from_str(&chat_json) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if chat_msg.role != "assistant" {
            continue;
        }

        let metadata = chat_msg.metadata;
        let metrics = metadata.as_ref().and_then(|m| m.metrics.as_ref());

        // Prefer the per-message generation_model over sessions.model, which can
        // be "adaptive" (a routing mode) or empty — neither is a real model id.
        let model_id = metadata
            .as_ref()
            .and_then(|m| m.generation_model.as_deref())
            .filter(|s| !s.is_empty())
            .or(session_model.as_deref())
            .filter(|s| !s.is_empty() && !is_devin_routing_mode(s))
            .unwrap_or_default()
            .to_string();
        if model_id.is_empty() {
            continue;
        }

        let provider = provider_identity::inferred_provider_from_model(&model_id)
            .map(str::to_string)
            .unwrap_or_else(|| "devin".to_string());

        let tokens = match metrics {
            Some(m) => TokenBreakdown {
                input: m.input_tokens.unwrap_or(0).max(0),
                output: m.output_tokens.unwrap_or(0).max(0),
                cache_read: m.cache_read_tokens.unwrap_or(0).max(0),
                cache_write: m.cache_creation_tokens.unwrap_or(0).max(0),
                reasoning: 0,
            },
            None => TokenBreakdown::default(),
        };

        // Fallback: if metrics are missing but num_tokens is present, attribute
        // everything to output so the message is still counted.
        let tokens = if tokens.total() == 0 {
            if let Some(num_tokens) = metadata.as_ref().and_then(|m| m.num_tokens) {
                TokenBreakdown {
                    output: num_tokens.max(0),
                    ..TokenBreakdown::default()
                }
            } else {
                tokens
            }
        } else {
            tokens
        };

        let timestamp = created_at_ms.unwrap_or(fallback_timestamp);
        let mut unified = UnifiedMessage::new_with_dedup(
            "devin-cli",
            model_id,
            provider,
            session_id,
            timestamp,
            tokens,
            0.0,
            Some(format!("devin-cli:{row_id}")),
        );

        if let Some(total_time_ms) = metrics.and_then(|m| m.total_time_ms) {
            unified.duration_ms = Some(total_time_ms.max(0));
        }

        if let Some(ws) = workspace {
            let workspace_key = normalize_workspace_key(&ws);
            let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
            unified.set_workspace(workspace_key, workspace_label);
        }

        messages.push(unified);
    }

    messages
}

// ---------------------------------------------------------------------------
// Devin Desktop (NDJSON)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DevinDesktopEvent {
    #[serde(default)]
    notification: Option<serde_json::Value>,
}

pub fn parse_devin_desktop_ndjson(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };

    let fallback_timestamp = file_modified_timestamp_ms(path);
    let session_id = session_id_from_ndjson_path(path);
    let mut messages = Vec::new();
    let mut seen = HashSet::new();
    let mut line_index: usize = 0;

    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { continue };
        if line.is_empty() {
            continue;
        }

        let Ok(event) = serde_json::from_str::<DevinDesktopEvent>(&line) else {
            continue;
        };

        // The Desktop app streams ACP events. Usage is not reliably present in
        // the NDJSON itself; the authoritative usage lives in the CLI SQLite DB.
        // We extract any embedded usage blocks we can find, but most files will
        // yield no messages. This keeps the parser future-proof and avoids
        // double-counting the CLI DB data.
        let Some(notification) = event.notification else {
            continue;
        };

        // Look for usage metrics nested inside the notification. Devin Desktop
        // stores them either under a `metrics` object or directly on `metadata`.
        let usage = notification
            .pointer("/content/metadata/metrics")
            .or_else(|| notification.pointer("/metadata/metrics"))
            .or_else(|| notification.pointer("/metrics"))
            .or_else(|| notification.pointer("/content/metadata"))
            .or_else(|| notification.pointer("/metadata"));

        let Some(usage) = usage else {
            continue;
        };

        let input = usage
            .get("input_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
        let output = usage
            .get("output_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
        let cache_read = usage
            .get("cache_read_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
        let cache_write = usage
            .get("cache_creation_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);

        if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 {
            continue;
        }

        let model_id = notification
            .pointer("/content/metadata/generation_model")
            .or_else(|| notification.pointer("/metadata/generation_model"))
            .and_then(|v| v.as_str())
            .unwrap_or("devin")
            .to_string();

        let provider = provider_identity::inferred_provider_from_model(&model_id)
            .map(str::to_string)
            .unwrap_or_else(|| "devin".to_string());

        let timestamp = notification
            .pointer("/content/metadata/created_at")
            .or_else(|| notification.pointer("/metadata/created_at"))
            .and_then(|v| v.as_str())
            .and_then(super::utils::parse_timestamp_str)
            .unwrap_or(fallback_timestamp);

        // Dedup by file-position line index rather than timestamp+tokens.
        // `created_at` is commonly absent, so all events in a file would share
        // the file-mtime fallback and collide on identical model+token counts.
        // Anchoring to the line position matches the qwen.rs pattern for
        // NDJSON sources without stable per-event identifiers.
        let dedup_key = format!("devin-desktop:{session_id}:{line_index}");
        if !seen.insert(dedup_key.clone()) {
            continue;
        }

        let message = UnifiedMessage::new_with_dedup(
            "devin-desktop",
            model_id,
            provider,
            session_id.clone(),
            timestamp,
            TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                reasoning: 0,
            },
            0.0,
            Some(dedup_key),
        );

        messages.push(message);
        line_index += 1;
    }

    messages
}

fn session_id_from_ndjson_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_devin_cli_db(dir: &TempDir) -> std::path::PathBuf {
        let db_path = dir.path().join("sessions.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                working_directory TEXT NOT NULL,
                backend_type TEXT NOT NULL,
                model TEXT NOT NULL,
                agent_mode TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_activity_at INTEGER NOT NULL
            );
            CREATE TABLE message_nodes (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                node_id INTEGER NOT NULL,
                parent_node_id INTEGER,
                chat_message TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                metadata TEXT
            );
            "#,
        )
        .unwrap();
        db_path
    }

    fn insert_session(conn: &Connection, id: &str, working_directory: &str, model: &str) {
        conn.execute(
            "INSERT INTO sessions (id, working_directory, backend_type, model, agent_mode, created_at, last_activity_at) VALUES (?1, ?2, 'windsurf', ?3, 'accept-edits', 1, 1)",
            rusqlite::params![id, working_directory, model],
        )
        .unwrap();
    }

    /// Insert a message_nodes row. In real Devin CLI databases the SQL
    /// `metadata` column is always NULL; token metrics and generation_model
    /// live inside the `chat_message` JSON blob under `$.metadata`.
    fn insert_message(
        conn: &Connection,
        session_id: &str,
        chat_message: &str,
        created_at: i64,
    ) -> i64 {
        conn.execute(
        "INSERT INTO message_nodes (session_id, node_id, chat_message, metadata, created_at) VALUES (?1, 1, ?2, NULL, ?3)",
        rusqlite::params![session_id, chat_message, created_at],
    )
    .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn test_parse_devin_cli_sqlite_reads_assistant_metrics() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        // sessions.model is "adaptive" (a routing mode), but the real model
        // is in chat_message.metadata.generation_model.
        insert_session(&conn, "sess-1", "/Users/alice/project", "adaptive");
        let chat = r#"{"role":"assistant","content":"hello","metadata":{"num_tokens":147,"generation_model":"glm-5-2-max-1m","metrics":{"input_tokens":31134,"output_tokens":147,"cache_read_tokens":8,"cache_creation_tokens":null,"total_time_ms":2846}}}"#;
        insert_message(&conn, "sess-1", chat, 1_700_000_000);
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(msg.client, "devin-cli");
        assert_eq!(msg.session_id, "sess-1");
        assert_eq!(msg.model_id, "glm-5-2-max-1m");
        assert_eq!(msg.provider_id, "devin");
        assert_eq!(msg.tokens.input, 31134);
        assert_eq!(msg.tokens.output, 147);
        assert_eq!(msg.tokens.cache_read, 8);
        assert_eq!(msg.tokens.cache_write, 0);
        assert_eq!(msg.timestamp, 1_700_000_000_000);
        assert_eq!(msg.duration_ms, Some(2846));
        assert_eq!(msg.workspace_key.as_deref(), Some("/Users/alice/project"));
    }

    #[test]
    fn test_parse_devin_cli_sqlite_skips_non_assistant_and_missing_model() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "glm-5-2-max-1m");
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"user","content":"hi","metadata":{"metrics":{"input_tokens":1}}}"#,
            1_700_000_000,
        );
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","content":"ok","metadata":{"generation_model":"glm-5-2","metrics":{"input_tokens":10,"output_tokens":5}}}"#,
            1_700_000_001,
        );
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 5);
    }

    #[test]
    fn test_parse_devin_cli_sqlite_falls_back_to_session_model() {
        // When generation_model is absent, fall back to sessions.model.
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "kimi-k2-7");
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","content":"ok","metadata":{"metrics":{"input_tokens":10,"output_tokens":5}}}"#,
            1_700_000_000,
        );
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "kimi-k2-7");
    }

    #[test]
    fn test_parse_devin_cli_sqlite_skips_adaptive_session_model() {
        // When generation_model is absent and sessions.model is "adaptive"
        // (a routing mode), the row should be skipped rather than reported
        // under a fictitious "adaptive" model.
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "adaptive");
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","metadata":{"metrics":{"input_tokens":10,"output_tokens":5}}}"#,
            1_700_000_000,
        );
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_devin_cli_sqlite_clamps_negative_values() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "glm-5-2-max-1m");
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","metadata":{"generation_model":"glm-5-2","metrics":{"input_tokens":-100,"output_tokens":-50,"cache_read_tokens":-10,"cache_creation_tokens":-5,"total_time_ms":-1}}}"#,
            1_700_000_000,
        );
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 0);
        assert_eq!(messages[0].tokens.output, 0);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.cache_write, 0);
        assert_eq!(messages[0].duration_ms, Some(0));
    }

    #[test]
    fn test_parse_devin_desktop_ndjson_extracts_usage() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("event.ndjson");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"providerId":"devin-cli","notification":{{"content":{{"text":"hello"}},"metadata":{{"input_tokens":100,"output_tokens":50,"generation_model":"claude-sonnet-4","created_at":"2026-06-16T12:00:00Z"}}}}}}"#
        ).unwrap();
        writeln!(
            file,
            r#"{{"providerId":"devin-cli","notification":{{"content":{{"text":"hi"}},"metadata":{{"input_tokens":0,"output_tokens":0}}}}}}"#
        ).unwrap();
        drop(file);

        let messages = parse_devin_desktop_ndjson(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "devin-desktop");
        assert_eq!(messages[0].model_id, "claude-sonnet-4");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].timestamp, 1_781_611_200_000);
    }

    #[test]
    fn test_parse_devin_desktop_ndjson_keeps_distinct_events_with_identical_usage() {
        // Two events with identical model/tokens/timestamp at different line
        // positions must both survive — they represent distinct API calls.
        // The line-index dedup key prevents collision without undercounting.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("event.ndjson");
        std::fs::write(
            &path,
            r#"{"providerId":"devin-cli","notification":{"metadata":{"input_tokens":10,"output_tokens":5,"generation_model":"gpt-5","created_at":"2026-06-16T12:00:00Z"}}}
{"providerId":"devin-cli","notification":{"metadata":{"input_tokens":10,"output_tokens":5,"generation_model":"gpt-5","created_at":"2026-06-16T12:00:00Z"}}}
"#,
        )
        .unwrap();

        let messages = parse_devin_desktop_ndjson(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[1].tokens.input, 10);
    }

    #[test]
    fn test_parse_devin_cli_sqlite_returns_empty_for_missing_db() {
        let messages = parse_devin_cli_sqlite(Path::new("/nonexistent/devin/sessions.db"));
        assert!(messages.is_empty());
    }
}
