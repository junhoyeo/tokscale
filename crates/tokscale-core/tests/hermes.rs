use rusqlite::{params, Connection};
use std::collections::HashSet;
use tempfile::TempDir;
use tokscale_core::bucket_tz::BucketTimezone;
use tokscale_core::sessions::hermes::{
    message_is_day_split, parse_hermes_sqlite, parse_hermes_sqlite_in,
};

#[test]
fn test_parse_hermes_sqlite_returns_empty_for_missing_database() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing.db");
    assert!(parse_hermes_sqlite(&missing).is_empty());
}

fn create_test_db(dir: &TempDir) -> std::path::PathBuf {
    let db_path = dir.path().join("state.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE sessions (
            id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            model TEXT,
            started_at REAL NOT NULL,
            message_count INTEGER DEFAULT 0,
            input_tokens INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            cache_read_tokens INTEGER DEFAULT 0,
            cache_write_tokens INTEGER DEFAULT 0,
            reasoning_tokens INTEGER DEFAULT 0,
            billing_provider TEXT,
            estimated_cost_usd REAL,
            actual_cost_usd REAL
        );
        CREATE TABLE session_model_usage (
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            model TEXT NOT NULL,
            billing_provider TEXT NOT NULL DEFAULT '',
            billing_base_url TEXT NOT NULL DEFAULT '',
            billing_mode TEXT NOT NULL DEFAULT '',
            task TEXT NOT NULL DEFAULT '',
            api_call_count INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            estimated_cost_usd REAL NOT NULL DEFAULT 0,
            actual_cost_usd REAL NOT NULL DEFAULT 0,
            cost_status TEXT,
            cost_source TEXT,
            first_seen REAL,
            last_seen REAL,
            PRIMARY KEY (session_id, model, billing_provider, billing_base_url, billing_mode, task)
        );
        "#,
    )
    .unwrap();
    db_path
}

/// Schema of a Hermes build whose `session_model_usage` predates the
/// `reasoning_tokens` column, so `PER_MODEL_QUERY` cannot even be prepared.
fn create_drifted_test_db(dir: &TempDir) -> std::path::PathBuf {
    let db_path = dir.path().join("drifted-state.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE sessions (
            id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            model TEXT,
            started_at REAL NOT NULL,
            message_count INTEGER DEFAULT 0,
            input_tokens INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            cache_read_tokens INTEGER DEFAULT 0,
            cache_write_tokens INTEGER DEFAULT 0,
            reasoning_tokens INTEGER DEFAULT 0,
            billing_provider TEXT,
            estimated_cost_usd REAL,
            actual_cost_usd REAL
        );
        CREATE TABLE session_model_usage (
            session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            model TEXT NOT NULL,
            billing_provider TEXT NOT NULL DEFAULT '',
            billing_base_url TEXT NOT NULL DEFAULT '',
            billing_mode TEXT NOT NULL DEFAULT '',
            task TEXT NOT NULL DEFAULT '',
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            estimated_cost_usd REAL NOT NULL DEFAULT 0,
            actual_cost_usd REAL NOT NULL DEFAULT 0,
            PRIMARY KEY (session_id, model, billing_provider, billing_base_url, billing_mode, task)
        );
        "#,
    )
    .unwrap();
    db_path
}

/// Schema of a Hermes build that predates `session_model_usage`.
fn create_legacy_test_db(dir: &TempDir) -> std::path::PathBuf {
    let db_path = dir.path().join("legacy-state.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE sessions (
            id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            model TEXT,
            started_at REAL NOT NULL,
            message_count INTEGER DEFAULT 0,
            input_tokens INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            cache_read_tokens INTEGER DEFAULT 0,
            cache_write_tokens INTEGER DEFAULT 0,
            reasoning_tokens INTEGER DEFAULT 0,
            billing_provider TEXT,
            estimated_cost_usd REAL,
            actual_cost_usd REAL
        );
        "#,
    )
    .unwrap();
    db_path
}

#[test]
fn test_parse_hermes_sqlite_reads_session_rows_and_preserves_message_count() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    // Session row (needed for JOIN: started_at, message_count, model)
    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "session-1",
            "cli",
            "claude-sonnet-4",
            1_750_000_000.25_f64,
            42_i64,
        ],
    )
    .unwrap();

    // Per-model token data
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-1",
            "claude-sonnet-4",
            "anthropic",
            "",
            "",
            "",
            1200_i64,
            300_i64,
            50_i64,
            20_i64,
            10_i64,
            0.12_f64,
            0.34_f64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);

    let msg = &messages[0];
    assert_eq!(msg.client, "hermes");
    assert_eq!(msg.agent.as_deref(), Some("Hermes Agent"));
    assert_eq!(msg.session_id, "session-1");
    assert_eq!(msg.model_id, "claude-sonnet-4");
    assert_eq!(msg.provider_id, "anthropic");
    assert_eq!(msg.timestamp, 1_750_000_000_250_i64);
    assert_eq!(msg.message_count, 42);
    assert_eq!(msg.tokens.input, 1200);
    // `output_tokens` is reasoning-inclusive in Hermes, so the additive output
    // bucket keeps only the non-reasoning remainder.
    assert_eq!(msg.tokens.output, 290);
    assert_eq!(msg.tokens.cache_read, 50);
    assert_eq!(msg.tokens.cache_write, 20);
    assert_eq!(msg.tokens.reasoning, 10);
    assert_eq!(msg.cost, 0.34);
    assert_eq!(
        msg.dedup_key.as_deref(),
        Some("hermes:session-1:claude-sonnet-4:anthropic")
    );
}

