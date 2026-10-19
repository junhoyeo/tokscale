//! Shelley client integration tests.
//!
//! The fixture is a temp SQLite database built from Shelley's real DDL subset
//! (`conversations` + `messages`), written to the registered root
//! `~/.config/shelley/shelley.db` under a temp home. Cases mirror the shape of
//! a live instance: an agent turn, an all-zero `usage_data` on both a non-agent
//! row and one `type = 'agent'` row, an indirect-only `slug` row, and a
//! subagent conversation.

mod common;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde_json::json;
use tokscale_core::pricing::{litellm::ModelPricing, PricingService};
use tokscale_core::scanner::ScannerSettings;
use tokscale_core::{
    parse_local_clients, parse_local_unified_messages_with_pricing,
    parse_local_unified_messages_with_pricing_uncached, ClientId, CostSource, LocalParseOptions,
    UnifiedMessage,
};

/// The wire id Shelley records, and the LiteLLM reseller key it prices against.
const FIREWORKS_MODEL: &str = "accounts/fireworks/models/deepseek-v4p1-flash";
const FIREWORKS_RESELLER_KEY: &str = "fireworks_ai/accounts/fireworks/models/deepseek-v4p1-flash";
const LUNA_MODEL: &str = "gpt-6-luna";

fn shelley_db_path(home: &Path) -> PathBuf {
    home.join(".config/shelley/shelley.db")
}

/// Which migration era's `messages` columns a fixture carries.
#[derive(Clone, Copy, PartialEq)]
enum SchemaVariant {
    /// Migration 036 and later: both usage columns.
    Current,
    /// Migrations 032 to 035: `forked_from_message_id` but no
    /// `other_usage_data`.
    PreOtherUsage,
    /// Before migration 032: no fork provenance either.
    PreForkProvenance,
}

/// Create the database with the columns the parser reads.
fn create_shelley_db(db_path: &Path) -> Connection {
    create_shelley_db_schema(db_path, SchemaVariant::Current)
}

/// Create the database at an older migration era, for the schema guard.
fn create_shelley_db_schema(db_path: &Path, variant: SchemaVariant) -> Connection {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let conn = Connection::open(db_path).unwrap();
    let mut columns = vec![
        "message_id TEXT PRIMARY KEY",
        "conversation_id TEXT NOT NULL",
        "sequence_id INTEGER NOT NULL",
        "type TEXT NOT NULL",
        "usage_data TEXT",
        "created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP",
        "excluded_from_context BOOLEAN NOT NULL DEFAULT FALSE",
        "generation INTEGER NOT NULL DEFAULT 1",
        "llm_api_url TEXT",
        "model_name TEXT",
    ];
    if variant != SchemaVariant::PreForkProvenance {
        columns.push("forked_from_message_id TEXT");
    }
    if variant == SchemaVariant::Current {
        columns.push("other_usage_data TEXT");
    }
    conn.execute_batch(&format!(
        r#"
        PRAGMA journal_mode = WAL;
        CREATE TABLE conversations (
            conversation_id TEXT PRIMARY KEY,
            slug TEXT,
            user_initiated BOOLEAN NOT NULL DEFAULT TRUE,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            cwd TEXT,
            archived BOOLEAN NOT NULL DEFAULT FALSE,
            parent_conversation_id TEXT REFERENCES conversations(conversation_id),
            model TEXT,
            current_generation INTEGER NOT NULL DEFAULT 1,
            is_draft BOOLEAN NOT NULL DEFAULT FALSE
        );
        CREATE TABLE messages ({});
        "#,
        columns.join(", ")
    ))
    .unwrap();
    conn
}

/// Insert an agent row into a fixture older than the usage columns, so the
/// `INSERT` names no column that schema lacks.
fn insert_legacy_agent_message(
    conn: &Connection,
    message_id: &str,
    conversation_id: &str,
    sequence_id: i64,
    usage_data: &str,
    model_name: Option<&str>,
) {
    conn.execute(
        "INSERT INTO messages
            (message_id, conversation_id, sequence_id, type, usage_data, created_at, model_name)
         VALUES (?1, ?2, ?3, 'agent', ?4, ?5, ?6)",
        params![
            message_id,
            conversation_id,
            sequence_id,
            usage_data,
            "2026-10-19 14:07:37",
            model_name
        ],
    )
    .unwrap();
}

fn insert_conversation(
    conn: &Connection,
    conversation_id: &str,
    slug: Option<&str>,
    parent_conversation_id: Option<&str>,
    user_initiated: bool,
    cwd: Option<&str>,
    archived: bool,
    is_draft: bool,
) {
    conn.execute(
        "INSERT INTO conversations
            (conversation_id, slug, parent_conversation_id, user_initiated, cwd, archived, is_draft)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            conversation_id,
            slug,
            parent_conversation_id,
            user_initiated,
            cwd,
            archived,
            is_draft
        ],
    )
    .unwrap();
}

