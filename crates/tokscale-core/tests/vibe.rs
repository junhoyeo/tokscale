mod common;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use tokscale_core::pricing::{litellm::ModelPricing, PricingService};
use tokscale_core::scanner::ScannerSettings;
use tokscale_core::sessions::vibe::parse_vibe_file;
use tokscale_core::{
    parse_local_clients, parse_local_unified_messages_with_pricing_uncached, ClientId,
    LocalParseOptions,
};

const SESSION_ID: &str = "019e1e27-6f8e-73cb-b0eb-78da684e5111";

fn write_vibe_session(home: &Path, session_name: &str, meta_json: &str) -> PathBuf {
    let session_dir = home.join(".vibe/logs/session").join(session_name);
    fs::create_dir_all(&session_dir).unwrap();
    let meta_path = session_dir.join("meta.json");
    fs::write(&meta_path, meta_json).unwrap();
    meta_path
}

fn vibe_options(home: &Path) -> LocalParseOptions {
    LocalParseOptions {
        home_dir: Some(home.to_str().unwrap().to_string()),
        use_env_roots: false,
        clients: Some(vec!["vibe".to_string()]),
        since: None,
        until: None,
        year: None,
        scanner_settings: ScannerSettings::default(),
    }
}

fn make_pricing_service() -> PricingService {
    let mut litellm_data = HashMap::new();
    litellm_data.insert(
        "mistralai/codestral-2501".to_string(),
        ModelPricing {
            input_cost_per_token: Some(0.0000003),
            output_cost_per_token: Some(0.0000009),
            cache_read_input_token_cost: Some(0.0000001),
            ..Default::default()
        },
    );
    // Mistral's published Medium rate, so an unpinned session that resolves to
    // the default alias has a row to price against.
    litellm_data.insert(
        "mistralai/mistral-medium-latest".to_string(),
        ModelPricing {
            input_cost_per_token: Some(0.0000015),
            output_cost_per_token: Some(0.0000075),
            cache_read_input_token_cost: Some(0.00000015),
            ..Default::default()
        },
    );
    PricingService::new(litellm_data, HashMap::new())
}

#[test]
fn test_vibe_parser_extracts_tokens_and_metadata() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let workspace = home.join("my-cool-project");
    fs::create_dir_all(&workspace).unwrap();
    let root = workspace.to_string_lossy().replace('\\', "/");

    let meta_json = format!(
        r#"{{
            "session_id": "{SESSION_ID}",
            "start_time": "2026-10-10T08:00:00Z",
            "end_time": "2026-10-10T08:05:00Z",
            "title": "Build a parser for Vibe",
            "origin_directory": "{root}",
            "environment": {{
                "working_directory": "{root}"
            }},
            "config": {{
                "active_model": "codestral-latest"
            }},
            "stats": {{
                "steps": 4,
                "session_prompt_tokens": 12000,
                "session_completion_tokens": 1500,
                "session_cached_tokens": 4000
            }}
        }}"#
    );

    let path = write_vibe_session(home, "session_20261010_080000_abc123", &meta_json);
    let messages = parse_vibe_file(&path);

    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message.client, "vibe");
    assert_eq!(message.session_id, SESSION_ID);
    assert_eq!(message.model_id, "codestral-latest");
    assert_eq!(message.provider_id, "mistralai");
    assert_eq!(message.tokens.input, 8000); // 12000 - 4000
    assert_eq!(message.tokens.output, 1500);
    assert_eq!(message.tokens.cache_read, 4000);
    assert_eq!(message.message_count, 4);
    assert_eq!(
        message.session_title.as_deref(),
        Some("Build a parser for Vibe")
    );
    assert_eq!(message.workspace_label.as_deref(), Some("my-cool-project"));
}

#[test]
fn test_vibe_legacy_format_and_zero_usage() {
    let home_dir = common::temp_home();
    let home = home_dir.path();

    let legacy_file = home
        .join(".vibe/logs/session")
        .join("session_20260901_102030.json");
    fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();

    let legacy_json = r#"{
        "metadata": {
            "session_id": "legacy-session-42",
            "start_time": "2026-09-01T10:20:30Z",
            "active_model": "mistral-large-latest",
            "stats": {
                "steps": 2,
                "session_prompt_tokens": 5000,
                "session_completion_tokens": 800,
                "session_cached_tokens": 1000
            }
        },
        "messages": []
    }"#;
    fs::write(&legacy_file, legacy_json).unwrap();

    let messages = parse_vibe_file(&legacy_file);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].session_id, "legacy-session-42");
    assert_eq!(messages[0].tokens.input, 4000);
    assert_eq!(messages[0].tokens.output, 800);
    assert_eq!(messages[0].tokens.cache_read, 1000);

    // Zero usage session should produce 0 messages
    let zero_file = home.join(".vibe/logs/session/zero/meta.json");
    fs::create_dir_all(zero_file.parent().unwrap()).unwrap();
    let zero_json = r#"{
        "session_id": "zero-session",
        "stats": {
            "session_prompt_tokens": 0,
            "session_completion_tokens": 0,
            "session_cached_tokens": 0
        }
    }"#;
    fs::write(&zero_file, zero_json).unwrap();
    assert!(parse_vibe_file(&zero_file).is_empty());
}