/// Hermes records `completion_tokens_details.reasoning_tokens` inside
/// `output_tokens`, but `TokenBreakdown` buckets are additive and `compute_cost`
/// prices output and reasoning at the same output rate. Mapping both fields
/// through unmodified would bill every reasoning token twice, so the reasoning
/// overlap must be split out of the output bucket without changing the row's
/// token total.
#[test]
fn test_parse_hermes_sqlite_bills_reasoning_once() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "session-reasoning",
            "cli",
            "deepseek-flash",
            1_775_002_000.0_f64,
            1_i64,
        ],
    )
    .unwrap();
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-reasoning",
            "deepseek-flash",
            "deepseek",
            "",
            "",
            "",
            1_000_i64,
            400_i64,
            50_i64,
            20_i64,
            150_i64,
            0.0_f64,
            0.0_f64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);

    let msg = &messages[0];
    assert_eq!(msg.tokens.output, 250);
    assert_eq!(msg.tokens.reasoning, 150);
    // The split must not lose tokens: additive buckets still sum to the raw
    // input + reasoning-inclusive output + cache.
    assert_eq!(msg.tokens.total(), 1_000 + 400 + 50 + 20);
}

/// A malformed row claiming more reasoning than output must not drive the
/// additive output bucket negative.
#[test]
fn test_parse_hermes_sqlite_clamps_reasoning_to_output() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "session-clamped",
            "cli",
            "deepseek-flash",
            1_775_002_000.0_f64,
            1_i64,
        ],
    )
    .unwrap();
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-clamped",
            "deepseek-flash",
            "deepseek",
            "",
            "",
            "",
            10_i64,
            30_i64,
            0_i64,
            0_i64,
            90_i64,
            0.0_f64,
            0.0_f64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);

    let msg = &messages[0];
    assert_eq!(msg.tokens.output, 0);
    assert_eq!(msg.tokens.reasoning, 30);
    assert_eq!(msg.tokens.total(), 10 + 30);
}

#[test]
fn test_parse_hermes_sqlite_skips_empty_sessions_and_falls_back_to_estimated_cost_and_provider_inference(
) {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    // Valid session with tokens
    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "session-valid",
            "telegram",
            "gpt-5.4",
            1_775_001_102.0_f64,
            3_i64,
        ],
    )
    .unwrap();
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-valid",
            "gpt-5.4",
            "",
            "",
            "",
            "",
            100_i64,
            20_i64,
            0_i64,
            0_i64,
            5_i64,
            1.25_f64,
            0.0_f64,
        ],
    )
    .unwrap();

    // Empty session: no smu row, and no session totals either, so neither the
    // per-model pass nor the session-totals reconciliation emits it.
    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "session-empty",
            "telegram",
            "gpt-5.4",
            1_775_001_103.0_f64,
            9_i64,
        ],
    )
    .unwrap();

    // Session with no model — should be excluded by WHERE
    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "session-no-model",
            "cli",
            Option::<String>::None,
            1_775_001_104.0_f64,
            1_i64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);

    let msg = &messages[0];
    assert_eq!(msg.session_id, "session-valid");
    assert_eq!(msg.provider_id, "openai");
    assert_eq!(msg.cost, 1.25);
    assert_eq!(msg.message_count, 3);
}

#[test]
fn test_parse_hermes_sqlite_ignores_unknown_billing_provider_and_falls_back_to_model_inference() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "session-unknown-provider",
            "cli",
            "gpt-5.4",
            1_775_001_105.0_f64,
            2_i64,
        ],
    )
    .unwrap();

    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-unknown-provider",
            "gpt-5.4",
            "unknown",
            "",
            "",
            "",
            100_i64,
            20_i64,
            0_i64,
            0_i64,
            0_i64,
            0.5_f64,
            0.0_f64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].provider_id, "openai");
}

#[test]
fn test_parse_hermes_sqlite_emits_per_model_rows_for_multi_model_session() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    // Single session with primary model "glm-5.2" and two smu models
    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "session-multi",
            "desktop",
            "glm-5.2",
            1_775_002_000.0_f64,
            15_i64,
        ],
    )
    .unwrap();

    // Primary model (matches sessions.model) — should get message_count
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-multi",
            "glm-5.2",
            "kilocode",
            "",
            "",
            "",
            5000_i64,
            800_i64,
            100_i64,
            0_i64,
            200_i64,
            0.0_f64,
            0.0_f64,
        ],
    )
    .unwrap();

    // Secondary model — should get message_count = 0
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-multi",
            "mimo-v2.5-free",
            "opencode-zen",
            "",
            "",
            "",
            3000_i64,
            500_i64,
            0_i64,
            0_i64,
            0_i64,
            0.0_f64,
            0.0_f64,
        ],
    )
    .unwrap();

    // Third model with two task-variant rows — should be SUMmed
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-multi",
            "cohere/north-mini-code:free",
            "bifrost",
            "",
            "",
            "",
            900_i64,
            100_i64,
            0_i64,
            0_i64,
            0_i64,
            0.0_f64,
            0.0_f64,
        ],
    )
    .unwrap();
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-multi",
            "cohere/north-mini-code:free",
            "bifrost",
            "",
            "",
            "title_generation",
            100_i64,
            50_i64,
            0_i64,
            0_i64,
            0_i64,
            0.0_f64,
            0.0_f64,
        ],
    )
    .unwrap();

    let mut messages = parse_hermes_sqlite(&db_path);
    messages.sort_by(|a, b| a.model_id.cmp(&b.model_id));

    assert_eq!(messages.len(), 3);

    // cohere/north-mini-code:free: SUM of 2 rows (900+100=1000 input, 100+50=150 output)
    let cohere = messages
        .iter()
        .find(|m| m.model_id == "cohere/north-mini-code:free")
        .unwrap();
    assert_eq!(cohere.tokens.input, 1000);
    assert_eq!(cohere.tokens.output, 150);
    assert_eq!(cohere.message_count, 0); // secondary model
    assert_eq!(
        cohere.dedup_key.as_deref(),
        Some("hermes:session-multi:cohere/north-mini-code:free:bifrost")
    );

    // glm-5.2: primary model, gets message_count
    let glm = messages.iter().find(|m| m.model_id == "glm-5.2").unwrap();
    assert_eq!(glm.tokens.input, 5000);
    assert_eq!(glm.tokens.output, 600);
    assert_eq!(glm.tokens.reasoning, 200);
    assert_eq!(glm.message_count, 15); // primary model
    assert_eq!(
        glm.dedup_key.as_deref(),
        Some("hermes:session-multi:glm-5.2:kilocode")
    );

    // mimo-v2.5-free: secondary model
    let mimo = messages
        .iter()
        .find(|m| m.model_id == "mimo-v2.5-free")
        .unwrap();
    assert_eq!(mimo.tokens.input, 3000);
    assert_eq!(mimo.tokens.output, 500);
    assert_eq!(mimo.message_count, 0); // secondary model
    assert_eq!(
        mimo.dedup_key.as_deref(),
        Some("hermes:session-multi:mimo-v2.5-free:opencode-zen")
    );
}

