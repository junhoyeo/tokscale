//! OpenCode session parser
//!
//! Parses messages from:
//! - SQLite database (OpenCode 1.2+): ~/.local/share/opencode/opencode.db
//! - Legacy JSON files: ~/.local/share/opencode/storage/message/

use super::utils::{open_readonly_sqlite, read_file_or_none};
use super::{normalize_opencode_agent_name, UnifiedMessage};
use crate::TokenBreakdown;
#[cfg(test)]
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// OpenCode message structure (from JSON files and SQLite data column)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OpenCodeMessage {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "sessionID", default)]
    pub session_id: Option<String>,
    pub role: String,
    #[serde(rename = "modelID")]
    pub model_id: Option<String>,
    #[serde(rename = "providerID")]
    pub provider_id: Option<String>,
    pub cost: Option<f64>,
    pub tokens: Option<OpenCodeTokens>,
    pub time: OpenCodeTime,
    pub agent: Option<String>,
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenCodeTokens {
    pub input: i64,
    pub output: i64,
    pub reasoning: Option<i64>,
    pub cache: OpenCodeCache,
}

#[derive(Debug, Deserialize)]
pub struct OpenCodeCache {
    pub read: i64,
    pub write: i64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OpenCodeTime {
    pub created: f64, // Unix timestamp in milliseconds (as float)
    pub completed: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OpenCodeSqliteFingerprint {
    created_bits: u64,
    completed_bits: Option<u64>,
    model_id: String,
    provider_id: String,
    input: i64,
    output: i64,
    reasoning: i64,
    cache_read: i64,
    cache_write: i64,
    cost_bits: u64,
    agent: Option<String>,
}

pub fn parse_opencode_file(path: &Path) -> Option<UnifiedMessage> {
    let data = read_file_or_none(path)?;
    let mut bytes = data;

    let msg: OpenCodeMessage = simd_json::from_slice(&mut bytes).ok()?;

    if msg.role != "assistant" {
        return None;
    }

    let tokens = msg.tokens?;
    let model_id = msg.model_id?;
    let agent_or_mode = msg.mode.or(msg.agent);
    let agent = agent_or_mode.map(|a| normalize_opencode_agent_name(&a));

    let session_id = msg.session_id.unwrap_or_else(|| "unknown".to_string());

    // Use message ID from JSON or derive from filename for deduplication
    let dedup_key = msg.id.or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    });

    let mut unified = UnifiedMessage::new_with_agent(
        "opencode",
        model_id,
        msg.provider_id.unwrap_or_else(|| "unknown".to_string()),
        session_id,
        msg.time.created as i64,
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
    Some(unified)
}

pub fn parse_opencode_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite(db_path) else {
        return Vec::new();
    };

    let query = r#"
        SELECT m.id, m.session_id, m.data
        FROM message m
        WHERE json_extract(m.data, '$.role') = 'assistant'
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

    let mut messages: Vec<UnifiedMessage> = Vec::new();
    let mut fingerprint_indices: HashMap<OpenCodeSqliteFingerprint, usize> = HashMap::new();
    let mut dedup_key_has_embedded_message_id: Vec<bool> = Vec::new();

    for row_result in rows {
        let (row_id, session_id, data_json) = match row_result {
            Ok(r) => r,
            Err(_) => continue,
        };

        let mut bytes = data_json.into_bytes();
        let msg: OpenCodeMessage = match simd_json::from_slice(&mut bytes) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if msg.role != "assistant" {
            continue;
        }

        let message_id = msg.id.clone();

        let tokens = match msg.tokens {
            Some(t) => t,
            None => continue,
        };

        let model_id = match msg.model_id {
            Some(m) => m,
            None => continue,
        };

        let provider_id = msg.provider_id.unwrap_or_else(|| "unknown".to_string());
        let agent_or_mode = msg.mode.or(msg.agent);
        let agent = agent_or_mode.map(|a| normalize_opencode_agent_name(&a));
        let input = tokens.input.max(0);
        let output = tokens.output.max(0);
        let reasoning = tokens.reasoning.unwrap_or(0).max(0);
        let cache_read = tokens.cache.read.max(0);
        let cache_write = tokens.cache.write.max(0);
        let cost = msg.cost.unwrap_or(0.0).max(0.0);
        let dedup_key = message_id.clone().unwrap_or(row_id);
        let fingerprint = OpenCodeSqliteFingerprint {
            created_bits: msg.time.created.to_bits(),
            completed_bits: msg.time.completed.map(f64::to_bits),
            model_id: model_id.clone(),
            provider_id: provider_id.clone(),
            input,
            output,
            reasoning,
            cache_read,
            cache_write,
            cost_bits: cost.to_bits(),
            agent: agent.clone(),
        };

        let mut unified = UnifiedMessage::new_with_agent(
            "opencode",
            model_id,
            provider_id,
            session_id,
            msg.time.created as i64,
            TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                reasoning,
            },
            cost,
            agent,
        );
        unified.dedup_key = Some(dedup_key);