/// One `usage_data` object, in the shape Shelley's Go structs serialize.
fn usage_object(
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    cost: f64,
    model: Option<&str>,
) -> String {
    let mut value = json!({
        "input_tokens": input,
        "cache_creation_input_tokens": cache_write,
        "cache_read_input_tokens": cache_read,
        "output_tokens": output,
        "cost_usd": cost,
    });
    if let Some(model) = model {
        value["model"] = json!(model);
        value["url"] = json!("https://llm.int.exe.xyz/v1/chat/completions");
        value["start_time"] = json!("2026-10-19T14:07:30.784235359Z");
        value["end_time"] = json!("2026-10-19T14:07:37.784235359Z");
    }
    value.to_string()
}

#[allow(clippy::too_many_arguments)]
fn insert_message(
    conn: &Connection,
    message_id: &str,
    conversation_id: &str,
    sequence_id: i64,
    message_type: &str,
    usage_data: Option<&str>,
    model_name: Option<&str>,
    forked_from_message_id: Option<&str>,
    other_usage_data: Option<&str>,
) {
    conn.execute(
        "INSERT INTO messages
            (message_id, conversation_id, sequence_id, type, usage_data, created_at,
             model_name, forked_from_message_id, other_usage_data)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            message_id,
            conversation_id,
            sequence_id,
            message_type,
            usage_data,
            "2026-10-19 14:07:37",
            model_name,
            forked_from_message_id,
            other_usage_data
        ],
    )
    .unwrap();
}

fn shelley_options(home: &Path) -> LocalParseOptions {
    LocalParseOptions {
        home_dir: Some(home.to_str().unwrap().to_string()),
        use_env_roots: false,
        clients: Some(vec!["shelley".to_string()]),
        since: None,
        until: None,
        year: None,
        scanner_settings: ScannerSettings::default(),
    }
}

fn pricing_for(keys: &[(&str, f64, f64)]) -> PricingService {
    let mut litellm_data = HashMap::new();
    for (key, input, output) in keys {
        litellm_data.insert(
            (*key).to_string(),
            ModelPricing {
                input_cost_per_token: Some(*input),
                output_cost_per_token: Some(*output),
                ..Default::default()
            },
        );
    }
    PricingService::new(litellm_data, HashMap::new())
}

/// A pricing service that only carries the Fireworks reseller key, so a row
/// priced against it proves provider-scoped resolution worked.
fn fireworks_pricing() -> PricingService {
    pricing_for(&[(FIREWORKS_RESELLER_KEY, 0.0000003, 0.0000004)])
}

async fn parse_uncached(home: &Path, pricing: &PricingService) -> Vec<UnifiedMessage> {
    parse_local_unified_messages_with_pricing_uncached(shelley_options(home), Some(pricing))
        .await
        .unwrap()
}

fn by_message<'a>(messages: &'a [UnifiedMessage], message_id: &str) -> &'a UnifiedMessage {
    messages
        .iter()
        .find(|message| message.dedup_key.as_deref() == Some(message_id))
        .unwrap_or_else(|| panic!("no parsed row for message {message_id}"))
}

// =========================================================================
// Case 1 — a direct agent turn
// =========================================================================

