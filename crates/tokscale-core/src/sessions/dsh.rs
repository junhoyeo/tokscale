//! DeepSeek Harness (DSH) session parser
//!
//! DSH persists one zstd-compressed JSONL transcript per session under
//! `<DSH_HOME>/sessions/<encoded-cwd>/<session-id>/session.jsonl.zstd`
//! (`DSH_HOME` defaults to `~/.dsh`). The transcript is an append-only event
//! stream; the rows Tokscale needs are:
//!
//! - `session`: session id, `createdAt` (ms), `cwd` (workspace root).
//! - `request/header`: the provider/model the request was routed to (fallback
//!   for messages whose `source` is absent).
//! - `assistant/message`: authoritative per-call usage on `data.usage`
//!   (`inputTokens`, `outputTokens`, `cacheReadTokens`, ...) plus the serving
//!   provider/model on `data.message.source`.
//!
//! DSH never embeds a cost, so every message leaves the parser at `0.0` and
//! pricing is its only cost source — the generic source cache is safe here.

use super::utils::lossy_lines;
use super::{workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// Parse one DSH `session.jsonl.zstd` transcript into unified messages.
///
/// Each `assistant/message` event with a non-zero `data.usage` becomes one
/// [`UnifiedMessage`]. Messages without usable timestamps are skipped; usage
/// with a zero total is skipped so noise rows (e.g. echoed tool-call-only
/// messages) do not produce zero-token contributions.
pub fn parse_dsh_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };
    let decoded = match zstd::stream::decode_all(file) {
        Ok(decoded) => decoded,
        // A truncated/foreign `.zstd` payload must not abort the whole scan.
        Err(_) => return Vec::new(),
    };

    // The transcript directory is named after the session id; it is the
    // fallback when the leading `session` event is missing.
    let session_id_from_path = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("unknown")
        .to_string();

    let mut session_id: Option<String> = None;
    let mut workspace_key: Option<String> = None;
    // Most recent request routing, used when a message lacks its own `source`.
    let mut fallback_provider: Option<String> = None;
    let mut fallback_model: Option<String> = None;

    let mut messages = Vec::new();
    let mut seen = HashSet::new();
    // Turn numbers that already emitted a turn-start message.
    let mut turn_started: HashSet<i64> = HashSet::new();
    // Fallback turn-start marker for transcripts without turn numbers: a
    // `user/message` arms the next assistant message as a turn start.
    let mut pending_user_turn = false;

    for line in lossy_lines(decoded.as_slice()) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(event_type) = value.get("type").and_then(Value::as_str) else {
            continue;
        };
        match event_type {
            "session" => {
                session_id = value.get("id").and_then(Value::as_str).map(str::to_string);
                workspace_key = value.get("cwd").and_then(Value::as_str).map(str::to_string);
            }
            "request/header" => {
                let config = value.pointer("/data/header/config");
                fallback_provider = config
                    .and_then(|c| c.get("provider"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                fallback_model = config
                    .and_then(|c| c.get("model"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            "user/message" => {
                pending_user_turn = true;
            }
            "assistant/message" => {
                let Some(usage) = value.pointer("/data/usage") else {
                    continue;
                };
                let tokens = tokens_from_usage(usage);
                if tokens.total() == 0 {
                    continue;
                }
                let Some(timestamp) = value.get("time").and_then(Value::as_i64) else {
                    continue;
                };
                if timestamp <= 0 {
                    continue;
                }

                let source = value.pointer("/data/message/source");
                let model_id = source
                    .and_then(|s| s.get("model"))
                    .and_then(Value::as_str)
                    .or(fallback_model.as_deref())
                    .unwrap_or("unknown")
                    .to_string();
                let provider_id = source
                    .and_then(|s| s.get("provider"))
                    .and_then(Value::as_str)
                    .or(fallback_provider.as_deref())
                    .unwrap_or("unknown")
                    .to_string();

                let sid = session_id
                    .clone()
                    .unwrap_or_else(|| session_id_from_path.clone());

                let turn = value.pointer("/data/turn").and_then(Value::as_i64);
                let is_turn_start = match turn {
                    Some(turn) => turn_started.insert(turn),
                    None => std::mem::take(&mut pending_user_turn),
                };

                let dedup_key = format!(
                    "dsh:{sid}:{timestamp}:{provider_id}:{model_id}:{}:{}:{}:{}:{}",
                    tokens.input,
                    tokens.output,
                    tokens.cache_read,
                    tokens.cache_write,
                    tokens.reasoning
                );
                if !seen.insert(dedup_key.clone()) {
                    continue;
                }

                let mut message = UnifiedMessage::new_with_dedup(
                    "dsh",
                    model_id,
                    provider_id,
                    &sid,
                    timestamp,
                    tokens,
                    0.0,
                    Some(dedup_key),
                );
                message.is_turn_start = is_turn_start;
                if let Some(cwd) = &workspace_key {
                    if let Some(key) = super::normalize_workspace_key(cwd) {
                        let label = workspace_label_from_key(&key);
                        message.set_workspace(Some(key), label);
                    }
                }
                messages.push(message);
            }
            _ => {}
        }
    }

    messages
}

fn tokens_from_usage(usage: &Value) -> TokenBreakdown {
    TokenBreakdown {
        input: int_field(usage, "inputTokens"),
        output: int_field(usage, "outputTokens"),
        cache_read: int_field(usage, "cacheReadTokens"),
        cache_write: int_field(usage, "cacheWriteTokens"),
        reasoning: int_field(usage, "reasoningTokens"),
    }
}

fn int_field(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_zstd_session(lines: &[&str]) -> tempfile::NamedTempFile {
        let payload = lines.join("\n");
        let compressed = zstd::encode_all(payload.as_bytes(), 3).unwrap();
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, &compressed).unwrap();
        file
    }

    #[test]
    fn parses_assistant_messages_with_usage() {
        let file = write_zstd_session(&[
            r#"{"type":"session","version":0,"id":"session-abc","createdAt":1786669406484,"cwd":"E:\\repo\\proj","delegationDepth":0,"agentPreset":"cordis"}"#,
            r#"{"type":"turn/start","seq":4,"time":1786669450000,"data":{"turn":1}}"#,
            r#"{"type":"user/message","seq":7,"time":1786669450001,"data":{"turn":1}}"#,
            r#"{"type":"assistant/message","seq":301,"time":1786669454772,"data":{"turn":1,"step":1,"message":{"role":"assistant","content":[],"source":{"kind":"model","provider":"irix","model":"deepseek-v4-flash"}},"usage":{"inputTokens":130,"outputTokens":159,"cacheReadTokens":13824}}}"#,
            r#"{"type":"assistant/message","seq":414,"time":1786669459063,"data":{"turn":1,"step":2,"message":{"role":"assistant","content":[],"source":{"kind":"model","provider":"irix","model":"deepseek-v4-flash"}},"usage":{"inputTokens":130,"outputTokens":159,"cacheReadTokens":13824}}}"#,
        ]);

        let messages = parse_dsh_file(file.path());
        assert_eq!(messages.len(), 2);

        let first = &messages[0];
        assert_eq!(first.client, "dsh");
        assert_eq!(first.model_id, "deepseek-v4-flash");
        assert_eq!(first.provider_id, "irix");
        assert_eq!(first.session_id, "session-abc");
        assert_eq!(first.timestamp, 1786669454772);
        assert_eq!(first.tokens.input, 130);
        assert_eq!(first.tokens.output, 159);
        assert_eq!(first.tokens.cache_read, 13824);
        assert_eq!(first.tokens.cache_write, 0);
        assert_eq!(first.tokens.reasoning, 0);
        assert_eq!(first.cost, 0.0);
        assert!(first.is_turn_start);
        assert_eq!(first.workspace_key.as_deref(), Some("E:/repo/proj"));
        assert_eq!(first.workspace_label.as_deref(), Some("proj"));
        assert!(first.dedup_key.as_deref().unwrap().starts_with("dsh:session-abc:"));

        // Same turn, later step: not a turn start.
        assert!(!messages[1].is_turn_start);
    }

    #[test]
    fn supports_cache_write_and_reasoning_buckets() {
        let file = write_zstd_session(&[
            r#"{"type":"session","id":"session-xyz","createdAt":1,"cwd":"/work"}"#,
            r#"{"type":"assistant/message","time":1786669454772,"data":{"turn":1,"message":{"source":{"provider":"deepseek","model":"deepseek-reasoner"}},"usage":{"inputTokens":10,"outputTokens":20,"cacheReadTokens":30,"cacheWriteTokens":40,"reasoningTokens":50}}}"#,
        ]);

        let messages = parse_dsh_file(file.path());
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.tokens.input, 10);
        assert_eq!(msg.tokens.output, 20);
        assert_eq!(msg.tokens.cache_read, 30);
        assert_eq!(msg.tokens.cache_write, 40);
        assert_eq!(msg.tokens.reasoning, 50);
        assert_eq!(msg.model_id, "deepseek-reasoner");
        assert_eq!(msg.provider_id, "deepseek");
    }

    #[test]
    fn falls_back_to_request_header_routing_and_folder_session_id() {
        let file = write_zstd_session(&[
            r#"{"type":"request/header","seq":11,"time":1786669450062,"data":{"header":{"config":{"provider":"irix","model":"deepseek-v4-flash"}}}}"#,
            // No `session` event and no `source` on the message: session id
            // comes from the folder, model/provider from the header.
            r#"{"type":"assistant/message","time":1786669454772,"data":{"turn":1,"message":{"role":"assistant","content":[]},"usage":{"inputTokens":5,"outputTokens":7}}}"#,
        ]);

        let messages = parse_dsh_file(file.path());
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        let folder = file
            .path()
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .unwrap();
        assert_eq!(msg.session_id, folder);
        assert_eq!(msg.model_id, "deepseek-v4-flash");
        assert_eq!(msg.provider_id, "irix");
    }

    #[test]
    fn skips_zero_usage_and_missing_timestamp() {
        let file = write_zstd_session(&[
            r#"{"type":"session","id":"session-zero","createdAt":1,"cwd":"/work"}"#,
            r#"{"type":"assistant/message","time":1786669454772,"data":{"message":{"source":{"provider":"p","model":"m"}},"usage":{"inputTokens":0,"outputTokens":0}}}"#,
            r#"{"type":"assistant/message","data":{"message":{"source":{"provider":"p","model":"m"}},"usage":{"inputTokens":1,"outputTokens":1}}}"#,
        ]);

        let messages = parse_dsh_file(file.path());
        assert!(messages.is_empty());
    }

    #[test]
    fn dedups_identical_replayed_rows_within_a_file() {
        let line = r#"{"type":"assistant/message","time":1786669454772,"data":{"turn":1,"message":{"source":{"provider":"p","model":"m"}},"usage":{"inputTokens":10,"outputTokens":20}}}"#;
        let file = write_zstd_session(&[
            r#"{"type":"session","id":"session-dedup","createdAt":1,"cwd":"/work"}"#,
            line,
            line,
        ]);

        let messages = parse_dsh_file(file.path());
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn missing_or_corrupt_files_yield_no_messages() {
        assert!(parse_dsh_file(Path::new("/nonexistent/dsh/session.jsonl.zstd")).is_empty());
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"this is not zstd").unwrap();
        assert!(parse_dsh_file(file.path()).is_empty());
    }

    #[test]
    fn marks_turn_start_when_no_turn_numbers_are_present() {
        let file = write_zstd_session(&[
            r#"{"type":"session","id":"session-noturn","createdAt":1,"cwd":"/work"}"#,
            r#"{"type":"user/message","time":1,"data":{}}"#,
            r#"{"type":"assistant/message","time":1786669454772,"data":{"message":{"source":{"provider":"p","model":"m"}},"usage":{"inputTokens":10,"outputTokens":20}}}"#,
            r#"{"type":"assistant/message","time":1786669455000,"data":{"message":{"source":{"provider":"p","model":"m"}},"usage":{"inputTokens":11,"outputTokens":21}}}"#,
        ]);

        let messages = parse_dsh_file(file.path());
        assert_eq!(messages.len(), 2);
        assert!(messages[0].is_turn_start);
        assert!(!messages[1].is_turn_start);
    }
}
