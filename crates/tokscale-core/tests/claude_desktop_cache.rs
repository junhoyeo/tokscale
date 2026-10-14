//! Synthetic Chromium fixture: no private conversation data.
use serde_json::{json, Value};
use std::path::Path;
use tokscale_core::claude_desktop;

#[test]
fn original_model_report_remains_source_and_serialization_compatible() {
    let report = tokscale_core::ModelReport {
        entries: vec![],
        total_input: 0,
        total_output: 0,
        total_cache_read: 0,
        total_cache_write: 0,
        total_messages: 0,
        total_cost: 0.0,
        processing_time_ms: 0,
    };
    let json = serde_json::to_value(report).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 8);
    assert!(json.get("undated_tokens").is_none());
}

fn varint(mut n: usize, out: &mut Vec<u8>) {
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn utf16(s: &str, out: &mut Vec<u8>) {
    let units: Vec<_> = s.encode_utf16().collect();
    varint(units.len(), out);
    for unit in units {
        out.extend(unit.to_be_bytes());
    }
}
fn v8(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.push(b'0'),
        Value::Bool(v) => out.push(if *v { b'T' } else { b'F' }),
        Value::Number(v) => {
            out.push(b'N');
            out.extend(v.as_f64().unwrap().to_le_bytes());
        }
        Value::String(v) => {
            out.push(b'S');
            varint(v.len(), out);
            out.extend(v.as_bytes());
        }
        Value::Array(v) => {
            out.push(b'A');
            varint(v.len(), out);
            for item in v {
                v8(item, out);
            }
            out.extend([b'$', 0]);
            varint(v.len(), out);
        }
        Value::Object(v) => {
            out.push(b'o');
            for (k, v) in v {
                v8(&json!(k), out);
                v8(v, out);
            }
            out.push(b'{');
            varint(v.len(), out);
        }
    }
}
fn log(payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() < 32761);
    let crc = crc32c::crc32c_append(crc32c::crc32c(&[1]), payload)
        .rotate_left(17)
        .wrapping_add(0xa282_ead8);
    let mut result = crc.to_le_bytes().to_vec();
    result.extend((payload.len() as u16).to_le_bytes());
    result.push(1);
    result.extend(payload);
    result
}
fn write_cache(home: &Path, tombstone: bool) {
    let dir = home
        .join("Library/Application Support/Claude/IndexedDB/https_claude.ai_0.indexeddb.leveldb");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("CURRENT"), "MANIFEST-000001\n").unwrap();
    std::fs::write(dir.join("MANIFEST-000001"), log(&[2, 2])).unwrap();
    let mut dbkey = vec![0, 0, 0, 0, 201];
    utf16("https_claude.ai", &mut dbkey);
    utf16("claude-conversation-store", &mut dbkey);
    let mut rows = vec![(dbkey, vec![1])];
    for (id, name) in [(1, "trees"), (2, "meta")] {
        rows.push((
            vec![0, 1, 0, 0, 50, id, 0],
            name.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        ));
    }
    let assistant = json!({"type":"assistant","session_id":"sdk","created_at":"2026-01-01T00:00:00Z","request_id":"r","message":{"id":"code-message","model":"claude-future-code-42","usage":{"input_tokens":3,"output_tokens":1}}});
    let result = json!({"type":"result","session_id":"sdk","created_at":"2026-01-01T00:00:01Z","modelUsage":{"claude-future-code-42[1m]":{"inputTokens":9,"outputTokens":6},"claude-future-code-42":{"inputTokens":3,"outputTokens":2}}});
    let code = json!({"accountUuid":"a","orgUuid":"o","conversationUuid":"code","product":"code","tree":{"messages":[assistant,result,result]}});
    let chat = json!({"accountUuid":"a","orgUuid":"o","conversationUuid":"chat","product":"chat","tree":{"chat_messages":[{"sender":"assistant","uuid":"chat-message","model":"claude-future-chat-43","created_at":"2026-01-02T00:00:00Z","usage":{"input_tokens":7,"output_tokens":3}}]}});
    let mut values = vec![(1, 1, code), (1, 2, chat)];
    if tombstone {
        values.push((
            2,
            3,
            json!({"accountUuid":"a","orgUuid":"o","conversationUuid":"chat","tombstone":true}),
        ));
    }
    for (store, key, value) in values {
        let mut encoded = vec![1, 0xff, 15, 0xff, 15];
        v8(&value, &mut encoded);
        rows.push((vec![0, 1, store, 1, 1, 1, 0, key], encoded));
    }
    let mut batch = 1u64.to_le_bytes().to_vec();
    batch.extend((rows.len() as u32).to_le_bytes());
    for (key, value) in rows {
        batch.push(1);
        varint(key.len(), &mut batch);
        batch.extend(key);
        varint(value.len(), &mut batch);
        batch.extend(value);
    }
    std::fs::write(dir.join("000002.log"), log(&batch)).unwrap();
}