#[tokio::test]
async fn test_direct_agent_turn_maps_every_token_bucket_and_session_metadata() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-root",
        Some("addition-calculation"),
        None,
        true,
        Some("/home/exedev/work"),
        false,
        false,
    );
    insert_message(&conn, "m-user", "c-root", 1, "user", None, None, None, None);
    insert_message(
        &conn,
        "m-turn-1",
        "c-root",
        2,
        "agent",
        Some(&usage_object(
            9424,
            117,
            200,
            50,
            0.0,
            Some(FIREWORKS_MODEL),
        )),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    // A second agent row in the same user gap is not a turn start.
    insert_message(
        &conn,
        "m-turn-2",
        "c-root",
        3,
        "agent",
        Some(&usage_object(10, 5, 0, 0, 0.0, Some(FIREWORKS_MODEL))),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert_eq!(messages.len(), 2);

    let turn = by_message(&messages, "m-turn-1");
    assert_eq!(turn.client, "shelley");
    assert_eq!(turn.model_id, FIREWORKS_MODEL);
    assert_eq!(turn.session_id, "c-root");
    assert_eq!(turn.session_title.as_deref(), Some("addition-calculation"));
    assert_eq!(turn.tokens.input, 9424);
    assert_eq!(turn.tokens.output, 117);
    assert_eq!(turn.tokens.cache_read, 200);
    assert_eq!(turn.tokens.cache_write, 50);
    // No reasoning bucket exists in Shelley's usage shape.
    assert_eq!(turn.tokens.reasoning, 0);
    assert_eq!(turn.tokens.cache_write_1h, 0);
    assert_eq!(turn.message_count, 1);
    // `start_time` is preferred over `created_at`, which lands at call end.
    assert_eq!(turn.timestamp, 1_792_418_850_784);
    assert_eq!(turn.duration_ms, Some(7_000));
    assert_eq!(turn.workspace_key.as_deref(), Some("/home/exedev/work"));
    assert_eq!(turn.workspace_label.as_deref(), Some("work"));
    // The first billable row after a user turn, and only that one.
    assert!(turn.is_turn_start);
    assert!(!by_message(&messages, "m-turn-2").is_turn_start);
}

// =========================================================================
// Case 2 + 3 — the all-zero filter, inside and outside `type = 'agent'`
// =========================================================================

#[tokio::test]
async fn test_all_zero_usage_on_non_agent_rows_produces_no_messages() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-root",
        Some("noise"),
        None,
        true,
        None,
        false,
        false,
    );

    let zero = usage_object(0, 0, 0, 0, 0.0, None);
    for (index, message_type) in ["user", "system", "modelchange", "error"]
        .iter()
        .enumerate()
    {
        insert_message(
            &conn,
            &format!("m-{message_type}"),
            "c-root",
            index as i64 + 1,
            message_type,
            Some(&zero),
            None,
            None,
            None,
        );
    }
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert!(
        messages.is_empty(),
        "all-zero usage_data must not become a row: {messages:?}"
    );
}

#[tokio::test]
async fn test_all_zero_usage_on_an_agent_row_produces_no_message() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-parent",
        Some("addition-calculation"),
        None,
        true,
        None,
        false,
        false,
    );
    insert_conversation(
        &conn,
        "c-subagent",
        Some("test-sum"),
        Some("c-parent"),
        false,
        None,
        false,
        false,
    );
    // The live instance's second subagent row: an agent row whose usage is
    // entirely zero, with `model_name` NULL and `excluded_from_context` set.
    let zero = json!({
        "input_tokens": 0,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "output_tokens": 0,
        "cost_usd": 0,
    })
    .to_string();
    insert_message(
        &conn,
        "m-zero-agent",
        "c-subagent",
        5,
        "agent",
        Some(&zero),
        None,
        None,
        None,
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert!(
        messages.is_empty(),
        "the all-zero filter is load-bearing inside agent rows too: {messages:?}"
    );
}

// =========================================================================
// Case 4 — forked copies
// =========================================================================

#[tokio::test]
async fn test_forked_message_copies_are_excluded() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-root",
        Some("forked"),
        None,
        true,
        None,
        false,
        false,
    );

    let usage = usage_object(500, 50, 0, 0, 0.0, Some(FIREWORKS_MODEL));
    insert_message(
        &conn,
        "m-original",
        "c-root",
        1,
        "agent",
        Some(&usage),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    // A fork copies the original's usage verbatim; counting it double counts.
    insert_message(
        &conn,
        "m-forked",
        "c-root",
        2,
        "agent",
        Some(&usage),
        Some(FIREWORKS_MODEL),
        Some("m-original"),
        None,
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].dedup_key.as_deref(), Some("m-original"));
    assert_eq!(messages[0].tokens.input, 500);
}

// =========================================================================
// Case 5 — indirect calls summed into the affiliated row
// =========================================================================

#[tokio::test]
async fn test_indirect_usage_is_summed_into_the_affiliated_row() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-root",
        Some("slug-call"),
        None,
        true,
        None,
        false,
        false,
    );

    // A `slug` row carries no direct usage, only indirect entries.
    let indirect = json!([{
        "purpose": "slug",
        "input_tokens": 146,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "output_tokens": 10,
        "cost_usd": 0.000056,
        "model": FIREWORKS_MODEL,
        "url": "https://llm.int.exe.xyz/v1/chat/completions",
        "start_time": "2026-10-19T13:37:27.159588510Z",
        "end_time": "2026-10-19T13:37:28.425896924Z",
    }])
    .to_string();
    insert_message(
        &conn,
        "m-slug",
        "c-root",
        1,
        "slug",
        None,
        None,
        None,
        Some(&indirect),
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert_eq!(messages.len(), 1);
    let slug = by_message(&messages, "m-slug");
    assert_eq!(slug.tokens.input, 146);
    assert_eq!(slug.tokens.output, 10);
    assert_eq!(slug.message_count, 1);
    assert_eq!(slug.model_id, FIREWORKS_MODEL);
    // `cost_usd` was non-zero, so the amount is taken as provider-reported.
    assert_eq!(slug.cost_source, CostSource::ProviderReported);
    assert!((slug.cost - 0.000056).abs() < f64::EPSILON);
    assert_eq!(slug.duration_ms, Some(1_266));
}

