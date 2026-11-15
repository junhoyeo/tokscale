mod common;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use serial_test::serial;
use tokscale_core::pricing::{litellm::ModelPricing, PricingService};
use tokscale_core::scanner::ScannerSettings;
use tokscale_core::sessions::capy::{credentials_identity, CapyCredentials};
use tokscale_core::{
    parse_local_clients, parse_local_unified_messages_with_pricing,
    parse_local_unified_messages_with_pricing_uncached, ClientId, LocalParseOptions,
};

fn config_dir(home: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        home.join("AppData/Roaming/tokscale")
    }
    #[cfg(not(target_os = "windows"))]
    {
        home.join(".config/tokscale")
    }
}

fn write_capy_cache(home: &Path, json: &str) -> PathBuf {
    let dir = config_dir(home).join("capy-cache");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("usage.json");
    fs::write(&path, json).unwrap();
    // The key file shares the directory and must never be parsed as usage.
    fs::write(dir.join("credentials.json"), r#"{"apiKey":"capy_test"}"#).unwrap();
    path
}

fn write_scoped_capy_cache(home: &Path, identity: Option<&str>) -> PathBuf {
    let mut cache: serde_json::Value = serde_json::from_str(CACHE).unwrap();
    if let Some(identity) = identity {
        cache["identity"] = serde_json::json!(identity);
    }
    let path = write_capy_cache(home, &serde_json::to_string(&cache).unwrap());
    fs::remove_file(path.parent().unwrap().join("credentials.json")).unwrap();
    path
}

fn isolate_capy_environment() -> common::EnvGuard {
    const KEYS: [&str; 5] = [
        "PRIME_AGENT_SESSION_DIR",
        "PRIME_AGENT_CODING_AGENT_SESSION_DIR",
        "PRIME_AGENT_CODING_AGENT_DIR",
        "CAPY_API_KEY",
        "CAPY_USER",
    ];
    let guard = common::EnvGuard::capture(&KEYS);
    for key in KEYS {
        std::env::remove_var(key);
    }
    guard
}

fn capy_options(home: &Path) -> LocalParseOptions {
    LocalParseOptions {
        home_dir: Some(home.to_str().unwrap().to_string()),
        use_env_roots: false,
        clients: Some(vec!["capy".to_string()]),
        since: None,
        until: None,
        year: None,
        scanner_settings: ScannerSettings::default(),
    }
}

fn make_pricing_service() -> PricingService {
    let mut litellm_data = HashMap::new();
    litellm_data.insert(
        "gpt-5.5".to_string(),
        ModelPricing {
            input_cost_per_token: Some(0.000001),
            output_cost_per_token: Some(0.00001),
            cache_read_input_token_cost: Some(0.0000001),
            // A tier that per-request pricing would select for these sums;
            // Capy rows aggregate many requests, so it must not apply.
            input_cost_per_token_above_272k_tokens: Some(0.000002),
            output_cost_per_token_above_272k_tokens: Some(0.00002),
            cache_read_input_token_cost_above_272k_tokens: Some(0.0000002),
            ..Default::default()
        },
    );
    PricingService::new(litellm_data, HashMap::new())
}

const CACHE: &str = r#"{
    "version": 1,
    "syncedAt": "2026-11-14T12:00:00.000Z",
    "userIds": ["user_1"],
    "items": [
        {
            "start": "2026-11-14T09:00:00.000Z",
            "end": "2026-11-14T10:00:00.000Z",
            "dimensions": {"userId": "user_1", "threadId": "thr_1", "itemType": "llm",
                "itemId": "openai/gpt-5.5", "model": "openai/gpt-5.5"},
            "billedDollars": 0,
            "tokens": {"inputTokens": 1000000, "outputTokens": 500,
                "cacheReadTokens": 600000, "cacheWriteTokens": 0}
        },
        {
            "start": "2026-11-14T10:00:00.000Z",
            "end": "2026-11-14T11:00:00.000Z",
            "dimensions": {"userId": "user_1", "threadId": "thr_2", "itemType": "llm",
                "itemId": "openai/gpt-5.5", "model": "openai/gpt-5.5"},
            "billedDollars": 0,
            "tokens": {"inputTokens": 2000, "outputTokens": 100,
                "cacheReadTokens": 0, "cacheWriteTokens": 0}
        },
        {
            "start": "2026-11-14T10:00:00.000Z",
            "end": "2026-11-14T11:00:00.000Z",
            "dimensions": {"userId": "user_1", "threadId": "thr_2", "itemType": "vm",
                "itemId": "vm-small"},
            "billedDollars": 0.02,
            "tokens": {"inputTokens": 0, "outputTokens": 0,
                "cacheReadTokens": 0, "cacheWriteTokens": 0}
        }
    ]
}"#;