#[test]
fn desktop_cache_reads_code_and_chat_usage_with_future_models() {
    let home = tempfile::tempdir().unwrap();
    write_cache(home.path(), false);
    let usage = claude_desktop::read(home.path());
    assert_eq!(usage.coverage.decode_errors, 0);
    assert_eq!(usage.coverage.products.get("code"), Some(&1));
    assert_eq!(usage.coverage.products.get("chat"), Some(&1));
    let mut messages = vec![];
    usage.merge_into(&mut messages);
    assert_eq!(messages.iter().map(|m| m.tokens.total()).sum::<i64>(), 30);
    assert_eq!(messages.iter().map(|m| m.message_count).sum::<i32>(), 2);
    assert_eq!(
        messages
            .iter()
            .filter(|m| claude_desktop::is_undated_balance(m))
            .map(|m| m.tokens.total())
            .sum::<i64>(),
        16
    );
    write_cache(home.path(), true);
    let usage = claude_desktop::read(home.path());
    assert_eq!(usage.coverage.conversations, 1);
    let mut messages = vec![];
    usage.merge_into(&mut messages);
    assert_eq!(messages.iter().map(|m| m.tokens.total()).sum::<i64>(), 20);
}

#[test]
fn local_claude_merge_keeps_desktop_usage_without_counting_advisors_as_replies() {
    let home = tempfile::tempdir().unwrap();
    write_cache(home.path(), false);
    let project = home.path().join(".claude/projects/repro");
    std::fs::create_dir_all(&project).unwrap();
    // This reply also exists in the Desktop cache. The transcript adds an
    // advisor call that the Desktop copy does not carry, so the combined
    // result must retain both sources without counting the reply twice.
    let transcript = json!({
        "type": "assistant",
        "timestamp": "2026-01-01T00:00:00Z",
        "requestId": "r",
        "message": {
            "id": "code-message",
            "model": "claude-future-code-42",
            "usage": {
                "input_tokens": 3,
                "output_tokens": 1,
                "iterations": [{
                    "type": "advisor_message",
                    "model": "claude-fable-5-1",
                    "input_tokens": 11,
                    "output_tokens": 2
                }]
            }
        }
    });
    std::fs::write(project.join("sdk.jsonl"), transcript.to_string()).unwrap();

    let parsed = tokscale_core::parse_local_clients(tokscale_core::LocalParseOptions {
        home_dir: Some(home.path().to_string_lossy().into_owned()),
        clients: Some(vec!["claude".to_string()]),
        use_env_roots: false,
        ..Default::default()
    })
    .unwrap();

    assert_eq!(
        parsed.messages.len(),
        4,
        "reply, advisor, chat and undated balance"
    );
    assert_eq!(parsed.counts.get(tokscale_core::ClientId::Claude), 2);
    assert_eq!(
        parsed.messages.iter().map(|m| m.message_count).sum::<i32>(),
        2
    );
    let tokens: i64 = parsed
        .messages
        .iter()
        .map(|m| m.input + m.output + m.cache_read + m.cache_write + m.reasoning)
        .sum();
    assert_eq!(
        tokens, 43,
        "30 Desktop tokens plus 13 advisor tokens, without replaying the shared reply"
    );
}
