mod common;

use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tokscale_core::scanner::{scan_all_clients_with_scanner_settings, ScannerSettings};
use tokscale_core::sessions::bb::parse_bb_sqlite;
use tokscale_core::{
    parse_local_clients, parse_local_unified_messages_with_pricing_uncached, ClientId,
    LocalParseOptions,
};

fn create_test_bb_db(path: &Path) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE threads (
            id TEXT PRIMARY KEY NOT NULL,
            project_id TEXT,
            provider_id TEXT NOT NULL,
            model_override TEXT,
            title TEXT,
            parent_thread_id TEXT,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE projects (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL
        );

        CREATE TABLE project_execution_defaults (
            project_id TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            model TEXT NOT NULL,
            service_tier TEXT NOT NULL
        );

        CREATE TABLE events (
            id TEXT PRIMARY KEY NOT NULL,
            thread_id TEXT NOT NULL,
            turn_id TEXT,
            sequence INTEGER NOT NULL,
            type TEXT NOT NULL,
            data TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        "#,
    )
    .unwrap();
    conn
}

fn setup_bb_home(home: &Path) -> PathBuf {
    let bb_dir = home.join(".bb");
    fs::create_dir_all(&bb_dir).unwrap();
    let db_path = bb_dir.join("bb.db");
    let conn = create_test_bb_db(&db_path);

    conn.execute(
        "INSERT INTO projects (id, name) VALUES (?1, ?2)",
        ("prj_bb", "my-project"),
    )
    .unwrap();

    conn.execute(
        "INSERT INTO project_execution_defaults (project_id, provider_id, model, service_tier) \
         VALUES (?1, ?2, ?3, ?4)",
        ("prj_bb", "claude-code", "claude-3-7-sonnet", "default"),
    )
    .unwrap();

    conn.execute(
        "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            "thr_root",
            "prj_bb",
            "claude-code",
            None::<String>,
            Some("Refactor auth system"),
            None::<String>,
            1_787_275_000_000_i64,
        ),
    )
    .unwrap();

    let event_1 = r#"{
        "tokenUsage": {
            "last": {
                "totalTokens": 5000,
                "inputTokens": 4000,
                "cachedInputTokens": 3000,
                "cacheReadInputTokens": 3000,
                "cacheWriteInputTokens": 0,
                "outputTokens": 1000,
                "reasoningOutputTokens": 0
            }
        }
    }"#;

    let event_2 = r#"{
        "tokenUsage": {
            "last": {
                "totalTokens": 6000,
                "inputTokens": 4500,
                "cachedInputTokens": 3500,
                "cacheReadInputTokens": 3000,
                "cacheWriteInputTokens": 500,
                "outputTokens": 1500,
                "reasoningOutputTokens": 0
            },
            "promptCacheTtlMs": 3600000
        }
    }"#;

    conn.execute(
        "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            "evt_turn1",
            "thr_root",
            Some("turn_1"),
            1,
            "thread/tokenUsage/updated",
            event_1,
            1_787_275_010_000_i64,
        ),
    )
    .unwrap();

    conn.execute(
        "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            "evt_turn2",
            "thr_root",
            Some("turn_2"),
            2,
            "thread/tokenUsage/updated",
            event_2,
            1_787_275_020_000_i64,
        ),
    )
    .unwrap();

    db_path
}

fn bb_options(home: &Path) -> LocalParseOptions {
    LocalParseOptions {
        home_dir: Some(home.to_str().unwrap().to_string()),
        use_env_roots: false,
        clients: Some(vec!["bb".to_string()]),
        since: None,
        until: None,
        year: None,
        scanner_settings: ScannerSettings::default(),
    }
}

#[test]
fn test_scanner_discovers_bb_db() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    let db_path = setup_bb_home(home);

    let scan = scan_all_clients_with_scanner_settings(
        home.to_str().unwrap(),
        &["bb".to_string()],
        false,
        &ScannerSettings::default(),
    );

    assert_eq!(scan.bb_dbs, vec![db_path]);
}

#[test]
fn test_parse_bb_sqlite_directly() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    let db_path = setup_bb_home(home);

    let messages = parse_bb_sqlite(&db_path);
    assert_eq!(messages.len(), 2);

    let msg1 = &messages[0];
    assert_eq!(msg1.client, "bb");
    assert_eq!(msg1.provider_id, "anthropic");
    assert_eq!(msg1.model_id, "claude-3-7-sonnet");
    assert_eq!(msg1.session_id, "thr_root");
    assert_eq!(msg1.workspace_key.as_deref(), Some("prj_bb"));
    assert_eq!(msg1.workspace_label.as_deref(), Some("my-project"));
    assert_eq!(msg1.session_title.as_deref(), Some("Refactor auth system"));
    assert_eq!(msg1.tokens.input, 1000);
    assert_eq!(msg1.tokens.output, 1000);
    assert_eq!(msg1.tokens.cache_read, 3000);
    assert_eq!(msg1.tokens.cache_write, 0);
    assert_eq!(msg1.tokens.total(), 5000);

    let msg2 = &messages[1];
    assert_eq!(msg2.tokens.input, 1000);
    assert_eq!(msg2.tokens.output, 1500);
    assert_eq!(msg2.tokens.cache_read, 3000);
    assert_eq!(msg2.tokens.cache_write, 500);
    assert_eq!(msg2.tokens.cache_write_1h, 500);
    assert_eq!(msg2.tokens.total(), 6000);
}

#[tokio::test]
async fn test_local_parse_bb_cached_and_uncached() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    setup_bb_home(home);

    let parsed = parse_local_clients(bb_options(home)).unwrap();
    assert_eq!(parsed.counts.get(ClientId::Bb), 2);
    assert_eq!(parsed.messages.len(), 2);

    let uncached = parse_local_unified_messages_with_pricing_uncached(bb_options(home), None)
        .await
        .unwrap();
    assert_eq!(uncached.len(), 2);
}