// =========================================================================
// Case 6 — the documented mixed-model collapse
// =========================================================================

#[tokio::test]
async fn test_mixed_model_indirect_entries_collapse_to_the_first_model() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-root",
        Some("mixed"),
        None,
        true,
        None,
        false,
        false,
    );

    let entry = |model: &str, input: i64, cost: f64| {
        json!({
            "purpose": "slug",
            "input_tokens": input,
            "output_tokens": 1,
            "cache_read_input_tokens": 0,
            "cache_creation_input_tokens": 0,
            "cost_usd": cost,
            "model": model,
        })
    };
    let indirect = json!([
        entry(FIREWORKS_MODEL, 100, 0.0001),
        entry(LUNA_MODEL, 200, 0.0002),
    ])
    .to_string();
    insert_message(
        &conn,
        "m-mixed",
        "c-root",
        1,
        "slug",
        None,
        None,
        None,
        Some(&indirect),
    );

    // The affiliated row's own `model_name`, when it has one, wins over the
    // entries' models.
    let indirect_named = json!([entry(LUNA_MODEL, 300, 0.0)]).to_string();
    insert_message(
        &conn,
        "m-named",
        "c-root",
        2,
        "tool",
        None,
        Some(FIREWORKS_MODEL),
        None,
        Some(&indirect_named),
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert_eq!(messages.len(), 2);

    let mixed = by_message(&messages, "m-mixed");
    // Tokens are summed across both entries...
    assert_eq!(mixed.tokens.input, 300);
    assert_eq!(mixed.tokens.output, 2);
    assert_eq!(mixed.message_count, 2);
    // ...and the single surviving model id is the first entry's.
    assert_eq!(mixed.model_id, FIREWORKS_MODEL);
    assert_eq!(mixed.cost_source, CostSource::ProviderReported);
    assert!((mixed.cost - 0.0003).abs() < 1e-12);

    // A row that names its own model keeps it, whatever the entries say.
    let named = by_message(&messages, "m-named");
    assert_eq!(named.model_id, FIREWORKS_MODEL);
    assert_eq!(named.tokens.input, 300);
    // Its only entry reported no cost, so the row is priced locally.
    assert_eq!(named.cost_source, CostSource::Estimated);
}

// =========================================================================
// Case 7 — subagent conversations
// =========================================================================

#[tokio::test]
async fn test_subagent_conversation_gets_parent_link_and_agent_label() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-parent",
        Some("addition-calculation"),
        None,
        true,
        None,
        false,
        false,
    );
    insert_conversation(
        &conn,
        "c-child",
        Some("test-sum"),
        Some("c-parent"),
        false,
        None,
        false,
        false,
    );
    let usage = usage_object(8792, 57, 0, 0, 0.0, Some(FIREWORKS_MODEL));
    insert_message(
        &conn,
        "m-parent",
        "c-parent",
        13,
        "agent",
        Some(&usage),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    insert_message(
        &conn,
        "m-child",
        "c-child",
        3,
        "agent",
        Some(&usage),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert_eq!(messages.len(), 2);

    // The subagent's usage is its own, not nested inside the parent's.
    let parent = by_message(&messages, "m-parent");
    assert_eq!(parent.session_id, "c-parent");
    assert_eq!(parent.parent_session_id, None);
    assert_eq!(parent.agent, None);

    let child = by_message(&messages, "m-child");
    assert_eq!(child.session_id, "c-child");
    assert_eq!(child.parent_session_id.as_deref(), Some("c-parent"));
    assert_eq!(child.agent.as_deref(), Some("Shelley Subagent"));
    assert_eq!(child.session_title.as_deref(), Some("test-sum"));
}

// =========================================================================
// Case 8 — `model_name` NULL with `usage_data.model` set
// =========================================================================

#[tokio::test]
async fn test_null_model_name_falls_back_to_the_usage_model() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-root",
        Some("fallback"),
        None,
        true,
        None,
        false,
        false,
    );
    insert_message(
        &conn,
        "m-null-name",
        "c-root",
        1,
        "agent",
        Some(&usage_object(1000, 100, 0, 0, 0.0, Some(FIREWORKS_MODEL))),
        None,
        None,
        None,
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].model_id, FIREWORKS_MODEL);
    assert_eq!(messages[0].provider_id, "fireworks_ai");
}

// =========================================================================
// Case 9 — provider scoping: the trap this client has to avoid
// =========================================================================

