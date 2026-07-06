use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use tokscale_core::pricing::{litellm::ModelPricing, PricingService};
use tokscale_core::scanner::ScannerSettings;
use tokscale_core::sessions::junie::parse_junie_file;
use tokscale_core::{
    parse_local_clients, parse_local_unified_messages_with_pricing, ClientId, LocalParseOptions,
};

fn write_junie_session(home: &Path, session_id: &str, events: &str) -> PathBuf {
    let session_dir = home.join(".junie/sessions").join(session_id);
    fs::create_dir_all(&session_dir).unwrap();
    let events_path = session_dir.join("events.jsonl");
    fs::write(&events_path, events).unwrap();
    events_path
}

fn junie_options(home: &Path) -> LocalParseOptions {
    LocalParseOptions {
        home_dir: Some(home.to_str().unwrap().to_string()),
        use_env_roots: false,
        clients: Some(vec!["junie".to_string()]),
        since: None,
        until: None,
        year: None,
        scanner_settings: ScannerSettings::default(),
    }
}

fn make_pricing_service() -> PricingService {
    let mut litellm_data = HashMap::new();
    litellm_data.insert(
        "junie-test-model".to_string(),
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

#[test]
fn test_junie_parser_reads_model_usage_cost_tokens_and_turn_start() {
    let home_dir = tempfile::TempDir::new().unwrap();
    let home = home_dir.path();
    let session_id = "session-260618-191750-jnus";

    let events_path = write_junie_session(
        home,
        session_id,
        concat!(
            r#"{"kind":"UserPromptEvent","timestampMs":1781803079339}"#,
            "\n",
            r#"{"kind":"SessionA2uxEvent","event":{"state":"IN_PROGRESS","agentEvent":{"kind":"LlmResponseMetadataEvent","agent":{"kind":"MainAgent","id":"main","name":"main"},"modelUsage":[{"model":"gpt-4.1-2025-04-14","cost":0.42,"inputTokens":100,"cacheInputTokens":20,"cacheCreateTokens":5,"outputTokens":10,"reasoningTokens":3,"time":2500}]}},"timestampMs":1781803080555}"#,
            "\n",
        ),
    );

    let messages = parse_junie_file(&events_path);

    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message.client, "junie");
    assert_eq!(message.session_id, session_id);
    assert_eq!(message.model_id, "gpt-4.1-2025-04-14");
    assert_eq!(message.provider_id, "openai");
    assert_eq!(message.tokens.input, 100);
    assert_eq!(message.tokens.cache_read, 20);
    assert_eq!(message.tokens.cache_write, 5);
    assert_eq!(message.tokens.output, 10);
    assert_eq!(message.tokens.reasoning, 3);
    assert_eq!(message.cost, 0.42);
    assert_eq!(message.duration_ms, Some(2500));
    assert_eq!(message.agent.as_deref(), Some("main"));
    assert!(message.is_turn_start);
}

#[test]
fn test_junie_parser_infers_provider_from_model() {
    let home_dir = tempfile::TempDir::new().unwrap();
    let home = home_dir.path();

    let events_path = write_junie_session(
        home,
        "session-provider-inference",
        concat!(
            r#"{"kind":"SessionA2uxEvent","event":{"agentEvent":{"kind":"LlmResponseMetadataEvent","agent":{"id":"main"},"modelUsage":[{"model":"claude-opus-4-8","cost":0.5,"inputTokens":10,"outputTokens":2}]}},"timestampMs":1781803080555}"#,
            "\n",
        ),
    );

    let messages = parse_junie_file(&events_path);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].provider_id, "anthropic");
}

#[test]
fn test_junie_parser_uses_session_id_timestamp_when_event_has_no_timestamp() {
    let home_dir = tempfile::TempDir::new().unwrap();
    let home = home_dir.path();

    let events_path = write_junie_session(
        home,
        "session-260618-191750-jnus",
        concat!(
            r#"{"kind":"SessionA2uxEvent","event":{"state":"IN_PROGRESS","agentEvent":{"kind":"LlmResponseMetadataEvent","modelUsage":[{"model":"gpt-4.1-2025-04-14","cost":0.5,"inputTokens":10,"outputTokens":2}]}}}"#,
            "\n",
        ),
    );

    let messages = parse_junie_file(&events_path);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].date, "2026-06-18");
    assert!(messages[0].timestamp > 0);
}

#[tokio::test]
async fn test_junie_end_to_end_preserves_embedded_cost_and_prices_missing_cost() {
    let home_dir = tempfile::TempDir::new().unwrap();
    let home = home_dir.path();

    write_junie_session(
        home,
        "session-priced",
        concat!(
            r#"{"kind":"UserPromptEvent","timestampMs":1781803079339}"#,
            "\n",
            r#"{"kind":"SessionA2uxEvent","event":{"agentEvent":{"kind":"LlmResponseMetadataEvent","agent":{"id":"main"},"modelUsage":[{"model":"junie-test-model","cost":0.123,"inputTokens":1000,"outputTokens":250},{"model":"junie-test-model","inputTokens":10,"cacheInputTokens":2,"cacheCreateTokens":3,"outputTokens":5,"reasoningTokens":1}]}},"timestampMs":1781803080555}"#,
            "\n",
        ),
    );

    let pricing = make_pricing_service();
    let messages = parse_local_unified_messages_with_pricing(junie_options(home), Some(&pricing))
        .await
        .unwrap();

    assert_eq!(messages.len(), 2);
    let embedded = messages
        .iter()
        .find(|message| (message.cost - 0.123).abs() < 1e-10)
        .expect("embedded-cost Junie message was not preserved");
    assert_eq!(embedded.tokens.input, 1000);

    let priced = messages
        .iter()
        .find(|message| message.tokens.input == 10)
        .expect("missing-cost Junie message was not repriced");
    let expected = 10.0 * 0.001 + (5.0 + 1.0) * 0.002 + 2.0 * 0.0001 + 3.0 * 0.0005;
    assert!((priced.cost - expected).abs() < 1e-10);

    let parsed = parse_local_clients(junie_options(home)).unwrap();
    assert_eq!(parsed.counts.get(ClientId::Junie), 2);
    assert_eq!(
        parsed
            .messages
            .iter()
            .filter(|message| message.client == "junie")
            .count(),
        2
    );
}
