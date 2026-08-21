//! MiMo Code session parser
//!
//! Parses messages from:
//! - SQLite database: ~/.local/share/mimocode/mimocode.db
//!
//! MiMo Code stores assistant turns in OpenCode's message schema, so the parse
//! itself lives in [`super::opencode_schema`]; only the places where MiMo's
//! behaviour departs from OpenCode's are declared here, as
//! [`OpenCodeSchemaConfig::micode`].

use super::opencode_schema::{parse_opencode_schema_sqlite, OpenCodeSchemaConfig};
use super::UnifiedMessage;
use std::path::Path;

pub fn parse_micode_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    parse_opencode_schema_sqlite(db_path, OpenCodeSchemaConfig::micode())
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
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );
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
        assert_eq!(
            kept.len(),
            2,
            "rowid collisions across DBs must stay distinct"
        );
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
    fn duplicate_explicit_zero_cost_upgrades_retained_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);
        let without_cost = r#"{
            "role": "assistant",
            "modelID": "unknown-model",
            "providerID": "mimo",
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000000.0 }
        }"#;
        let with_zero_cost = r#"{
            "role": "assistant",
            "modelID": "unknown-model",
            "providerID": "mimo",
            "cost": 0,
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000000.0 }
        }"#;
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["row_a", "session_a", without_cost],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["row_b", "session_b", with_zero_cost],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 0.0);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );
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

    /// Regression for PR #710: `time.created` was hard-assumed to be epoch
    /// milliseconds. If MiMo writes epoch *seconds*, the date landed ~1000x in
    /// the past (1970-era). A ms-valued and a seconds-valued `time.created` that
    /// denote the SAME instant must normalize to the same date and the same
    /// (millisecond-scale) timestamp. Without `micode_timestamp_to_ms`, the
    /// seconds variant would yield 1970-01-20 instead of 2023-11-14.
    #[test]
    fn test_parse_micode_sqlite_normalizes_seconds_and_milliseconds() {
        let dir = tempfile::tempdir().unwrap();
        let db_ms = dir.path().join("ms.db");
        let db_secs = dir.path().join("secs.db");

        // 1_700_000_000 s == 1_700_000_000_000 ms == 2023-11-14T22:13:20Z.
        let msg_ms = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000000.0, "completed": 1700000001234.0 }
        }"#;
        // Same instant, expressed in epoch SECONDS (the bugged-input shape).
        let msg_secs = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000.0, "completed": 1700000001.234 }
        }"#;

        for (db, data) in [(&db_ms, msg_ms), (&db_secs, msg_secs)] {
            let conn = create_micode_sqlite_db(db);
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params!["msg_1", "ses_1", data],
            )
            .unwrap();
            drop(conn);
        }

        let ms = parse_micode_sqlite(&db_ms);
        let secs = parse_micode_sqlite(&db_secs);
        assert_eq!(ms.len(), 1);
        assert_eq!(secs.len(), 1);

        // Both inputs resolve to the SAME instant: identical timestamp (ms) and
        // identical, non-empty (i.e. not 1970-era-then-formatted) date.
        assert_eq!(ms[0].timestamp, 1_700_000_000_000);
        assert_eq!(secs[0].timestamp, 1_700_000_000_000);
        assert_eq!(ms[0].date, secs[0].date);
        assert!(!ms[0].date.is_empty());

        // Duration is in milliseconds for BOTH representations (~1234 ms), not
        // ~1 (which is what the seconds input would have produced unnormalized).
        assert_eq!(ms[0].duration_ms, Some(1234));
        assert_eq!(secs[0].duration_ms, Some(1234));
    }

    /// A non-object `path` field (e.g. a bare string instead of `{ "root": .. }`)
    /// must not crash deserialization or fail the whole message: the custom
    /// `deserialize_micode_path` extracts `root` defensively, leaving it `None`.
    /// The message must still parse and have no embedded-path workspace.
    #[test]
    fn test_parse_micode_sqlite_non_object_path_field() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);

        // `path` is a string, not an object — the deserializer's `.get("root")`
        // returns None rather than erroring, so the message survives.
        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 100, "output": 50 },
            "path": "/some/string/not/an/object",
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_badpath", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            1,
            "non-object path must not drop the message"
        );
        assert_eq!(messages[0].tokens.input, 100);
        // No usable root -> no workspace derived from the embedded path.
        assert_eq!(messages[0].workspace_key, None);
        assert_eq!(messages[0].workspace_label, None);
    }

    /// Legacy-query fallback: when the database has no `session` table, the
    /// modern query (which JOINs `session`) fails to prepare and the parser
    /// falls back to `legacy_query`. In that path `workspace_root` from the row
    /// is NULL, so the workspace must come from the message's EMBEDDED `path.root`.
    #[test]
    fn test_parse_micode_sqlite_legacy_fallback_embedded_path_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        // Note: create_micode_sqlite_db creates ONLY the `message` table, so the
        // modern query's `LEFT JOIN session` cannot prepare and we exercise the
        // legacy fallback.
        let conn = create_micode_sqlite_db(&db_path);

        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 100, "output": 50 },
            "path": { "root": "/Users/bob/embedded-repo" },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_embedded", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        // Row workspace_root is NULL on the legacy path, so the embedded
        // `path.root` supplies the workspace.
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/bob/embedded-repo")
        );
        assert_eq!(
            messages[0].workspace_label.as_deref(),
            Some("embedded-repo")
        );
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
