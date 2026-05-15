//! Cursor CLI local usage parser.
//!
//! Cursor's documented CLI output currently does not expose token counts, so
//! this parser reads the local Cursor state database as a best-effort metadata
//! source. It intentionally queries only usage fields from `bubbleId:*` rows and
//! never selects prompt text, rich text, tool results, blob payloads, or auth
//! fields from the database.

use super::utils::parse_timestamp_str;
use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::path::Path;
use tracing::warn;

const CURSOR_CLI_CLIENT_ID: &str = "cursorcli";
const MAX_CURSOR_BUBBLE_JSON_BYTES: i64 = 512 * 1024;

#[derive(Debug)]
struct CursorCliBubbleRow {
    key: String,
    bubble_id: Option<String>,
    request_id: Option<String>,
    created_at: Option<String>,
    model_name: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
}

pub fn parse_cursorcli_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let conn = match Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => conn,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open Cursor CLI state database"
            );
            return Vec::new();
        }
    };

    let mut stmt = match conn.prepare(
        "SELECT
            key,
            json_extract(CAST(value AS TEXT), '$.bubbleId') AS bubble_id,
            json_extract(CAST(value AS TEXT), '$.requestId') AS request_id,
            json_extract(CAST(value AS TEXT), '$.createdAt') AS created_at,
            json_extract(CAST(value AS TEXT), '$.modelInfo.modelName') AS model_name,
            CAST(json_extract(CAST(value AS TEXT), '$.tokenCount.inputTokens') AS INTEGER) AS input_tokens,
            CAST(json_extract(CAST(value AS TEXT), '$.tokenCount.outputTokens') AS INTEGER) AS output_tokens
         FROM cursorDiskKV
         WHERE key LIKE 'bubbleId:%'
           AND length(value) <= ?1
           AND json_valid(CAST(value AS TEXT))
         LIMIT 100000",
    ) {
        Ok(stmt) => stmt,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to prepare Cursor CLI usage query"
            );
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([MAX_CURSOR_BUBBLE_JSON_BYTES], |row| {
        Ok(CursorCliBubbleRow {
            key: row.get(0)?,
            bubble_id: row.get(1)?,
            request_id: row.get(2)?,
            created_at: row.get(3)?,
            model_name: row.get(4)?,
            input_tokens: row.get(5)?,
            output_tokens: row.get(6)?,
        })
    }) {
        Ok(rows) => rows,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to query Cursor CLI usage rows"
            );
            return Vec::new();
        }
    };

    rows.filter_map(|row| match row {
        Ok(row) => parse_bubble_row(row),
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to decode Cursor CLI usage row"
            );
            None
        }
    })
    .collect()
}

fn parse_bubble_row(row: CursorCliBubbleRow) -> Option<UnifiedMessage> {
    let created_at = row.created_at.as_deref()?.trim();
    let timestamp = parse_timestamp_str(created_at)?;

    let model_id = row.model_name.as_deref()?.trim();
    if model_id.is_empty() {
        return None;
    }

    let tokens = TokenBreakdown {
        input: row.input_tokens.unwrap_or(0).max(0),
        output: row.output_tokens.unwrap_or(0).max(0),
        cache_read: 0,
        cache_write: 0,
        reasoning: 0,
    };
    if tokens.total() <= 0 {
        return None;
    }

    let session_id_seed = session_id_from_key(&row.key)
        .or_else(|| non_empty(row.request_id.as_deref()))
        .or_else(|| non_empty(row.bubble_id.as_deref()))
        .unwrap_or_else(|| "unknown".to_string());
    let session_id = format!("cursorcli-session-{}", stable_hash(&session_id_seed));
    let provider_id = provider_identity::inferred_provider_from_model(model_id).unwrap_or("cursor");
    let dedup_seed = non_empty(row.request_id.as_deref())
        .or_else(|| non_empty(row.bubble_id.as_deref()))
        .unwrap_or_else(|| row.key.clone());

    let mut message = UnifiedMessage::new(
        CURSOR_CLI_CLIENT_ID,
        model_id,
        provider_id,
        session_id,
        timestamp,
        tokens,
        0.0,
    );
    message.dedup_key = Some(format!("cursorcli:{}", stable_hash(&dedup_seed)));
    Some(message)
}