#[tokio::test]
async fn test_fireworks_scoped_ids_keep_provider_scoped_pricing() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-root",
        Some("scoped"),
        None,
        true,
        None,
        false,
        false,
    );
    insert_message(
        &conn,
        "m-scoped",
        "c-root",
        1,
        "agent",
        Some(&usage_object(
            1_000_000,
            100_000,
            0,
            0,
            0.0,
            Some(FIREWORKS_MODEL),
        )),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    drop(conn);

    let pricing = fireworks_pricing();
    let messages = parse_uncached(home, &pricing).await;
    assert_eq!(messages.len(), 1);

    // The scoped vendor segment is canonicalized, not the model family.
    assert_eq!(messages[0].provider_id, "fireworks_ai");
    assert_ne!(messages[0].provider_id, "deepseek");

    // ...which is what keeps the reseller key reachable at all. The family hint
    // a naive derivation would produce cannot price this id, so the row would
    // silently cost zero.
    assert!(
        pricing
            .lookup_with_source_and_provider(FIREWORKS_MODEL, None, Some("deepseek"))
            .is_none(),
        "family inference must not be able to price a scoped vendor path"
    );
    assert!(
        pricing
            .lookup_with_source_and_provider(FIREWORKS_MODEL, None, Some("fireworks_ai"))
            .is_some(),
        "the scoped vendor hint must reach the reseller key"
    );

    // End to end: the row is priced.
    assert!(messages[0].cost > 0.0, "scoped row priced at zero");
}

// =========================================================================
// Case 10 — cost source selection
// =========================================================================

#[tokio::test]
async fn test_cost_is_provider_reported_when_present_and_estimated_otherwise() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-root",
        Some("cost"),
        None,
        true,
        None,
        false,
        false,
    );

    // Reported: taken verbatim, never re-estimated.
    insert_message(
        &conn,
        "m-reported",
        "c-root",
        1,
        "agent",
        Some(&usage_object(1000, 100, 0, 0, 0.42, Some(FIREWORKS_MODEL))),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    // Not reported: priced from the engine.
    insert_message(
        &conn,
        "m-estimated",
        "c-root",
        2,
        "agent",
        Some(&usage_object(
            1_000_000,
            100_000,
            0,
            0,
            0.0,
            Some(FIREWORKS_MODEL),
        )),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    let reported = by_message(&messages, "m-reported");
    assert_eq!(reported.cost_source, CostSource::ProviderReported);
    assert!((reported.cost - 0.42).abs() < f64::EPSILON);

    let estimated = by_message(&messages, "m-estimated");
    assert_eq!(estimated.cost_source, CostSource::Estimated);
    // 1,000,000 * 3e-7 + 100,000 * 4e-7
    assert!((estimated.cost - 0.34).abs() < 1e-9);
}

// =========================================================================
// Case 11 — cache invalidation on a WAL-only commit
// =========================================================================

#[tokio::test]
#[serial_test::serial]
async fn test_wal_only_commit_invalidates_the_source_cache() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let config_dir = home.join("tokscale-config");
    fs::create_dir_all(&config_dir).unwrap();
    let _env = common::EnvGuard::set(&[("TOKSCALE_CONFIG_DIR", config_dir.as_os_str())]);

    let db_path = shelley_db_path(home);
    // Hold the write connection open for the whole test. Shelley's own server
    // does the same, which is what keeps recent commits in the `-wal` instead
    // of letting the last close checkpoint them into the main file.
    let conn = create_shelley_db(&db_path);
    insert_conversation(&conn, "c-root", Some("wal"), None, true, None, false, false);
    let usage = usage_object(100, 10, 0, 0, 0.0, Some(FIREWORKS_MODEL));
    insert_message(
        &conn,
        "m-first",
        "c-root",
        1,
        "agent",
        Some(&usage),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );

    let pricing = fireworks_pricing();
    let first = parse_local_unified_messages_with_pricing(shelley_options(home), Some(&pricing))
        .await
        .unwrap();
    assert_eq!(first.len(), 1);

    let main_mtime = fs::metadata(&db_path).unwrap().modified().unwrap();
    insert_message(
        &conn,
        "m-second",
        "c-root",
        2,
        "agent",
        Some(&usage),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );

    // The row landed in the `-wal` and the main file did not move, so a
    // fingerprint over the main file alone would serve the stale one-message
    // result forever.
    let wal_path = db_path.with_file_name("shelley.db-wal");
    assert!(wal_path.exists(), "expected a -wal sidecar");
    assert_eq!(
        fs::metadata(&db_path).unwrap().modified().unwrap(),
        main_mtime,
        "the main database file must be untouched by a WAL-only commit"
    );

    let second = parse_local_unified_messages_with_pricing(shelley_options(home), Some(&pricing))
        .await
        .unwrap();
    assert_eq!(
        second.len(),
        2,
        "a WAL-only commit must invalidate the cached parse"
    );
}

// =========================================================================
// Case 12 — archived and draft conversations are both counted
// =========================================================================

