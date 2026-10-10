use tokscale_core::scanner::scan_all_clients_with_env_strategy;
use tokscale_core::{parse_local_unified_messages_with_pricing_uncached, LocalParseOptions};

mod common;

fn assert_aside_provider_fallback(model: &str, expected: &str) {
    // Given absent/routing markers and an explicit provider that differs from inference.
    let home = tempfile::tempdir().unwrap();
    let rows = [
        None,
        Some(""),
        Some(" AsIdE "),
        Some(" UNKNOWN "),
        Some("custom-provider"),
    ]
    .map(|provider| {
        serde_json::json!({
            "role": "assistant", "model": model, "provider": provider,
            "usage": {"input": 1}
        })
        .to_string()
            + "\n"
    })
    .concat();
    write_session(home.path(), &rows);
    // When the real parser resolves provider identity.
    let messages = tokscale_core::sessions::aside::parse_aside_file(
        &home
            .path()
            .join(".aside/u/0/sessions/synthetic/messages.jsonl"),
    );
    // Then only routing markers use inference; explicit providers always win.
    assert_eq!(messages.len(), 5);
    assert_eq!(
        messages
            .iter()
            .map(|m| m.provider_id.as_str())
            .collect::<Vec<_>>(),
        [expected, expected, expected, expected, "custom-provider"],
        "{model}"
    );
}

#[test]
fn aside_customgptish_is_not_an_openai_family() {
    assert_aside_provider_fallback("customgptish-solver", "unknown");
}

#[test]
fn aside_notqwen_is_not_a_qwen_family() {
    assert_aside_provider_fallback("notqwen-custom", "unknown");
}

#[test]
fn aside_versioned_families_infer_without_overriding_explicit_providers() {
    for model in [
        "gpt-6.1-sol",
        "gpt-6-astra",
        "gpt-5.6",
        "gpt-5.6-sol",
        "gpt-5.6-astra",
        "gpt-5.4-mini",
    ] {
        assert_aside_provider_fallback(model, "openai");
    }
    assert_aside_provider_fallback("qwen3-coder", "qwen");
}

#[test]
fn aside_cost_only_parser_preserves_cost_without_fabricating_tokens() {
    use tokscale_core::sessions::{aside::parse_aside_file, CostSource};
    // Given positive recorded cost with missing/zero output and standalone reasoning.
    let home = tempfile::tempdir().unwrap();
    let rows = [serde_json::Value::Null, serde_json::json!(0)]
        .map(|output| {
            serde_json::json!({
                "role": "assistant", "timestamp": 1_788_609_600_000_i64,
                "responseId": "cost-only", "provider": "openai-codex",
                "usage": {"input": -1, "output": output, "reasoning": 99,
                    "cost": {"total": 0.25}}
            })
            .to_string()
                + "\n"
        })
        .concat();
    write_session(home.path(), &rows);
    // When parsing without aggregation or pricing.
    let messages = parse_aside_file(
        &home
            .path()
            .join(".aside/u/0/sessions/synthetic/messages.jsonl"),
    );
    // Then recorded cost and identity survive, while reasoning remains bounded by output.
    assert_eq!(messages.len(), 2);
    for message in messages {
        assert_eq!(message.tokens, Default::default());
        assert!((message.cost - 0.25).abs() < 1e-12);
        assert_eq!(message.cost_source, CostSource::ProviderReported);
        assert_eq!(message.timestamp, 1_788_609_600_000);
        assert_eq!(message.provider_id, "openai-codex");
        assert!(message.dedup_key.is_some());
    }
}

