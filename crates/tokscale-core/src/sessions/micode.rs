//! MiMo Code session parser
//!
//! Parses messages from:
//! - SQLite database: ~/.local/share/micode/mimocode.db

use super::utils::open_readonly_sqlite;
use super::{
    normalize_opencode_agent_name, normalize_workspace_key, workspace_label_from_key,
    UnifiedMessage,
};
use crate::TokenBreakdown;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// MiMo Code message structure (from SQLite data column)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct MiMoCodeMessage {
    #[serde(default)]
    pub id: Option<String>,
    pub role: String,
    #[serde(rename = "modelID")]
    pub model_id: Option<String>,
    #[serde(rename = "providerID")]
    pub provider_id: Option<String>,
    pub cost: Option<f64>,
    pub tokens: Option<MiMoCodeTokens>,
    pub time: MiMoCodeTime,
    pub agent: Option<String>,
    pub mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_micode_path")]
    pub path: Option<MiMoCodePath>,
}

#[derive(Debug, Deserialize)]
pub struct MiMoCodePath {
    pub root: Option<String>,
}

fn deserialize_micode_path<'de, D>(deserializer: D) -> Result<Option<MiMoCodePath>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let root = value
        .get("root")
        .and_then(|root| root.as_str())
        .map(str::to_string);

    Ok(Some(MiMoCodePath { root }))
}