#[test]
fn test_parse_hermes_sqlite_falls_back_to_session_totals_without_session_model_usage() {
    let dir = TempDir::new().unwrap();
    let db_path = create_legacy_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            billing_provider, estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "legacy-session",
            "cli",
            "claude-sonnet-4",
            1_750_000_000.25_f64,
            42_i64,
            1200_i64,
            300_i64,
            50_i64,
            20_i64,
            10_i64,
            "anthropic",
            0.12_f64,
            0.34_f64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);

    let msg = &messages[0];
    assert_eq!(msg.client, "hermes");
    assert_eq!(msg.agent.as_deref(), Some("Hermes Agent"));
    assert_eq!(msg.session_id, "legacy-session");
    assert_eq!(msg.model_id, "claude-sonnet-4");
    assert_eq!(msg.provider_id, "anthropic");
    assert_eq!(msg.timestamp, 1_750_000_000_250_i64);
    assert_eq!(msg.message_count, 42);
    assert_eq!(msg.tokens.input, 1200);
    assert_eq!(msg.tokens.output, 290);
    assert_eq!(msg.tokens.cache_read, 50);
    assert_eq!(msg.tokens.cache_write, 20);
    assert_eq!(msg.tokens.reasoning, 10);
    assert_eq!(msg.cost, 0.34);
    // The session-totals path emits one message per session, so it keeps the
    // bare session id as the dedup key.
    assert_eq!(msg.dedup_key.as_deref(), Some("legacy-session"));
}

#[test]
fn test_parse_hermes_sqlite_legacy_skips_empty_sessions_and_falls_back_to_estimated_cost() {
    let dir = TempDir::new().unwrap();
    let db_path = create_legacy_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    // Valid session with tokens but no actual cost and no billing provider
    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count,
            input_tokens, output_tokens, reasoning_tokens,
            billing_provider, estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            "legacy-valid",
            "telegram",
            "gpt-5.4",
            1_775_001_102.0_f64,
            3_i64,
            100_i64,
            20_i64,
            5_i64,
            Option::<String>::None,
            1.25_f64,
            Option::<f64>::None,
        ],
    )
    .unwrap();

    // Session with no usage at all — should be excluded by WHERE
    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "legacy-empty",
            "telegram",
            "gpt-5.4",
            1_775_001_103.0_f64,
            9_i64,
        ],
    )
    .unwrap();

    // Session with no model — should be excluded by WHERE
    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count, input_tokens
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            "legacy-no-model",
            "cli",
            Option::<String>::None,
            1_775_001_104.0_f64,
            1_i64,
            500_i64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);

    let msg = &messages[0];
    assert_eq!(msg.session_id, "legacy-valid");
    assert_eq!(msg.provider_id, "openai");
    assert_eq!(msg.cost, 1.25);
    assert_eq!(msg.message_count, 3);
    assert_eq!(msg.dedup_key.as_deref(), Some("legacy-valid"));
}

/// A Hermes upgrade can create `session_model_usage` without backfilling
/// existing sessions. Those sessions have real usage on `sessions` and no smu
/// child row, so a table-level probe plus an inner JOIN drops them entirely.
#[test]
fn test_parse_hermes_sqlite_emits_session_totals_for_sessions_without_smu_rows() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    // Backfilled session: has smu rows, so smu is authoritative for it.
    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count,
            input_tokens, output_tokens, billing_provider, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        params![
            "session-backfilled",
            "cli",
            "claude-sonnet-4",
            1_775_003_000.0_f64,
            5_i64,
            1200_i64,
            300_i64,
            "anthropic",
            0.10_f64,
        ],
    )
    .unwrap();
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-backfilled",
            "claude-sonnet-4",
            "anthropic",
            "",
            "",
            "",
            1200_i64,
            300_i64,
            0_i64,
            0_i64,
            0_i64,
            0.0_f64,
            0.10_f64,
        ],
    )
    .unwrap();

    // Pre-upgrade session: substantial usage, no smu child row at all.
    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count,
            input_tokens, output_tokens, billing_provider, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        params![
            "session-not-backfilled",
            "desktop",
            "claude-opus-4-6",
            1_775_003_001.0_f64,
            42_i64,
            999_000_i64,
            12_000_i64,
            "anthropic",
            2.50_f64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 2);

    // The backfilled session is emitted exactly once — the two passes are
    // disjoint, so it is not counted by both.
    let backfilled: Vec<_> = messages
        .iter()
        .filter(|m| m.session_id == "session-backfilled")
        .collect();
    assert_eq!(backfilled.len(), 1);
    assert_eq!(backfilled[0].tokens.input, 1200);
    assert_eq!(
        backfilled[0].dedup_key.as_deref(),
        Some("hermes:session-backfilled:claude-sonnet-4:anthropic")
    );

    let recovered = messages
        .iter()
        .find(|m| m.session_id == "session-not-backfilled")
        .expect("session without smu rows must still be reported");
    assert_eq!(recovered.model_id, "claude-opus-4-6");
    assert_eq!(recovered.provider_id, "anthropic");
    assert_eq!(recovered.tokens.input, 999_000);
    assert_eq!(recovered.tokens.output, 12_000);
    assert_eq!(recovered.cost, 2.50);
    assert_eq!(recovered.message_count, 42);
    assert_eq!(
        recovered.dedup_key.as_deref(),
        Some("session-not-backfilled")
    );
}

