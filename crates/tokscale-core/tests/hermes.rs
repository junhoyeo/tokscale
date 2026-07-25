use rusqlite::{params, Connection};
use tempfile::TempDir;
use tokscale_core::sessions::hermes::parse_hermes_sqlite;

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
    assert_eq!(msg.tokens.output, 300);
    assert_eq!(msg.tokens.cache_read, 50);
    assert_eq!(msg.tokens.cache_write, 20);
    assert_eq!(msg.tokens.reasoning, 10);
    assert_eq!(msg.cost, 0.34);
    assert_eq!(
        msg.dedup_key.as_deref(),
        Some("hermes:session-1:claude-sonnet-4")
    );
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

    // Empty session (no smu row — excluded by JOIN, never reaches HAVING)
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
        Some("hermes:session-multi:cohere/north-mini-code:free")
    );

    // glm-5.2: primary model, gets message_count
    let glm = messages.iter().find(|m| m.model_id == "glm-5.2").unwrap();
    assert_eq!(glm.tokens.input, 5000);
    assert_eq!(glm.tokens.output, 800);
    assert_eq!(glm.message_count, 15); // primary model
    assert_eq!(
        glm.dedup_key.as_deref(),
        Some("hermes:session-multi:glm-5.2")
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
        Some("hermes:session-multi:mimo-v2.5-free")
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
    assert_eq!(msg.tokens.output, 300);
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
