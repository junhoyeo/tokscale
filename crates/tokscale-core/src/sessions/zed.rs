//! Zed Agent session parser
//!
//! Parses hosted Zed Agent thread rows from Zed's SQLite database:
//! - Linux/FreeBSD: `$XDG_DATA_HOME/zed/threads/threads.db`
//! - macOS: `~/Library/Application Support/Zed/threads/threads.db`
//! - Windows: `%LOCALAPPDATA%\Zed\threads\threads.db`
//!
//! Only Zed-hosted model rows (`provider == "zed.dev"`) are counted. External
//! ACP agents are billed and logged by their own providers/CLIs, and counting
//! their Zed UI rows would duplicate those sources.

use super::utils::parse_timestamp_str;
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use tracing::warn;

pub(crate) const ZED_HOSTED_PROVIDER: &str = "zed.dev";
const MAX_ZED_THREAD_JSON_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug)]
struct ZedThreadRow {
    id: String,
    updated_at: String,
    created_at: Option<String>,
    folder_paths: Option<String>,
    folder_paths_order: Option<String>,
    data_type: String,
    data: Vec<u8>,
}

pub fn parse_zed_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let conn = match Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => conn,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open Zed threads database"
            );
            return Vec::new();
        }
    };

    let query = build_threads_query(&conn);
    let mut stmt = match conn.prepare(&query) {
        Ok(stmt) => stmt,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to prepare Zed thread query"
            );
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok(ZedThreadRow {
            id: row.get(0)?,
            updated_at: row.get(1)?,
            created_at: row.get(2)?,
            folder_paths: row.get(3)?,
            folder_paths_order: row.get(4)?,
            data_type: row.get(5)?,
            data: row.get(6)?,
        })
    }) {
        Ok(rows) => rows,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to execute Zed thread query"
            );
            return Vec::new();
        }
    };

    rows.filter_map(|row| match row {
        Ok(row) => parse_thread_row(db_path, row),
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to decode Zed thread row"
            );
            None
        }
    })
    .collect()
}

fn build_threads_query(conn: &Connection) -> String {
    let columns = thread_columns(conn);
    let created_at = optional_column(&columns, "created_at");
    let folder_paths = optional_column(&columns, "folder_paths");
    let folder_paths_order = optional_column(&columns, "folder_paths_order");

    format!(
        "SELECT id, updated_at, {created_at}, {folder_paths}, {folder_paths_order}, data_type, data FROM threads"
    )
}

fn optional_column(columns: &HashSet<String>, column: &'static str) -> &'static str {
    if columns.contains(column) {
        column
    } else {
        "NULL"
    }
}

fn thread_columns(conn: &Connection) -> HashSet<String> {
    let mut stmt = match conn.prepare("PRAGMA table_info(threads)") {
        Ok(stmt) => stmt,
        Err(_) => return HashSet::new(),
    };

    let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
        Ok(rows) => rows,
        Err(_) => return HashSet::new(),
    };

    rows.filter_map(Result::ok).collect()
}

fn parse_thread_row(db_path: &Path, row: ZedThreadRow) -> Option<UnifiedMessage> {
    let json = match decode_thread_json(&row.data_type, &row.data) {
        Ok(json) => json,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                thread_id = %row.id,
                error = %err,
                "Failed to decode Zed thread payload"
            );
            return None;
        }
    };

    let thread: Value = match serde_json::from_slice(&json) {
        Ok(thread) => thread,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                thread_id = %row.id,
                error = %err,
                "Failed to parse Zed thread JSON"
            );
            return None;
        }
    };

    if thread
        .get("imported")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }

    let model = thread.get("model")?;
    let provider = model.get("provider")?.as_str()?.trim();
    if !provider.eq_ignore_ascii_case(ZED_HOSTED_PROVIDER) {
        return None;
    }

    let model_id = model.get("model")?.as_str()?.trim();
    if model_id.is_empty() {
        return None;
    }

    let (tokens, message_count) = thread_usage(&thread)?;
    let timestamp = timestamp_ms(&row, &thread)?;

    let mut message = UnifiedMessage::new(
        "zed",
        model_id,
        ZED_HOSTED_PROVIDER,
        row.id.clone(),
        timestamp,
        tokens,
        0.0,
    );
    message.message_count = message_count;
    message.dedup_key = Some(format!("zed:{}", row.id));

    if let Some(workspace_key) = workspace_key_from_folders(
        row.folder_paths.as_deref(),
        row.folder_paths_order.as_deref(),
    ) {
        let workspace_label = workspace_label_from_key(&workspace_key);
        message.set_workspace(Some(workspace_key), workspace_label);
    }

    Some(message)
}