/// A session with *some* smu coverage is reported from smu alone. Topping it up
/// from the session totals would double count whatever smu already recorded.
#[test]
fn test_parse_hermes_sqlite_does_not_top_up_partially_covered_sessions() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count, input_tokens, output_tokens
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        params![
            "session-partial",
            "cli",
            "glm-5.2",
            1_775_003_100.0_f64,
            9_i64,
            5000_i64,
            900_i64,
        ],
    )
    .unwrap();
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-partial",
            "glm-5.2",
            "kilocode",
            "",
            "",
            "",
            3000_i64,
            400_i64,
            0_i64,
            0_i64,
            0_i64,
            0.0_f64,
            0.0_f64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].tokens.input, 3000);
    assert_eq!(messages[0].tokens.output, 400);
}

/// Schema drift (here: a `session_model_usage` without `reasoning_tokens`)
/// makes the per-model query fail to prepare. That must degrade to the session
/// totals, not to reporting nothing at all.
#[test]
fn test_parse_hermes_sqlite_falls_back_to_session_totals_when_per_model_query_cannot_prepare() {
    let dir = TempDir::new().unwrap();
    let db_path = create_drifted_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            billing_provider, estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "drifted-session",
            "cli",
            "claude-sonnet-4",
            1_750_000_000.25_f64,
            42_i64,
            1200_i64,
            300_i64,
            50_i64,
            20_i64,
            10_i64,
            "anthropic",
            0.12_f64,
            0.34_f64,
        ],
    )
    .unwrap();
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            "drifted-session",
            "claude-sonnet-4",
            "anthropic",
            "",
            "",
            "",
            1200_i64,
            300_i64,
            50_i64,
            20_i64,
            0.12_f64,
            0.34_f64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);

    let msg = &messages[0];
    assert_eq!(msg.session_id, "drifted-session");
    assert_eq!(msg.tokens.input, 1200);
    assert_eq!(msg.tokens.output, 290);
    assert_eq!(msg.tokens.reasoning, 10);
    assert_eq!(msg.cost, 0.34);
    assert_eq!(msg.message_count, 42);
    assert_eq!(msg.dedup_key.as_deref(), Some("drifted-session"));
}

/// The composite primary key lets one (session, model) span several billing
/// providers. Collapsing them would credit every token to one provider and hide
/// the other from the provider breakdown.
#[test]
fn test_parse_hermes_sqlite_keeps_billing_providers_separate_for_same_model() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "session-split",
            "desktop",
            "glm-5.2",
            1_775_004_000.0_f64,
            7_i64,
        ],
    )
    .unwrap();

    for (provider, input) in [("kilocode", 1000_i64), ("opencode-zen", 2000_i64)] {
        conn.execute(
            r#"
            INSERT INTO session_model_usage (
                session_id, model, billing_provider, billing_base_url, billing_mode, task,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
                estimated_cost_usd, actual_cost_usd
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            params![
                "session-split",
                "glm-5.2",
                provider,
                "",
                "",
                "",
                input,
                0_i64,
                0_i64,
                0_i64,
                0_i64,
                0.0_f64,
                0.0_f64,
            ],
        )
        .unwrap();
    }

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 2);

    let kilocode = messages
        .iter()
        .find(|m| m.provider_id == "kilocode")
        .expect("kilocode must survive the grouping");
    let opencode = messages
        .iter()
        .find(|m| m.provider_id == "opencode_zen")
        .expect("opencode-zen must survive the grouping");

    assert_eq!(kilocode.tokens.input, 1000);
    assert_eq!(opencode.tokens.input, 2000);
    assert_eq!(
        kilocode.dedup_key.as_deref(),
        Some("hermes:session-split:glm-5.2:kilocode")
    );
    assert_eq!(
        opencode.dedup_key.as_deref(),
        Some("hermes:session-split:glm-5.2:opencode-zen")
    );

    // `sessions.message_count` is a per-session total, so splitting the primary
    // model across providers must not credit it twice.
    assert_eq!(
        kilocode.message_count + opencode.message_count,
        7,
        "session message_count must be credited exactly once"
    );
}

/// Cost is resolved per row before summing: a row with a reconciled actual cost
/// must not discard the estimated cost of its task-variant siblings.
#[test]
fn test_parse_hermes_sqlite_sums_actual_and_estimated_costs_per_row() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params!["session-cost", "cli", "gpt-5.4", 1_775_005_000.0_f64, 4_i64,],
    )
    .unwrap();

    // Reconciled row: actual cost known.
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-cost",
            "gpt-5.4",
            "openai",
            "",
            "",
            "",
            1000_i64,
            200_i64,
            0_i64,
            0_i64,
            0_i64,
            0.0_f64,
            0.50_f64,
        ],
    )
    .unwrap();

    // Task-variant sibling: only an estimate so far.
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-cost",
            "gpt-5.4",
            "openai",
            "",
            "",
            "title_generation",
            100_i64,
            20_i64,
            0_i64,
            0_i64,
            0_i64,
            0.05_f64,
            0.0_f64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);
    assert!(
        (messages[0].cost - 0.55).abs() < 1e-9,
        "expected 0.50 actual + 0.05 estimated, got {}",
        messages[0].cost
    );
    assert_eq!(messages[0].tokens.input, 1100);
}