#[test]
fn aside_rejects_standalone_reasoning_without_eligible_cost() {
    // Given no disjoint tokens and missing, zero, negative or non-finite recorded cost.
    let home = tempfile::tempdir().unwrap();
    let rows = ["null", "0", "-1", "1e999", "-1e999"]
        .into_iter()
        .flat_map(|cost| ["", "\"output\":0,"].map(move |output| format!(
            "{{\"role\":\"assistant\",\"usage\":{{{output}\"reasoning\":99,\"cost\":{{\"total\":{cost}}}}}}}\n"
        )))
        .collect::<String>();
    write_session(
        home.path(),
        &(rows + "{\"role\":\"assistant\",\"usage\":{\"reasoning\":99}}\n"),
    );
    // When parsing the metadata-only rows.
    let messages = tokscale_core::sessions::aside::parse_aside_file(
        &home
            .path()
            .join(".aside/u/0/sessions/synthetic/messages.jsonl"),
    );
    // Then neither reasoning nor ineligible costs create a billable record.
    assert!(messages.is_empty());
}

#[tokio::test]
#[serial_test::serial]
async fn aside_cost_only_cached_uncached_and_legacy_preserve_provider_scoped_ids() {
    use tokscale_core::sessions::CostSource;
    use tokscale_core::{parse_local_clients, parse_local_unified_messages_with_pricing, ClientId};
    // Given cross-account duplicate IDs, provider reuse, and two ID-less cost-only calls.
    let home = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let _env = common::EnvGuard::set(&[
        ("TOKSCALE_CONFIG_DIR", cache.path().as_os_str()),
        ("XDG_CACHE_HOME", cache.path().as_os_str()),
    ]);
    let row = serde_json::json!({
        "role": "assistant", "timestamp": 1_788_609_600_000_i64,
        "model": "gpt-4o", "provider": "openai-codex", "responseId": "cost-only",
        "usage": {"cost": {"total": 0.25}}
    });
    let mut reused = row.clone();
    reused["provider"] = "anthropic".into();
    let mut no_id = row.clone();
    no_id.as_object_mut().unwrap().remove("responseId");
    let rows = [&row, &row, &reused, &no_id, &no_id]
        .map(|r| r.to_string() + "\n")
        .concat();
    let options = write_session(home.path(), &rows);
    let duplicate = home.path().join(".aside/u/1/sessions/other/messages.jsonl");
    std::fs::create_dir_all(duplicate.parent().unwrap()).unwrap();
    std::fs::write(duplicate, row.to_string() + "\n").unwrap();
    // When each public lane consumes the same native fixture.
    let uncached = parse_local_unified_messages_with_pricing_uncached(options.clone(), None)
        .await
        .unwrap();
    let cold = parse_local_unified_messages_with_pricing(options.clone(), None)
        .await
        .unwrap();
    let warm = parse_local_unified_messages_with_pricing(options.clone(), None)
        .await
        .unwrap();
    let legacy = parse_local_clients(options).unwrap();
    // Then all lanes retain costs/counts without tokens and deduplicate only scoped IDs.
    assert_eq!(
        [
            uncached.len(),
            cold.len(),
            warm.len(),
            legacy.messages.len()
        ],
        [4; 4]
    );
    assert_eq!(cold, warm);
    for messages in [&uncached, &cold, &warm] {
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.provider_id == "anthropic")
                .count(),
            1
        );
        assert_eq!(messages.iter().filter(|m| m.dedup_key.is_none()).count(), 2);
        assert!((messages.iter().map(|m| m.cost).sum::<f64>() - 1.0).abs() < 1e-12);
        assert!(messages.iter().all(|m| m.tokens == Default::default()
            && m.client == "aside"
            && m.timestamp == 1_788_609_600_000
            && m.cost_source == CostSource::ProviderReported));
    }
    assert_eq!(legacy.counts.get(ClientId::Aside), 4);
    assert!((legacy.messages.iter().map(|m| m.cost).sum::<f64>() - 1.0).abs() < 1e-12);
    assert!(legacy.messages.iter().all(|m| m.input == 0
        && m.output == 0
        && m.cache_read == 0
        && m.cache_write == 0
        && m.reasoning == 0
        && m.client == "aside"
        && m.timestamp == 1_788_609_600_000
        && m.cost_source == CostSource::ProviderReported));
}