fn decode_thread_json(data_type: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    match data_type.trim().to_ascii_lowercase().as_str() {
        "json" => {
            if data.len() as u64 > MAX_ZED_THREAD_JSON_BYTES {
                return Err(format!(
                    "decoded thread payload exceeds {} bytes",
                    MAX_ZED_THREAD_JSON_BYTES
                ));
            }
            Ok(data.to_vec())
        }
        "zstd" => {
            let decoder = zstd::Decoder::new(data).map_err(|err| err.to_string())?;
            let mut decoded = Vec::new();
            decoder
                .take(MAX_ZED_THREAD_JSON_BYTES + 1)
                .read_to_end(&mut decoded)
                .map_err(|err| err.to_string())?;
            if decoded.len() as u64 > MAX_ZED_THREAD_JSON_BYTES {
                return Err(format!(
                    "decoded thread payload exceeds {} bytes",
                    MAX_ZED_THREAD_JSON_BYTES
                ));
            }
            Ok(decoded)
        }
        other => Err(format!("unsupported data_type {other:?}")),
    }
}

fn thread_usage(thread: &Value) -> Option<(TokenBreakdown, i32)> {
    let (request_usage, request_count) = sum_request_token_usage(thread.get("request_token_usage"));
    if request_usage.total() > 0 {
        return Some((request_usage, request_count.max(1)));
    }

    let cumulative = token_usage_from_value(thread.get("cumulative_token_usage")?)?;
    if cumulative.total() > 0 {
        Some((cumulative, 1))
    } else {
        None
    }
}

fn sum_request_token_usage(value: Option<&Value>) -> (TokenBreakdown, i32) {
    let mut total = TokenBreakdown::default();
    let mut count = 0_i32;

    let Some(value) = value else {
        return (total, count);
    };

    let usages: Box<dyn Iterator<Item = &Value> + '_> = match value {
        Value::Object(map) => Box::new(map.values()),
        Value::Array(values) => Box::new(values.iter()),
        _ => return (total, count),
    };

    for usage_value in usages {
        let Some(usage) = token_usage_from_value(usage_value) else {
            continue;
        };
        if usage.total() <= 0 {
            continue;
        }
        total.input = total.input.saturating_add(usage.input);
        total.output = total.output.saturating_add(usage.output);
        total.cache_read = total.cache_read.saturating_add(usage.cache_read);
        total.cache_write = total.cache_write.saturating_add(usage.cache_write);
        total.reasoning = total.reasoning.saturating_add(usage.reasoning);
        count = count.saturating_add(1);
    }

    (total, count)
}

// Zed persists `language_model::TokenUsage`, which currently stores only
// input/output/cache fields in `threads.db`. Until upstream adds a dedicated
// reasoning token field there, `reasoning` stays zero in Tokscale.
fn token_usage_from_value(value: &Value) -> Option<TokenBreakdown> {
    Some(TokenBreakdown {
        input: usage_field(value, "input_tokens"),
        output: usage_field(value, "output_tokens"),
        cache_read: usage_field(value, "cache_read_input_tokens"),
        cache_write: usage_field(value, "cache_creation_input_tokens"),
        reasoning: 0,
    })
}

fn usage_field(value: &Value, field: &str) -> i64 {
    let Some(value) = value.get(field) else {
        return 0;
    };

    let parsed = value
        .as_i64()
        .or_else(|| value.as_u64().map(|n| i64::try_from(n).unwrap_or(i64::MAX)))
        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
        .unwrap_or(0);

    parsed.max(0)
}