#[derive(Debug, Deserialize)]
pub struct MiMoCodeTokens {
    pub input: i64,
    pub output: i64,
    pub reasoning: Option<i64>,
    // MiMo assistant messages may omit `cache` (or its read/write); without a
    // default a missing field would fail deserialization and silently drop the
    // message in the parse loop's `Err(_) => continue` arm.
    #[serde(default)]
    pub cache: Option<MiMoCodeCache>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MiMoCodeCache {
    #[serde(default)]
    pub read: i64,
    #[serde(default)]
    pub write: i64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct MiMoCodeTime {
    pub created: f64,
    pub completed: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MiMoCodeSqliteFingerprint {
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

#[derive(Debug, Clone)]
struct MiMoCodeSqliteDedupState {
    has_embedded_message_id: bool,
    has_workspace_conflict: bool,
}

fn workspace_from_root(root: Option<&str>) -> (Option<String>, Option<String>) {
    let workspace_key = root.and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
    (workspace_key, workspace_label)
}

fn set_workspace_from_root(message: &mut UnifiedMessage, root: Option<&str>) {
    let (workspace_key, workspace_label) = workspace_from_root(root);
    message.set_workspace(workspace_key, workspace_label);
}

fn merge_duplicate_workspace(
    message: &mut UnifiedMessage,
    state: &mut MiMoCodeSqliteDedupState,
    root: Option<&str>,
) {
    if state.has_workspace_conflict {
        return;
    }

    let (candidate_key, candidate_label) = workspace_from_root(root);
    match (message.workspace_key.as_deref(), candidate_key) {
        (None, Some(key)) => message.set_workspace(Some(key), candidate_label),
        (Some(existing), Some(candidate)) if existing != candidate => {
            state.has_workspace_conflict = true;
            message.set_workspace(None, None);
        }
        _ => {}
    }
}

fn micode_duration_ms(time: &MiMoCodeTime) -> Option<i64> {
    let duration = time.completed? - time.created;
    if duration.is_finite() && duration > 0.0 {
        Some(duration as i64)
    } else {
        None
    }
}

pub fn parse_micode_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite(db_path) else {
        return Vec::new();
    };

    let modern_query = r#"
        SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root
        FROM message m
        LEFT JOIN session s ON s.id = m.session_id
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#;

    let legacy_query = r#"
        SELECT m.id, m.session_id, m.data, NULL AS workspace_root
        FROM message m
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#;

    let mut stmt = match conn
        .prepare(modern_query)
        .or_else(|_| conn.prepare(legacy_query))
    {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let rows = match stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let session_id: String = row.get(1)?;
        let data_json: String = row.get(2)?;
        let workspace_root: Option<String> = row.get(3)?;
        Ok((id, session_id, data_json, workspace_root))
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut messages: Vec<UnifiedMessage> = Vec::new();
    let mut fingerprint_indices: HashMap<MiMoCodeSqliteFingerprint, usize> = HashMap::new();
    let mut dedup_states: Vec<MiMoCodeSqliteDedupState> = Vec::new();

    // Namespace ONLY the row-id fallback by the database. MiMo Code uses
    // channel-suffixed databases (mimocode.db and mimocode-<channel>.db), and a
    // mid-session channel switch can write the SAME message to both files. The
    // embedded message id is globally unique, so it must stay un-namespaced to
    // collapse those duplicates across files. SQLite rowids, by contrast, are
    // per-database and not globally unique, so the fallback path namespaces them
    // to avoid falsely merging two different messages that share a rowid.
    let db_namespace = db_path.to_string_lossy().to_string();

    for row_result in rows {
        let (row_id, session_id, data_json, row_workspace_root) = match row_result {
            Ok(r) => r,
            Err(_) => continue,
        };

        let mut bytes = data_json.into_bytes();
        let msg: MiMoCodeMessage = match simd_json::from_slice(&mut bytes) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if msg.role != "assistant" {
            continue;
        }

        let message_id = msg.id.clone();
        let embedded_workspace_root = msg
            .path
            .as_ref()
            .and_then(|path| path.root.as_deref())
            .map(str::to_string);

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
        let cache = tokens.cache.unwrap_or_default();
        let cache_read = cache.read.max(0);
        let cache_write = cache.write.max(0);
        let cost = msg.cost.unwrap_or(0.0).max(0.0);
        let dedup_key = match message_id.clone() {
            // Embedded ids are globally unique: keep them un-namespaced so the
            // same message in mimocode.db and mimocode-<channel>.db collapses.
            Some(id) => id,
            // Rowids are per-database: namespace to avoid false cross-DB merges.
            None => format!("{db_namespace}:{row_id}"),
        };
        let fingerprint = MiMoCodeSqliteFingerprint {
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
            "micode",
            model_id,
            provider_id,
            session_id,
            // `time.created` is epoch milliseconds (matching OpenCode);
            // UnifiedMessage's timestamp_to_date treats it as ms.
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
        unified.duration_ms = micode_duration_ms(&msg.time);
        unified.dedup_key = Some(dedup_key);
        let workspace_root = row_workspace_root
            .as_deref()
            .or(embedded_workspace_root.as_deref());
        set_workspace_from_root(&mut unified, workspace_root);

        if let Some(index) = fingerprint_indices.get(&fingerprint).copied() {
            let dedup_state = &mut dedup_states[index];
            if message_id.is_some() && !dedup_state.has_embedded_message_id {
                dedup_state.has_embedded_message_id = true;
                messages[index].dedup_key = unified.dedup_key;
            }
            merge_duplicate_workspace(&mut messages[index], dedup_state, workspace_root);
            continue;
        }

        dedup_states.push(MiMoCodeSqliteDedupState {
            has_embedded_message_id: message_id.is_some(),
            has_workspace_conflict: false,
        });
        fingerprint_indices.insert(fingerprint, messages.len());
        messages.push(unified);
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn create_micode_sqlite_db(db_path: &Path) -> Connection {
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
    fn test_parse_micode_sqlite_basic() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");

        let conn = create_micode_sqlite_db(&db_path);

        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 100,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000001234.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_001", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "micode");
        assert_eq!(messages[0].model_id, "mimo-v2.5-pro");
        assert_eq!(messages[0].provider_id, "mimo");
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[0].tokens.output, 500);
        assert_eq!(messages[0].tokens.reasoning, 100);
        assert_eq!(messages[0].tokens.cache_read, 200);
        assert_eq!(messages[0].tokens.cache_write, 50);
        assert!((messages[0].cost - 0.05).abs() < 1e-9);
        assert_eq!(messages[0].duration_ms, Some(1234));
    }

    #[test]
    fn test_parse_micode_sqlite_skips_user_messages() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");

        let conn = create_micode_sqlite_db(&db_path);

        let user_msg = r#"{
            "role": "user",
            "modelID": "mimo-v2.5-pro",
            "time": { "created": 1700000000000.0 }
        }"#;

        let assistant_msg = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "tokens": { "input": 100, "output": 50, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000001000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_user", "ses_001", user_msg],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_assistant", "ses_001", assistant_msg],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        // This message carries no embedded JSON id, so the dedup key falls back
        // to the SQLite row id and is namespaced by the database path.
        assert!(messages[0]
            .dedup_key
            .as_deref()
            .is_some_and(|key| key.ends_with(":msg_assistant")));
    }

    /// Regression: MiMo Code uses channel-suffixed databases (mimocode.db and
    /// mimocode-<channel>.db). A mid-session channel switch can write the SAME
    /// message (same embedded id) to both files. The embedded id must NOT be
    /// namespaced by the database, otherwise the cross-file dedup set produces
    /// two distinct keys and the message's cost + tokens get counted twice.
    #[test]
    fn embedded_message_id_is_not_namespaced_by_database() {
        let dir = tempfile::tempdir().unwrap();
        let db_a = dir.path().join("mimocode.db");
        let db_b = dir.path().join("mimocode-beta.db");
        // Embedded JSON "id" is the globally unique message id.
        let msg = r#"{
            "id": "msg_shared",
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000000.0 }
        }"#;
        // Different SQLite row ids prove the collapse is driven by the embedded
        // id (not the row id), exactly as a mid-session channel switch records.
        for (db, row_id) in [(&db_a, "row_a"), (&db_b, "row_b")] {
            let conn = create_micode_sqlite_db(db);
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params![row_id, "ses_1", msg],
            )
            .unwrap();
            drop(conn);
        }

        let a = parse_micode_sqlite(&db_a);
        let b = parse_micode_sqlite(&db_b);
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        // Same embedded id across both channel databases yields IDENTICAL,
        // un-namespaced dedup keys, so a shared dedup set collapses the
        // duplicate to a single count.
        assert_eq!(a[0].dedup_key, Some("msg_shared".to_string()));
        assert_eq!(b[0].dedup_key, Some("msg_shared".to_string()));

        // Prove the collapse end-to-end with the same HashSet logic used by the
        // cross-file aggregation in lib.rs.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let kept: Vec<_> = a
            .into_iter()
            .chain(b)
            .filter(|m| m.dedup_key.as_ref().is_none_or(|k| seen.insert(k.clone())))
            .collect();
        assert_eq!(kept.len(), 1, "shared embedded id must be counted once");
    }