#[tokio::test]
async fn test_archived_and_draft_conversations_are_counted() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-archived",
        Some("archived-chat"),
        None,
        true,
        None,
        true,
        false,
    );
    insert_conversation(
        &conn,
        "c-draft",
        Some("draft-chat"),
        None,
        true,
        None,
        false,
        true,
    );
    let usage = usage_object(700, 70, 0, 0, 0.0, Some(FIREWORKS_MODEL));
    insert_message(
        &conn,
        "m-archived",
        "c-archived",
        1,
        "agent",
        Some(&usage),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    insert_message(
        &conn,
        "m-draft",
        "c-draft",
        1,
        "agent",
        Some(&usage),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert_eq!(
        messages.len(),
        2,
        "archiving is a UI action; the tokens were billed either way"
    );

    let parsed = parse_local_clients(shelley_options(home)).unwrap();
    assert_eq!(parsed.counts.get(ClientId::Shelley), 2);
}

// =========================================================================
// Discovery — the registered root, and a relocated database
// =========================================================================

#[tokio::test]
async fn test_default_root_is_discovered_without_extra_scan_paths() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-root",
        Some("default"),
        None,
        true,
        None,
        false,
        false,
    );
    insert_message(
        &conn,
        "m-default",
        "c-root",
        1,
        "agent",
        Some(&usage_object(100, 10, 0, 0, 0.0, Some(FIREWORKS_MODEL))),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    drop(conn);

    let parsed = parse_local_clients(shelley_options(home)).unwrap();
    assert_eq!(parsed.counts.get(ClientId::Shelley), 1);
    assert_eq!(parsed.messages.len(), 1);
    assert_eq!(parsed.messages[0].client, "shelley");
}

#[tokio::test]
async fn test_relocated_database_is_discovered_through_extra_scan_paths() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    // A `-db` pointed somewhere else entirely, as Shelley's own flag allows.
    let relocated_dir = home.join("elsewhere/shelley-data");
    let relocated_db = relocated_dir.join("shelley.db");
    let conn = create_shelley_db(&relocated_db);
    insert_conversation(
        &conn,
        "c-root",
        Some("relocated"),
        None,
        true,
        None,
        false,
        false,
    );
    insert_message(
        &conn,
        "m-relocated",
        "c-root",
        1,
        "agent",
        Some(&usage_object(100, 10, 0, 0, 0.0, Some(FIREWORKS_MODEL))),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    drop(conn);

    // Nothing at the registered root.
    let parsed_default = parse_local_clients(shelley_options(home)).unwrap();
    assert_eq!(parsed_default.counts.get(ClientId::Shelley), 0);
    assert!(parsed_default.messages.is_empty());

    let mut extra_scan_paths = std::collections::BTreeMap::new();
    extra_scan_paths.insert("shelley".to_string(), vec![relocated_dir]);
    let parsed = parse_local_clients(LocalParseOptions {
        scanner_settings: ScannerSettings {
            extra_scan_paths,
            ..Default::default()
        },
        ..shelley_options(home)
    })
    .unwrap();

    assert_eq!(parsed.counts.get(ClientId::Shelley), 1);
    assert_eq!(parsed.messages.len(), 1);
    assert_eq!(parsed.messages[0].model_id, FIREWORKS_MODEL);
}

/// A `-db` pointed elsewhere is also reachable through `TOKSCALE_EXTRA_DIRS`,
/// the environment-variable half of the same relocation mechanism as the
/// settings key. Unlike the settings test, this one needs `use_env_roots`,
/// which is what gates the environment variable at the scanner.
#[tokio::test]
#[serial_test::serial]
async fn test_relocated_database_is_discovered_through_extra_dirs_env() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let relocated_dir = home.join("elsewhere/shelley-data");
    let relocated_db = relocated_dir.join("shelley.db");
    let conn = create_shelley_db(&relocated_db);
    insert_conversation(&conn, "c-env", Some("env"), None, true, None, false, false);
    insert_message(
        &conn,
        "m-env",
        "c-env",
        1,
        "agent",
        Some(&usage_object(100, 10, 0, 0, 0.0, Some(FIREWORKS_MODEL))),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    drop(conn);

    // Nothing at the registered root, so a count of 1 can only come from the
    // environment-variable root.
    let parsed_default = parse_local_clients(shelley_options(home)).unwrap();
    assert_eq!(parsed_default.counts.get(ClientId::Shelley), 0);
    assert!(parsed_default.messages.is_empty());

    let extra_dirs = format!("shelley:{}", relocated_dir.display());
    let _env = common::EnvGuard::set(&[("TOKSCALE_EXTRA_DIRS", std::ffi::OsStr::new(&extra_dirs))]);

    let parsed = parse_local_clients(LocalParseOptions {
        use_env_roots: true,
        ..shelley_options(home)
    })
    .unwrap();

    assert_eq!(parsed.counts.get(ClientId::Shelley), 1);
    assert_eq!(parsed.messages.len(), 1);
    assert_eq!(parsed.messages[0].model_id, FIREWORKS_MODEL);
}

