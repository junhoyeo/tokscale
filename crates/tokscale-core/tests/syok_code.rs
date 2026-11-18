use std::path::Path;
use tempfile::TempDir;
use tokscale_core::{
    parse_local_clients, parse_local_unified_messages_with_pricing_uncached,
    scanner::{scan_all_clients_with_scanner_settings, ScannerSettings},
    ClientId, LocalParseOptions,
};

fn syok_options(home: &Path) -> LocalParseOptions {
    LocalParseOptions {
        home_dir: Some(home.to_str().unwrap().to_string()),
        use_env_roots: false,
        clients: Some(vec!["syok-code".to_string()]),
        since: None,
        until: None,
        year: None,
        scanner_settings: ScannerSettings::default(),
    }
}

#[tokio::test]
async fn test_syok_code_usage_history_scanning_and_parsing() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    let usage = home.join(".syok/usage");
    std::fs::create_dir_all(&usage).unwrap();

    // Two monthly files; one call answered after a provider switch.
    std::fs::write(
        usage.join("2026-10.jsonl"),
        concat!(
            r#"{"v":1,"ts":"2026-10-31T23:59:00.000Z","session":"20261031235800-aa11","project":"/work/shop","client":"syok-code","requested":"syok-coder","provider":"gemini","model":"gemini-3-flash","input":2000,"output":40,"cache_read":0,"reasoning":0,"total":2040}"#,
            "\n"
        ),
    )
    .unwrap();
    std::fs::write(
        usage.join("2026-11.jsonl"),
        concat!(
            r#"{"v":1,"ts":"2026-11-18T19:41:02.511Z","session":"20261118194101-ab12","project":"/work/shop","client":"syok-code","requested":"syok-coder","provider":"claude","model":"claude-sonnet-5","input":5210,"output":312,"cache_read":4096,"reasoning":120,"total":5522}"#,
            "\n",
            r#"{"v":1,"ts":"2026-11-18T19:41:09.002Z","session":"20261118194101-ab12","project":"/work/shop","client":"syok-code","requested":"syok-coder","provider":"mistral","model":"codestral-latest","input":6100,"output":25,"cache_read":0,"reasoning":0,"total":6125}"#,
            "\n",
            "{broken line\n",
        ),
    )
    .unwrap();
    // Files outside the usage folder are not Syok Code usage.
    std::fs::write(home.join(".syok/config.json"), "{}").unwrap();

    let scan = scan_all_clients_with_scanner_settings(
        home.to_str().unwrap(),
        &["syok-code".to_string()],
        false,
        &ScannerSettings::default(),
    );
    assert_eq!(scan.get(ClientId::SyokCode).len(), 2);

    let parsed = parse_local_clients(syok_options(home)).unwrap();
    assert_eq!(parsed.counts.get(ClientId::SyokCode), 3);
    assert_eq!(parsed.messages.len(), 3);

    let uncached = parse_local_unified_messages_with_pricing_uncached(syok_options(home), None)
        .await
        .unwrap();
    assert_eq!(uncached.len(), 3);
    assert!(uncached.iter().all(|m| m.client == "syok-code"));
    let sonnet = uncached
        .iter()
        .find(|m| m.model_id == "claude-sonnet-5")
        .unwrap();
    assert_eq!(sonnet.provider_id, "anthropic");
    assert_eq!(sonnet.tokens.input, 5210 - 4096);
    assert_eq!(sonnet.tokens.cache_read, 4096);
    assert_eq!(sonnet.tokens.output, 312 - 120);
    assert_eq!(sonnet.tokens.reasoning, 120);
    assert_eq!(sonnet.workspace_label.as_deref(), Some("shop"));
    let total: i64 = uncached.iter().map(|m| m.tokens.total()).sum();
    assert_eq!(total, 2040 + 5522 + 6125);
}
