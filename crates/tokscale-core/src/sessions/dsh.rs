//! DeepSeek Harness (DSH) session parser
//!
//! DSH persists one JSONL transcript per session under
//! `<DSH_HOME>/sessions/<encoded-cwd>/<session-id>/session.jsonl.zstd`
//! (`DSH_HOME` defaults to `~/.dsh`). The `.zstd` suffix marks the physical
//! encoding only: a backend configured with `compression: none` writes the
//! same rows to a plain `session.jsonl` in the same directory, so this parser
//! dispatches on the zstd frame magic rather than on the file name.
//!
//! The transcript is an append-only event stream; the rows Tokscale needs are:
//!
//! - `session`: session id, `createdAt` (ms), `cwd` (workspace root), and the
//!   `seedLength` fork boundary.
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
use std::io::Read;
use std::path::Path;

/// Zstandard frame magic number (RFC 8478 section 3.1.1).
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Decode buffer for the streaming zstd reader.
const ZSTD_CHUNK_BYTES: usize = 128 * 1024;

/// Read a DSH transcript, decoding zstd frames when the payload carries them.
///
/// A live DSH session appends one zstd frame per flush, so a scan racing a
/// writer routinely sees a torn trailing frame. DSH itself treats that as
/// expected and recovers the complete frames plus whatever prefix of the torn
/// one decodes (`session-persistence-jsonl/src/index.ts`, `readZstdPrefix`),
/// so decoding must be streaming: `decode_all` would surface one error and
/// throw the entire session away, reporting zero tokens for a session that is
/// merely being written to.
fn read_session_bytes(path: &Path) -> Vec<u8> {
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(_) => return Vec::new(),
    };
    if raw.len() < ZSTD_MAGIC.len() || raw[..ZSTD_MAGIC.len()] != ZSTD_MAGIC {
        // `compression: none` writes the same rows uncompressed.
        return raw;
    }

    let Ok(mut decoder) = zstd::stream::read::Decoder::new(raw.as_slice()) else {
        return Vec::new();
    };
    let mut decoded = Vec::new();
    let mut chunk = vec![0u8; ZSTD_CHUNK_BYTES];
    loop {
        match decoder.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => decoded.extend_from_slice(&chunk[..read]),
            // Torn trailing frame (or foreign payload): keep the prefix that
            // did decode. `lossy_lines` then drops the partial final record.
            Err(_) => break,
        }
    }
    decoded
}

