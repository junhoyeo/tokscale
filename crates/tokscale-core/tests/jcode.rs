mod common;

use std::collections::HashMap;
use std::fs;

use tokscale_core::pricing::{litellm::ModelPricing, PricingService};
use tokscale_core::scanner::ScannerSettings;
use tokscale_core::{parse_local_unified_messages_with_pricing_uncached, LocalParseOptions};

fn make_pricing_service() -> PricingService {
    let mut litellm_data = HashMap::new();
    litellm_data.insert(
        "jcode-test-model".to_string(),
        ModelPricing {
            input_cost_per_token: Some(0.001),
            output_cost_per_token: Some(0.002),
            cache_read_input_token_cost: Some(0.0001),
            cache_creation_input_token_cost: Some(0.0005),
            ..Default::default()
        },
    );
    PricingService::new(litellm_data, HashMap::new())
}

fn write_jcode_session(home: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let dir = home.join(".jcode/sessions");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

#[tokio::test]
async fn test_jcode_end_to_end_parsing_and_pricing() {
    let home_dir = common::temp_home();
    let home = home_dir.path();

    write_jcode_session(
        home,
        "session_test.json",
        r#"{
  "id":"session_test",
  "provider_key":"cliproxyapi",
  "model":"jcode-test-model",
  "working_dir":"/work/project",
  "messages":[
    {"id":"user_1","role":"user","timestamp":"2026-06-16T00:00:00Z"},
    {"id":"assistant_1","role":"assistant","timestamp":"2026-06-16T00:00:01Z","token_usage":{"input_tokens":1000,"output_tokens":250,"cache_read_input_tokens":400,"cache_creation_input_tokens":100,"reasoning_output_tokens":25},"tool_duration_ms":2500}
  ]
}"#,
    );

    let pricing = make_pricing_service();
    let messages = parse_local_unified_messages_with_pricing_uncached(
        LocalParseOptions {
            home_dir: Some(home.to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["jcode".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: ScannerSettings::default(),
        },
        Some(&pricing),
    )
    .await
    .unwrap();

    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message.client, "jcode");
    assert_eq!(message.session_id, "session_test");
    assert_eq!(message.model_id, "jcode-test-model");
    assert_eq!(message.provider_id, "cliproxyapi");
    assert_eq!(message.tokens.input, 1000);
    assert_eq!(message.tokens.cache_read, 400);
    assert_eq!(message.tokens.cache_write, 100);
    assert_eq!(message.tokens.output, 250);
    assert_eq!(message.tokens.reasoning, 25);
    assert_eq!(message.duration_ms, Some(2500));
    assert_eq!(message.workspace_label.as_deref(), Some("project"));

    // Reasoning tokens are intentionally billed through the output-token price.
    let expected_cost = 1000.0 * 0.001 + (250.0 + 25.0) * 0.002 + 400.0 * 0.0001 + 100.0 * 0.0005;
    assert!((message.cost - expected_cost).abs() < 1e-10);
}

#[tokio::test]
async fn test_jcode_deduplicates_replayed_message_ids() {
    let home_dir = common::temp_home();
    let home = home_dir.path();

    let session_body = r#"{
  "id":"session_replay",
  "provider_key":"openai",
  "model":"jcode-test-model",
  "messages":[
    {"id":"assistant_replayed","role":"assistant","timestamp":"2026-06-16T00:00:01Z","token_usage":{"input_tokens":100,"output_tokens":10}},
    {"id":"assistant_replayed","role":"assistant","timestamp":"2026-06-16T00:00:02Z","token_usage":{"input_tokens":100,"output_tokens":10}}
  ]
}"#;
    write_jcode_session(home, "session_a.json", session_body);

    let messages = parse_local_unified_messages_with_pricing_uncached(
        LocalParseOptions {
            home_dir: Some(home.to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["jcode".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: ScannerSettings::default(),
        },
        None,
    )
    .await
    .unwrap();

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].tokens.input, 100);
}