#[tokio::test]
#[serial_test::serial]
async fn aside_cold_warm_and_legacy_dispatch_agree() {
    use tokscale_core::{parse_local_clients, parse_local_unified_messages_with_pricing, ClientId};
    // Given a repeated response and two ID-less calls, with isolated persistent caches.
    let home = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let _env = common::EnvGuard::set(&[
        ("TOKSCALE_CONFIG_DIR", cache.path().as_os_str()),
        ("XDG_CACHE_HOME", cache.path().as_os_str()),
    ]);
    let row = "{\"role\":\"assistant\",\"timestamp\":1788609600000,\"model\":\"gpt-4o\",\"provider\":\"openai-codex\",\"responseId\":\"repeat\",\"usage\":{\"input\":10,\"output\":20,\"cacheRead\":30,\"cacheWrite\":4,\"reasoning\":7,\"cost\":{\"total\":0.25}}}\n";
    let no_id = row.replace("\"responseId\":\"repeat\",", "");
    let options = write_session(home.path(), &(row.to_owned() + row + &no_id + &no_id));
    // When both cached passes and the legacy export path consume the same source.
    let cold = parse_local_unified_messages_with_pricing(options.clone(), None)
        .await
        .unwrap();
    let warm = parse_local_unified_messages_with_pricing(options.clone(), None)
        .await
        .unwrap();
    let legacy = parse_local_clients(options).unwrap();
    // Then cache hits preserve keys, source cost provenance and disjoint buckets.
    assert_eq!(cold.len(), 3);
    assert_eq!(cold, warm);
    assert_eq!(legacy.counts.get(ClientId::Aside), 3);
    assert_eq!(legacy.messages.len(), 3);
    assert_eq!(cold.iter().map(|m| m.tokens.total()).sum::<i64>(), 192);
    assert!((cold.iter().map(|m| m.cost).sum::<f64>() - 0.75).abs() < 1e-12);
    assert_eq!(legacy.messages.iter().map(|m| m.output).sum::<i64>(), 39);
}

#[test]
fn aside_timestamp_fallback_and_partial_tail_are_recoverable() {
    use tokscale_core::sessions::aside::parse_aside_file;
    // Given a row without a timestamp followed by an unfinished write.
    let home = tempfile::tempdir().unwrap();
    write_session(
        home.path(),
        "{\"role\":\"assistant\",\"usage\":{\"input\":1}}\n{\"role\":\"assistant\",\"usage\":{",
    );
    let path = home
        .path()
        .join(".aside/u/0/sessions/synthetic/messages.jsonl");
    let modified = i64::try_from(
        std::fs::metadata(&path)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    // When parsing a native transcript still being appended.
    let messages = parse_aside_file(&path);
    // Then the completed row survives with the file timestamp, without fabricated content.
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].timestamp, modified);
    assert_eq!(messages[0].tokens.total(), 1);
    assert_eq!(messages[0].model_id, "unknown");
    assert!(messages[0].dedup_key.is_none());
    assert!(messages[0].session_title.is_none());
}

#[cfg(unix)]
#[test]
fn aside_discovery_does_not_follow_account_or_transcript_symlinks() {
    use std::os::unix::fs::symlink;
    // Given aliases into memory and a transcript alias to a password file.
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join(".aside/u");
    let target = home.path().join("private/sessions/fake");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("messages.jsonl"), "{}").unwrap();
    std::fs::create_dir_all(root.join("0/sessions/fake")).unwrap();
    symlink(home.path().join("private"), root.join("linked-account")).unwrap();
    symlink(
        target.join("messages.jsonl"),
        root.join("0/sessions/fake/messages.jsonl"),
    )
    .unwrap();
    // When native discovery enumerates the account boundary.
    let scan =
        scan_all_clients_with_env_strategy(home.path().to_str().unwrap(), &["aside".into()], false);
    // Then symlinks cannot widen the allowed transcript tree.
    assert_eq!(scan.total_files(), 0);
}

