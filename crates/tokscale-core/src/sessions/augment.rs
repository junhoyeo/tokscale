//! Augment Code (Auggie CLI) session parser
//!
//! Parses local session snapshots from `~/.augment/sessions/<sessionId>.json`.
//!
//! ## Token accounting
//!
//! Each completed turn stores one authoritative `token_usage` observation on a
//! `response_nodes[]` entry. Verified against real sessions: turns almost never
//! carry multiple usage nodes. If they do, we take the **last** non-empty usage
//! (final streamed totals) rather than summing — summing would double-count if
//! the format ever repeated cumulative values.
//!
//! Input and cache buckets are reported independently (Anthropic-style split
//! accounting). Do not subtract `cache_read` from `input`.
//!
//! ## Completeness gate
//!
//! Only turns with `completed: true` are counted. Snapshots may retain
//! in-progress or aborted turns that already carry a partial `token_usage`.
//!
//! ## Timestamps
//!
//! Auggie records `finishedAt` only — there is no per-turn start time or
//! duration in the on-disk schema, so messages are **end-anchored**. Cost and
//! token totals are unaffected; duration-based metrics stay empty.
//!
//! ## Cost
//!
//! This parser always emits `cost = 0`. Downstream pricing estimates public
//! model API list prices from the model id. Augment credits /
//! `subAgentCostUsd` are intentionally ignored.
//!
//! ## Turn shape
//!
//! One unified message is emitted per completed turn, so `is_turn_start` is
//! always set. If a future format needs multiple messages per turn, revisit
//! that flag before counting turns.