#[tokio::test]
async fn test_vibe_end_to_end_discovers_sessions_and_reports_counts() {
    let home_dir = common::temp_home();
    let home = home_dir.path();

    let meta_json = r#"{
        "session_id": "sess-end-to-end",
        "start_time": "2026-10-10T10:00:00Z",
        "end_time": "2026-10-10T10:10:00Z",
        "config": {
            "active_model": "mistral-small-latest"
        },
        "stats": {
            "steps": 5,
            "session_prompt_tokens": 10000,
            "session_completion_tokens": 2000,
            "session_cached_tokens": 2500
        }
    }"#;

    write_vibe_session(home, "session_20261010_100000_123456", meta_json);

    let pricing = make_pricing_service();
    let messages =
        parse_local_unified_messages_with_pricing_uncached(vibe_options(home), Some(&pricing))
            .await
            .unwrap();

    assert_eq!(messages.len(), 1);
    let msg = &messages[0];
    assert_eq!(msg.client, "vibe");
    assert_eq!(msg.session_id, "sess-end-to-end");
    assert_eq!(msg.model_id, "mistral-small-latest");
    assert_eq!(msg.provider_id, "mistralai");
    assert_eq!(msg.tokens.input, 7500); // 10000 - 2500
    assert_eq!(msg.tokens.output, 2000);
    assert_eq!(msg.tokens.cache_read, 2500);

    let parsed = parse_local_clients(vibe_options(home)).unwrap();
    assert_eq!(parsed.counts.get(ClientId::Vibe), 5);
    assert_eq!(
        parsed
            .messages
            .iter()
            .filter(|message| message.client == "vibe")
            .count(),
        1
    );
}

#[test]
fn test_vibe_empty_origin_directory_falls_back_to_environment_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let meta_file = dir.path().join("meta.json");

    let json = r#"{
        "session_id": "sess-empty-origin",
        "origin_directory": "   ",
        "environment": {
            "working_directory": "/home/ubuntu/workspace"
        },
        "stats": {
            "session_prompt_tokens": 100,
            "session_completion_tokens": 50
        }
    }"#;
    fs::write(&meta_file, json).unwrap();

    let msgs = parse_vibe_file(&meta_file);
    assert_eq!(msgs.len(), 1);
    assert_eq!(
        msgs[0].workspace_key.as_deref(),
        Some("/home/ubuntu/workspace")
    );
    assert_eq!(msgs[0].dedup_key.as_deref(), Some("vibe:sess-empty-origin"));
}

#[tokio::test]
async fn test_vibe_deduplicates_duplicate_session_copies() {
    let home_dir = common::temp_home();
    let home = home_dir.path();

    let meta_json = r#"{
        "session_id": "sess-duplicate",
        "start_time": "2026-10-10T10:00:00Z",
        "config": { "active_model": "mistral-small-latest" },
        "stats": {
            "steps": 2,
            "session_prompt_tokens": 1000,
            "session_completion_tokens": 100
        }
    }"#;

    write_vibe_session(home, "session_20261010_100000_original", meta_json);
    write_vibe_session(home, "session_20261010_100000_copy", meta_json);

    let pricing = make_pricing_service();
    let messages =
        parse_local_unified_messages_with_pricing_uncached(vibe_options(home), Some(&pricing))
            .await
            .unwrap();

    assert_eq!(
        messages.len(),
        1,
        "identical session copies must be deduplicated"
    );
    assert_eq!(messages[0].session_id, "sess-duplicate");
    assert_eq!(
        messages[0].dedup_key.as_deref(),
        Some("vibe:sess-duplicate")
    );

    let parsed = parse_local_clients(vibe_options(home)).unwrap();
    assert_eq!(parsed.messages.len(), 1);
}