/// A group whose only signal is cost still has to be emitted: the HAVING clause
/// and the projection must agree on how cost is computed.
#[test]
fn test_parse_hermes_sqlite_emits_cost_only_rows() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "session-cost-only",
            "cli",
            "gpt-5.4",
            1_775_006_000.0_f64,
            1_i64,
        ],
    )
    .unwrap();
    conn.execute(
        r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            "session-cost-only",
            "gpt-5.4",
            "openai",
            "",
            "",
            "",
            0_i64,
            0_i64,
            0_i64,
            0_i64,
            0_i64,
            0.07_f64,
            0.0_f64,
        ],
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);
    assert!((messages[0].cost - 0.07).abs() < 1e-9);
}

/// `sessions.message_count` is credited to the row matching `sessions.model`,
/// but nothing guarantees such a row exists — the session can have no model, or
/// name one that `session_model_usage` never recorded. The count still belongs
/// to the session, so a deterministic stand-in row has to carry it.
#[test]
fn test_parse_hermes_sqlite_keeps_message_count_when_no_row_matches_session_model() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source, model, started_at, message_count
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
        params![
            "session-orphan-count",
            "cli",
            "glm-5.2",
            1_775_007_000.0_f64,
            11_i64,
        ],
    )
    .unwrap();

    for (model, input) in [("gpt-5.4", 2000_i64), ("mimo-v2.5-free", 1000_i64)] {
        conn.execute(
            r#"
            INSERT INTO session_model_usage (
                session_id, model, billing_provider, billing_base_url, billing_mode, task,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
                estimated_cost_usd, actual_cost_usd
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            params![
                "session-orphan-count",
                model,
                "openai",
                "",
                "",
                "",
                input,
                0_i64,
                0_i64,
                0_i64,
                0_i64,
                0.0_f64,
                0.0_f64,
            ],
        )
        .unwrap();
    }

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages.iter().map(|m| m.message_count).sum::<i32>(),
        11,
        "session message_count must survive, and exactly once"
    );
}

/// A nullable `billing_provider` lets one (session, model) group twice — once
/// under NULL, once under ''. Both rows carry real tokens, so their dedup keys
/// must differ or the caller's dedup pass drops one of them.
#[test]
fn test_parse_hermes_sqlite_separates_null_and_empty_billing_providers() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("nullable-provider.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE sessions (
            id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            model TEXT,
            started_at REAL NOT NULL,
            message_count INTEGER DEFAULT 0,
            input_tokens INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            cache_read_tokens INTEGER DEFAULT 0,
            cache_write_tokens INTEGER DEFAULT 0,
            reasoning_tokens INTEGER DEFAULT 0,
            billing_provider TEXT,
            estimated_cost_usd REAL,
            actual_cost_usd REAL
        );
        CREATE TABLE session_model_usage (
            session_id TEXT NOT NULL,
            model TEXT NOT NULL,
            billing_provider TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            estimated_cost_usd REAL NOT NULL DEFAULT 0,
            actual_cost_usd REAL NOT NULL DEFAULT 0
        );
        INSERT INTO sessions (id, source, model, started_at, message_count)
        VALUES ('session-nullable', 'cli', 'glm-5.2', 1775008000.0, 3);
        INSERT INTO session_model_usage (session_id, model, billing_provider, input_tokens)
        VALUES ('session-nullable', 'glm-5.2', NULL, 2000),
               ('session-nullable', 'glm-5.2', '',   1000);
        "#,
    )
    .unwrap();

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 2);

    let keys: HashSet<&str> = messages
        .iter()
        .filter_map(|m| m.dedup_key.as_deref())
        .collect();
    assert_eq!(keys.len(), 2, "NULL and '' must not share a dedup key");
    assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 3000);
}

// ── cross-day attribution via the agent log (junhoyeo/tokscale#993) ──────────

/// A local wall-clock stamp `days_ago` days back at `hour:12:00.000`, the shape
/// Hermes' logger writes.
fn log_stamp(days_ago: i64, hour: u32) -> String {
    let day = (chrono::Local::now() - chrono::Duration::days(days_ago)).date_naive();
    let stamp = day
        .and_hms_opt(hour, 12, 0)
        .unwrap()
        .format("%Y-%m-%d %H:%M:%S");
    format!("{stamp},000")
}

/// One `API call #N` line for the fixed model/provider. `prompt` is the whole
/// prompt; `cache` of it is cached, so the stored fresh-input share is
/// `prompt - cache`.
fn log_call(session: &str, call: i64, stamp: &str, prompt: i64, output: i64, cache: i64) -> String {
    let (model, provider) = ("glm-5.2", "zai");
    let total = prompt + output;
    format!(
        "{stamp} INFO [{session}] agent.conversation_loop: API call #{call}: model={model} provider={provider} in={prompt} out={output} total={total} latency=1.0s cache={cache}/{prompt} (50%) id=gen-{call}"
    )
}

#[allow(clippy::too_many_arguments)]
fn log_call_with_provider_and_write(
    session: &str,
    call: i64,
    stamp: &str,
    provider: &str,
    prompt: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
) -> String {
    let model = "glm-5.2";
    let total = prompt + output;
    format!(
        "{stamp} INFO [{session}] agent.conversation_loop: API call #{call}: model={model} provider={provider} in={prompt} out={output} total={total} latency=1.0s cache={cache_read}/{prompt} (50%) write={cache_write} id=gen-{call}"
    )
}

