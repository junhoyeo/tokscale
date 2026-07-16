//! Devin session parser
//!
//! Parses local session data from:
//! - Devin CLI SQLite database (`~/.local/share/devin/cli/sessions.db`)
//! - Devin Desktop NDJSON event streams (`~/Library/Application Support/Devin/User/acp-events/*.ndjson`)

use super::utils::{back_anchor_timestamp, file_modified_timestamp_ms, open_readonly_sqlite};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use std::collections::HashMap;
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

/// Metadata from the authoritative Devin CLI session database that lets ACP
/// event files recover a stable session id and model. Desktop ACP file names
/// are independent UUIDs, so they cannot be compared directly with the CLI
/// database's session ids.
#[derive(Debug, Clone)]
struct DevinDesktopSession {
    session_id: String,
    model_id: Option<String>,
    workspace: Option<String>,
}

/// Title-to-session lookup for Desktop ACP streams. A title shared by more
/// than one database session is deliberately treated as ambiguous: using an
/// arbitrary match could suppress unrelated Desktop usage when CLI data is
/// also present.
#[derive(Debug, Default)]
pub struct DevinDesktopSessionLookup {
    by_title: HashMap<String, Option<DevinDesktopSession>>,
}

impl DevinDesktopSessionLookup {
    fn insert(&mut self, title: String, session: DevinDesktopSession) {
        match self.by_title.entry(title) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Some(session));
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if entry
                    .get()
                    .as_ref()
                    .is_some_and(|existing| existing.session_id != session.session_id)
                {
                    entry.insert(None);
                }
            }
        }
    }

    fn resolve(&self, title: &str) -> Option<&DevinDesktopSession> {
        self.by_title.get(title)?.as_ref()
    }
}

/// Load the CLI-session metadata needed to resolve Desktop ACP file titles.
///
/// Older or partial databases may not yet expose `sessions.title`; those
/// databases remain usable for CLI usage while Desktop streams fall back to
/// their file-based identity instead of failing the whole scan.
pub fn load_devin_desktop_session_lookup(
    db_paths: &[std::path::PathBuf],
) -> DevinDesktopSessionLookup {
    let mut lookup = DevinDesktopSessionLookup::default();

    for db_path in db_paths {
        let Some(conn) = open_readonly_sqlite(db_path) else {
            continue;
        };
        let mut stmt = match conn.prepare(
            "SELECT id, title, model, working_directory FROM sessions \
             WHERE title IS NOT NULL AND TRIM(title) != ''",
        ) {
            Ok(stmt) => stmt,
            Err(_) => continue,
        };
        let rows = match stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        }) {
            Ok(rows) => rows,
            Err(_) => continue,
        };

        for row in rows.flatten() {
            let (session_id, title, model_id, workspace) = row;
            let title = title.trim();
            if title.is_empty() {
                continue;
            }
            lookup.insert(
                title.to_string(),
                DevinDesktopSession {
                    session_id,
                    model_id: model_id.filter(|model| !model.is_empty()),
                    workspace,
                },
            );
        }
    }

    lookup
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
        ORDER BY m.row_id
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
        // Assistant rows without any attributable usage must not become
        // precedence markers for a matching Desktop ACP session. Otherwise a
        // zero-metric CLI row could suppress the only real usage record.
        if tokens.total() == 0 {
            continue;
        }

        let recorded_timestamp = created_at_ms.unwrap_or(fallback_timestamp);
        // `message_nodes.created_at` is stamped when the row is written, which
        // happens once the assistant message (including `metrics`) is
        // finalized, i.e. the turn's *end*, not its start. `total_time_ms` is
        // that turn's elapsed generation time, so sessionize()'s
        // `[timestamp, timestamp + duration_ms]` span would otherwise project
        // forward past the actual completion into phantom idle time.
        // Back-calculate the start anchor the same way #890 did for
        // Copilot's `endTime`-only records.
        let duration_ms = metrics
            .and_then(|m| m.total_time_ms)
            .map(|total_time_ms| total_time_ms.max(0));
        // Only back-calculate when `created_at_ms` is this row's own recorded
        // completion time: when it's absent, `recorded_timestamp` is
        // `fallback_timestamp` (the database file's mtime), not this
        // message's own end time, and subtracting `total_time_ms` from it
        // would shift the message into the wrong day rather than anchor it
        // correctly.
        let timestamp = match (created_at_ms, duration_ms.filter(|duration| *duration > 0)) {
            (Some(end), Some(duration)) => back_anchor_timestamp(end, duration),
            _ => recorded_timestamp,
        };
        let dedup_key = format!("devin-cli:{session_id}:{row_id}");
        let mut unified = UnifiedMessage::new_with_dedup(
            "devin-cli",
            model_id,
            provider,
            session_id,
            timestamp,
            tokens,
            0.0,
            Some(dedup_key),
        );

        unified.duration_ms = duration_ms;

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