fn write_session(home: &std::path::Path, content: &str) -> LocalParseOptions {
    let path = home.join(".aside/u/0/sessions/synthetic/messages.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
    LocalParseOptions {
        home_dir: Some(home.to_string_lossy().into_owned()),
        clients: Some(vec!["aside".into()]),
        ..Default::default()
    }
}

#[tokio::test]
async fn aside_skips_malformed_and_empty_usage_and_clamps_buckets() {
    // Given malformed, non-assistant, empty, negative and valid flat records.
    let home = tempfile::tempdir().unwrap();
    let options = write_session(home.path(), concat!(
        "not json\n",
        "{\"role\":\"user\",\"usage\":{\"input\":900}}\n",
        "{\"role\":\"assistant\",\"usage\":{\"input\":-10,\"output\":0}}\n",
        "{\"role\":\"assistant\",\"usage\":null}\n",
        "{\"role\":\"assistant\",\"timestamp\":\"2026-09-05T12:00:00Z\",\"model\":\"gpt-4o\",\"usage\":{\"input\":-2,\"output\":5,\"reasoning\":99,\"cacheRead\":3,\"cacheWrite\":-4,\"cost\":{\"total\":-1}}}\n",
        "{\"role\":\"assistant\",\"timestamp\":1788609600000,\"model\":\"gpt-4o\",\"provider\":\"aside\",\"usage\":{\"input\":2,\"output\":4,\"reasoning\":-1}}\n",
        "{\"role\":\"assistant\",\"usage\":{"
    ));
    // When parsed through discovery and native dispatch.
    let messages = parse_local_unified_messages_with_pricing_uncached(options, None)
        .await
        .unwrap();
    // Then negatives cannot reduce totals and reasoning cannot exceed output.
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].tokens.input, 0);
    assert_eq!(messages[0].tokens.output, 0);
    assert_eq!(messages[0].tokens.reasoning, 5);
    assert_eq!(messages[0].tokens.cache_read, 3);
    assert_eq!(messages[0].tokens.cache_write, 0);
    assert_eq!(messages[0].tokens.total(), 8);
    assert_eq!(messages[1].tokens.total(), 6);
    assert_eq!(messages[1].tokens.reasoning, 0);
    assert!(messages
        .iter()
        .all(|m| m.timestamp == 1_788_609_600_000 && m.cost == 0.0));
    assert!(messages.iter().all(|m| m.provider_id == "openai"));
}

#[tokio::test]
async fn aside_source_cost_and_estimates_do_not_double_bill_reasoning() {
    use std::collections::HashMap;
    use tokscale_core::pricing::{litellm::ModelPricing, PricingService};
    use tokscale_core::sessions::CostSource;
    // Given synthetic fixed rates and a paid response beside a gateway zero-cost row.
    let home = tempfile::tempdir().unwrap();
    let options = write_session(home.path(), concat!(
        "{\"role\":\"assistant\",\"model\":\"gpt-4o\",\"provider\":\"openai-codex\",\"usage\":{\"input\":10,\"output\":20,\"reasoning\":7,\"cacheRead\":30,\"cacheWrite\":4,\"cost\":{\"total\":0.25}}}\n",
        "{\"role\":\"assistant\",\"model\":\"gpt-4o\",\"provider\":\"aside\",\"usage\":{\"input\":10,\"output\":20,\"reasoning\":7,\"cacheRead\":30,\"cacheWrite\":4,\"cost\":{\"total\":0}}}\n"
    ));
    let pricing = PricingService::new(
        HashMap::from([(
            "gpt-4o".to_string(),
            ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                cache_read_input_token_cost: Some(0.0001),
                cache_creation_input_token_cost: Some(0.003),
                ..Default::default()
            },
        )]),
        HashMap::new(),
    );
    // When the standard pricing lane applies its existing cost contract.
    let messages = parse_local_unified_messages_with_pricing_uncached(options, Some(&pricing))
        .await
        .unwrap();
    // Then the recorded positive cost wins, zero falls back, and output is billed once.
    assert_eq!(messages.len(), 2);
    assert!((messages[0].cost - 0.25).abs() < 1e-12);
    assert_eq!(messages[0].cost_source, CostSource::ProviderReported);
    assert!(
        (messages[1].cost - 0.065).abs() < 1e-12,
        "{}",
        messages[1].cost
    );
    assert_eq!(messages[1].cost_source, CostSource::Estimated);
}