fn write_agent_log(dir: &TempDir, lines: &[String]) {
    let logs = dir.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("agent.log"), lines.join("\n") + "\n").unwrap();
}

fn local_day_key(timestamp_ms: i64) -> String {
    use chrono::TimeZone;

    chrono::Local
        .timestamp_millis_opt(timestamp_ms)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string()
}

/// Insert one smu row with the fixed model/provider and the given totals.
fn insert_usage_row(
    conn: &Connection,
    session: &str,
    input: i64,
    output: i64,
    cache_read: i64,
    cost: f64,
) {
    insert_usage_row_with_route(conn, session, "zai", input, output, cache_read, 0, cost);
}

#[allow(clippy::too_many_arguments)]
fn insert_usage_row_with_route(
    conn: &Connection,
    session: &str,
    provider: &str,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    cost: f64,
) {
    let sql = r#"
        INSERT INTO session_model_usage (
            session_id, model, billing_provider, billing_base_url, billing_mode, task,
            input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens,
            estimated_cost_usd, actual_cost_usd
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#;
    conn.execute(
        sql,
        params![
            session,
            "glm-5.2",
            provider,
            "",
            "",
            "",
            input,
            output,
            cache_read,
            cache_write,
            0_i64,
            0.0_f64,
            cost
        ],
    )
    .unwrap();
}

/// A session that started two days ago and was used again yesterday reports one
/// message per active day, with each logged bucket kept on the day that
/// produced it — the database totals stay authoritative.
#[test]
fn test_parse_hermes_sqlite_splits_cross_day_session_by_logged_calls() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    let day_a = log_stamp(2, 9);
    let day_b = log_stamp(1, 15);
    conn.execute(
        "INSERT INTO sessions (id, source, model, started_at, message_count) VALUES (?1, ?2, ?3, ?4, ?5)",
        params!["session-cross-day", "desktop", "glm-5.2", 1_775_100_000.0_f64, 12_i64],
    )
    .unwrap();
    // Totals equal the log sums: fresh input 200+200+1000, output 100+100+300,
    // cache read 800+800+2000.
    insert_usage_row(&conn, "session-cross-day", 1400, 500, 3600, 1.0);
    write_agent_log(
        &dir,
        &[
            log_call("session-cross-day", 1, &day_a, 1000, 100, 800),
            log_call("session-cross-day", 2, &day_a, 1000, 100, 800),
            log_call("session-cross-day", 3, &day_b, 3000, 300, 2000),
        ],
    );

    let mut messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 2, "one message per active day");
    messages.sort_by_key(|m| m.timestamp);

    // Each bucket stays on the day whose calls reported it. A shared overall
    // weight would incorrectly move fresh-input and cache-read tokens between
    // these differently shaped days.
    let (a, b) = (&messages[0], &messages[1]);
    assert_eq!(a.tokens.input, 400);
    assert_eq!(a.tokens.output, 200);
    assert_eq!(a.tokens.cache_read, 1600);
    assert_eq!(b.tokens.input, 1000);
    assert_eq!(b.tokens.output, 300);
    assert_eq!(b.tokens.cache_read, 2000);
    // Exact split: the parts sum back to the authoritative row.
    assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 1400);
    assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 500);
    assert_eq!(
        messages.iter().map(|m| m.tokens.cache_read).sum::<i64>(),
        3600
    );
    assert!((a.cost - 0.4).abs() < 1e-9 && (b.cost - 0.6).abs() < 1e-9);

    // Each message lands on the day its logged calls fall in, and the day is
    // part of the dedup key so no day is dropped by the caller's dedup pass.
    for msg in &messages {
        let day_key = local_day_key(msg.timestamp);
        let expected =
            format!("hermes:session-cross-day:glm-5.2:zai:__hermes_logged_day_v1__:{day_key}");
        assert_eq!(msg.dedup_key.as_deref(), Some(expected.as_str()));
    }
    assert_ne!(a.dedup_key, b.dedup_key);

    // The session message_count is still credited exactly once.
    assert_eq!(messages.iter().map(|m| m.message_count).sum::<i32>(), 12);
}

/// `in=` includes fresh, cache-read, and cache-write prompt tokens. Cache
/// writes must be removed from fresh input as well as retained in their own
/// bucket, or the over-report guard rejects every cache-write session.
#[test]
fn test_parse_hermes_sqlite_splits_cache_write_calls() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        "INSERT INTO sessions (id, source, model, started_at, message_count) VALUES (?1, ?2, ?3, ?4, ?5)",
        params!["session-cache-write", "desktop", "glm-5.2", 1_775_100_000.0_f64, 2_i64],
    )
    .unwrap();
    insert_usage_row_with_route(
        &conn,
        "session-cache-write",
        "zai",
        300,
        100,
        400,
        800,
        0.75,
    );
    let day_a = log_stamp(2, 9);
    let day_b = log_stamp(1, 9);
    write_agent_log(
        &dir,
        &[
            log_call_with_provider_and_write(
                "session-cache-write",
                1,
                &day_a,
                "zai",
                1000,
                40,
                400,
                500,
            ),
            log_call_with_provider_and_write(
                "session-cache-write",
                2,
                &day_b,
                "zai",
                500,
                60,
                0,
                300,
            ),
        ],
    );

    let mut messages = parse_hermes_sqlite(&db_path);
    messages.sort_by_key(|message| message.timestamp);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].tokens.input, 100);
    assert_eq!(messages[0].tokens.output, 40);
    assert_eq!(messages[0].tokens.cache_read, 400);
    assert_eq!(messages[0].tokens.cache_write, 500);
    assert_eq!(messages[1].tokens.input, 200);
    assert_eq!(messages[1].tokens.output, 60);
    assert_eq!(messages[1].tokens.cache_read, 0);
    assert_eq!(messages[1].tokens.cache_write, 300);
}

