mod common;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use tokscale_core::pricing::{litellm::ModelPricing, PricingService};
use tokscale_core::scanner::ScannerSettings;
use tokscale_core::sessions::muse::parse_muse_file;
use tokscale_core::{
    parse_local_clients, parse_local_unified_messages_with_pricing_uncached, ClientId,
    LocalParseOptions,
};

const SESSION_ID: &str = "01a0b7d1-b7af-7de2-8369-4f4815ae1d72";

fn write_muse_session(home: &Path, date: &str, session_id: &str, lines: &str) -> PathBuf {
    let session_dir = home
        .join(".local/share/muse/sessions")
        .join(date)
        .join(session_id);
    fs::create_dir_all(&session_dir).unwrap();
    let session_path = session_dir.join("session.jsonl");
    fs::write(&session_path, lines).unwrap();
    session_path
}

fn muse_options(home: &Path) -> LocalParseOptions {
    LocalParseOptions {
        home_dir: Some(home.to_str().unwrap().to_string()),
        use_env_roots: false,
        clients: Some(vec!["muse".to_string()]),
        since: None,
        until: None,
        year: None,
        scanner_settings: ScannerSettings::default(),
    }
}

fn make_pricing_service() -> PricingService {
    let mut litellm_data = HashMap::new();
    litellm_data.insert(
        "muse-spark-1.3-contributor".to_string(),
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

fn usage_line(
    sequence: i64,
    recorded_at: i64,
    model: &str,
    usage: &str,
    duration_ms: i64,
) -> String {
    format!(
        r#"{{"schema_version":1,"stream":{{"kind":"session","id":"{SESSION_ID}"}},"sequence":{sequence},"recorded_at":{recorded_at},"record_type":"event","payload_type":"runtime.session","payload_schema_version":1,"payload":{{"kind":"run","run_id":"898f5ab5-88d7-4baa-a281-86040f9d6a57","event":{{"kind":"model_completed","usage":{usage},"duration_ms":{duration_ms},"finish_reason":"tool_calls","model":"{model}"}}}}}}"#
    )
}

#[test]
fn test_muse_parser_reads_usage_skips_aggregates_and_attaches_workspace() {
    let home_dir = common::temp_home();
    let home = home_dir.path();
    let workspace = home.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let root = workspace.to_string_lossy().replace('\\', "\\\\");

    let lines = [
        format!(
            r#"{{"sequence":3,"recorded_at":1789790369778491,"payload_type":"runtime.session.metadata","payload":{{"kind":"metadata","record":{{"workspace_root":"{root}","provider_id":"meta","model_id":"muse-spark-1.3-contributor"}}}}}}"#
        ),
        // A tool-effect record carrying no usage.
        r#"{"sequence":30,"recorded_at":1789790455896390,"payload_type":"tool_batch.effect.started","payload":{"kind":"tool"}}"#.to_string(),
        usage_line(
            39,
            1789790455896395,
            "muse-spark-1.3-contributor",
            r#"{"input_tokens":26964,"output_tokens":379,"cached_tokens":5105,"cache_write_tokens":0,"cache_read_tokens":5105,"reasoning_tokens":278}"#,
            5819,
        ),
        // Parent-side aggregate of a workflow child: the child transcript is
        // scanned separately, so this must not produce a message.
        r#"{"sequence":282,"recorded_at":1789790455896400,"payload_type":"runtime.session","payload":{"kind":"run","event":{"kind":"workflow_child_lifecycle","status":"usage","usage":{"input_tokens":917394,"output_tokens":5917,"cached_tokens":845908,"cache_read_tokens":845908,"reasoning_tokens":926}}}}"#.to_string(),
    ]
    .join("\n");
    let path = write_muse_session(home, "2026/09/18", SESSION_ID, &lines);

    let messages = parse_muse_file(&path);
    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message.client, "muse");
    assert_eq!(message.session_id, SESSION_ID);
    assert_eq!(message.model_id, "muse-spark-1.3-contributor");
    assert_eq!(message.provider_id, "muse");
    assert_eq!(message.tokens.input, 26964 - 5105);
    assert_eq!(message.tokens.cache_read, 5105);
    assert_eq!(message.tokens.output, 379 - 278);
    assert_eq!(message.tokens.reasoning, 278);
    assert_eq!(message.duration_ms, Some(5819));
    assert_eq!(message.workspace_label.as_deref(), Some("repo"));
}

#[tokio::test]
async fn test_muse_end_to_end_discovers_nested_sessions_and_prices_tokens() {
    let home_dir = common::temp_home();
    let home = home_dir.path();

    write_muse_session(
        home,
        "2026/09/18",
        SESSION_ID,
        &usage_line(
            39,
            1789790455896395,
            "muse-spark-1.3-contributor",
            r#"{"input_tokens":1000,"output_tokens":250,"cached_tokens":400,"cache_write_tokens":50,"cache_read_tokens":400,"reasoning_tokens":25}"#,
            1000,
        ),
    );
    // A subagent transcript beside the parent session is its own session.
    let subagent_dir = home
        .join(".local/share/muse/sessions/2026/09/18")
        .join(SESSION_ID)
        .join("subagent/01a0b7df-bac6-75d2-b28e-8dab127205f0");
    fs::create_dir_all(&subagent_dir).unwrap();
    fs::write(
        subagent_dir.join("session.jsonl"),
        usage_line(
            40,
            1789791288021470,
            "muse-spark-1.3-contributor",
            r#"{"input_tokens":100,"output_tokens":10,"cached_tokens":0,"cache_write_tokens":0,"cache_read_tokens":0,"reasoning_tokens":0}"#,
            100,
        )
        .replace(SESSION_ID, "01a0b7df-bac6-75d2-b28e-8dab127205f0"),
    )
    .unwrap();

    let pricing = make_pricing_service();
    let messages =
        parse_local_unified_messages_with_pricing_uncached(muse_options(home), Some(&pricing))
            .await
            .unwrap();

    assert_eq!(messages.len(), 2);
    let parent = messages
        .iter()
        .find(|message| message.session_id == SESSION_ID)
        .expect("parent Muse session was not discovered");
    // Cache reads are split out of input; reasoning is split out of output.
    assert_eq!(parent.tokens.input, 1000 - 400);
    assert_eq!(parent.tokens.cache_read, 400);
    assert_eq!(parent.tokens.output, 250 - 25);
    assert_eq!(parent.tokens.reasoning, 25);
    assert_eq!(parent.tokens.cache_write, 50);
    let expected = 600.0 * 0.001 + (225.0 + 25.0) * 0.002 + 400.0 * 0.0001 + 50.0 * 0.0005;
    assert!((parent.cost - expected).abs() < 1e-10);

    let parsed = parse_local_clients(muse_options(home)).unwrap();
    assert_eq!(parsed.counts.get(ClientId::Muse), 2);
    assert_eq!(
        parsed
            .messages
            .iter()
            .filter(|message| message.client == "muse")
            .count(),
        2
    );
}