#[tokio::test]
#[serial]
async fn capy_cache_is_discovered_parsed_and_priced() {
    let _guard = isolate_capy_environment();
    let home_dir = common::temp_home();
    let home = home_dir.path();
    write_capy_cache(home, CACHE);

    let pricing = make_pricing_service();
    let messages =
        parse_local_unified_messages_with_pricing_uncached(capy_options(home), Some(&pricing))
            .await
            .unwrap();

    assert_eq!(messages.len(), 2);
    assert!(messages.iter().all(|m| m.client == "capy"));
    assert!(messages.iter().all(|m| m.model_id == "gpt-5.5"));

    let first = messages
        .iter()
        .find(|m| m.session_id == "capy-thr_1")
        .unwrap();
    assert_eq!(first.tokens.input, 400_000); // 1M - 600K cache read
    assert_eq!(first.tokens.output, 500);
    assert_eq!(first.tokens.cache_read, 600_000);
    // Priced by Tokscale at base rates (the row is an aggregate), not by
    // Capy's billedDollars, which is zero for subscription routes.
    let expected = 400_000.0 * 0.000001 + 500.0 * 0.00001 + 600_000.0 * 0.0000001;
    assert!(
        (first.cost - expected).abs() < 1e-9,
        "cost {} != {expected}",
        first.cost
    );

    let parsed = parse_local_clients(capy_options(home)).unwrap();
    assert_eq!(parsed.counts.get(ClientId::Capy), 2);
}

#[tokio::test]
#[serial]
async fn missing_capy_cache_yields_no_messages() {
    let _guard = isolate_capy_environment();
    let home_dir = common::temp_home();
    let home = home_dir.path();

    let messages = parse_local_unified_messages_with_pricing_uncached(capy_options(home), None)
        .await
        .unwrap();
    assert!(messages.is_empty());
}

#[tokio::test]
#[serial]
async fn midnight_hour_and_daily_residual_survive_cached_and_uncached_dedup() {
    let _guard = isolate_capy_environment();
    let home_dir = common::temp_home();
    let home = home_dir.path();
    write_capy_cache(
        home,
        r#"{
            "version": 1,
            "syncedAt": "2026-11-14T12:00:00.000Z",
            "items": [
                {
                    "start": "2026-10-14T00:00:00Z",
                    "end": "2026-10-14T01:00:00Z",
                    "dimensions": {"userId": "u", "threadId": "t", "itemType": "llm",
                        "itemId": "openai/gpt-5.5", "model": "openai/gpt-5.5"},
                    "tokens": {"inputTokens": 100, "outputTokens": 10,
                        "cacheReadTokens": 0, "cacheWriteTokens": 0}
                },
                {
                    "start": "2026-10-14T00:00:00-00:00",
                    "end": "2026-10-14T01:00:00+00:00",
                    "dimensions": {"userId": "u", "threadId": "t", "itemType": "llm",
                        "itemId": "openai/gpt-5.5", "model": "openai/gpt-5.5"},
                    "tokens": {"inputTokens": 100, "outputTokens": 10,
                        "cacheReadTokens": 0, "cacheWriteTokens": 0}
                },
                {
                    "start": "2026-10-14T00:00:00.000Z",
                    "end": "2026-10-15T00:00:00.000Z",
                    "dimensions": {"userId": "u", "threadId": "t", "itemType": "llm",
                        "itemId": "openai/gpt-5.5", "model": "openai/gpt-5.5"},
                    "tokens": {"inputTokens": 200, "outputTokens": 20,
                        "cacheReadTokens": 0, "cacheWriteTokens": 0}
                },
                {
                    "start": "2026-10-14T00:00:00+00:00",
                    "end": "2026-10-15T00:00:00Z",
                    "dimensions": {"userId": "u", "threadId": "t", "itemType": "llm",
                        "itemId": "openai/gpt-5.5", "model": "openai/gpt-5.5"},
                    "tokens": {"inputTokens": 200, "outputTokens": 20,
                        "cacheReadTokens": 0, "cacheWriteTokens": 0}
                }
            ]
        }"#,
    );

    let assert_usage = |messages: &[tokscale_core::UnifiedMessage]| {
        assert_eq!(messages.len(), 2);
        let mut amounts: Vec<(i64, i64)> = messages
            .iter()
            .map(|message| (message.tokens.input, message.tokens.output))
            .collect();
        amounts.sort_unstable();
        assert_eq!(amounts, [(100, 10), (200, 20)]);
    };

    let uncached = parse_local_unified_messages_with_pricing_uncached(capy_options(home), None)
        .await
        .unwrap();
    assert_usage(&uncached);

    let cold_cached = parse_local_unified_messages_with_pricing(capy_options(home), None)
        .await
        .unwrap();
    assert_usage(&cold_cached);
    let warm_cached = parse_local_unified_messages_with_pricing(capy_options(home), None)
        .await
        .unwrap();
    assert_usage(&warm_cached);
}