/// Current Hermes persists a missing provider as an empty string but logs it
/// as `provider=unknown`. The lookup forms must meet without changing the
/// output row's provider/dedup identity.
#[test]
fn test_parse_hermes_sqlite_matches_providerless_log_calls() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        "INSERT INTO sessions (id, source, model, started_at, message_count) VALUES (?1, ?2, ?3, ?4, ?5)",
        params!["session-providerless", "desktop", "glm-5.2", 1_775_100_000.0_f64, 2_i64],
    )
    .unwrap();
    insert_usage_row_with_route(&conn, "session-providerless", "", 400, 40, 0, 0, 0.2);
    let day_a = log_stamp(2, 9);
    let day_b = log_stamp(1, 9);
    write_agent_log(
        &dir,
        &[
            log_call_with_provider_and_write(
                "session-providerless",
                1,
                &day_a,
                "unknown",
                200,
                20,
                0,
                0,
            ),
            log_call_with_provider_and_write(
                "session-providerless",
                2,
                &day_b,
                "unknown",
                200,
                20,
                0,
                0,
            ),
        ],
    );

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 2, "both providerless log days must split");
    assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 400);
    assert!(messages.iter().all(message_is_day_split));
}

/// Hermes starts `session_api_calls` at zero for each agent process, including
/// when an existing conversation is resumed. Distinct calls can therefore
/// reuse the same number on different days. Exact rotated-log duplicates still
/// count once, but the resumed call must survive and create its later day.
#[test]
fn test_parse_hermes_sqlite_keeps_reused_call_numbers_after_resume() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        "INSERT INTO sessions (id, source, model, started_at, message_count) VALUES (?1, ?2, ?3, ?4, ?5)",
        params!["session-resumed", "desktop", "glm-5.2", 1_775_100_000.0_f64, 4_i64],
    )
    .unwrap();
    // Two valid calls each contribute 200 fresh input, 100 output, and 800
    // cache-read tokens. The first line is repeated to simulate overlap from a
    // rotated log and must not inflate the authoritative row.
    insert_usage_row(&conn, "session-resumed", 400, 200, 1600, 0.5);
    let day_a = log_stamp(2, 9);
    let day_b = log_stamp(1, 9);
    let first = log_call("session-resumed", 1, &day_a, 1000, 100, 800);
    let resumed = log_call("session-resumed", 1, &day_b, 1000, 100, 800);
    write_agent_log(&dir, &[first.clone(), first, resumed]);

    let mut messages = parse_hermes_sqlite(&db_path);
    messages.sort_by_key(|message| message.timestamp);
    assert_eq!(messages.len(), 2, "both active days must survive");
    assert_eq!(messages[0].tokens.input, 200);
    assert_eq!(messages[1].tokens.input, 200);
    assert_eq!(messages[0].tokens.output, 100);
    assert_eq!(messages[1].tokens.output, 100);
    assert_eq!(messages[0].tokens.cache_read, 800);
    assert_eq!(messages[1].tokens.cache_read, 800);
    assert!(messages[0]
        .dedup_key
        .as_deref()
        .is_some_and(|key| key.ends_with(&day_a[..10])));
    assert!(messages[1]
        .dedup_key
        .as_deref()
        .is_some_and(|key| key.ends_with(&day_b[..10])));
    assert_eq!(messages.iter().map(|m| m.message_count).sum::<i32>(), 4);
}

/// Logs that claim more than the authoritative row (rotated into a foreign
/// session, or lines from a database that was rewritten) are not evidence: the
/// row keeps its whole total on the session's start day.
#[test]
fn test_parse_hermes_sqlite_falls_back_when_logs_over_report() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        "INSERT INTO sessions (id, source, model, started_at, message_count) VALUES (?1, ?2, ?3, ?4, ?5)",
        params!["session-over", "cli", "glm-5.2", 1_775_100_000.0_f64, 3_i64],
    )
    .unwrap();
    insert_usage_row(&conn, "session-over", 500, 100, 100, 0.25);
    let stamp = log_stamp(1, 10);
    write_agent_log(
        &dir,
        &[log_call("session-over", 1, &stamp, 9000, 900, 8000)],
    );

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].timestamp, 1_775_100_000_000_i64);
    assert_eq!(messages[0].tokens.input, 500);
    assert_eq!(
        messages[0].dedup_key.as_deref(),
        Some("hermes:session-over:glm-5.2:zai")
    );
}

/// Log lines for sessions the database never recorded have no row to allocate
/// to and must not surface as usage; the usage-unavailable line variant carries
/// no figures and is skipped.
#[test]
fn test_parse_hermes_sqlite_ignores_log_lines_for_unknown_sessions() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        "INSERT INTO sessions (id, source, model, started_at, message_count) VALUES (?1, ?2, ?3, ?4, ?5)",
        params!["session-known", "cli", "glm-5.2", 1_775_100_000.0_f64, 2_i64],
    )
    .unwrap();
    insert_usage_row(&conn, "session-known", 200, 50, 0, 0.1);
    let stamp = log_stamp(1, 10);
    let unavailable = format!(
        "{stamp} INFO [session-known] agent.conversation_loop: API call #2: model=glm-5.2 provider=zai in=? out=? total=? latency=1.0s usage=unavailable"
    );
    write_agent_log(
        &dir,
        &[
            log_call("session-known", 1, &stamp, 200, 50, 0),
            log_call("session-never-persisted", 1, &stamp, 9000, 900, 8000),
            unavailable,
        ],
    );

    let messages = parse_hermes_sqlite(&db_path);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].session_id, "session-known");
    assert_eq!(messages[0].tokens.input, 200);
}