    /// Two DIFFERENT messages that happen to share a SQLite rowid across two
    /// databases (rowids are per-database, not globally unique) must NOT be
    /// collapsed by the cross-file dedup set. The row-id fallback path is
    /// namespaced by database precisely to keep them distinct.
    #[test]
    fn rowid_fallback_is_namespaced_by_database() {
        let dir = tempfile::tempdir().unwrap();
        let db_a = dir.path().join("a.db");
        let db_b = dir.path().join("b.db");
        // No embedded "id" field -> the parser falls back to the SQLite rowid.
        let msg = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000000.0 }
        }"#;
        for db in [&db_a, &db_b] {
            let conn = create_micode_sqlite_db(db);
            // Same SQLite row id ("id" column) in both databases. With no
            // embedded JSON id, the parser falls back to this row id.
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params!["row_shared", "ses_1", msg],
            )
            .unwrap();
            drop(conn);
        }

        let a = parse_micode_sqlite(&db_a);
        let b = parse_micode_sqlite(&db_b);
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        // Same row id ("row_shared") in two databases must yield DISTINCT,
        // db-namespaced keys so the two unrelated messages are not merged.
        assert_ne!(a[0].dedup_key, b[0].dedup_key);
        assert!(a[0].dedup_key.as_deref().unwrap().ends_with(":row_shared"));
        assert!(b[0].dedup_key.as_deref().unwrap().ends_with(":row_shared"));

        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let kept: Vec<_> = a
            .into_iter()
            .chain(b)
            .filter(|m| m.dedup_key.as_ref().is_none_or(|k| seen.insert(k.clone())))
            .collect();
        assert_eq!(kept.len(), 2, "rowid collisions across DBs must stay distinct");
    }

    #[test]
    fn test_parse_micode_sqlite_negative_values_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");

        let conn = create_micode_sqlite_db(&db_path);

        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": -0.05,
            "tokens": {
                "input": -100,
                "output": -50,
                "reasoning": -25,
                "cache": { "read": -200, "write": -10 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_negative", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 0);
        assert_eq!(messages[0].tokens.output, 0);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.cache_write, 0);
        assert_eq!(messages[0].tokens.reasoning, 0);
        assert!(messages[0].cost >= 0.0);
    }

    #[test]
    fn test_parse_micode_sqlite_dedup_forked_history() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);

        let root_msg = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 25,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000000500.0 }
        }"#;

        let new_msg = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.08,
            "tokens": {
                "input": 1300,
                "output": 650,
                "reasoning": 40,
                "cache": { "read": 100, "write": 0 }
            },
            "time": { "created": 1700000001000.0, "completed": 1700000001500.0 }
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

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[1].tokens.input, 1300);
    }

    #[test]
    fn test_parse_micode_sqlite_workspace_from_session() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                directory TEXT NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory) VALUES (?1, ?2)",
            rusqlite::params!["ses_001", "/Users/alice/micode-repo"],
        )
        .unwrap();

        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
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
            rusqlite::params!["msg_ws", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/alice/micode-repo")
        );
        assert_eq!(messages[0].workspace_label.as_deref(), Some("micode-repo"));
    }

    #[test]
    fn test_parse_micode_sqlite_with_agent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);

        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "agent": "build",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 100,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_agent", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent, Some("Build".to_string()));
    }

    #[test]
    fn test_parse_micode_sqlite_missing_cache_defaults_to_zero() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);

        // Assistant payload with no `cache` object at all — must parse (not be
        // dropped) with cache tokens defaulting to 0.
        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 100
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_no_cache", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[0].tokens.output, 500);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.cache_write, 0);
    }
}