/// Parse one DSH `session.jsonl.zstd` transcript into unified messages.
///
/// Each `assistant/message` event with a non-zero `data.usage` becomes one
/// [`UnifiedMessage`]. Messages without usable timestamps are skipped; usage
/// with a zero total is skipped so noise rows (e.g. echoed tool-call-only
/// messages) do not produce zero-token contributions.
pub fn parse_dsh_file(path: &Path) -> Vec<UnifiedMessage> {
    let decoded = read_session_bytes(path);
    if decoded.is_empty() {
        return Vec::new();
    }

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
    // Fork boundary: how many leading events this session inherited verbatim
    // from its parent. Zero for a session that was never forked.
    let mut seed_length: i64 = 0;
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
                seed_length = value
                    .get("seedLength")
                    .and_then(Value::as_i64)
                    .filter(|length| *length > 0)
                    .unwrap_or(0);
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
                // Fork/continuation ownership boundary. Forking copies the
                // parent's completed prefix into the child transcript verbatim
                // — same `seq`, `time`, `usage` and `message.id` — and records
                // how many events were inherited as the header's `seedLength`
                // (`core/session/src/index.ts`, `SessionStore::fork`). Only
                // events at or after that boundary are this session's own work;
                // counting the seed again bills the parent's calls twice.
                if seed_length > 0
                    && value
                        .get("seq")
                        .and_then(Value::as_i64)
                        .is_some_and(|seq| seq < seed_length)
                {
                    continue;
                }
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

                // `data.message.id` is a per-call `crypto.randomUUID()`
                // (`llm/llm/src/message.ts`) that a fork copies verbatim, so
                // scoping the key to it instead of the session id collapses a
                // seeded copy against the parent's original even when the two
                // live in different files under different session ids — the
                // seq boundary above only fires for headers that actually
                // carry `seedLength`. The rest of the identity stays in the
                // key: a sanitized or otherwise non-unique id then still
                // separates calls that differ in time, routing or usage
                // instead of silently folding them into one.
                let identity = value
                    .pointer("/data/message/id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map_or_else(|| format!("sid:{sid}"), |id| format!("msg:{id}"));
                let dedup_key = format!(
                    "dsh:{identity}:{timestamp}:{provider_id}:{model_id}:{}:{}:{}:{}:{}",
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

/// Split DSH's usage row into Tokscale's five additive buckets.
///
/// DSH documents `TokenUsage` as disjoint on the input side — `inputTokens` is
/// uncached input only, with cache hits reported separately, and the DeepSeek
/// adapter subtracts them out of `prompt_tokens` before persisting
/// (`llm/llm/src/types.ts`, `llm-deepseek/src/translate.ts`). `reasoningTokens`
/// is the exception: it is `completion_tokens_details.reasoning_tokens`, a
/// SUBSET of the `completion_tokens` that becomes `outputTokens`, which is why
/// DSH's own token meter sums input + cache + output and omits reasoning
/// entirely (`llm/token-meter/src/index.ts`, `usageTokens`).
///
/// [`TokenBreakdown`] buckets are additive and pricing bills `output` and
/// `reasoning` at the same output rate, so mapping both fields through would
/// bill every reasoning token twice. Subtract the overlap, as `senpi.rs`,
/// `grok.rs` and `zcode.rs` do for the same shape.
fn tokens_from_usage(usage: &Value) -> TokenBreakdown {
    let output = int_field(usage, "outputTokens").max(0);
    let reasoning = int_field(usage, "reasoningTokens").max(0);
    TokenBreakdown {
        input: int_field(usage, "inputTokens"),
        output: output.saturating_sub(reasoning),
        cache_read: int_field(usage, "cacheReadTokens"),
        cache_write: int_field(usage, "cacheWriteTokens"),
        reasoning,
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
        assert!(first
            .dedup_key
            .as_deref()
            .unwrap()
            .starts_with("dsh:sid:session-abc:"));

        // Same turn, later step: not a turn start.
        assert!(!messages[1].is_turn_start);
    }

    #[test]
    fn supports_cache_write_and_reasoning_buckets() {
        let file = write_zstd_session(&[
            r#"{"type":"session","id":"session-xyz","createdAt":1,"cwd":"/work"}"#,
            r#"{"type":"assistant/message","time":1786669454772,"data":{"turn":1,"message":{"source":{"provider":"deepseek","model":"deepseek-reasoner"}},"usage":{"inputTokens":10,"outputTokens":60,"cacheReadTokens":30,"cacheWriteTokens":40,"reasoningTokens":50}}}"#,
        ]);

        let messages = parse_dsh_file(file.path());
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.tokens.input, 10);
        // `reasoningTokens` is a subset of `outputTokens`, so the additive
        // output bucket keeps only the non-reasoning remainder.
        assert_eq!(msg.tokens.output, 10);
        assert_eq!(msg.tokens.cache_read, 30);
        assert_eq!(msg.tokens.cache_write, 40);
        assert_eq!(msg.tokens.reasoning, 50);
        assert_eq!(msg.model_id, "deepseek-reasoner");
        assert_eq!(msg.provider_id, "deepseek");
    }

    #[test]
    fn reasoning_tokens_do_not_inflate_the_additive_output_bucket() {
        // given: DSH's `outputTokens` is the provider's `completion_tokens`
        // and `reasoningTokens` is `completion_tokens_details.reasoning_tokens`
        // — a subset of it, which is why DSH's own token meter sums
        // input + cache + output and never adds reasoning. Tokscale's buckets
        // are additive and pricing bills output and reasoning at the same
        // output rate, so mapping both fields through bills reasoning twice.
        // Numbers taken from a committed DSH transcript
        // (`examples/acp-agent/tests/snapshots/subagent-fork-in-process`).
        let file = write_zstd_session(&[
            r#"{"type":"session","id":"session-reasoning","createdAt":1,"cwd":"/work"}"#,
            r#"{"type":"assistant/message","seq":39,"time":1785730448979,"data":{"turn":1,"message":{"id":"m-1","source":{"provider":"deepseek","model":"deepseek-reasoner"}},"usage":{"inputTokens":2885,"outputTokens":25,"cacheReadTokens":0,"reasoningTokens":23}}}"#,
        ]);

        let messages = parse_dsh_file(file.path());

        assert_eq!(messages.len(), 1);
        let tokens = &messages[0].tokens;
        assert_eq!(tokens.output, 2);
        assert_eq!(tokens.reasoning, 23);
        // Mirrors DSH's own `usageTokens`: input + cacheRead + cacheWrite +
        // output, with reasoning already inside output.
        assert_eq!(tokens.total(), 2885 + 25);
    }

    #[test]
    fn reasoning_equal_to_output_leaves_a_non_zero_message() {
        // A reasoning-only completion (all output tokens were reasoning)
        // must survive the zero-usage filter with an empty output bucket.
        let file = write_zstd_session(&[
            r#"{"type":"session","id":"session-all-reasoning","createdAt":1,"cwd":"/work"}"#,
            r#"{"type":"assistant/message","time":1786669454772,"data":{"turn":1,"message":{"source":{"provider":"p","model":"m"}},"usage":{"inputTokens":0,"outputTokens":31,"reasoningTokens":31}}}"#,
        ]);

        let messages = parse_dsh_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.output, 0);
        assert_eq!(messages[0].tokens.reasoning, 31);
        assert_eq!(messages[0].tokens.total(), 31);
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
    fn a_repeated_message_id_still_separates_calls_that_differ() {
        // The sanitized snapshots DSH commits redact `message.id` to a single
        // placeholder shared by every call in the file, so the id alone is not
        // a safe dedup key: the rest of the call identity has to stay in it or
        // distinct calls disappear.
        let file = write_zstd_session(&[
            r#"{"type":"session","id":"session-placeholder","createdAt":1,"cwd":"/work"}"#,
            r#"{"type":"assistant/message","time":1786669454772,"data":{"turn":1,"message":{"id":"{{sessionId}}","source":{"provider":"p","model":"m"}},"usage":{"inputTokens":20,"outputTokens":8}}}"#,
            r#"{"type":"assistant/message","time":1786669455000,"data":{"turn":1,"message":{"id":"{{sessionId}}","source":{"provider":"p","model":"m"}},"usage":{"inputTokens":28,"outputTokens":2}}}"#,
        ]);

        let messages = parse_dsh_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_ne!(messages[0].dedup_key, messages[1].dedup_key);
    }

    #[test]
    fn skips_the_seeded_prefix_a_fork_inherits_from_its_parent() {
        // given: DSH forks by copying the parent's completed prefix into the
        // child transcript verbatim and recording its length as `seedLength`.
        // Both rows below are the real duplicated pair from
        // `examples/acp-agent/tests/snapshots/subagent-fork-in-process`:
        // the parent's seq-39 message reappears in the child under a different
        // session id with the same time, usage and message id.
        let parent = write_zstd_session(&[
            r#"{"type":"session","version":0,"id":"96cf59c9-b347-48b9-b234-a5200913ad05","createdAt":1783352134832,"cwd":"/work","delegationDepth":0}"#,
            r#"{"type":"assistant/message","seq":39,"time":1785730448979,"data":{"turn":1,"message":{"id":"7ac2e3d7-d558-4b24-b71e-40fc2f42216d","source":{"provider":"deepseek","model":"deepseek-reasoner"}},"usage":{"inputTokens":2885,"outputTokens":25,"cacheReadTokens":0,"reasoningTokens":23}}}"#,
        ]);
        let child = write_zstd_session(&[
            r#"{"type":"session","version":0,"id":"ada8966c-9fa3-441b-8721-37ff1e795e6a","createdAt":1783352137161,"cwd":"/work","parentSession":"96cf59c9-b347-48b9-b234-a5200913ad05","seedLength":42,"origin":"subagent","delegationDepth":1}"#,
            r#"{"type":"assistant/message","seq":39,"time":1785730448979,"data":{"turn":1,"message":{"id":"7ac2e3d7-d558-4b24-b71e-40fc2f42216d","source":{"provider":"deepseek","model":"deepseek-reasoner"}},"usage":{"inputTokens":2885,"outputTokens":25,"cacheReadTokens":0,"reasoningTokens":23}}}"#,
            r#"{"type":"assistant/message","seq":96,"time":1786358035361,"data":{"turn":2,"message":{"id":"cdc56e00-c648-4669-92b2-7299e41cb743","source":{"provider":"deepseek","model":"deepseek-reasoner"}},"usage":{"inputTokens":97,"outputTokens":39,"cacheReadTokens":2816,"reasoningTokens":34}}}"#,
        ]);

        // when
        let parent_messages = parse_dsh_file(parent.path());
        let child_messages = parse_dsh_file(child.path());

        // then: the child contributes only its own post-seed work.
        assert_eq!(parent_messages.len(), 1);
        assert_eq!(child_messages.len(), 1);
        assert_eq!(child_messages[0].timestamp, 1786358035361);
        assert_eq!(child_messages[0].tokens.input, 97);
    }

    #[test]
    fn seeded_rows_share_the_parent_dedup_key_across_files() {
        // The seq boundary only fires when the header carries `seedLength`;
        // a resumed or re-exported transcript that lost it still repeats the
        // parent's per-call `message.id`, so the dedup key must be keyed on
        // that id rather than on the session id, and stay identical across the
        // two files for the cross-file pass in `lib.rs` to collapse them.
        let row = r#"{"type":"assistant/message","seq":39,"time":1785730448979,"data":{"turn":1,"message":{"id":"7ac2e3d7-d558-4b24-b71e-40fc2f42216d","source":{"provider":"deepseek","model":"deepseek-reasoner"}},"usage":{"inputTokens":2885,"outputTokens":25,"reasoningTokens":23}}}"#;
        let parent = write_zstd_session(&[
            r#"{"type":"session","id":"96cf59c9-b347-48b9-b234-a5200913ad05","createdAt":1,"cwd":"/work"}"#,
            row,
        ]);
        let child = write_zstd_session(&[
            r#"{"type":"session","id":"ada8966c-9fa3-441b-8721-37ff1e795e6a","createdAt":2,"cwd":"/work","parentSession":"96cf59c9-b347-48b9-b234-a5200913ad05"}"#,
            row,
        ]);

        let parent_messages = parse_dsh_file(parent.path());
        let child_messages = parse_dsh_file(child.path());

        assert_eq!(parent_messages.len(), 1);
        assert_eq!(child_messages.len(), 1);
        assert_ne!(parent_messages[0].session_id, child_messages[0].session_id);
        assert_eq!(
            parent_messages[0].dedup_key.as_deref(),
            Some(
                "dsh:msg:7ac2e3d7-d558-4b24-b71e-40fc2f42216d:1785730448979:deepseek:deepseek-reasoner:2885:2:0:0:23"
            )
        );
        assert_eq!(parent_messages[0].dedup_key, child_messages[0].dedup_key);
    }

    #[test]
    fn recovers_the_decodable_prefix_of_a_torn_trailing_frame() {
        // given: DSH appends one zstd frame per flush, so a scan racing a live
        // writer sees a complete prefix plus a truncated final frame. DSH's own
        // reader recovers the complete frames rather than refusing the log, and
        // `decode_all` would report zero tokens for the whole session.
        let header = zstd::encode_all(
            concat!(
                r#"{"type":"session","id":"session-torn","createdAt":1,"cwd":"/work"}"#,
                "
"
            )
            .as_bytes(),
            3,
        )
        .unwrap();
        let committed = zstd::encode_all(
            concat!(
                r#"{"type":"assistant/message","time":1786669454772,"data":{"turn":1,"message":{"id":"m-committed","source":{"provider":"p","model":"m"}},"usage":{"inputTokens":10,"outputTokens":20}}}"#,
                "
"
            )
            .as_bytes(),
            3,
        )
        .unwrap();
        let torn = zstd::encode_all(
            concat!(
                r#"{"type":"assistant/message","time":1786669455000,"data":{"turn":2,"message":{"id":"m-torn","source":{"provider":"p","model":"m"}},"usage":{"inputTokens":11,"outputTokens":21}}}"#,
                "
"
            )
            .as_bytes(),
            3,
        )
        .unwrap();

        let mut payload = header;
        payload.extend_from_slice(&committed);
        // Cut the final frame short, the way an interrupted append leaves it.
        payload.extend_from_slice(&torn[..torn.len() / 2]);

        // Non-vacuity: the one-shot decoder this parser used to call refuses
        // the whole payload, which is exactly the 0-token report being fixed.
        assert!(zstd::stream::decode_all(payload.as_slice()).is_err());

        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, &payload).unwrap();

        // when
        let messages = parse_dsh_file(file.path());

        // then: the committed frame still counts.
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "session-torn");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 20);
    }

    #[test]
    fn parses_the_uncompressed_session_jsonl_spelling() {
        // `compression: none` writes the same rows to a plain `session.jsonl`
        // in the same session directory, so dispatch on the frame magic rather
        // than the file name.
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("session-plain");
        std::fs::create_dir_all(&session_dir).unwrap();
        let path = session_dir.join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"session","id":"session-plain","createdAt":1,"cwd":"/work"}"#,
                "
",
                r#"{"type":"assistant/message","time":1786669454772,"data":{"turn":1,"message":{"id":"m-plain","source":{"provider":"p","model":"m"}},"usage":{"inputTokens":12,"outputTokens":34,"reasoningTokens":4}}}"#,
                "
"
            ),
        )
        .unwrap();

        let messages = parse_dsh_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "session-plain");
        assert_eq!(messages[0].tokens.input, 12);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.reasoning, 4);
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