fn timestamp_ms(row: &ZedThreadRow, thread: &Value) -> Option<i64> {
    row.created_at
        .as_deref()
        .and_then(parse_timestamp_str)
        .or_else(|| parse_timestamp_str(&row.updated_at))
        .or_else(|| {
            thread
                .get("updated_at")
                .and_then(Value::as_str)
                .and_then(parse_timestamp_str)
        })
}

fn workspace_key_from_folders(paths: Option<&str>, order: Option<&str>) -> Option<String> {
    let paths: Vec<&str> = paths?
        .lines()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .collect();
    if paths.is_empty() {
        return None;
    }

    let selected = order
        .and_then(|order| first_ordered_path_index(order, paths.len()))
        .and_then(|index| paths.get(index).copied())
        .unwrap_or(paths[0]);

    normalize_workspace_key(selected)
}

fn first_ordered_path_index(order: &str, path_count: usize) -> Option<usize> {
    order
        .split(',')
        .map(str::trim)
        .enumerate()
        .filter_map(|(index, order)| {
            let order = order.parse::<usize>().ok()?;
            (index < path_count).then_some((index, order))
        })
        .min_by_key(|(_, order)| *order)
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn create_threads_db(dir: &TempDir) -> (std::path::PathBuf, Connection) {
        let db_path = dir.path().join("threads.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                summary TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                data_type TEXT NOT NULL,
                data BLOB NOT NULL,
                parent_id TEXT,
                folder_paths TEXT,
                folder_paths_order TEXT,
                created_at TEXT
            );
            "#,
        )
        .unwrap();
        (db_path, conn)
    }

    fn thread_json(provider: &str, model: &str, request_token_usage: Value) -> String {
        json!({
            "version": "0.3.0",
            "title": "Test thread",
            "messages": [],
            "updated_at": "2026-05-01T12:30:00Z",
            "request_token_usage": request_token_usage,
            "cumulative_token_usage": {
                "input_tokens": 999,
                "output_tokens": 999
            },
            "model": {
                "provider": provider,
                "model": model
            },
            "imported": false
        })
        .to_string()
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_thread(
        conn: &Connection,
        id: &str,
        json: &str,
        data_type: &str,
        updated_at: &str,
        created_at: Option<&str>,
        folder_paths: Option<&str>,
        folder_paths_order: Option<&str>,
    ) {
        let data = match data_type {
            "zstd" => zstd::encode_all(json.as_bytes(), 3).unwrap(),
            "json" => json.as_bytes().to_vec(),
            _ => panic!("unsupported test data_type"),
        };

        conn.execute(
            r#"
            INSERT INTO threads (
                id, summary, updated_at, data_type, data, created_at, folder_paths, folder_paths_order
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                id,
                "Test thread",
                updated_at,
                data_type,
                data,
                created_at,
                folder_paths,
                folder_paths_order
            ],
        )
        .unwrap();
    }

    #[test]
    fn parse_zed_sqlite_reads_zstd_hosted_thread_usage() {
        let dir = TempDir::new().unwrap();
        let (db_path, conn) = create_threads_db(&dir);
        let payload = thread_json(
            ZED_HOSTED_PROVIDER,
            "claude-sonnet-4-5",
            json!({
                "user-1": {
                    "input_tokens": 100,
                    "output_tokens": 20,
                    "cache_creation_input_tokens": 5,
                    "cache_read_input_tokens": 10
                },
                "user-2": {
                    "input_tokens": 50,
                    "output_tokens": 7
                }
            }),
        );
        insert_thread(
            &conn,
            "thread-1",
            &payload,
            "zstd",
            "2026-05-01T12:30:00Z",
            Some("2026-05-01T12:00:00Z"),
            Some("/workspace/a\n/workspace/b"),
            Some("1,0"),
        );

        let messages = parse_zed_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "zed");
        assert_eq!(message.provider_id, ZED_HOSTED_PROVIDER);
        assert_eq!(message.model_id, "claude-sonnet-4-5");
        assert_eq!(message.session_id, "thread-1");
        assert_eq!(
            message.timestamp,
            parse_timestamp_str("2026-05-01T12:00:00Z").unwrap()
        );
        assert_eq!(message.tokens.input, 150);
        assert_eq!(message.tokens.output, 27);
        assert_eq!(message.tokens.cache_write, 5);
        assert_eq!(message.tokens.cache_read, 10);
        assert_eq!(message.message_count, 2);
        assert_eq!(message.workspace_key.as_deref(), Some("/workspace/b"));
        assert_eq!(message.workspace_label.as_deref(), Some("b"));
        assert_eq!(message.dedup_key.as_deref(), Some("zed:thread-1"));
    }

    #[test]
    fn parse_zed_sqlite_skips_non_hosted_threads() {
        let dir = TempDir::new().unwrap();
        let (db_path, conn) = create_threads_db(&dir);
        let payload = thread_json(
            "anthropic",
            "claude-sonnet-4-5",
            json!({
                "user-1": {
                    "input_tokens": 100,
                    "output_tokens": 20
                }
            }),
        );
        insert_thread(
            &conn,
            "thread-1",
            &payload,
            "zstd",
            "2026-05-01T12:30:00Z",
            None,
            None,
            None,
        );

        assert!(parse_zed_sqlite(&db_path).is_empty());
    }

    #[test]
    fn parse_zed_sqlite_uses_cumulative_usage_when_request_usage_is_absent() {
        let dir = TempDir::new().unwrap();
        let (db_path, conn) = create_threads_db(&dir);
        let payload = json!({
            "version": "0.3.0",
            "title": "Test thread",
            "messages": [],
            "updated_at": "2026-05-01T12:30:00Z",
            "request_token_usage": {},
            "cumulative_token_usage": {
                "input_tokens": 12,
                "output_tokens": 3,
                "cache_creation_input_tokens": 2,
                "cache_read_input_tokens": 4
            },
            "model": {
                "provider": ZED_HOSTED_PROVIDER,
                "model": "gpt-5.2"
            },
            "imported": false
        })
        .to_string();
        insert_thread(
            &conn,
            "thread-1",
            &payload,
            "json",
            "2026-05-01T12:30:00Z",
            None,
            None,
            None,
        );

        let messages = parse_zed_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 12);
        assert_eq!(messages[0].tokens.output, 3);
        assert_eq!(messages[0].tokens.cache_write, 2);
        assert_eq!(messages[0].tokens.cache_read, 4);
        assert_eq!(messages[0].message_count, 1);
    }

    #[test]
    fn parse_zed_sqlite_supports_pre_created_at_schema() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("threads.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                summary TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                data_type TEXT NOT NULL,
                data BLOB NOT NULL
            );
            "#,
        )
        .unwrap();
        let payload = thread_json(
            ZED_HOSTED_PROVIDER,
            "gpt-5.2",
            json!({
                "user-1": {
                    "input_tokens": 12,
                    "output_tokens": 3
                }
            }),
        );
        let data = zstd::encode_all(payload.as_bytes(), 3).unwrap();
        conn.execute(
            "INSERT INTO threads (id, summary, updated_at, data_type, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["thread-1", "Test thread", "2026-05-01T12:30:00Z", "zstd", data],
        )
        .unwrap();

        let messages = parse_zed_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].timestamp,
            parse_timestamp_str("2026-05-01T12:30:00Z").unwrap()
        );
    }

    #[test]
    fn workspace_key_from_folders_uses_original_order_when_available() {
        assert_eq!(
            workspace_key_from_folders(Some("/sorted/a\n/sorted/b"), Some("1,0")).as_deref(),
            Some("/sorted/b")
        );
        assert_eq!(
            workspace_key_from_folders(Some("/sorted/a\n/sorted/b"), None).as_deref(),
            Some("/sorted/a")
        );
    }

    #[test]
    fn decode_thread_json_rejects_unknown_data_type() {
        let err = decode_thread_json("brotli", b"{}").unwrap_err();
        assert!(err.contains("unsupported data_type"));
    }

    #[test]
    fn parse_zed_sqlite_returns_empty_for_missing_database() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("missing.db");
        assert!(parse_zed_sqlite(&missing).is_empty());
        fs::create_dir_all(dir.path().join("threads")).unwrap();
    }
}