/// Regression: when the snapshot replays a duplicate message id, the journal
/// update for that id must target the *surviving* (first) occurrence, because
/// downstream dedup is first-wins. A previous version built the merge index
/// with `collect()`, which kept the *last* duplicate index — so the journal
/// overwrote a row that dedup later discarded, preserving the stale first
/// snapshot token_usage instead of the corrected journal value.
#[tokio::test]
async fn test_jcode_journal_corrects_replayed_snapshot_duplicate() {
    let home_dir = common::temp_home();
    let home = home_dir.path();

    // Snapshot replays the same message id twice (stale token_usage = 100).
    let session_body = r#"{
  "id":"session_dup",
  "provider_key":"openai",
  "model":"jcode-test-model",
  "messages":[
    {"id":"assistant_replayed","role":"assistant","timestamp":"2026-06-16T00:00:01Z","token_usage":{"input_tokens":100,"output_tokens":10}},
    {"id":"assistant_replayed","role":"assistant","timestamp":"2026-06-16T00:00:02Z","token_usage":{"input_tokens":100,"output_tokens":10}}
  ]
}"#;
    let session_path = write_jcode_session(home, "session_dup.json", session_body);

    // Journal carries the authoritative correction (input_tokens = 999) for
    // the same message id.
    let journal_path = session_path.with_file_name("session_dup.journal.jsonl");
    fs::write(
        &journal_path,
        r#"{"append_messages":[{"id":"assistant_replayed","role":"assistant","timestamp":"2026-06-16T00:00:03Z","token_usage":{"input_tokens":999,"output_tokens":10}}]}
"#,
    )
    .unwrap();

    let messages = parse_local_unified_messages_with_pricing_uncached(
        LocalParseOptions {
            home_dir: Some(home.to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["jcode".to_string()]),
            since: None,
            until: None,
            year: None,
            scanner_settings: ScannerSettings::default(),
        },
        None,
    )
    .await
    .unwrap();

    assert_eq!(messages.len(), 1);
    // The journal correction must win over the stale snapshot duplicate.
    assert_eq!(messages[0].tokens.input, 999);
}

#[test]
fn test_jcode_billing_snapshot_switch_does_not_relabel_history() {
    use serde_json::json;
    use tokscale_core::sessions::{jcode::parse_jcode_file, BillingIdentity};

    let home_dir = common::temp_home();
    let mut session = json!({
        "id": "session_switch",
        "provider_key": "anthropic-api-key",
        "model": "claude-sonnet-4",
        "messages": [{
            "id": "before_switch",
            "role": "assistant",
            "timestamp": "2026-06-16T00:00:01Z",
            "token_usage": {
                "input_tokens": 100,
                "output_tokens": 10,
                "cache_read_input_tokens": 40,
                "cache_creation_input_tokens": 0
            }
        }]
    });
    let path = write_jcode_session(home_dir.path(), "session_switch.json", &session.to_string());
    let original = parse_jcode_file(&path);
    assert_eq!(original.len(), 1);
    assert_eq!(original[0].billing_identity, BillingIdentity::Unknown);

    // Jcode changes provider_key in place and checkpoints the whole session.
    // It does not record the historical route on each StoredMessage.
    session["provider_key"] = json!("openrouter");
    session["messages"].as_array_mut().unwrap().push(json!({
        "id": "after_switch",
        "role": "assistant",
        "timestamp": "2026-06-16T00:00:02Z",
        "token_usage": {
            "input_tokens": 100,
            "output_tokens": 20,
            "cache_read_input_tokens": 40
        }
    }));
    fs::write(&path, session.to_string()).unwrap();
    let switched = parse_jcode_file(&path);
    assert_eq!(switched.len(), 2);
    for message in &switched {
        assert_eq!(message.billing_identity, BillingIdentity::Unknown);
        // Preserve the established display attribution, even though it cannot
        // prove which billing channel served a historical message.
        assert_eq!(message.provider_id, "openrouter");
        assert_eq!(message.model_id, "claude-sonnet-4");
    }
    // Message-local cache accounting still distinguishes the two API shapes.
    assert_eq!(switched[0].tokens.input, 100);
    assert_eq!(switched[1].tokens.input, 60);
    assert_eq!(switched[0].tokens.cache_read, 40);
    assert_eq!(switched[1].tokens.cache_read, 40);
}