        if let Some(index) = fingerprint_indices.get(&fingerprint).copied() {
            if message_id.is_some() && !dedup_key_has_embedded_message_id[index] {
                dedup_key_has_embedded_message_id[index] = true;
                messages[index].dedup_key = unified.dedup_key;
            }
            continue;
        }

        dedup_key_has_embedded_message_id.push(message_id.is_some());
        fingerprint_indices.insert(fingerprint, messages.len());
        messages.push(unified);
    }

    messages
}

// =============================================================================
// Migration cache: skip redundant legacy JSON scanning after full migration
// =============================================================================

const MIGRATION_CACHE_FILENAME: &str = "opencode-migration.json";

/// Persisted migration status for OpenCode JSON → SQLite migration.
/// Stored at <config_dir>/cache/opencode-migration.json.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenCodeMigrationCache {
    /// True when every legacy JSON message was already present in SQLite.
    pub migration_complete: bool,
    /// Number of JSON files in the message directory at detection time.
    pub json_file_count: u64,
    /// Modification time of the JSON directory (Unix seconds) at detection time.
    pub json_dir_mtime_secs: u64,
    /// When this entry was written (Unix seconds).
    pub checked_at_secs: u64,
}

fn migration_cache_dir() -> std::path::PathBuf {
    crate::paths::get_cache_dir()
}

fn migration_cache_path() -> std::path::PathBuf {
    migration_cache_dir().join(MIGRATION_CACHE_FILENAME)
}

