mod common;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serial_test::serial;
use tokscale_core::pricing::{litellm::ModelPricing, PricingService};
use tokscale_core::scanner::ScannerSettings;
use tokscale_core::{
    parse_local_clients, parse_local_unified_messages_with_pricing_uncached, ClientId,
    LocalParseOptions,
};

/// The schema Broca's run index uses for its metering export.
const EXPORT_FACTS_SCHEMA: &str = "CREATE TABLE export_facts (
    export_seq INTEGER PRIMARY KEY AUTOINCREMENT,
    fact_id TEXT NOT NULL UNIQUE,
    session TEXT NOT NULL,
    run_id TEXT NOT NULL,
    segment_json TEXT NOT NULL
);";

fn create_run_index(state_root: &Path) -> (PathBuf, Connection) {
    fs::create_dir_all(state_root).unwrap();
    let db_path = state_root.join("run-index.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(EXPORT_FACTS_SCHEMA).unwrap();
    (db_path, conn)
}

fn insert_fact(conn: &Connection, fact_id: &str, session: &str, segment_json: &str) {
    conn.execute(
        "INSERT INTO export_facts (fact_id, session, run_id, segment_json)
         VALUES (?1, ?2, ?3, ?4)",
        params![fact_id, session, format!("run-{fact_id}"), segment_json],
    )
    .unwrap();
}

fn segment(fact_id: &str, session: &str, project_root: &str, occurred_at_ms: i64) -> String {
    serde_json::json!({
        "fact_id": fact_id,
        "session": {
            "project_root": project_root,
            "harness": "runner",
            "session": session
        },
        "provider": "anthropic",
        "model": "claude-opus-5-5",
        "usage": {
            "input_tokens": 120,
            "cached_input_tokens": 4800,
            "cache_write_tokens": 300,
            "output_tokens": 900,
            "reasoning_tokens": 250
        },
        "occurred_at_ms": occurred_at_ms
    })
    .to_string()
}

fn broca_options(home: &Path, use_env_roots: bool) -> LocalParseOptions {
    LocalParseOptions {
        home_dir: Some(home.to_str().unwrap().to_string()),
        use_env_roots,
        clients: Some(vec!["broca".to_string()]),
        since: None,
        until: None,
        year: None,
        scanner_settings: ScannerSettings::default(),
    }
}

fn make_pricing_service() -> PricingService {
    let mut litellm_data = HashMap::new();
    litellm_data.insert(
        "claude-opus-5-5".to_string(),
        ModelPricing {
            input_cost_per_token: Some(0.000004),
            output_cost_per_token: Some(0.00002),
            cache_read_input_token_cost: Some(0.0000002),
            cache_creation_input_token_cost: Some(0.000005),
            ..Default::default()
        },
    );
    PricingService::new(litellm_data, HashMap::new())
}

/// Discovery from the home fallback through parsing and pricing. Malformed and
/// untimestamped facts are skipped without losing the valid ones around them,
/// and Broca's reasoning subset is split out of output so no token is counted
/// twice.
#[tokio::test]
async fn test_broca_end_to_end_maps_disjoint_buckets_and_skips_bad_facts() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let (_db_path, conn) = create_run_index(&home.join(".local/share/cortexkit/broca"));

    insert_fact(
        &conn,
        "fact-a",
        "alfonso:main",
        &segment("fact-a", "alfonso:main", "/work/alpha", 1_786_181_660_799),
    );
    insert_fact(&conn, "fact-broken", "alfonso:main", "{not json");
    let mut untimestamped: serde_json::Value =
        serde_json::from_str(&segment("fact-no-time", "alfonso:main", "/work/alpha", 1)).unwrap();
    untimestamped["occurred_at_ms"] = serde_json::Value::Null;
    insert_fact(
        &conn,
        "fact-no-time",
        "alfonso:main",
        &untimestamped.to_string(),
    );
    insert_fact(
        &conn,
        "fact-b",
        "alfonso:review",
        &segment("fact-b", "alfonso:review", "/work/beta", 1_786_181_700_000),
    );
    drop(conn);

    let pricing = make_pricing_service();
    let mut messages = parse_local_unified_messages_with_pricing_uncached(
        broca_options(home, false),
        Some(&pricing),
    )
    .await
    .unwrap();
    messages.sort_by_key(|message| message.timestamp);

    assert_eq!(
        messages.len(),
        2,
        "only the two well-formed, dated facts count"
    );

    let first = &messages[0];
    assert_eq!(first.client, "broca");
    assert_eq!(first.session_id, "alfonso:main");
    assert_eq!(first.model_id, "claude-opus-5-5");
    assert_eq!(first.provider_id, "anthropic");
    assert_eq!(first.timestamp, 1_786_181_660_799);
    assert_eq!(first.dedup_key.as_deref(), Some("fact-a"));
    assert_eq!(first.workspace_key.as_deref(), Some("/work/alpha"));
    assert_eq!(first.tokens.input, 120);
    assert_eq!(first.tokens.cache_read, 4800);
    assert_eq!(first.tokens.cache_write, 300);
    assert_eq!(first.tokens.cache_write_1h, 0);
    assert_eq!(
        first.tokens.output, 650,
        "900 output minus the 250 reasoning subset"
    );
    assert_eq!(first.tokens.reasoning, 250);
    assert_eq!(first.tokens.total(), 6120);
    assert!(
        first.cost > 0.0,
        "the fact must be priced from the catalog row"
    );

    let second = &messages[1];
    assert_eq!(second.session_id, "alfonso:review");
    assert_eq!(second.timestamp, 1_786_181_700_000);
    assert_eq!(second.dedup_key.as_deref(), Some("fact-b"));
    assert_eq!(second.workspace_key.as_deref(), Some("/work/beta"));

    let parsed = parse_local_clients(broca_options(home, false)).unwrap();
    assert_eq!(parsed.counts.get(ClientId::Broca), 2);
    assert_eq!(parsed.messages.len(), 2);
}

/// `BROCA_STATE_ROOT` moves the run index outside the home directory. With
/// environment roots enabled it must win over the home fallback, which here
/// holds a different database that must not be read.
#[test]
#[serial]
fn test_broca_state_root_env_overrides_home_fallback() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let state_dir = common::temp_home();

    let (_fallback_db, fallback) = create_run_index(&home.join(".local/share/cortexkit/broca"));
    insert_fact(
        &fallback,
        "fact-fallback",
        "alfonso:fallback",
        &segment(
            "fact-fallback",
            "alfonso:fallback",
            "/work/fallback",
            1_786_181_660_799,
        ),
    );
    drop(fallback);

    let (_env_db, env_conn) = create_run_index(state_dir.path());
    insert_fact(
        &env_conn,
        "fact-env",
        "alfonso:env",
        &segment("fact-env", "alfonso:env", "/work/env", 1_786_181_660_799),
    );
    drop(env_conn);

    let _guard = common::EnvGuard::set(&[("BROCA_STATE_ROOT", state_dir.path().as_os_str())]);

    let from_env = parse_local_clients(broca_options(home, true)).unwrap();
    assert_eq!(from_env.messages.len(), 1);
    assert_eq!(from_env.messages[0].session_id, "alfonso:env");

    let from_home = parse_local_clients(broca_options(home, false)).unwrap();
    assert_eq!(from_home.messages.len(), 1);
    assert_eq!(from_home.messages[0].session_id, "alfonso:fallback");
}