#[tokio::test]
#[serial]
async fn active_env_scope_excludes_a_warm_cache_without_mutating_it() {
    let _guard = isolate_capy_environment();
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let usage_path = write_scoped_capy_cache(home, Some("6b04be6a8213d6b1"));
    let original = fs::read(&usage_path).unwrap();

    std::env::set_var("CAPY_API_KEY", OsStr::new("capy_scope_a_synthetic"));
    let scope_a = parse_local_unified_messages_with_pricing(capy_options(home), None)
        .await
        .unwrap();
    assert_eq!(scope_a.len(), 2);

    std::env::set_var("CAPY_API_KEY", OsStr::new("capy_scope_b_synthetic"));
    let warm_scope_b = parse_local_unified_messages_with_pricing(capy_options(home), None)
        .await
        .unwrap();
    assert!(warm_scope_b.is_empty());
    let cold_scope_b = parse_local_unified_messages_with_pricing_uncached(capy_options(home), None)
        .await
        .unwrap();
    assert!(cold_scope_b.is_empty());
    assert_eq!(fs::read(&usage_path).unwrap(), original);
}

#[tokio::test]
#[serial]
async fn active_scope_rejects_missing_or_opaque_identity_but_logout_retains_cache() {
    let _guard = isolate_capy_environment();
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let usage_path = write_scoped_capy_cache(home, None);
    std::env::set_var("CAPY_API_KEY", OsStr::new("capy_scope_a_synthetic"));

    let missing = parse_local_unified_messages_with_pricing_uncached(capy_options(home), None)
        .await
        .unwrap();
    assert!(missing.is_empty());

    let mut cache: serde_json::Value =
        serde_json::from_slice(&fs::read(&usage_path).unwrap()).unwrap();
    cache["identity"] = serde_json::json!("opaque-old-cache");
    fs::write(&usage_path, serde_json::to_vec(&cache).unwrap()).unwrap();
    let opaque = parse_local_unified_messages_with_pricing_uncached(capy_options(home), None)
        .await
        .unwrap();
    assert!(opaque.is_empty());

    std::env::remove_var("CAPY_API_KEY");
    let retained = parse_local_unified_messages_with_pricing_uncached(capy_options(home), None)
        .await
        .unwrap();
    assert_eq!(retained.len(), 2);
}

#[tokio::test]
#[serial]
async fn saved_credentials_and_user_filter_outrank_env_scope() {
    let _guard = isolate_capy_environment();
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let credentials_path = config_dir(home).join("capy-cache/credentials.json");
    let saved_scope = CapyCredentials {
        api_key: "saved_scope_synthetic".to_string(),
        user_ids: vec!["user_1".to_string()],
        created_at: "synthetic".to_string(),
    };
    let usage_path = write_scoped_capy_cache(home, Some(&credentials_identity(&saved_scope)));
    fs::write(
        &credentials_path,
        r#"{"apiKey":"saved_scope_synthetic","userIds":["user_1"],"createdAt":"synthetic"}"#,
    )
    .unwrap();
    let original_usage = fs::read(&usage_path).unwrap();
    std::env::set_var("CAPY_API_KEY", OsStr::new("env_scope_synthetic"));

    let cold_saved_scope = parse_local_unified_messages_with_pricing(capy_options(home), None)
        .await
        .unwrap();
    assert_eq!(cold_saved_scope.len(), 2);
    let warm_saved_scope = parse_local_unified_messages_with_pricing(capy_options(home), None)
        .await
        .unwrap();
    assert_eq!(warm_saved_scope, cold_saved_scope);

    let filtered_scope = CapyCredentials {
        user_ids: vec!["user_2".to_string()],
        ..saved_scope.clone()
    };
    fs::write(
        &credentials_path,
        serde_json::to_vec(&filtered_scope).unwrap(),
    )
    .unwrap();
    let warm_filtered = parse_local_unified_messages_with_pricing(capy_options(home), None)
        .await
        .unwrap();
    assert!(warm_filtered.is_empty());
    let cold_filtered =
        parse_local_unified_messages_with_pricing_uncached(capy_options(home), None)
            .await
            .unwrap();
    assert!(cold_filtered.is_empty());
    assert_eq!(fs::read(&usage_path).unwrap(), original_usage);

    fs::write(&credentials_path, serde_json::to_vec(&saved_scope).unwrap()).unwrap();
    let restored = parse_local_unified_messages_with_pricing(capy_options(home), None)
        .await
        .unwrap();
    assert_eq!(restored, cold_saved_scope);
}