/// One database reached by two roots counts once, while a second database only
/// the extra root can reach still counts. The registered root is the file
/// itself, so an `extraScanPaths` entry naming the directory that contains it is
/// a different canonical path and is not collapsed as a scan task; the overlap
/// is caught later, where found files are deduped by canonical path.
#[tokio::test]
async fn test_registered_root_and_overlapping_extra_scan_path_count_once() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let registered_dir = home.join(".config/shelley");

    // The database at the registered root.
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-registered",
        Some("registered"),
        None,
        true,
        None,
        false,
        false,
    );
    insert_message(
        &conn,
        "m-registered",
        "c-registered",
        1,
        "agent",
        Some(&usage_object(100, 10, 0, 0, 0.0, Some(FIREWORKS_MODEL))),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    drop(conn);

    // A second database the registered root cannot reach — only the extra
    // directory root can. Its row is what proves that root was actually walked,
    // so the registered row appearing once below cannot be an artifact of the
    // extra root being ignored.
    let nested_dir = registered_dir.join("nested");
    let conn = create_shelley_db(&nested_dir.join("shelley.db"));
    insert_conversation(
        &conn,
        "c-nested",
        Some("nested"),
        None,
        true,
        None,
        false,
        false,
    );
    insert_message(
        &conn,
        "m-nested",
        "c-nested",
        1,
        "agent",
        Some(&usage_object(200, 20, 0, 0, 0.0, Some(FIREWORKS_MODEL))),
        Some(FIREWORKS_MODEL),
        None,
        None,
    );
    drop(conn);

    let mut extra_scan_paths = std::collections::BTreeMap::new();
    extra_scan_paths.insert("shelley".to_string(), vec![registered_dir]);

    let parsed = parse_local_clients(LocalParseOptions {
        scanner_settings: ScannerSettings {
            extra_scan_paths,
            ..Default::default()
        },
        ..shelley_options(home)
    })
    .unwrap();

    // Two databases, so two rows — and the registered one appears once even
    // though both roots reach it.
    assert_eq!(parsed.counts.get(ClientId::Shelley), 2);
    assert_eq!(parsed.messages.len(), 2);
    assert_eq!(
        parsed.messages.iter().filter(|m| m.input == 100).count(),
        1,
        "the registered database's row was counted twice"
    );
    assert_eq!(
        parsed.messages.iter().filter(|m| m.input == 200).count(),
        1,
        "the nested database only the extra root reaches was not counted"
    );
}

#[tokio::test]
async fn test_a_foreign_database_named_shelley_db_yields_nothing() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let db_path = shelley_db_path(home);
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch("CREATE TABLE unrelated (id INTEGER PRIMARY KEY)")
        .unwrap();
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert!(messages.is_empty());
}

// =========================================================================
// Case 16 — schemas older than the usage columns
// =========================================================================