#[test]
fn test_jcode_billing_journal_metadata_is_scoped_to_its_batch() {
    use serde_json::json;
    use tokscale_core::sessions::{
        jcode::parse_jcode_file, BillingIdentity, BillingIdentityOrigin,
    };

    let home_dir = common::temp_home();
    let path = write_jcode_session(
        home_dir.path(),
        "session_batch_scope.json",
        r#"{"id":"session_batch_scope","provider_key":"snapshot-route","model":"jcode-test-model","messages":[]}"#,
    );
    let metadata = [
        Some(json!({"provider_key":" Gateway-A "})),
        None,
        Some(json!({"model":"journal-model"})),
        Some(json!({"provider_key":null})),
        Some(json!({"provider_key":" "})),
        Some(json!({"provider_key":"unknown"})),
        Some(json!({"provider_key":"gateway-b"})),
    ];
    let lines = metadata
        .into_iter()
        .enumerate()
        .map(|(index, meta)| {
            let mut entry = json!({"append_messages":[{
                "id":format!("message_{index}"),
                "role":"assistant",
                "timestamp":"2026-06-16T00:00:01Z",
                "token_usage":{"input_tokens":100,"output_tokens":10}
            }]});
            if let Some(meta) = meta {
                entry["meta"] = meta;
            }
            entry.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(path.with_extension("journal.jsonl"), lines).unwrap();

    let messages = parse_jcode_file(&path);
    assert_eq!(messages.len(), 7);
    assert_eq!(
        messages[0].billing_identity,
        BillingIdentity::Known {
            id: "gateway-a".to_string(),
            origin: BillingIdentityOrigin::ExplicitSession,
        }
    );
    for message in &messages[1..6] {
        assert_eq!(message.billing_identity, BillingIdentity::Unknown);
    }
    assert_eq!(
        messages[6].billing_identity,
        BillingIdentity::Known {
            id: "gateway-b".to_string(),
            origin: BillingIdentityOrigin::ExplicitSession,
        }
    );
    assert_eq!(messages[0].model_id, "jcode-test-model");
    assert_eq!(messages[6].model_id, "journal-model");
}

#[test]
fn test_jcode_billing_journal_corrections_preserve_original_scope() {
    use tokscale_core::sessions::{
        jcode::parse_jcode_file, BillingIdentity, BillingIdentityOrigin,
    };

    let home_dir = common::temp_home();
    let path = write_jcode_session(
        home_dir.path(),
        "session_correction_scope.json",
        r#"{
  "id":"session_correction_scope",
  "provider_key":"current-route",
  "model":"jcode-test-model",
  "messages":[
    {"id":"historical","role":"assistant","timestamp":"2026-06-16T00:00:01Z","token_usage":{"input_tokens":100,"output_tokens":10}}
  ]
}"#,
    );
    fs::write(
        path.with_extension("journal.jsonl"),
        r#"{"meta":{"provider_key":"gateway-a"},"append_messages":[{"id":"historical","role":"assistant","timestamp":"2026-06-16T00:00:01Z","token_usage":{"input_tokens":110,"output_tokens":10}},{"id":"new_a","role":"assistant","timestamp":"2026-06-16T00:00:02Z","token_usage":{"input_tokens":200,"output_tokens":20}}]}
{"meta":{"provider_key":"gateway-b"},"append_messages":[{"id":"new_a","role":"assistant","timestamp":"2026-06-16T00:00:02Z","token_usage":{"input_tokens":220,"output_tokens":20}},{"id":"new_b","role":"assistant","timestamp":"2026-06-16T00:00:03Z","token_usage":{"input_tokens":300,"output_tokens":30}}]}
{"append_messages":[{"id":"new_b","role":"assistant","timestamp":"2026-06-16T00:00:03Z","token_usage":{"input_tokens":330,"output_tokens":30}}]}
"#,
    )
    .unwrap();

    let messages = parse_jcode_file(&path);
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].billing_identity, BillingIdentity::Unknown);
    for (message, provider, input) in [
        (&messages[1], "gateway-a", 220),
        (&messages[2], "gateway-b", 330),
    ] {
        assert_eq!(
            message.billing_identity,
            BillingIdentity::Known {
                id: provider.to_string(),
                origin: BillingIdentityOrigin::ExplicitSession,
            }
        );
        assert_eq!(message.tokens.input, input);
    }
    assert_eq!(messages[0].tokens.input, 110);
    // Keep upstream correction/display behavior separate from billing evidence.
    assert_eq!(messages[1].provider_id, "gateway_b");
}

