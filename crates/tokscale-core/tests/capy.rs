mod common;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use tokscale_core::pricing::{litellm::ModelPricing, PricingService};
use tokscale_core::scanner::ScannerSettings;
use tokscale_core::{
    parse_local_clients, parse_local_unified_messages_with_pricing_uncached, ClientId,
    LocalParseOptions,
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
async fn capy_cache_is_discovered_parsed_and_priced() {
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
async fn missing_capy_cache_yields_no_messages() {
    let home_dir = common::temp_home();
    let home = home_dir.path();

    let messages = parse_local_unified_messages_with_pricing_uncached(capy_options(home), None)
        .await
        .unwrap();
    assert!(messages.is_empty());
}