#[tokio::test]
async fn test_pre_036_schema_still_reports_the_direct_calls() {
    // Before migration 036 Shelley recorded no indirect usage anywhere, so the
    // indirect query cannot prepare. That is the legacy schema rather than a
    // lost read: the agent turns are still the whole answer, and the parser
    // must not turn a missing column into an empty report.
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db_schema(&shelley_db_path(home), SchemaVariant::PreOtherUsage);
    insert_conversation(
        &conn,
        "c-root",
        Some("legacy-schema"),
        None,
        true,
        None,
        false,
        false,
    );
    insert_legacy_agent_message(
        &conn,
        "m-legacy",
        "c-root",
        1,
        &usage_object(1_000, 200, 300, 0, 0.0025, Some(FIREWORKS_MODEL)),
        Some(FIREWORKS_MODEL),
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert_eq!(messages.len(), 1);
    let message = by_message(&messages, "m-legacy");
    assert_eq!(message.tokens.input, 1_000);
    assert_eq!(message.tokens.output, 200);
    assert_eq!(message.tokens.cache_read, 300);
    assert_eq!(message.model_id, FIREWORKS_MODEL);
    assert_eq!(message.cost_source, CostSource::ProviderReported);
    assert!((message.cost - 0.0025).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_pre_032_schema_reports_nothing_rather_than_counting_forked_copies() {
    // Before migration 032 there is no `forked_from_message_id` to filter on,
    // so a forked conversation's copied rows would be counted a second time.
    // The parser has no honest number to report here, and a wrong total is
    // worse than none: it yields nothing, which also keeps the cached lane
    // from storing the result as if the database were empty.
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db_schema(&shelley_db_path(home), SchemaVariant::PreForkProvenance);
    insert_conversation(
        &conn,
        "c-root",
        Some("pre-fork-schema"),
        None,
        true,
        None,
        false,
        false,
    );
    insert_legacy_agent_message(
        &conn,
        "m-ancient",
        "c-root",
        1,
        &usage_object(1_000, 200, 0, 0, 0.0025, Some(FIREWORKS_MODEL)),
        Some(FIREWORKS_MODEL),
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert!(messages.is_empty());
}

// =========================================================================
// Case 17 — a message that carries both call groups
// =========================================================================

/// One indirect entry, in the shape `other_usage_data` holds.
fn indirect_entry(model: &str, input: i64, output: i64, cost: f64) -> serde_json::Value {
    json!({
        "purpose": "slug",
        "input_tokens": input,
        "output_tokens": output,
        "cache_read_input_tokens": 0,
        "cache_creation_input_tokens": 0,
        "cost_usd": cost,
        "model": model,
    })
}

#[tokio::test]
async fn test_a_message_carrying_both_call_groups_becomes_two_rows() {
    // Shelley's schema allows one message row to carry a direct call and
    // indirect ones at once. They are separate calls with their own models, so
    // the row is recorded twice: the direct call under the message id, the
    // indirect calls under a derived key.
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-root",
        Some("both-halves"),
        None,
        true,
        None,
        false,
        false,
    );
    insert_message(&conn, "m-user", "c-root", 1, "user", None, None, None, None);
    let indirect = json!([indirect_entry(LUNA_MODEL, 300, 1, 0.0002)]).to_string();
    insert_message(
        &conn,
        "m-both",
        "c-root",
        2,
        "agent",
        Some(&usage_object(
            1_000,
            200,
            0,
            0,
            0.004,
            Some(FIREWORKS_MODEL),
        )),
        Some(FIREWORKS_MODEL),
        None,
        Some(&indirect),
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert_eq!(messages.len(), 2, "the two call groups are two rows");

    let direct = by_message(&messages, "m-both");
    assert_eq!(direct.model_id, FIREWORKS_MODEL);
    assert_eq!(direct.tokens.input, 1_000);
    assert_eq!(direct.tokens.output, 200);
    assert_eq!(direct.message_count, 1);
    assert!(direct.is_turn_start);
    assert_eq!(direct.cost_source, CostSource::ProviderReported);
    assert!((direct.cost - 0.004).abs() < f64::EPSILON);

    let indirect = by_message(&messages, "m-both:indirect");
    // The indirect half names its own model rather than inheriting the direct
    // half's, and carries only its own tokens.
    assert_eq!(indirect.model_id, LUNA_MODEL);
    assert_eq!(indirect.tokens.input, 300);
    assert_eq!(indirect.tokens.output, 1);
    assert_eq!(indirect.cost_source, CostSource::ProviderReported);
    assert!((indirect.cost - 0.0002).abs() < 1e-12);
    // Count-neutral: one Shelley message stays one message, and only the half
    // that is the message itself starts the turn.
    assert_eq!(indirect.message_count, 0);
    assert!(!indirect.is_turn_start);
    assert_eq!(indirect.session_id, direct.session_id);
}

#[tokio::test]
async fn test_a_partially_reported_message_prices_each_half_on_its_own() {
    // The case the single-row rule got wrong: the direct call reported a cost
    // and the indirect call did not. One row could only price them together, so
    // it discarded the reported amount and billed all 1,201 tokens at the direct
    // half's rate. Split, the reported half is taken verbatim and the unreported
    // half is priced from the engine.
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let conn = create_shelley_db(&shelley_db_path(home));
    insert_conversation(
        &conn,
        "c-root",
        Some("partial-report"),
        None,
        true,
        None,
        false,
        false,
    );
    let indirect = json!([indirect_entry(FIREWORKS_MODEL, 300, 1, 0.0)]).to_string();
    insert_message(
        &conn,
        "m-partial",
        "c-root",
        1,
        "agent",
        Some(&usage_object(
            1_000,
            200,
            0,
            0,
            0.004,
            Some(FIREWORKS_MODEL),
        )),
        Some(FIREWORKS_MODEL),
        None,
        Some(&indirect),
    );
    drop(conn);

    let messages = parse_uncached(home, &fireworks_pricing()).await;
    assert_eq!(messages.len(), 2);

    let direct = by_message(&messages, "m-partial");
    assert_eq!(direct.cost_source, CostSource::ProviderReported);
    assert!((direct.cost - 0.004).abs() < f64::EPSILON);

    let indirect = by_message(&messages, "m-partial:indirect");
    assert_eq!(indirect.cost_source, CostSource::Estimated);
    // 300 * 3e-7 + 1 * 4e-7, the reseller key's rates.
    assert!((indirect.cost - 0.000_090_4).abs() < 1e-9);
}