#[tokio::test]
async fn test_jcode_billing_pricing_uses_only_proven_journal_scope() {
    use tokscale_core::pricing::custom::CustomPricing;
    use tokscale_core::sessions::{BillingIdentity, BillingIdentityOrigin};

    let home_dir = common::temp_home();
    let path = write_jcode_session(
        home_dir.path(),
        "session_scope_pricing.json",
        r#"{
  "id":"session_scope_pricing",
  "provider_key":"gateway-b",
  "model":"jcode-test-model",
  "messages":[
    {"id":"before_switch","role":"assistant","timestamp":"2026-06-16T00:00:01Z","token_usage":{"input_tokens":1000000,"output_tokens":0}},
    {"id":"after_switch","role":"assistant","timestamp":"2026-06-16T00:00:02Z","token_usage":{"input_tokens":1000000,"output_tokens":0}}
  ]
}"#,
    );
    fs::write(
        path.with_extension("journal.jsonl"),
        r#"{"meta":{"provider_key":"gateway-b","model":"jcode-test-model"},"append_messages":[{"id":"journal","role":"assistant","timestamp":"2026-06-16T00:00:03Z","token_usage":{"input_tokens":1000000,"output_tokens":0}}]}"#,
    )
    .unwrap();
    let custom = CustomPricing::load_from_str(
        r#"{
  "models":{"jcode-test-model":{"input_cost_per_million_tokens":2}},
  "provider_overrides":[
    {"provider_id":"gateway-b","model_id":"jcode-test-model","pricing":{"input_cost_per_million_tokens":7}}
  ]
}"#,
        std::path::Path::new("jcode-scope-pricing.json"),
    );
    assert!(custom.diagnostics().is_empty());
    let pricing = PricingService::new_with_custom(custom, HashMap::new(), HashMap::new());
    let messages = parse_local_unified_messages_with_pricing_uncached(
        LocalParseOptions {
            home_dir: Some(home_dir.path().to_str().unwrap().to_string()),
            use_env_roots: false,
            clients: Some(vec!["jcode".to_string()]),
            ..Default::default()
        },
        Some(&pricing),
    )
    .await
    .unwrap();

    assert_eq!(messages.len(), 3);
    for message in &messages {
        if message.dedup_key.as_deref() == Some("jcode:session_scope_pricing:journal") {
            assert_eq!(message.cost, 7.0);
            assert_eq!(
                message.billing_identity,
                BillingIdentity::Known {
                    id: "gateway-b".to_string(),
                    origin: BillingIdentityOrigin::ExplicitSession,
                }
            );
        } else {
            assert_eq!(message.cost, 2.0);
            assert_eq!(message.billing_identity, BillingIdentity::Unknown);
        }
    }
}

