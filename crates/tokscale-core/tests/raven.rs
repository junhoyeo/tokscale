use std::fs;

use tempfile::tempdir;
use tokscale_core::{
    scanner::scan_all_clients_with_env_strategy,
    sessions::{raven::parse_raven_file, CostSource},
    ClientId,
};

#[test]
fn scans_raven_telemetry_with_versioned_token_buckets() {
    let home = tempdir().unwrap();
    let raven = home.path().join(".raven");
    let telemetry = raven.join("telemetry");
    fs::create_dir_all(&telemetry).unwrap();
    let row = r#"{"ts":"2026-11-23T23:59:59.999+00:00","schema_version":2,"model":"gpt-5","input_tokens":60,"output_tokens":40,"cache_read_tokens":30,"cache_write_tokens":10,"reasoning_tokens":10,"cost_usd":0,"session_key":"child","root_session_key":"parent"}"#;
    let log = telemetry.join("usage-2026-11-24.jsonl");
    fs::write(
        &log,
        format!(
            "{row}\n{row}\n{}\n{}\n{}\n{{",
            r#"{"ts":"2026-11-23T00:01:00Z","schema_version":1,"model":"gpt-5","input_tokens":7,"output_tokens":5,"reasoning_tokens":2,"cache_write_tokens":null,"cost_usd":null,"session_key":"parent"}"#,
            r#"{"ts":"2026-11-23T00:02:00Z","schema_version":2,"_type":"tool_call","name":"example","output_tokens":99,"session_key":"parent"}"#,
            r#"{"ts":"2026-11-23T00:03:00Z","schema_version":3,"input_tokens":99}"#,
        ),
    )
    .unwrap();
    fs::write(raven.join("session.jsonl"), row).unwrap();
    fs::write(telemetry.join("usage-2026-11-24-copy.jsonl"), row).unwrap();

    let scan =
        scan_all_clients_with_env_strategy(home.path().to_str().unwrap(), &["raven".into()], false);
    assert_eq!(scan.get(ClientId::Raven), std::slice::from_ref(&log));

    let messages = parse_raven_file(&log);
    assert_eq!(messages.len(), 3, "identical calls are still separate rows");
    let call = &messages[0];
    assert_eq!(call.timestamp, 1_795_478_399_999);
    assert_eq!(call.tokens.total(), 140);
    assert_eq!(call.tokens.input, 60);
    assert_eq!(call.tokens.output, 30);
    assert_eq!(call.tokens.reasoning, 10);
    assert_eq!(call.tokens.cache_read, 30);
    assert_eq!(call.tokens.cache_write, 10);
    assert_eq!(call.cost, 0.0);
    assert_eq!(call.cost_source, CostSource::ProviderReported);
    assert_eq!(call.session_id.as_ref(), "child");
    assert_eq!(call.parent_session_id.as_deref(), Some("parent"));
    assert_eq!(messages[1].tokens, call.tokens);
    assert_eq!(messages[2].tokens.total(), 14);
    assert_eq!(messages[2].tokens.output, 5);
    assert_eq!(messages[2].cost_source, CostSource::Unknown);
}