#[derive(Debug, Default)]
struct DevinDesktopAcpUsage {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    model_id: Option<String>,
    timestamp: Option<i64>,
}

fn nonnegative_number(value: Option<&serde_json::Value>) -> Option<i64> {
    value
        .and_then(|value| value.as_i64())
        .map(|value| value.max(0))
}

fn notification_timestamp(notification: &serde_json::Value) -> Option<i64> {
    notification
        .pointer("/content/metadata/created_at")
        .or_else(|| notification.pointer("/metadata/created_at"))
        .or_else(|| notification.get("created_at"))
        .or_else(|| notification.get("timestamp"))
        .and_then(|value| value.as_str())
        .and_then(super::utils::parse_timestamp_str)
}

fn notification_model(notification: &serde_json::Value) -> Option<String> {
    notification
        .pointer("/content/metadata/generation_model")
        .or_else(|| notification.pointer("/metadata/generation_model"))
        .or_else(|| notification.pointer("/_meta/cognition.ai/model"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

struct DevinDesktopMessage<'a> {
    file_session_id: &'a str,
    title: Option<&'a str>,
    model_hint: Option<&'a str>,
    timestamp: i64,
    tokens: TokenBreakdown,
}

fn desktop_message(
    path: &Path,
    lookup: &DevinDesktopSessionLookup,
    message: DevinDesktopMessage<'_>,
    dedup_suffix: impl std::fmt::Display,
) -> UnifiedMessage {
    let resolved = message.title.and_then(|title| lookup.resolve(title));
    let session_id = resolved
        .map(|session| session.session_id.clone())
        .unwrap_or_else(|| message.file_session_id.to_string());
    let model_id = resolved
        .and_then(|session| session.model_id.as_deref())
        .filter(|model| !is_devin_routing_mode(model))
        .or(message.model_hint)
        .filter(|model| !model.is_empty() && !is_devin_routing_mode(model))
        .unwrap_or("devin")
        .to_string();
    let provider = provider_identity::inferred_provider_from_model(&model_id)
        .map(str::to_string)
        .unwrap_or_else(|| "devin".to_string());
    let source_key = path.to_string_lossy();
    let mut message = UnifiedMessage::new_with_dedup(
        "devin-desktop",
        model_id,
        provider,
        session_id,
        message.timestamp,
        message.tokens,
        0.0,
        Some(format!("devin-desktop:{source_key}:{dedup_suffix}")),
    );

    if let Some(workspace) = resolved.and_then(|session| session.workspace.as_deref()) {
        let workspace_key = normalize_workspace_key(workspace);
        let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
        message.set_workspace(workspace_key, workspace_label);
    }

    message
}

pub fn parse_devin_desktop_ndjson(path: &Path) -> Vec<UnifiedMessage> {
    parse_devin_desktop_ndjson_with_lookup(path, &DevinDesktopSessionLookup::default())
}

/// Parse a Devin Desktop ACP event stream.
///
/// Canonical ACP `usage_update` events contain cumulative input/cache counts
/// and per-step output counts. They are therefore reduced to one aggregate
/// message per file. The older embedded-metrics shape remains supported as a
/// best-effort fallback for historical captures.
pub fn parse_devin_desktop_ndjson_with_lookup(
    path: &Path,
    lookup: &DevinDesktopSessionLookup,
) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };

    let fallback_timestamp = file_modified_timestamp_ms(path);
    let file_session_id = session_id_from_ndjson_path(path);
    let mut legacy_messages = Vec::new();
    let mut acp_usage: Option<DevinDesktopAcpUsage> = None;
    let mut title: Option<String> = None;

    for (line_index, line) in BufReader::new(file).lines().enumerate() {
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

        if notification
            .get("sessionUpdate")
            .and_then(|value| value.as_str())
            == Some("session_info_update")
        {
            if let Some(updated_title) = notification
                .get("title")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_string)
            {
                title = Some(updated_title);
            }
            continue;
        }

        if notification
            .get("sessionUpdate")
            .and_then(|value| value.as_str())
            == Some("usage_update")
        {
            let meta = notification.get("_meta");
            let input =
                nonnegative_number(meta.and_then(|meta| meta.get("cognition.ai/inputTokens")));
            let cache_read =
                nonnegative_number(meta.and_then(|meta| meta.get("cognition.ai/cachedReadTokens")));
            let cache_write = nonnegative_number(
                meta.and_then(|meta| meta.get("cognition.ai/cachedWriteTokens")),
            );
            let output =
                nonnegative_number(meta.and_then(|meta| meta.get("cognition.ai/outputTokens")));

            // A few historical captures label the legacy embedded-metrics
            // shape as `usage_update` but do not contain ACP `_meta` fields.
            // Only claim the event for ACP aggregation when at least one
            // canonical token field is present; otherwise let the legacy
            // extraction below handle it.
            if input.is_some() || cache_read.is_some() || cache_write.is_some() || output.is_some()
            {
                let usage = acp_usage.get_or_insert_with(DevinDesktopAcpUsage::default);
                if let Some(input) = input {
                    usage.input = input;
                }
                if let Some(cache_read) = cache_read {
                    usage.cache_read = cache_read;
                }
                if let Some(cache_write) = cache_write {
                    usage.cache_write = cache_write;
                }
                if let Some(output) = output {
                    usage.output = usage.output.saturating_add(output);
                }
                if usage.model_id.is_none() {
                    usage.model_id = notification_model(&notification);
                }
                if let Some(timestamp) = notification_timestamp(&notification) {
                    usage.timestamp = Some(timestamp);
                }
                continue;
            }
        }

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

        let model_hint = notification_model(&notification);
        legacy_messages.push(desktop_message(
            path,
            lookup,
            DevinDesktopMessage {
                file_session_id: &file_session_id,
                title: title.as_deref(),
                model_hint: model_hint.as_deref(),
                timestamp: notification_timestamp(&notification).unwrap_or(fallback_timestamp),
                tokens: TokenBreakdown {
                    input,
                    output,
                    cache_read,
                    cache_write,
                    reasoning: 0,
                },
            },
            line_index,
        ));
    }

    if let Some(usage) = acp_usage {
        let tokens = TokenBreakdown {
            // ACP's inputTokens is the complete prompt, including the
            // cachedReadTokens subset. Tokscale stores uncached input and
            // cache reads separately, so subtract the overlap before totals
            // and pricing add both fields.
            input: usage.input.saturating_sub(usage.cache_read),
            output: usage.output,
            cache_read: usage.cache_read,
            cache_write: usage.cache_write,
            reasoning: 0,
        };
        if tokens.total() == 0 {
            return Vec::new();
        }
        return vec![desktop_message(
            path,
            lookup,
            DevinDesktopMessage {
                file_session_id: &file_session_id,
                title: title.as_deref(),
                model_hint: usage.model_id.as_deref(),
                timestamp: usage.timestamp.unwrap_or(fallback_timestamp),
                tokens,
            },
            "usage",
        )];
    }

    legacy_messages
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
                title TEXT,
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
            "INSERT INTO sessions (id, working_directory, backend_type, model, title, agent_mode, created_at, last_activity_at) VALUES (?1, ?2, 'windsurf', ?3, NULL, 'accept-edits', 1, 1)",
            rusqlite::params![id, working_directory, model],
        )
        .unwrap();
    }

    fn set_session_title(conn: &Connection, id: &str, title: &str) {
        conn.execute(
            "UPDATE sessions SET title = ?2 WHERE id = ?1",
            rusqlite::params![id, title],
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
        // `created_at` is the message row's write time (the turn's end), so
        // the message timestamp is back-calculated to the turn start:
        // created_at_ms - total_time_ms. See #890 (follow-up).
        assert_eq!(msg.timestamp, 1_700_000_000_000 - 2846);
        assert_eq!(msg.duration_ms, Some(2846));
        assert_eq!(msg.workspace_key.as_deref(), Some("/Users/alice/project"));
    }

    #[test]
    fn test_total_time_ms_timestamp_is_start_anchored() {
        // Regression (follow-up to #890): `message_nodes.created_at` is
        // stamped when the row is written, which happens once the assistant
        // message (including `metrics`) is finalized, i.e. the turn's *end*,
        // not its start. `total_time_ms` is that turn's elapsed generation
        // time, so sessionize()'s `[timestamp, timestamp + duration_ms]` span
        // would otherwise project forward past the actual completion into
        // phantom idle time. The parser must back-calculate the start anchor
        // instead.
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "claude-sonnet-4");
        let chat = r#"{"role":"assistant","content":"hello","metadata":{"generation_model":"claude-sonnet-4","metrics":{"input_tokens":100,"output_tokens":50,"total_time_ms":5000}}}"#;
        insert_message(&conn, "sess-1", chat, 1_700_000_010);
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(
            msg.timestamp,
            1_700_000_010_000 - 5000,
            "timestamp must be back-calculated to the turn start (end - duration)"
        );
        assert_eq!(
            msg.duration_ms,
            Some(5000),
            "duration_ms must still span from start to the recorded end timestamp"
        );
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
    fn test_parse_devin_cli_sqlite_skips_zero_usage() {
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
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_devin_cli_sqlite_skips_malformed_rows_without_losing_later_usage() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "gpt-5");
        insert_message(&conn, "sess-1", "{not valid json", 1_700_000_000);
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","metadata":{"metrics":{"input_tokens":10,"output_tokens":5}}}"#,
            1_700_000_001,
        );
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 5);
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
    fn test_parse_devin_desktop_usage_update_without_acp_fields_keeps_legacy_metrics() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("legacy-usage-update.ndjson");
        std::fs::write(
            &path,
            r#"{"notification":{"sessionUpdate":"usage_update","metadata":{"input_tokens":12,"output_tokens":3,"generation_model":"gpt-5"}}}
"#,
        )
        .unwrap();

        let messages = parse_devin_desktop_ndjson(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5");
        assert_eq!(messages[0].tokens.input, 12);
        assert_eq!(messages[0].tokens.output, 3);
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
    fn test_parse_devin_desktop_acp_usage_aggregates_and_resolves_cli_title() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        insert_session(&conn, "cli-session-1", "/Users/alice/project", "gpt-5");
        set_session_title(&conn, "cli-session-1", "Build the release");
        drop(conn);

        let path = dir.path().join("desktop-file-id.ndjson");
        std::fs::write(
            &path,
            concat!(
                r#"{"notification":{"sessionUpdate":"session_info_update","title":"Build the release"}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"session_info_update"}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"usage_update","_meta":{"cognition.ai/inputTokens":100,"cognition.ai/outputTokens":7,"cognition.ai/cachedReadTokens":20}}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"usage_update","_meta":{"cognition.ai/inputTokens":150,"cognition.ai/outputTokens":8,"cognition.ai/cachedReadTokens":30}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let lookup = load_devin_desktop_session_lookup(std::slice::from_ref(&db_path));
        let messages = parse_devin_desktop_ndjson_with_lookup(&path, &lookup);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "cli-session-1");
        assert_eq!(messages[0].model_id, "gpt-5");
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].tokens.output, 15);
        assert_eq!(messages[0].tokens.cache_read, 30);
        assert_eq!(messages[0].tokens.total(), 165);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/alice/project")
        );
    }

    #[test]
    fn test_parse_devin_desktop_does_not_resolve_an_ambiguous_title() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        insert_session(&conn, "cli-session-1", "/Users/alice/project-a", "gpt-5");
        insert_session(
            &conn,
            "cli-session-2",
            "/Users/alice/project-b",
            "claude-sonnet-4",
        );
        set_session_title(&conn, "cli-session-1", "Untitled task");
        set_session_title(&conn, "cli-session-2", "Untitled task");
        drop(conn);

        let path = dir.path().join("desktop-file-id.ndjson");
        std::fs::write(
            &path,
            concat!(
                r#"{"notification":{"sessionUpdate":"session_info_update","title":"Untitled task"}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"usage_update","_meta":{"cognition.ai/inputTokens":100,"cognition.ai/outputTokens":7}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let lookup = load_devin_desktop_session_lookup(std::slice::from_ref(&db_path));
        let messages = parse_devin_desktop_ndjson_with_lookup(&path, &lookup);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "desktop-file-id");
        assert_eq!(messages[0].model_id, "devin");
    }

    #[test]
    fn test_parse_devin_cli_sqlite_returns_empty_for_missing_db() {
        let messages = parse_devin_cli_sqlite(Path::new("/nonexistent/devin/sessions.db"));
        assert!(messages.is_empty());
    }
}