/// A session that was never pinned with `/model` records
/// `config.active_model: ""` — upstream's `UNPINNED_ACTIVE_MODEL` sentinel for
/// "follow the configured default", and the state every session starts in. The
/// empty string must not survive as the model id: it blanks the Model column
/// and matches no pricing row, so the session would be counted with tokens but
/// priced at zero.
#[tokio::test]
async fn test_vibe_unpinned_session_resolves_configured_model_and_prices_it() {
    let home_dir = common::temp_home();
    let home = home_dir.path();

    let meta_json = r#"{
        "session_id": "sess-unpinned",
        "start_time": "2026-10-16T08:11:55.873982+00:00",
        "end_time": "2026-10-16T08:11:55.986988+00:00",
        "config": {
            "active_model": "",
            "models": {
                "mistral-medium-3.5": {
                    "name": "mistral-vibe-cli-latest",
                    "provider": "mistral",
                    "alias": "mistral-medium-3.5",
                    "display_name": "Mistral Medium 3.5",
                    "input_price": 1.5,
                    "output_price": 7.5,
                    "cached_input_price": 0.15
                }
            }
        },
        "stats": {
            "steps": 3,
            "session_prompt_tokens": 10000,
            "session_completion_tokens": 1000,
            "session_cached_tokens": 2000
        }
    }"#;

    let path = write_vibe_session(home, "session_20261016_081155_unpinne", meta_json);
    let messages = parse_vibe_file(&path);
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].model_id, "mistral-medium-3.5",
        "an unpinned session must resolve to its configured model alias, not \"\""
    );
    assert_eq!(messages[0].tokens.input, 8000);
    assert_eq!(messages[0].tokens.cache_read, 2000);

    // An empty model id matches no pricing row at all, so the whole session
    // would be reported free. A resolved alias reaches the lookup chain.
    let pricing = make_pricing_service();
    let priced =
        parse_local_unified_messages_with_pricing_uncached(vibe_options(home), Some(&pricing))
            .await
            .unwrap();
    assert_eq!(priced.len(), 1);
    assert_eq!(priced[0].model_id, "mistral-medium-3.5");
    assert!(
        !priced[0].model_id.is_empty(),
        "an empty model id prices the session at zero and blanks the Model column"
    );
}

/// With nothing pinned and no `config.models` table to read, the published
/// default alias stands in: a blank model id is never a valid attribution.
#[test]
fn test_vibe_unpinned_session_without_model_table_uses_default_alias() {
    let dir = tempfile::tempdir().unwrap();
    let meta_file = dir.path().join("meta.json");
    fs::write(
        &meta_file,
        r#"{
            "session_id": "sess-no-models",
            "config": { "active_model": "   " },
            "stats": {
                "session_prompt_tokens": 500,
                "session_completion_tokens": 50
            }
        }"#,
    )
    .unwrap();

    let messages = parse_vibe_file(&meta_file);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].model_id, "mistral-medium-latest");
    assert_eq!(messages[0].provider_id, "mistralai");
}

/// The session directory holds two siblings that are not sessions: the shared
/// index `.session_index.json` and the atomic-write temporaries upstream names
/// `tmp<random>.json.tmp`. Neither carries the `session_` prefix the first
/// implementation keyed its exclusions on, so both have to be refused by dot
/// and suffix instead.
#[test]
fn test_vibe_scanner_skips_index_and_atomic_write_temporaries() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let root = home.join(".vibe/logs/session");
    fs::create_dir_all(&root).unwrap();

    // Real sibling names from upstream: session_index.py:19 and the
    // NamedTemporaryFile(suffix=".json.tmp") / prefix=".session_index." writes.
    fs::write(root.join(".session_index.json"), "{}").unwrap();
    fs::write(root.join(".session_index.abc123.tmp"), "{}").unwrap();
    fs::write(root.join("tmpa1b2c3.json.tmp"), "{}").unwrap();

    let session_meta = r#"{
        "session_id": "sess-real",
        "config": { "active_model": "mistral-small-latest" },
        "stats": { "session_prompt_tokens": 10, "session_completion_tokens": 1 }
    }"#;
    write_vibe_session(home, "session_20261016_090000_real0001", session_meta);

    // A legacy flat file keeps being discovered, including under a custom
    // `session_logging.session_prefix` (upstream's default is `session`, but a
    // subagent's prefix is its agent name).
    fs::write(
        root.join("explorer_20260101_010101.json"),
        r#"{"metadata":{"session_id":"legacy-prefixed","active_model":"mistral-small-latest","stats":{"session_prompt_tokens":77,"session_completion_tokens":7}},"messages":[]}"#,
    )
    .unwrap();

    let files = tokscale_core::scanner::scan_directory(root.to_str().unwrap(), "vibe-session");
    let names: Vec<String> = files
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();

    assert!(
        names.iter().any(|name| name == "meta.json"),
        "real session meta.json must be scanned, got {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|name| name == "explorer_20260101_010101.json"),
        "legacy flat session under a custom prefix must be scanned, got {names:?}"
    );
    for refused in [
        ".session_index.json",
        ".session_index.abc123.tmp",
        "tmpa1b2c3.json.tmp",
    ] {
        assert!(
            !names.iter().any(|name| name == refused),
            "{refused} is not a session transcript, got {names:?}"
        );
    }

    let parsed = parse_local_clients(vibe_options(home)).unwrap();
    assert_eq!(
        parsed
            .messages
            .iter()
            .filter(|message| message.client == "vibe")
            .count(),
        2,
        "one real session plus one legacy session"
    );
}
