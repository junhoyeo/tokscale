use std::path::Path;
use tempfile::TempDir;
use tokscale_core::{
    parse_local_clients, parse_local_unified_messages_with_pricing_uncached,
    scanner::{scan_all_clients_with_scanner_settings, ScannerSettings},
    ClientId, LocalParseOptions,
};

fn qoder_options(home: &Path) -> LocalParseOptions {
    LocalParseOptions {
        home_dir: Some(home.to_str().unwrap().to_string()),
        use_env_roots: false,
        clients: Some(vec!["qoder".to_string()]),
        since: None,
        until: None,
        year: None,
        scanner_settings: ScannerSettings::default(),
    }
}

#[tokio::test]
async fn test_qoder_and_qoder_cn_scanning_and_fork_dedup() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();

    let qoder_proj = home.join(".qoder/projects/app-intl");
    let qoder_cn_proj = home.join(".qoder-cn/projects/app-cn");
    std::fs::create_dir_all(&qoder_proj).unwrap();
    std::fs::create_dir_all(&qoder_cn_proj).unwrap();

    // Parent session in ~/.qoder
    std::fs::write(
        qoder_proj.join("sess-intl-1.jsonl"),
        concat!(
            r#"{"type":"user","uuid":"u-1","sessionId":"sess-intl-1","cwd":"/work/app-intl","timestamp":"2026-11-07T10:00:00.000Z","message":{"role":"user","content":"Hello Qoder"}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a-1","sessionId":"sess-intl-1","cwd":"/work/app-intl","timestamp":"2026-11-07T10:00:02.000Z","message":{"id":"msg_intl_1","role":"assistant","model":"qwen3.8-max","stop_reason":"end_turn","content":[{"type":"text","text":"Hi!"}],"usage":{"input_tokens":500,"output_tokens":120,"cache_read_input_tokens":300,"cache_creation_input_tokens":50}}}"#,
            "\n"
        ),
    )
    .unwrap();

    // Forked session in ~/.qoder replaying msg_intl_1 plus a new turn msg_intl_2
    std::fs::write(
        qoder_proj.join("sess-intl-fork.jsonl"),
        concat!(
            r#"{"type":"assistant","uuid":"a-1","sessionId":"sess-intl-fork","cwd":"/work/app-intl","timestamp":"2026-11-07T10:00:02.000Z","message":{"id":"msg_intl_1","role":"assistant","model":"qwen3.8-max","stop_reason":"end_turn","content":[{"type":"text","text":"Hi!"}],"usage":{"input_tokens":500,"output_tokens":120,"cache_read_input_tokens":300,"cache_creation_input_tokens":50}}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"a-2","sessionId":"sess-intl-fork","cwd":"/work/app-intl","timestamp":"2026-11-07T10:01:00.000Z","message":{"id":"msg_intl_2","role":"assistant","model":"qwen3.8-max","stop_reason":"end_turn","content":[{"type":"text","text":"Fork reply"}],"usage":{"input_tokens":600,"output_tokens":80,"cache_read_input_tokens":400,"cache_creation_input_tokens":0}}}"#,
            "\n"
        ),
    )
    .unwrap();

    // Qoder CN session in ~/.qoder-cn with scrubbed managed-model usage
    let cn_prompt = "x".repeat(400); // ~100 input tokens
    let cn_reply = "y".repeat(200); // ~50 output tokens
    std::fs::write(
        qoder_cn_proj.join("sess-cn-1.jsonl"),
        format!(
            concat!(
                r#"{{"type":"user","uuid":"u-cn-1","sessionId":"sess-cn-1","cwd":"/work/app-cn","timestamp":"2026-11-07T11:00:00.000Z","message":{{"role":"user","content":"{cn_prompt}"}}}}"#,
                "\n",
                r#"{{"type":"assistant","uuid":"a-cn-1","sessionId":"sess-cn-1","cwd":"/work/app-cn","timestamp":"2026-11-07T11:00:03.000Z","message":{{"id":"msg_cn_1","role":"assistant","model":"qwen3.8-max","stop_reason":"end_turn","content":[{{"type":"text","text":"{cn_reply}"}}],"usage":{{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"credits":0.5,"billable":true}}}}}}"#,
                "\n"
            ),
            cn_prompt = cn_prompt,
            cn_reply = cn_reply,
        ),
    )
    .unwrap();

    let scan = scan_all_clients_with_scanner_settings(
        home.to_str().unwrap(),
        &["qoder".to_string()],
        false,
        &ScannerSettings::default(),
    );
    assert_eq!(scan.get(ClientId::Qoder).len(), 3);

    let parsed = parse_local_clients(qoder_options(home)).unwrap();
    // 2 unique messages from ~/.qoder (msg_intl_1 deduplicated across fork + msg_intl_2)
    // + 1 message from ~/.qoder-cn (msg_cn_1) = 3 total messages
    assert_eq!(parsed.counts.get(ClientId::Qoder), 3);
    assert_eq!(parsed.messages.len(), 3);

    let uncached = parse_local_unified_messages_with_pricing_uncached(qoder_options(home), None)
        .await
        .unwrap();
    assert_eq!(uncached.len(), 3);
}