/// Older `agent.log*` files rotated away leave the logs covering only part of
/// a row. The covered part splits across the logged days; the rest stays on
/// the session's start day, so one recent call can never book a whole lifetime
/// row.
#[test]
fn test_parse_hermes_sqlite_leaves_unlogged_residual_on_start_day() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    // Started 2026-03-01, long before any day the surviving logs cover.
    let started_at = 1_775_000_000.0_f64;
    conn.execute(
        "INSERT INTO sessions (id, source, model, started_at, message_count) VALUES (?1, ?2, ?3, ?4, ?5)",
        params!["session-rotated", "desktop", "glm-5.2", started_at, 12_i64],
    )
    .unwrap();
    // The logs account for only 400/200/1600 of the row's 3400/900/7600: the
    // two surviving calls each contribute 1000-800 fresh input.
    insert_usage_row(&conn, "session-rotated", 3400, 900, 7600, 2.0);
    let day = log_stamp(2, 9);
    write_agent_log(
        &dir,
        &[
            log_call("session-rotated", 1, &day, 1000, 100, 800),
            log_call("session-rotated", 2, &day, 1000, 100, 800),
        ],
    );

    let mut messages = parse_hermes_sqlite(&db_path);
    assert_eq!(
        messages.len(),
        2,
        "logged day plus the residual's start day"
    );
    messages.sort_by_key(|m| m.timestamp);
    let (start, logged) = (&messages[0], &messages[1]);
    // The logged day carries exactly what the logs account for.
    assert_eq!(logged.tokens.input, 400);
    assert_eq!(logged.tokens.output, 200);
    assert_eq!(logged.tokens.cache_read, 1600);
    // The unlogged remainder stays on the start day, at the start day's
    // timestamp.
    assert_eq!(start.timestamp, 1_775_000_000_000_i64);
    assert_eq!(start.tokens.input, 3000);
    assert_eq!(start.tokens.output, 700);
    assert_eq!(start.tokens.cache_read, 6000);
    // Totals still reconcile exactly with the authoritative row.
    assert_eq!(messages.iter().map(|m| m.tokens.input).sum::<i64>(), 3400);
    assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 900);
    assert_eq!(
        messages.iter().map(|m| m.tokens.cache_read).sum::<i64>(),
        7600
    );
    // Cost rides the emitted tokens: 2200 of 11900 for the logged day.
    assert!((logged.cost - 2.0 * 2200.0 / 11900.0).abs() < 1e-9);
    assert!((start.cost - (2.0 - 2.0 * 2200.0 / 11900.0)).abs() < 1e-9);
    assert!((messages.iter().map(|m| m.cost).sum::<f64>() - 2.0).abs() < 1e-9);
    assert_eq!(messages.iter().map(|m| m.message_count).sum::<i32>(), 12);
}

/// The split groups the logged calls in the scan's bucket timezone, not the
/// host zone: two calls two hours apart straddle local midnight but fall on a
/// single day in Pacific/Kiritimati. The test skips on a host that is itself
/// on the fixture's zone, where the two groupings coincide.
#[test]
fn test_parse_hermes_sqlite_groups_split_by_bucket_timezone() {
    let dir = TempDir::new().unwrap();
    let db_path = create_test_db(&dir);
    let conn = Connection::open(&db_path).unwrap();

    conn.execute(
        "INSERT INTO sessions (id, source, model, started_at, message_count) VALUES (?1, ?2, ?3, ?4, ?5)",
        params!["session-tz", "desktop", "glm-5.2", 1_700_000_000.0_f64, 4_i64],
    )
    .unwrap();
    // Row totals equal the logs' fresh-input sums: (1000-800)*2 fresh input.
    insert_usage_row(&conn, "session-tz", 400, 200, 1600, 0.5);
    // 22:30 and 00:30 local: two host-local days, one Kiritimati day.
    write_agent_log(
        &dir,
        &[
            log_call("session-tz", 1, "2026-03-02 22:30:00,000", 1000, 100, 800),
            log_call("session-tz", 2, "2026-03-03 00:30:00,000", 1000, 100, 800),
        ],
    );

    let pinned = BucketTimezone::from_pinned_name(Some("Pacific/Kiritimati"));
    // The fixture only distinguishes the two zones while the host is not
    // itself on the fixture's zone. On such a host the parser reads the
    // zone-less stamps as the fixture zone, both groupings coincide, and the
    // assertions below would be vacuous — skip there.
    use chrono::{Local, TimeZone};
    let naive_a =
        chrono::NaiveDateTime::parse_from_str("2026-03-02 22:30:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let naive_b =
        chrono::NaiveDateTime::parse_from_str("2026-03-03 00:30:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let ms_a = Local.from_local_datetime(&naive_a).earliest().unwrap();
    let ms_b = Local.from_local_datetime(&naive_b).earliest().unwrap();
    if pinned.day_key(ms_a.timestamp_millis()) == local_day_key(ms_a.timestamp_millis())
        && pinned.day_key(ms_b.timestamp_millis()) == local_day_key(ms_b.timestamp_millis())
    {
        return;
    }
    let pinned_messages = parse_hermes_sqlite_in(&db_path, &pinned);
    assert_eq!(
        pinned_messages.len(),
        1,
        "Kiritimati keeps both calls on one day"
    );
    assert_eq!(pinned_messages[0].tokens.input, 400);
    assert_eq!(pinned_messages[0].tokens.output, 200);
    assert_eq!(pinned_messages[0].tokens.cache_read, 1600);

    // The host-zone parse still splits them, so the grouping really is the pin.
    let local_messages = parse_hermes_sqlite(&db_path);
    assert_eq!(
        local_messages.len(),
        2,
        "the host zone splits the same calls"
    );
    assert_eq!(
        local_messages.iter().map(|m| m.tokens.input).sum::<i64>(),
        400
    );
}