use super::utils::{
    file_modified_timestamp_ms, parse_timestamp_str, read_file_or_none, AnthropicUsage,
};
use super::UnifiedMessage;
use crate::provider_identity;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct AugmentSession {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "agentState")]
    agent_state: Option<AugmentAgentState>,
    #[serde(default, rename = "chatHistory")]
    chat_history: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct AugmentAgentState {
    #[serde(rename = "modelId")]
    model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AugmentTurn {
    #[serde(rename = "finishedAt")]
    finished_at: Option<String>,
    /// Only completed turns are counted. Snapshots may retain in-progress or
    /// aborted turns that already carry a partial `token_usage` observation.
    #[serde(default)]
    completed: Option<bool>,
    exchange: Option<AugmentExchange>,
    #[serde(rename = "sequenceId")]
    sequence_id: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct AugmentExchange {
    model_id: Option<String>,
    request_id: Option<String>,
    #[serde(default)]
    response_nodes: Vec<AugmentResponseNode>,
}

/// Response node subset. Extra wire fields are ignored so unknown node shapes
/// do not fail the turn.
#[derive(Debug, Deserialize)]
struct AugmentResponseNode {
    token_usage: Option<AnthropicUsage>,
}

fn model_id(model: Option<&str>) -> String {
    let model = model.unwrap_or("unknown").trim();
    if model.is_empty() {
        "unknown".to_string()
    } else {
        model.to_string()
    }
}

fn provider_for_model(model: &str) -> String {
    // Unrecognized ids fall back to "augment". Pricing tables will not match
    // that provider, so estimated cost stays $0 while tokens still count.
    provider_identity::inferred_provider_from_model(model)
        .unwrap_or("augment")
        .to_string()
}

fn usage_is_nonzero(usage: &AnthropicUsage) -> bool {
    usage.to_breakdown().total() > 0
}

/// Prefer the last non-empty observation so a later full total wins over an
/// earlier partial if the format ever streams multiple usage nodes.
fn last_token_usage(nodes: &[AugmentResponseNode]) -> Option<&AnthropicUsage> {
    nodes
        .iter()
        .rev()
        .find_map(|node| node.token_usage.as_ref().filter(|u| usage_is_nonzero(u)))
}

fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn turn_dedup_key(session_id: &str, turn: &AugmentTurn, index: usize) -> String {
    if let Some(request_id) = turn
        .exchange
        .as_ref()
        .and_then(|e| e.request_id.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return format!("augment:{session_id}:req:{request_id}");
    }
    if let Some(seq) = turn.sequence_id.as_ref() {
        let seq_str = match seq {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if !seq_str.is_empty() && seq_str != "null" {
            return format!("augment:{session_id}:seq:{seq_str}");
        }
    }
    format!("augment:{session_id}:turn:{index}")
}

/// Parse an Augment/Auggie session JSON file into unified messages (one per turn).
///
/// Best-effort: unreadable files, invalid JSON, and malformed turns yield no
/// messages for that input rather than hard errors (same contract as peer
/// local session parsers).
pub fn parse_augment_file(path: &Path) -> Vec<UnifiedMessage> {
    let Some(data) = read_file_or_none(path) else {
        return vec![];
    };

    let session: AugmentSession = match serde_json::from_slice(&data) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let session_id = session
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| session_id_from_path(path));

    if session_id.is_empty() {
        return vec![];
    }

    let default_model = model_id(
        session
            .agent_state
            .as_ref()
            .and_then(|s| s.model_id.as_deref()),
    );
    let fallback_ts = file_modified_timestamp_ms(path);

    let mut messages = Vec::with_capacity(session.chat_history.len());
    for (index, raw_turn) in session.chat_history.into_iter().enumerate() {
        let turn: AugmentTurn = match serde_json::from_value(raw_turn) {
            Ok(t) => t,
            Err(_) => continue,
        };

        // Skip incomplete/aborted turns even if a partial token_usage was streamed.
        if turn.completed != Some(true) {
            continue;
        }

        let Some(exchange) = turn.exchange.as_ref() else {
            continue;
        };
        let Some(usage) = last_token_usage(&exchange.response_nodes) else {
            continue;
        };

        let tokens = usage.to_breakdown();

        let model = model_id(
            exchange
                .model_id
                .as_deref()
                .filter(|m| !m.trim().is_empty())
                .or(Some(default_model.as_str())),
        );
        let provider = provider_for_model(&model);
        let timestamp = turn
            .finished_at
            .as_deref()
            .and_then(parse_timestamp_str)
            .unwrap_or(fallback_ts);

        let mut msg = UnifiedMessage::new_with_dedup(
            "augment",
            model,
            provider,
            session_id.clone(),
            timestamp,
            tokens,
            0.0,
            Some(turn_dedup_key(&session_id, &turn, index)),
        );
        // One message per completed turn (see module docs).
        msg.is_turn_start = true;
        messages.push(msg);
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::NamedTempFile;

    fn write_temp_json(json: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(json.as_bytes()).unwrap();
        f
    }

    #[test]
    fn test_parse_valid_session_one_message_per_turn() {
        let json = r#"{
            "sessionId": "11111111-2222-3333-4444-555555555555",
            "created": "2026-01-15T12:00:00.000Z",
            "modified": "2026-01-15T12:10:00.000Z",
            "agentState": { "modelId": "grok-4-5" },
            "chatHistory": [
                {
                    "completed": true,
                    "finishedAt": "2026-01-15T12:01:00.000Z",
                    "sequenceId": 1,
                    "exchange": {
                        "model_id": "grok-4-5",
                        "request_id": "req-1",
                        "response_nodes": [
                            { "type": 1 },
                            {
                                "type": 10,
                                "token_usage": {
                                    "input_tokens": 1000,
                                    "output_tokens": 50,
                                    "cache_read_input_tokens": 200,
                                    "cache_creation_input_tokens": 0
                                }
                            }
                        ]
                    }
                },
                {
                    "completed": true,
                    "finishedAt": "2026-01-15T12:05:00.000Z",
                    "sequenceId": 2,
                    "exchange": {
                        "model_id": "claude-opus-4-8",
                        "request_id": "req-2",
                        "response_nodes": [
                            {
                                "token_usage": {
                                    "input_tokens": 400,
                                    "output_tokens": 100,
                                    "cache_read_input_tokens": 800,
                                    "cache_creation_input_tokens": 25
                                }
                            }
                        ]
                    }
                },
                {
                    "completed": false,
                    "finishedAt": "2026-01-15T12:09:00.000Z",
                    "exchange": {
                        "model_id": "grok-4-5",
                        "response_nodes": []
                    }
                }
            ]
        }"#;
        let f = write_temp_json(json);
        let messages = parse_augment_file(f.path());
        assert_eq!(messages.len(), 2);

        let first = &messages[0];
        assert_eq!(first.client, "augment");
        assert_eq!(first.session_id, "11111111-2222-3333-4444-555555555555");
        assert_eq!(first.model_id, "grok-4-5");
        assert_eq!(first.provider_id, "xai");
        assert_eq!(first.tokens.input, 1000);
        assert_eq!(first.tokens.output, 50);
        assert_eq!(first.tokens.cache_read, 200);
        assert_eq!(first.tokens.cache_write, 0);
        assert!(first.is_turn_start);
        assert_eq!(
            first.dedup_key.as_deref(),
            Some("augment:11111111-2222-3333-4444-555555555555:req:req-1")
        );
        assert_eq!(
            first.timestamp,
            parse_timestamp_str("2026-01-15T12:01:00.000Z").unwrap()
        );

        let second = &messages[1];
        assert_eq!(second.model_id, "claude-opus-4-8");
        assert_eq!(second.provider_id, "anthropic");
        assert_eq!(second.tokens.input, 400);
        assert_eq!(second.tokens.output, 100);
        assert_eq!(second.tokens.cache_read, 800);
        assert_eq!(second.tokens.cache_write, 25);
    }

    #[test]
    fn test_falls_back_to_session_model_and_filename_session_id() {
        let json = r#"{
            "agentState": { "modelId": "gpt-5-4" },
            "chatHistory": [
                {
                    "completed": true,
                    "finishedAt": "2026-07-20T13:33:00.000Z",
                    "sequenceId": "seq-a",
                    "exchange": {
                        "response_nodes": [
                            {
                                "token_usage": {
                                    "input_tokens": 10,
                                    "output_tokens": 5,
                                    "cache_read_input_tokens": 0,
                                    "cache_creation_input_tokens": 0
                                }
                            }
                        ]
                    }
                }
            ]
        }"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-from-name.json");
        std::fs::write(&path, json).unwrap();
        let messages = parse_augment_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "session-from-name");
        assert_eq!(messages[0].model_id, "gpt-5-4");
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("augment:session-from-name:seq:seq-a")
        );
    }

    #[test]
    fn test_prefers_last_nonempty_usage_node_when_totals_diverge() {
        // If a future format streams a partial usage then a fuller final one,
        // take the last non-empty observation (not the first, not the sum).
        let json = r#"{
            "sessionId": "s1",
            "agentState": { "modelId": "grok-4-5" },
            "chatHistory": [
                {
                    "completed": true,
                    "finishedAt": "2026-07-20T13:33:00.000Z",
                    "exchange": {
                        "model_id": "grok-4-5",
                        "request_id": "r1",
                        "response_nodes": [
                            {
                                "token_usage": {
                                    "input_tokens": 100,
                                    "output_tokens": 10,
                                    "cache_read_input_tokens": 50,
                                    "cache_creation_input_tokens": 0
                                }
                            },
                            {
                                "token_usage": {
                                    "input_tokens": 250,
                                    "output_tokens": 40,
                                    "cache_read_input_tokens": 75,
                                    "cache_creation_input_tokens": 5
                                }
                            }
                        ]
                    }
                }
            ]
        }"#;
        let f = write_temp_json(json);
        let messages = parse_augment_file(f.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 250);
        assert_eq!(messages[0].tokens.output, 40);
        assert_eq!(messages[0].tokens.cache_read, 75);
        assert_eq!(messages[0].tokens.cache_write, 5);
    }

    #[test]
    fn test_identical_multi_usage_nodes_still_count_once() {
        let json = r#"{
            "sessionId": "s1b",
            "agentState": { "modelId": "grok-4-5" },
            "chatHistory": [
                {
                    "completed": true,
                    "finishedAt": "2026-07-20T13:33:00.000Z",
                    "exchange": {
                        "model_id": "grok-4-5",
                        "request_id": "r1",
                        "response_nodes": [
                            {
                                "token_usage": {
                                    "input_tokens": 100,
                                    "output_tokens": 10,
                                    "cache_read_input_tokens": 50,
                                    "cache_creation_input_tokens": 0
                                }
                            },
                            {
                                "token_usage": {
                                    "input_tokens": 100,
                                    "output_tokens": 10,
                                    "cache_read_input_tokens": 50,
                                    "cache_creation_input_tokens": 0
                                }
                            }
                        ]
                    }
                }
            ]
        }"#;
        let f = write_temp_json(json);
        let messages = parse_augment_file(f.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 10);
        assert_eq!(messages[0].tokens.cache_read, 50);
    }

    #[test]
    fn test_invalid_json_and_missing_file() {
        let f = write_temp_json("not json");
        assert!(parse_augment_file(f.path()).is_empty());
        assert!(parse_augment_file(Path::new("/nonexistent/augment.json")).is_empty());
    }

    #[test]
    fn test_skips_malformed_turns() {
        let json = r#"{
            "sessionId": "s2",
            "agentState": { "modelId": "grok-4-5" },
            "chatHistory": [
                "bad-turn",
                {
                    "completed": true,
                    "finishedAt": "2026-07-20T13:33:00.000Z",
                    "exchange": {
                        "model_id": "grok-4-5",
                        "request_id": "ok",
                        "response_nodes": [
                            {
                                "token_usage": {
                                    "input_tokens": 1,
                                    "output_tokens": 2,
                                    "cache_read_input_tokens": 0,
                                    "cache_creation_input_tokens": 0
                                }
                            }
                        ]
                    }
                }
            ]
        }"#;
        let f = write_temp_json(json);
        let messages = parse_augment_file(f.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 1);
        assert_eq!(messages[0].tokens.output, 2);
    }

    #[test]
    fn test_skips_incomplete_turns_even_with_token_usage() {
        let json = r#"{
            "sessionId": "s3",
            "agentState": { "modelId": "grok-4-5" },
            "chatHistory": [
                {
                    "completed": false,
                    "finishedAt": "2026-01-15T12:01:00.000Z",
                    "exchange": {
                        "model_id": "grok-4-5",
                        "request_id": "partial",
                        "response_nodes": [
                            {
                                "token_usage": {
                                    "input_tokens": 999,
                                    "output_tokens": 50,
                                    "cache_read_input_tokens": 0,
                                    "cache_creation_input_tokens": 0
                                }
                            }
                        ]
                    }
                },
                {
                    "finishedAt": "2026-01-15T12:02:00.000Z",
                    "exchange": {
                        "model_id": "grok-4-5",
                        "request_id": "missing-completed",
                        "response_nodes": [
                            {
                                "token_usage": {
                                    "input_tokens": 100,
                                    "output_tokens": 10,
                                    "cache_read_input_tokens": 0,
                                    "cache_creation_input_tokens": 0
                                }
                            }
                        ]
                    }
                }
            ]
        }"#;
        let f = write_temp_json(json);
        assert!(parse_augment_file(f.path()).is_empty());
    }

    #[test]
    fn test_missing_finished_at_falls_back_to_file_mtime() {
        let json = r#"{
            "sessionId": "s-mtime",
            "agentState": { "modelId": "grok-4-5" },
            "chatHistory": [
                {
                    "completed": true,
                    "exchange": {
                        "model_id": "grok-4-5",
                        "request_id": "r-mtime",
                        "response_nodes": [
                            {
                                "token_usage": {
                                    "input_tokens": 3,
                                    "output_tokens": 1,
                                    "cache_read_input_tokens": 0,
                                    "cache_creation_input_tokens": 0
                                }
                            }
                        ]
                    }
                }
            ]
        }"#;
        let f = write_temp_json(json);
        let messages = parse_augment_file(f.path());
        assert_eq!(messages.len(), 1);
        let mtime = file_modified_timestamp_ms(f.path());
        // Allow small skew between metadata reads.
        assert!((messages[0].timestamp - mtime).abs() < 5_000);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!(messages[0].timestamp > 0);
        assert!(messages[0].timestamp <= now_ms + 5_000);
    }

    #[test]
    fn test_empty_exchange_model_falls_back_then_unknown_provider() {
        let json = r#"{
            "sessionId": "s-unknown",
            "agentState": { "modelId": "   " },
            "chatHistory": [
                {
                    "completed": true,
                    "finishedAt": "2026-07-20T13:33:00.000Z",
                    "exchange": {
                        "model_id": "",
                        "request_id": "r-u",
                        "response_nodes": [
                            {
                                "token_usage": {
                                    "input_tokens": 7,
                                    "output_tokens": 1,
                                    "cache_read_input_tokens": 0,
                                    "cache_creation_input_tokens": 0
                                }
                            }
                        ]
                    }
                }
            ]
        }"#;
        let f = write_temp_json(json);
        let messages = parse_augment_file(f.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "unknown");
        assert_eq!(messages[0].provider_id, "augment");
    }

    #[test]
    fn test_numeric_sequence_id_dedup_key() {
        let json = r#"{
            "sessionId": "s-seq",
            "agentState": { "modelId": "grok-4-5" },
            "chatHistory": [
                {
                    "completed": true,
                    "finishedAt": "2026-07-20T13:33:00.000Z",
                    "sequenceId": 42,
                    "exchange": {
                        "response_nodes": [
                            {
                                "token_usage": {
                                    "input_tokens": 2,
                                    "output_tokens": 1,
                                    "cache_read_input_tokens": 0,
                                    "cache_creation_input_tokens": 0
                                }
                            }
                        ]
                    }
                }
            ]
        }"#;
        let f = write_temp_json(json);
        let messages = parse_augment_file(f.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("augment:s-seq:seq:42")
        );
    }
}