fn legacy_migration_cache_paths() -> Vec<std::path::PathBuf> {
    if crate::paths::is_config_dir_overridden() {
        return Vec::new();
    }

    [
        crate::paths::legacy_dirs_cache_dir().map(|d| d.join(MIGRATION_CACHE_FILENAME)),
        crate::paths::legacy_dot_cache_tokscale_dir().map(|d| d.join(MIGRATION_CACHE_FILENAME)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Load the migration cache from disk. Returns `None` if the file is missing or
/// unparseable.
pub fn load_opencode_migration_cache() -> Option<OpenCodeMigrationCache> {
    std::iter::once(migration_cache_path())
        .chain(legacy_migration_cache_paths())
        .find_map(|path| {
            let content = std::fs::read_to_string(path).ok()?;
            serde_json::from_str(&content).ok()
        })
}

/// Persist the migration cache atomically (write to temp file, then rename).
pub fn save_opencode_migration_cache(cache: &OpenCodeMigrationCache) {
    use std::io::Write as _;

    let dir = migration_cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let content = match serde_json::to_string(cache) {
        Ok(c) => c,
        Err(_) => return,
    };

    let final_path = migration_cache_path();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let tmp_name = format!(".opencode-migration.{}.{:x}.tmp", std::process::id(), nanos);
    let tmp_path = dir.join(tmp_name);

    // INVARIANT: All cache writes use atomic temp-file rename. NEVER delete
    // the canonical cache file before writing — a partial save or process
    // crash between delete and rename would lose the cache. The temp-file
    // pattern makes corruption-on-crash impossible.
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        if std::fs::rename(&tmp_path, &final_path).is_err() {
            std::fs::copy(&tmp_path, &final_path)?;
            std::fs::remove_file(&tmp_path)?;
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
}

/// Return the modification time of `json_dir` as Unix seconds, or `None` on
/// error (directory absent, permissions, etc.).
pub fn get_json_dir_mtime(json_dir: &Path) -> Option<u64> {
    std::fs::metadata(json_dir)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Current Unix timestamp in seconds.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, previous) in self.0.drain(..) {
                unsafe {
                    match previous {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    fn create_opencode_sqlite_db(db_path: &Path) -> Connection {
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                data TEXT NOT NULL
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_parse_opencode_structure() {
        let json = r#"{
            "id": "msg_123",
            "sessionID": "ses_456",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 100,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let mut bytes = json.as_bytes().to_vec();
        let msg: OpenCodeMessage = simd_json::from_slice(&mut bytes).unwrap();

        assert_eq!(msg.model_id, Some("claude-sonnet-4".to_string()));
        assert_eq!(msg.tokens.unwrap().input, 1000);
        assert_eq!(msg.agent, None);
    }

    #[test]
    fn test_parse_opencode_with_agent() {
        let json = r#"{
            "id": "msg_123",
            "sessionID": "ses_456",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "agent": "OmO",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 100,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let mut bytes = json.as_bytes().to_vec();
        let msg: OpenCodeMessage = simd_json::from_slice(&mut bytes).unwrap();

        assert_eq!(msg.agent, Some("OmO".to_string()));
    }

    /// Verify negative token values are clamped to 0 (defense-in-depth for PR #147)
    #[test]
    fn test_negative_values_clamped_to_zero() {
        use std::io::Write;

        let json = r#"{
            "id": "msg_negative",
            "sessionID": "ses_negative",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": -0.05,
            "tokens": {
                "input": -100,
                "output": -50,
                "reasoning": -25,
                "cache": { "read": -200, "write": -10 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let mut temp_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        temp_file.write_all(json.as_bytes()).unwrap();

        let result = parse_opencode_file(temp_file.path());
        assert!(result.is_some(), "Should parse file with negative values");

        let msg = result.unwrap();
        assert_eq!(msg.tokens.input, 0, "Negative input should be clamped to 0");
        assert_eq!(
            msg.tokens.output, 0,
            "Negative output should be clamped to 0"
        );
        assert_eq!(
            msg.tokens.cache_read, 0,
            "Negative cache_read should be clamped to 0"
        );
        assert_eq!(
            msg.tokens.cache_write, 0,
            "Negative cache_write should be clamped to 0"
        );
        assert_eq!(
            msg.tokens.reasoning, 0,
            "Negative reasoning should be clamped to 0"
        );
        assert!(
            msg.cost >= 0.0,
            "Negative cost should be clamped to 0.0, got {}",
            msg.cost
        );
    }

    /// JSON dedup_key uses msg.id when present
    #[test]
    fn test_dedup_key_from_json_message_id() {
        use std::io::Write;

        let json = r#"{
            "id": "msg_dedup_001",
            "sessionID": "ses_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.01,
            "tokens": {
                "input": 100,
                "output": 50,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let mut temp_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        temp_file.write_all(json.as_bytes()).unwrap();

        let msg = parse_opencode_file(temp_file.path()).expect("Should parse");
        assert_eq!(
            msg.dedup_key,
            Some("msg_dedup_001".to_string()),
            "dedup_key should use msg.id from JSON"
        );
    }

    /// JSON dedup_key falls back to file stem when msg.id is absent
    #[test]
    fn test_dedup_key_falls_back_to_file_stem() {
        let json = r#"{
            "sessionID": "ses_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.01,
            "tokens": {
                "input": 100,
                "output": 50,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("msg_fallback_999.json");
        std::fs::write(&file_path, json).unwrap();

        let msg = parse_opencode_file(&file_path).expect("Should parse");
        assert_eq!(
            msg.dedup_key,
            Some("msg_fallback_999".to_string()),
            "dedup_key should fall back to file stem when id is missing"
        );
    }

    /// Non-assistant messages are skipped (no dedup_key produced)
    #[test]
    fn test_dedup_key_skips_non_assistant() {
        let json = r#"{
            "id": "msg_user_001",
            "sessionID": "ses_001",
            "role": "user",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": {
                "input": 100,
                "output": 50,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("msg_user_001.json");
        std::fs::write(&file_path, json).unwrap();

        let result = parse_opencode_file(&file_path);
        assert!(result.is_none(), "User messages should be skipped");
    }

    /// SQLite dedup_key falls back to the database row id when the message has no embedded id.
    #[test]
    fn test_sqlite_dedup_key_falls_back_to_row_id() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");

        let conn = create_opencode_sqlite_db(&db_path);

        let data_json = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 0,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_sqlite_001", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key,
            Some("msg_sqlite_001".to_string()),
            "SQLite dedup_key should fall back to the row id when no embedded id exists"
        );
        assert_eq!(messages[0].model_id, "claude-sonnet-4");
        assert_eq!(messages[0].tokens.input, 1000);
    }

    /// SQLite prefers the embedded message id when present so JSON/SQLite overlap keeps deduplicating.
    #[test]
    fn test_sqlite_dedup_key_prefers_embedded_message_id() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");

        let conn = create_opencode_sqlite_db(&db_path);

        let valid = r#"{
            "id": "embedded_msg_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 100, "output": 50, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["row_msg_001", "ses_001", valid],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key,
            Some("embedded_msg_001".to_string()),
            "SQLite dedup_key should prefer the embedded message id for cross-source overlap"
        );
    }

    /// SQLite skips rows without tokens or with non-assistant role
    #[test]
    fn test_sqlite_skips_invalid_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");

        let conn = create_opencode_sqlite_db(&db_path);

        let valid = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 100, "output": 50, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;

        let user_msg = r#"{
            "role": "user",
            "modelID": "claude-sonnet-4",
            "time": { "created": 1700000000000.0 }
        }"#;

        let no_tokens = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_valid", "ses_001", valid],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_user", "ses_001", user_msg],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_no_tokens", "ses_001", no_tokens],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            1,
            "Should only parse valid assistant message"
        );
        assert_eq!(messages[0].dedup_key, Some("msg_valid".to_string()));
    }

    /// Forked SQLite sessions should not count copied history more than once.
    #[test]
    fn test_sqlite_deduplicates_forked_history_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");
        let conn = create_opencode_sqlite_db(&db_path);

        let root_msg = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 25,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000000500.0 },
            "mode": "build"
        }"#;

        let new_msg = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.08,
            "tokens": {
                "input": 1300,
                "output": 650,
                "reasoning": 40,
                "cache": { "read": 100, "write": 0 }
            },
            "time": { "created": 1700000001000.0, "completed": 1700000001500.0 },
            "mode": "build"
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["root_row", "root_session", root_msg],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["fork_copy_row", "fork_session", root_msg],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["fork_new_row", "fork_session", new_msg],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            2,
            "Forked copies of the same assistant history should collapse inside SQLite parsing"
        );
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[1].tokens.input, 1300);
    }

    /// Same-timestamp messages with different payloads should remain distinct.
    #[test]
    fn test_sqlite_same_timestamp_distinct_payloads_survive() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");
        let conn = create_opencode_sqlite_db(&db_path);

        let first = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000000100.0 },
            "mode": "build"
        }"#;

        let second = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1500,
                "output": 750,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000000100.0 },
            "mode": "build"
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["row_one", "session_one", first],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["row_two", "session_two", second],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            2,
            "Distinct assistant calls should survive even when they share the same creation timestamp"
        );
    }

    /// Cross-source dedup: matching IDs between SQLite and JSON should deduplicate
    #[test]
    fn test_cross_source_dedup_by_message_id() {
        use std::collections::HashSet;

        let dir = tempfile::tempdir().unwrap();

        // --- SQLite source ---
        let db_path = dir.path().join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                data TEXT NOT NULL
            );",
        )
        .unwrap();

        let shared_data_json = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 500, "output": 200, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;
        let sqlite_only_data_json = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 700, "output": 250, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000001000.0 }
        }"#;

        // Insert two messages into SQLite
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_shared_001", "ses_001", shared_data_json],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_sqlite_only", "ses_001", sqlite_only_data_json],
        )
        .unwrap();
        drop(conn);

        // --- JSON source ---
        let json_dir = dir.path().join("json");
        std::fs::create_dir_all(&json_dir).unwrap();

        // Duplicate of SQLite msg_shared_001
        let json_shared = r#"{
            "id": "msg_shared_001",
            "sessionID": "ses_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 500, "output": 200, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;
        std::fs::write(json_dir.join("msg_shared_001.json"), json_shared).unwrap();

        // JSON-only message (not in SQLite)
        let json_only = r#"{
            "id": "msg_json_only",
            "sessionID": "ses_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 100, "output": 50, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;
        std::fs::write(json_dir.join("msg_json_only.json"), json_only).unwrap();

        // --- Simulate the dedup logic from lib.rs ---
        let sqlite_messages = parse_opencode_sqlite(&db_path);
        assert_eq!(sqlite_messages.len(), 2);

        // Build seen set from SQLite (same as lib.rs)
        let mut seen: HashSet<String> = HashSet::new();
        for msg in &sqlite_messages {
            if let Some(ref key) = msg.dedup_key {
                seen.insert(key.clone());
            }
        }
        assert_eq!(seen.len(), 2);

        // Parse JSON files
        let json_msg_shared = parse_opencode_file(&json_dir.join("msg_shared_001.json")).unwrap();
        let json_msg_only = parse_opencode_file(&json_dir.join("msg_json_only.json")).unwrap();

        // Filter JSON through seen set (same logic as lib.rs)
        let json_messages = vec![json_msg_shared, json_msg_only];
        let deduped: Vec<UnifiedMessage> = json_messages
            .into_iter()
            .filter(|msg| {
                msg.dedup_key
                    .as_ref()
                    .is_none_or(|key| seen.insert(key.clone()))
            })
            .collect();

        // msg_shared_001 should be filtered (duplicate), msg_json_only should survive
        assert_eq!(
            deduped.len(),
            1,
            "Only the JSON-only message should survive dedup"
        );
        assert_eq!(
            deduped[0].dedup_key,
            Some("msg_json_only".to_string()),
            "Surviving message should be the JSON-only one"
        );

        // Total unique messages = 2 from SQLite + 1 from JSON
        let total = sqlite_messages.len() + deduped.len();
        assert_eq!(total, 3, "Should have 3 unique messages total");
    }

    // -------------------------------------------------------------------------
    // Migration cache tests
    // -------------------------------------------------------------------------

    /// Round-trip: save then load returns identical data.
    #[test]
    fn test_migration_cache_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        // Point the cache at a temp dir by overriding via a temporary env var is
        // impractical here; instead we test the structs and serde directly.
        let cache = OpenCodeMigrationCache {
            migration_complete: true,
            json_file_count: 42,
            json_dir_mtime_secs: 1_700_000_000,
            checked_at_secs: 1_700_100_000,
        };

        let json = serde_json::to_string(&cache).unwrap();
        let loaded: OpenCodeMigrationCache = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded, cache);

        // Ensure the JSON contains all expected keys
        assert!(json.contains("migration_complete"));
        assert!(json.contains("json_file_count"));
        assert!(json.contains("json_dir_mtime_secs"));
        assert!(json.contains("checked_at_secs"));

        drop(dir);
    }

    /// Cache is valid when file count and mtime are unchanged.
    #[test]
    fn test_migration_cache_valid_when_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let json_dir = dir.path().join("message");
        std::fs::create_dir_all(&json_dir).unwrap();

        // Write a dummy file so the directory exists and has a stable mtime
        std::fs::write(json_dir.join("msg.json"), b"{}").unwrap();

        let current_mtime = get_json_dir_mtime(&json_dir).expect("should stat dir");
        let current_file_count = 1u64;

        let cache = OpenCodeMigrationCache {
            migration_complete: true,
            json_file_count: current_file_count,
            json_dir_mtime_secs: current_mtime, // same mtime
            checked_at_secs: now_secs(),
        };

        // Simulate the validity check from lib.rs
        let is_valid = cache.migration_complete
            && current_file_count == cache.json_file_count
            && get_json_dir_mtime(&json_dir).is_some_and(|m| m <= cache.json_dir_mtime_secs);

        assert!(is_valid, "Cache should be valid when count and mtime match");
    }

    /// Cache is invalid when file count has changed.
    #[test]
    fn test_migration_cache_invalid_when_file_count_changes() {
        let dir = tempfile::tempdir().unwrap();
        let json_dir = dir.path().join("message");
        std::fs::create_dir_all(&json_dir).unwrap();
        std::fs::write(json_dir.join("msg1.json"), b"{}").unwrap();

        let current_mtime = get_json_dir_mtime(&json_dir).unwrap();

        let cache = OpenCodeMigrationCache {
            migration_complete: true,
            json_file_count: 1,
            json_dir_mtime_secs: current_mtime,
            checked_at_secs: now_secs(),
        };

        // Simulate: a new file was added → current_file_count = 2
        let current_file_count = 2u64; // changed
        let is_valid = cache.migration_complete
            && current_file_count == cache.json_file_count
            && get_json_dir_mtime(&json_dir).is_some_and(|m| m <= cache.json_dir_mtime_secs);

        assert!(!is_valid, "Cache should be invalid when file count changes");
    }

    /// Cache is invalid when directory mtime is newer than cached value.
    #[test]
    fn test_migration_cache_invalid_when_mtime_newer() {
        let dir = tempfile::tempdir().unwrap();
        let json_dir = dir.path().join("message");
        std::fs::create_dir_all(&json_dir).unwrap();
        std::fs::write(json_dir.join("msg.json"), b"{}").unwrap();

        let current_mtime = get_json_dir_mtime(&json_dir).unwrap();

        // Simulate: cache recorded an older mtime → directory is now newer
        let stale_mtime = current_mtime.saturating_sub(1);
        let cache = OpenCodeMigrationCache {
            migration_complete: true,
            json_file_count: 1,
            json_dir_mtime_secs: stale_mtime, // older than current
            checked_at_secs: now_secs(),
        };

        let is_valid = cache.migration_complete
            && 1u64 == cache.json_file_count
            && get_json_dir_mtime(&json_dir).is_some_and(|m| m <= cache.json_dir_mtime_secs);

        assert!(
            !is_valid,
            "Cache should be invalid when directory mtime is newer than cached value"
        );
    }

    /// Cache is not loaded when the file is missing (load returns None).
    #[test]
    fn test_migration_cache_missing_returns_none() {
        // load_opencode_migration_cache reads from ~/.cache/tokscale/opencode-migration.json
        // We can't easily override the path in a unit test, but we can verify that
        // serde_json::from_str returns None for invalid input (simulating missing file).
        let result: Option<OpenCodeMigrationCache> = serde_json::from_str("").ok();
        assert!(
            result.is_none(),
            "Empty/missing content should produce None"
        );
    }

    /// migration_complete=false disables the cache even if count/mtime match.
    #[test]
    fn test_migration_cache_not_skipped_when_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let json_dir = dir.path().join("message");
        std::fs::create_dir_all(&json_dir).unwrap();
        std::fs::write(json_dir.join("msg.json"), b"{}").unwrap();

        let current_mtime = get_json_dir_mtime(&json_dir).unwrap();

        let cache = OpenCodeMigrationCache {
            migration_complete: false, // migration not complete
            json_file_count: 1,
            json_dir_mtime_secs: current_mtime,
            checked_at_secs: now_secs(),
        };

        let is_valid = cache.migration_complete
            && 1u64 == cache.json_file_count
            && get_json_dir_mtime(&json_dir).is_some_and(|m| m <= cache.json_dir_mtime_secs);

        assert!(
            !is_valid,
            "Cache should not allow skipping when migration_complete=false"
        );
    }

    #[test]
    #[serial_test::serial]
    fn migration_record_falls_back_to_legacy_path() {
        use std::env;

        let temp_home = tempfile::tempdir().unwrap();
        let temp_xdg_cache = tempfile::tempdir().unwrap();
        let prev_home = env::var_os("HOME");
        let prev_xdg_cache = env::var_os("XDG_CACHE_HOME");
        let prev_override = env::var_os("TOKSCALE_CONFIG_DIR");
        let _guard = EnvGuard(vec![
            ("TOKSCALE_CONFIG_DIR", prev_override),
            ("XDG_CACHE_HOME", prev_xdg_cache),
            ("HOME", prev_home),
        ]);
        unsafe {
            env::set_var("HOME", temp_home.path());
            env::set_var("XDG_CACHE_HOME", temp_xdg_cache.path());
            env::remove_var("TOKSCALE_CONFIG_DIR");
        }

        let legacy_path = crate::paths::legacy_dirs_cache_dir()
            .unwrap()
            .join(MIGRATION_CACHE_FILENAME);
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        std::fs::write(
            &legacy_path,
            r#"{"migration_complete":true,"json_file_count":2,"json_dir_mtime_secs":3,"checked_at_secs":4}"#,
        )
        .unwrap();

        let loaded = load_opencode_migration_cache().unwrap();
        assert!(loaded.migration_complete);
        assert_eq!(loaded.json_file_count, 2);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[ignore] // Run manually with: cargo test integration -- --ignored
    fn test_parse_real_sqlite_db() {
        let home = std::env::var("HOME").unwrap();
        let db_path = PathBuf::from(format!("{}/.local/share/opencode/opencode.db", home));

        if !db_path.exists() {
            println!("Skipping: OpenCode database not found at {:?}", db_path);
            return;
        }

        let messages = parse_opencode_sqlite(&db_path);
        println!("Parsed {} messages from SQLite", messages.len());

        if !messages.is_empty() {
            let first = &messages[0];
            println!(
                "First message: model={}, provider={}, tokens={:?}",
                first.model_id, first.provider_id, first.tokens
            );
        }

        assert!(
            !messages.is_empty(),
            "Expected to parse some messages from SQLite"
        );
    }
}