fn stable_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn session_id_from_key(key: &str) -> Option<String> {
    let mut parts = key.split(':');
    match (parts.next(), parts.next()) {
        (Some("bubbleId"), Some(session_id)) if !session_id.trim().is_empty() => {
            Some(session_id.trim().to_string())
        }
        _ => None,
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::TempDir;

    fn create_db(rows: &[(&str, &str)]) -> (TempDir, std::path::PathBuf) {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("state.vscdb");
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)",
            [],
        )
        .unwrap();
        for (key, value) in rows {
            conn.execute(
                "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
                params![key, value.as_bytes()],
            )
            .unwrap();
        }
        drop(conn);
        (temp, path)
    }

    #[test]
    fn parses_valid_bubble_usage_row() {
        let (_temp, path) = create_db(&[(
            "bubbleId:session-123:bubble-456",
            r#"{
                "bubbleId":"bubble-456",
                "requestId":"request-789",
                "createdAt":"2026-05-15T12:34:56.000Z",
                "modelInfo":{"modelName":"gpt-5.5"},
                "tokenCount":{"inputTokens":123,"outputTokens":45},
                "text":"must not be selected by the parser",
                "richText":"must not be selected by the parser",
                "toolResults":[{"secret":"ignored"}]
            }"#,
        )]);

        let messages = parse_cursorcli_sqlite(&path);
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "cursorcli");
        assert_eq!(
            message.session_id,
            format!("cursorcli-session-{}", stable_hash("session-123"))
        );
        assert_eq!(message.model_id, "gpt-5.5");
        assert_eq!(message.provider_id, "openai");
        assert_eq!(message.tokens.input, 123);
        assert_eq!(message.tokens.output, 45);
        assert_eq!(
            message.dedup_key.as_deref(),
            Some(format!("cursorcli:{}", stable_hash("request-789")).as_str())
        );
        assert!(!message.session_id.contains("session-123"));
        assert_ne!(message.dedup_key.as_deref(), Some("cursorcli:request-789"));
    }

    #[test]
    fn ignores_malformed_json_and_non_bubble_rows() {
        let (_temp, path) = create_db(&[
            ("bubbleId:session:bad", "{not json"),
            (
                "composerData:session",
                r#"{"createdAt":"2026-05-15T12:34:56.000Z","modelInfo":{"modelName":"gpt-5.5"},"tokenCount":{"inputTokens":1,"outputTokens":1}}"#,
            ),
        ]);

        assert!(parse_cursorcli_sqlite(&path).is_empty());
    }

    #[test]
    fn skips_zero_token_rows() {
        let (_temp, path) = create_db(&[(
            "bubbleId:session:zero",
            r#"{"createdAt":"2026-05-15T12:34:56.000Z","modelInfo":{"modelName":"gpt-5.5"},"tokenCount":{"inputTokens":0,"outputTokens":0}}"#,
        )]);

        assert!(parse_cursorcli_sqlite(&path).is_empty());
    }

    #[test]
    fn clamps_negative_tokens() {
        let (_temp, path) = create_db(&[(
            "bubbleId:session:negative",
            r#"{"createdAt":"2026-05-15T12:34:56.000Z","modelInfo":{"modelName":"claude-sonnet-4-5"},"tokenCount":{"inputTokens":-12,"outputTokens":8}}"#,
        )]);

        let messages = parse_cursorcli_sqlite(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 0);
        assert_eq!(messages[0].tokens.output, 8);
        assert_eq!(messages[0].provider_id, "anthropic");
    }

    #[test]
    fn skips_oversized_rows_before_json_extraction() {
        let large_text = "x".repeat((MAX_CURSOR_BUBBLE_JSON_BYTES as usize) + 1);
        let value = format!(
            r#"{{"createdAt":"2026-05-15T12:34:56.000Z","modelInfo":{{"modelName":"gpt-5.5"}},"tokenCount":{{"inputTokens":1,"outputTokens":1}},"text":"{large_text}"}}"#
        );
        let (_temp, path) = create_db(&[("bubbleId:session:large", value.as_str())]);

        assert!(parse_cursorcli_sqlite(&path).is_empty());
    }
}
