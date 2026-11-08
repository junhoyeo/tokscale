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

#[tokio::test]
async fn test_qoder_gist_representative_schema_periods() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    let qoder_cn_proj = home.join(".qoder-cn/projects/sample-periods");
    std::fs::create_dir_all(&qoder_cn_proj).unwrap();

    // 1. Period 01 (v1.1.5): assistant message has NO usage field at all.
    // Also contains lines without `version` field (workspace-directories, runtime-config, ai-title).
    // Prompt = 400 'a's (~100 tokens), thinking = 200 'b's (~50 tokens), reply = 600 'c's (~150 tokens).
    let p1_prompt = "a".repeat(400);
    let p1_thinking = "b".repeat(200);
    let p1_reply = "c".repeat(600);
    std::fs::write(
        qoder_cn_proj.join("01_v115_no_usage.jsonl"),
        format!(
            concat!(
                r#"{{"type":"workspace-directories","sessionId":"sess-01","directories":["C:\\Users\\user\\work"]}}"#, "\n",
                r#"{{"type":"runtime-config","sessionId":"sess-01","model":"qmodel_preview","timestamp":1784988512000}}"#, "\n",
                r#"{{"type":"user","uuid":"u-01","sessionId":"sess-01","version":"1.1.5","timestamp":"2026-07-25T14:10:00.000Z","message":{{"role":"user","content":"{p1_prompt}"}}}}"#, "\n",
                r#"{{"type":"ai-title","sessionId":"sess-01","aiTitle":"Test 01"}}"#, "\n",
                r#"{{"type":"assistant","uuid":"a-01-0","sessionId":"sess-01","version":"1.1.5","timestamp":"2026-07-25T14:12:05.000Z","message":{{"id":"msg_01","type":"message","role":"assistant","model":"qmodel_preview","content":[{{"type":"thinking","thinking":"{p1_thinking}"}}]}}}}"#, "\n",
                r#"{{"type":"assistant","uuid":"a-01-1","sessionId":"sess-01","version":"1.1.5","timestamp":"2026-07-25T14:12:05.000Z","message":{{"id":"msg_01","type":"message","role":"assistant","model":"qmodel_preview","content":[{{"type":"text","text":"{p1_reply}"}}]}}}}"#, "\n"
            ),
            p1_prompt = p1_prompt,
            p1_thinking = p1_thinking,
            p1_reply = p1_reply,
        ),
    )
    .unwrap();

    // 2. Period 02 (v1.1.28): usage present but all token fields are zeroed (credits & billable present).
    let p2_prompt = "d".repeat(800); // ~200 tokens
    let p2_reply = "e".repeat(400); // ~100 tokens
    std::fs::write(
        qoder_cn_proj.join("02_v1128_zero_usage.jsonl"),
        format!(
            concat!(
                r#"{{"type":"workspace-directories","sessionId":"sess-02","directories":["C:\\Users\\user\\work"]}}"#, "\n",
                r#"{{"type":"runtime-config","sessionId":"sess-02","model":"qmodel_38max","timestamp":1787384131000}}"#, "\n",
                r#"{{"type":"user","uuid":"u-02","sessionId":"sess-02","version":"1.1.28","timestamp":"2026-08-22T06:35:31.000Z","message":{{"role":"user","content":"{p2_prompt}"}}}}"#, "\n",
                r#"{{"type":"assistant","uuid":"a-02","sessionId":"sess-02","version":"1.1.28","timestamp":"2026-08-22T06:35:40.000Z","message":{{"id":"msg_02","type":"message","role":"assistant","model":"qmodel_38max","content":[{{"type":"text","text":"{p2_reply}"}}],"usage":{{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"credits":8.229,"billable":true,"context_usage_ratio":0.135}}}}}}"#, "\n"
            ),
            p2_prompt = p2_prompt,
            p2_reply = p2_reply,
        ),
    )
    .unwrap();

    // 3. Period 05 (v1.1.76): authoritative non-zero usage with token exposure enabled.
    std::fs::write(
        qoder_cn_proj.join("05_v1176_nonzero_tokens.jsonl"),
        concat!(
            r#"{"type":"workspace-directories","sessionId":"sess-05","directories":["C:\\Users\\user\\work"]}"#, "\n",
            r#"{"type":"runtime-config","sessionId":"sess-05","model":"qfmodel","timestamp":1792658910000}"#, "\n",
            r#"{"type":"user","uuid":"u-05","sessionId":"sess-05","version":"1.1.76","businessModelKey":"qfmodel","timestamp":"2026-10-22T08:48:30.000Z","message":{"role":"user","content":"Optimize build"}}"#, "\n",
            r#"{"type":"assistant","uuid":"a-05","sessionId":"sess-05","version":"1.1.76","timestamp":"2026-10-22T08:48:35.000Z","message":{"id":"msg_05","type":"message","role":"assistant","model":"qfmodel","content":[{"type":"text","text":"Done"}],"usage":{"input_tokens":4304,"output_tokens":16,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"credits":0.1204,"billable":false}}}"#, "\n"
        ),
    )
    .unwrap();

    // 4. Period 07 (v1.1.84): model is generic "auto", resolved via user.businessModelKey and runtime-config.model.
    std::fs::write(
        qoder_cn_proj.join("07_v1184_model_auto.jsonl"),
        concat!(
            r#"{"type":"workspace-directories","sessionId":"sess-07","directories":["C:\\Users\\user\\work"]}"#, "\n",
            r#"{"type":"runtime-config","sessionId":"sess-07","model":"gfmodel","timestamp":1794041573000}"#, "\n",
            r#"{"type":"user","uuid":"u-07-1","sessionId":"sess-07","version":"1.1.84","businessModelKey":"gfmodel","timestamp":"2026-11-07T08:56:16.000Z","message":{"role":"user","content":"First turn"}}"#, "\n",
            r#"{"type":"assistant","uuid":"a-07-1","sessionId":"sess-07","version":"1.1.84","timestamp":"2026-11-07T08:56:28.000Z","message":{"id":"msg_07_1","type":"message","role":"assistant","model":"auto","content":[{"type":"text","text":"Turn 1 reply"}],"usage":{"input_tokens":28067,"output_tokens":272,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#, "\n",
            r#"{"type":"user","uuid":"u-07-2","sessionId":"sess-07","version":"1.1.84","timestamp":"2026-11-07T08:56:33.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":"tool output"}]}}"#, "\n",
            r#"{"type":"assistant","uuid":"a-07-2","sessionId":"sess-07","version":"1.1.84","timestamp":"2026-11-07T08:56:44.000Z","message":{"id":"msg_07_2","type":"message","role":"assistant","model":"auto","content":[{"type":"text","text":"Turn 2 reply"}],"usage":{"input_tokens":32057,"output_tokens":295,"cache_read_input_tokens":9856,"cache_creation_input_tokens":0}}}"#, "\n"
        ),
    )
    .unwrap();

    // 5. Fallback session: model: "auto", no user businessModelKey -> fallback to runtime-config.model = "dfmodel"
    std::fs::write(
        qoder_cn_proj.join("07_fallback_runtime_config.jsonl"),
        concat!(
            r#"{"type":"runtime-config","sessionId":"sess-df","model":"dfmodel","timestamp":1794050000000}"#, "\n",
            r#"{"type":"user","uuid":"u-df","sessionId":"sess-df","version":"1.1.84","timestamp":"2026-11-07T12:00:00.000Z","message":{"role":"user","content":"Prompt without key"}}"#, "\n",
            r#"{"type":"assistant","uuid":"a-df","sessionId":"sess-df","version":"1.1.84","timestamp":"2026-11-07T12:00:05.000Z","message":{"id":"msg_df","type":"message","role":"assistant","model":"auto","content":[{"type":"text","text":"DF reply"}],"usage":{"input_tokens":15000,"output_tokens":120,"cache_read_input_tokens":5000,"cache_creation_input_tokens":0}}}"#, "\n"
        ),
    )
    .unwrap();

    // 6. Dynamic runtime-config switch: session starts with gfmodel, runtime-config changes to dfmodel,
    // subsequent turn without businessModelKey resolves to dfmodel (deepseek-v3 / deepseek).
    std::fs::write(
        qoder_cn_proj.join("08_dynamic_runtime_config.jsonl"),
        concat!(
            r#"{"type":"runtime-config","sessionId":"sess-dyn","model":"gfmodel","timestamp":1794050000000}"#, "\n",
            r#"{"type":"user","uuid":"u-dyn-1","sessionId":"sess-dyn","version":"1.1.84","businessModelKey":"gfmodel","timestamp":"2026-11-07T13:00:00.000Z","message":{"role":"user","content":"Initial prompt"}}"#, "\n",
            r#"{"type":"assistant","uuid":"a-dyn-1","sessionId":"sess-dyn","version":"1.1.84","timestamp":"2026-11-07T13:00:05.000Z","message":{"id":"msg_dyn_1","type":"message","role":"assistant","model":"auto","content":[{"type":"text","text":"GF reply"}],"usage":{"input_tokens":10000,"output_tokens":80,"cache_read_input_tokens":2000,"cache_creation_input_tokens":0}}}"#, "\n",
            r#"{"type":"runtime-config","sessionId":"sess-dyn","model":"dfmodel","timestamp":1794050010000}"#, "\n",
            r#"{"type":"user","uuid":"u-dyn-2","sessionId":"sess-dyn","version":"1.1.84","timestamp":"2026-11-07T13:01:00.000Z","message":{"role":"user","content":"Followup without key"}}"#, "\n",
            r#"{"type":"assistant","uuid":"a-dyn-2","sessionId":"sess-dyn","version":"1.1.84","timestamp":"2026-11-07T13:01:05.000Z","message":{"id":"msg_dyn_2","type":"message","role":"assistant","model":"auto","content":[{"type":"text","text":"DF dynamic reply"}],"usage":{"input_tokens":12000,"output_tokens":90,"cache_read_input_tokens":3000,"cache_creation_input_tokens":0}}}"#, "\n"
        ),
    )
    .unwrap();

    let scan = scan_all_clients_with_scanner_settings(
        home.to_str().unwrap(),
        &["qoder".to_string()],
        false,
        &ScannerSettings::default(),
    );
    assert_eq!(scan.get(ClientId::Qoder).len(), 6);

    let parsed = parse_local_clients(qoder_options(home)).unwrap();
    // 1 (p1) + 1 (p2) + 1 (p5) + 2 (p7 turns 1 & 2) + 1 (fallback) + 2 (dynamic) = 8 messages
    assert_eq!(parsed.counts.get(ClientId::Qoder), 8);
    assert_eq!(parsed.messages.len(), 8);

    // Check Period 01: absent usage -> estimated tokens
    let m01 = parsed
        .messages
        .iter()
        .find(|m| m.session_id.as_ref() == "sess-01")
        .expect("sess-01");
    assert_eq!(m01.model_id.as_ref(), "qwen3-coder");
    assert_eq!(m01.provider_id.as_ref(), "qwen");
    assert_eq!(m01.pricing_model_id.as_deref(), Some("qmodel_preview"));
    assert_eq!(m01.input, 100);
    assert_eq!(m01.output, 150);
    assert_eq!(m01.reasoning, 50);

    // Check Period 02: zeroed usage -> estimated tokens
    let m02 = parsed
        .messages
        .iter()
        .find(|m| m.session_id.as_ref() == "sess-02")
        .expect("sess-02");
    assert_eq!(m02.model_id.as_ref(), "qwen3.8-max");
    assert_eq!(m02.provider_id.as_ref(), "qwen");
    assert_eq!(m02.pricing_model_id.as_deref(), Some("qmodel_38max"));
    assert_eq!(m02.input, 200);
    assert_eq!(m02.output, 100);

    // Check Period 05: authoritative non-zero usage
    let m05 = parsed
        .messages
        .iter()
        .find(|m| m.session_id.as_ref() == "sess-05")
        .expect("sess-05");
    assert_eq!(m05.model_id.as_ref(), "qwen-flash");
    assert_eq!(m05.provider_id.as_ref(), "qwen");
    assert_eq!(m05.pricing_model_id.as_deref(), Some("qfmodel"));
    assert_eq!(m05.input, 4304);
    assert_eq!(m05.output, 16);

    // Check Period 07: model "auto" resolved via businessModelKey gfmodel -> glm-4-flash / zai
    let mut sess_07_msgs: Vec<_> = parsed
        .messages
        .iter()
        .filter(|m| m.session_id.as_ref() == "sess-07")
        .collect();
    sess_07_msgs.sort_by_key(|m| m.timestamp);
    assert_eq!(sess_07_msgs.len(), 2);

    let m07_1 = sess_07_msgs[0];
    assert_eq!(m07_1.model_id.as_ref(), "glm-4-flash");
    assert_eq!(m07_1.provider_id.as_ref(), "zai");
    assert_eq!(m07_1.pricing_model_id.as_deref(), Some("gfmodel"));
    assert_eq!(m07_1.input, 28067);
    assert_eq!(m07_1.output, 272);

    let m07_2 = sess_07_msgs[1];
    assert_eq!(m07_2.model_id.as_ref(), "glm-4-flash");
    assert_eq!(m07_2.provider_id.as_ref(), "zai");
    assert_eq!(m07_2.pricing_model_id.as_deref(), Some("gfmodel"));
    assert_eq!(m07_2.input, 32057);
    assert_eq!(m07_2.output, 295);
    assert_eq!(m07_2.cache_read, 9856);

    // Check Fallback: model "auto" resolved via runtime-config dfmodel -> deepseek-v3 / deepseek
    let m_df = parsed
        .messages
        .iter()
        .find(|m| m.session_id.as_ref() == "sess-df")
        .expect("sess-df");
    assert_eq!(m_df.model_id.as_ref(), "deepseek-v3");
    assert_eq!(m_df.provider_id.as_ref(), "deepseek");
    assert_eq!(m_df.pricing_model_id.as_deref(), Some("dfmodel"));
    assert_eq!(m_df.input, 15000);
    assert_eq!(m_df.output, 120);
    assert_eq!(m_df.cache_read, 5000);

    // Check Dynamic runtime-config change: Turn 1 gfmodel, Turn 2 dfmodel
    let mut sess_dyn_msgs: Vec<_> = parsed
        .messages
        .iter()
        .filter(|m| m.session_id.as_ref() == "sess-dyn")
        .collect();
    sess_dyn_msgs.sort_by_key(|m| m.timestamp);
    assert_eq!(sess_dyn_msgs.len(), 2);

    let mdyn_1 = sess_dyn_msgs[0];
    assert_eq!(mdyn_1.model_id.as_ref(), "glm-4-flash");
    assert_eq!(mdyn_1.provider_id.as_ref(), "zai");
    assert_eq!(mdyn_1.pricing_model_id.as_deref(), Some("gfmodel"));
    assert_eq!(mdyn_1.input, 10000);
    assert_eq!(mdyn_1.output, 80);
    assert_eq!(mdyn_1.cache_read, 2000);

    let mdyn_2 = sess_dyn_msgs[1];
    assert_eq!(mdyn_2.model_id.as_ref(), "deepseek-v3");
    assert_eq!(mdyn_2.provider_id.as_ref(), "deepseek");
    assert_eq!(mdyn_2.pricing_model_id.as_deref(), Some("dfmodel"));
    assert_eq!(mdyn_2.input, 12000);
    assert_eq!(mdyn_2.output, 90);
    assert_eq!(mdyn_2.cache_read, 3000);

    let uncached = parse_local_unified_messages_with_pricing_uncached(qoder_options(home), None)
        .await
        .unwrap();
    assert_eq!(uncached.len(), 8);
}