#[test]
fn test_jcode_billing_snapshot_ids_without_emitted_usage_stay_unknown() {
    use serde_json::json;
    use tokscale_core::sessions::{jcode::parse_jcode_file, BillingIdentity};

    let home_dir = common::temp_home();
    let mut historical = Vec::new();
    let mut corrections = Vec::new();
    for (index, usage) in [
        None,
        Some(serde_json::Value::Null),
        Some(json!({"input_tokens":0,"output_tokens":0})),
        Some(json!({"input_tokens":-100,"output_tokens":-10})),
        Some(json!("corrupt")),
        Some(json!({"input_tokens":"corrupt"})),
    ]
    .into_iter()
    .enumerate()
    {
        let mut message = json!({
            "id":format!("historical_{index}"),
            "role":"assistant",
            "timestamp":"2026-06-16T00:00:01Z"
        });
        if let Some(usage) = usage {
            message["token_usage"] = usage;
        }
        historical.push(message.clone());
        message["token_usage"] = json!({"input_tokens":100,"output_tokens":10});
        corrections.push(message);
    }
    let path = write_jcode_session(
        home_dir.path(),
        "session_nonemitted_snapshot.json",
        &json!({
            "id":"session_nonemitted_snapshot",
            "provider_key":"gateway-b",
            "model":"jcode-test-model",
            "messages":historical
        })
        .to_string(),
    );
    assert!(parse_jcode_file(&path).is_empty());
    fs::write(
        path.with_extension("journal.jsonl"),
        json!({"meta":{"provider_key":"gateway-b"},"append_messages":corrections}).to_string(),
    )
    .unwrap();

    let messages = parse_jcode_file(&path);
    assert_eq!(messages.len(), 6);
    for message in &messages {
        assert_eq!(message.billing_identity, BillingIdentity::Unknown);
        assert_eq!(message.tokens.input, 100);
        assert_eq!(message.tokens.output, 10);
    }
}

#[test]
fn test_jcode_billing_nonemitted_journal_ids_keep_their_first_batch_scope() {
    use serde_json::json;
    use tokscale_core::sessions::{
        jcode::parse_jcode_file, BillingIdentity, BillingIdentityOrigin,
    };

    for first_provider in [Some("gateway-a"), None] {
        let home_dir = common::temp_home();
        let path = write_jcode_session(
            home_dir.path(),
            "session_nonemitted_journal.json",
            r#"{"id":"session_nonemitted_journal","provider_key":"snapshot-route","model":"jcode-test-model","messages":[]}"#,
        );
        let mut original = Vec::new();
        let mut corrections = Vec::new();
        for (index, usage) in [
            None,
            Some(json!({"input_tokens":0,"output_tokens":0})),
            Some(json!("corrupt")),
        ]
        .into_iter()
        .enumerate()
        {
            let mut message = json!({
                "id":format!("journal_{index}"),
                "role":"assistant",
                "timestamp":"2026-06-16T00:00:01Z"
            });
            if let Some(usage) = usage {
                message["token_usage"] = usage;
            }
            original.push(message.clone());
            message["token_usage"] = json!({"input_tokens":200,"output_tokens":20});
            corrections.push(message);
        }
        let mut first_entry = json!({"append_messages":original});
        if let Some(provider) = first_provider {
            first_entry["meta"] = json!({"provider_key":provider});
        }
        let journal = path.with_extension("journal.jsonl");
        fs::write(&journal, first_entry.to_string()).unwrap();
        assert!(parse_jcode_file(&path).is_empty());
        let later_entry =
            json!({"meta":{"provider_key":"gateway-b"},"append_messages":corrections});
        fs::write(&journal, format!("{first_entry}\n{later_entry}\n")).unwrap();

        let messages = parse_jcode_file(&path);
        assert_eq!(messages.len(), 3);
        let expected = first_provider.map_or(BillingIdentity::Unknown, |provider| {
            BillingIdentity::Known {
                id: provider.to_string(),
                origin: BillingIdentityOrigin::ExplicitSession,
            }
        });
        for message in &messages {
            assert_eq!(message.billing_identity, expected);
            assert_eq!(message.tokens.input, 200);
            assert_eq!(message.tokens.output, 20);
        }
    }
}
