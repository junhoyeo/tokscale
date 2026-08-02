use std::collections::HashMap;
use std::fs;

use tokscale_core::pricing::{litellm::ModelPricing, PricingService};
use tokscale_core::scanner::ScannerSettings;
use tokscale_core::{parse_local_unified_messages_with_pricing, LocalParseOptions};

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
    let home_dir = tempfile::TempDir::new().unwrap();
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
    let messages = parse_local_unified_messages_with_pricing(
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
    let home_dir = tempfile::TempDir::new().unwrap();
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

    let messages = parse_local_unified_messages_with_pricing(
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
    let home_dir = tempfile::TempDir::new().unwrap();
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

    let messages = parse_local_unified_messages_with_pricing(
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