#[tokio::test]
async fn aside_accounts_responses_and_native_timestamps() {
    // Given a repeated response, a different provider, and two ID-less calls.
    let home = tempfile::tempdir().unwrap();
    let row = serde_json::json!({
        "role": "assistant", "timestamp": 1_788_598_095_439_i64,
        "model": "gpt-4o", "provider": "openai-codex", "responseId": "response-one",
        "usage": {"input": 10, "output": 20, "cacheRead": 30, "cacheWrite": 4,
            "reasoning": 7, "totalTokens": 64, "cost": {"total": 0.25}}
    });
    let mut other_provider = row.clone();
    other_provider["provider"] = "anthropic".into();
    let mut no_id = row.clone();
    no_id.as_object_mut().unwrap().remove("responseId");
    for (account, rows) in [
        ("0", vec![&row, &no_id, &no_id]),
        ("1", vec![&row, &other_provider]),
    ] {
        let path = home.path().join(format!(
            ".aside/u/{account}/sessions/session/messages.jsonl"
        ));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            rows.iter()
                .map(|row| row.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
    }

    // When the public native API scans and dispatches the flat format.
    let messages = parse_local_unified_messages_with_pricing_uncached(
        LocalParseOptions {
            home_dir: Some(home.path().to_string_lossy().into_owned()),
            clients: Some(vec!["aside".into()]),
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap();

    // Then routing identity is retained and reasoning is not counted twice.
    assert_eq!(messages.len(), 4);
    assert!(messages
        .iter()
        .all(|m| m.client == "aside" && m.timestamp == 1_788_598_095_439));
    assert_eq!(messages.iter().map(|m| m.tokens.total()).sum::<i64>(), 256);
    assert_eq!(messages.iter().map(|m| m.tokens.reasoning).sum::<i64>(), 28);
    assert_eq!(messages.iter().map(|m| m.tokens.output).sum::<i64>(), 52);
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.provider_id == "openai-codex")
            .count(),
        3
    );
    assert!((messages.iter().map(|m| m.cost).sum::<f64>() - 1.0).abs() < 1e-12);
}

#[test]
fn aside_discovers_only_session_transcripts() {
    // Given synthetic sessions and usage-shaped files outside the session boundary.
    let home = tempfile::tempdir().unwrap();
    let allowed = [
        ".aside/u/0/sessions/2026-09-05_one/messages.jsonl",
        ".aside/u/1/sessions/2026-09-06_two/messages.jsonl",
    ];
    let excluded = [
        ".aside/accounts.json",
        ".aside/u/0/passwords/messages.jsonl",
        ".aside/u/0/memory/messages.jsonl",
        ".aside/u/0/sessions/2026-09-05_one/artifacts/messages.jsonl",
        ".aside/u/0/sessions/2026-09-05_one/other.jsonl",
    ];
    for relative in allowed.iter().chain(excluded.iter()) {
        let path = home.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{}\n").unwrap();
    }

    // When the existing scanner is asked for the native client.
    let scan = scan_all_clients_with_env_strategy(
        home.path().to_str().unwrap(),
        &["aside".to_string()],
        false,
    );

    // Then only the exact transcripts are discoverable across accounts.
    let mut actual: Vec<_> = scan.all_files().into_iter().map(|(_, path)| path).collect();
    actual.sort();
    let expected: Vec<_> = allowed.iter().map(|path| home.path().join(path)).collect();
    assert_eq!(actual, expected);
}
