//! Codebuff (formerly Manicode) session parser
//!
//! Codebuff persists chat history under `~/.config/manicode/projects/<project>/
//! chats/<chatId>/`:
//!
//! - `chat-messages.json` – serialized ChatMessage[]; assistant messages carry
//!   token usage on `metadata.usage`, `metadata.codebuff.usage`, or (for
//!   provider-routed calls) `metadata.runState.sessionState.mainAgentState.
//!   messageHistory[*].providerOptions`.
//! - `run-state.json` – SDK RunState snapshot (not consumed here).
//!
//! Dev and staging channels use the same layout under `manicode-dev` and
//! `manicode-staging` roots. `chatId` is the chat's ISO-8601 timestamp with
//! `:` replaced by `-` for filesystem safety (e.g. `2025-12-14T10-00-00.000Z`).

use super::utils::{
    file_modified_timestamp_ms, parse_timestamp_str, parse_timestamp_value, read_file_or_none,
};
use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use serde_json::Value;
use std::path::Path;

const DEFAULT_MODEL: &str = "codebuff-unknown";

/// Parse a single `chat-messages.json` file into UnifiedMessages.
pub fn parse_codebuff_file(path: &Path) -> Vec<UnifiedMessage> {
    let Some(bytes) = read_file_or_none(path) else {
        return Vec::new();
    };
    let mut bytes = bytes;
    let root: Value = match simd_json::from_slice(&mut bytes) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let messages = match root.as_array() {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    let (channel, project_basename, chat_id) = derive_context_from_path(path);
    let session_id = format!("{}/{}/{}", channel, project_basename, chat_id);

    let chat_id_ts = parse_chat_id_to_millis(&chat_id).unwrap_or(0);
    let file_mtime_ms = file_modified_timestamp_ms(path);

    let mut results = Vec::new();
    for msg in messages {
        if !is_assistant_role(msg) {
            continue;
        }

        let usage = extract_assistant_usage(msg);
        if !usage.has_signal() {
            continue;
        }

        let chat_id_fallback = if chat_id_ts > 0 {
            Some(chat_id_ts)
        } else {
            None
        };
        let ts = message_timestamp(msg)
            .or(chat_id_fallback)
            .unwrap_or(file_mtime_ms);

        let model = usage
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        // Fall back to a neutral provider when we cannot infer one from the
        // model id. Codebuff routes calls across multiple providers, so
        // hardcoding "anthropic" would skew per-provider stats for any
        // token-bearing message that lacks a recognizable model hint.
        let provider = provider_identity::inferred_provider_from_model(&model).unwrap_or("unknown");

        // Stable dedup key so the same chat history scanned from multiple
        // roots (or re-imported into a parallel channel) is not double
        // counted. Prefer the upstream ChatMessage `id`; otherwise derive a
        // deterministic key from the session, timestamp, model and token
        // shape so identical re-imports collapse to a single record.
        let dedup_key = upstream_message_id(msg)
            .unwrap_or_else(|| derive_dedup_key(&session_id, ts, &model, &usage));

        results.push(UnifiedMessage::new_with_dedup(
            "codebuff",
            &model,
            provider,
            &session_id,
            ts,
            TokenBreakdown {
                input: usage.input_tokens.max(0),
                output: usage.output_tokens.max(0),
                cache_read: usage.cache_read_input_tokens.max(0),
                cache_write: usage.cache_creation_input_tokens.max(0),
                reasoning: 0,
            },
            usage.credits.max(0.0),
            Some(dedup_key),
        ));
    }

    results
}

/// Extract the upstream `ChatMessage.id` if present, so dedup keys remain
/// stable across re-imports of the same chat history.
fn upstream_message_id(msg: &Value) -> Option<String> {
    msg.get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Build a deterministic fallback dedup key for messages that don't expose
/// a stable upstream id. Combines the session, timestamp, model and full
/// token breakdown so two structurally identical messages collapse, while
/// genuinely different messages stay distinct.
fn derive_dedup_key(session_id: &str, ts: i64, model: &str, usage: &AssistantUsage) -> String {
    format!(
        "codebuff:{session_id}:{ts}:{model}:{i}:{o}:{cr}:{cw}",
        i = usage.input_tokens.max(0),
        o = usage.output_tokens.max(0),
        cr = usage.cache_read_input_tokens.max(0),
        cw = usage.cache_creation_input_tokens.max(0),
    )
}

/// Convert a filesystem-safe `chatId` back to epoch milliseconds.
///
/// Codebuff's `chatId` is the chat's ISO-8601 timestamp with the three `:`
/// separators in the time portion (`HH:MM:SS`) replaced by `-` for cross-
/// platform filesystem safety (e.g. `2025-12-14T10-00-00.000Z`). Only the
/// separators *after* the `T` need to be flipped back to `:`; the date
/// portion retains its normal `-` separators. A naive global
/// `chat_id.replace('-', ":")` corrupts the date to `2025:12:14T...` and
/// makes RFC3339 parsing fail silently.
fn parse_chat_id_to_millis(chat_id: &str) -> Option<i64> {
    let t_index = chat_id.find('T')?;
    let (date, time_with_separator) = chat_id.split_at(t_index);
    // `time_with_separator` starts with 'T'; rebuild "<date>T<HH:MM:SS...>".
    let rebuilt = format!("{}{}", date, time_with_separator.replacen('-', ":", 2));
    // We only touch the two time separators (`HH:MM` and `MM:SS`); any
    // leftover `-` afterwards belongs to the millisecond/timezone portion
    // (e.g. `2025-12-14T10:00:00.000+00:00`) and must stay intact.
    parse_timestamp_str(&rebuilt)
}

/// Walks up a `chat-messages.json` file path and returns
/// `(channel, project_basename, chat_id)` by reading the three relevant
/// ancestor directory names. Missing ancestors fall back to empty strings so
/// that malformed layouts still produce a deterministic (but lossy) session
/// identifier instead of panicking.
fn derive_context_from_path(path: &Path) -> (String, String, String) {
    let chat_id = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // chats/<chatId>/chat-messages.json → jump up to projects/<project>/chats
    let chats_dir = path.parent().and_then(|p| p.parent());
    let project_basename = chats_dir
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // ../<project>/chats/<chatId>/ → projects dir’s parent is the channel root
    let channel = chats_dir
        .and_then(|p| p.parent()) // project dir
        .and_then(|p| p.parent()) // "projects"
        .and_then(|p| p.parent()) // channel root (e.g. manicode[-dev])
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("manicode")
        .to_string();

    (channel, project_basename, chat_id)
}

fn is_assistant_role(msg: &Value) -> bool {
    let variant = msg
        .get("variant")
        .and_then(|v| v.as_str())
        .or_else(|| msg.get("role").and_then(|v| v.as_str()))
        .unwrap_or("");
    matches!(variant, "ai" | "agent" | "assistant")
}

fn message_timestamp(msg: &Value) -> Option<i64> {
    for key in ["timestamp", "createdAt"] {
        if let Some(v) = msg.get(key) {
            if let Some(ts) = parse_timestamp_value(v) {
                return Some(ts);
            }
        }
    }
    if let Some(meta_ts) = msg.get("metadata").and_then(|m| m.get("timestamp")) {
        return parse_timestamp_value(meta_ts);
    }
    None
}

#[derive(Default, Debug, Clone)]
struct AssistantUsage {
    model: Option<String>,
    credits: f64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_input_tokens: i64,
    cache_creation_input_tokens: i64,
}

impl AssistantUsage {
    fn has_signal(&self) -> bool {
        self.input_tokens > 0
            || self.output_tokens > 0
            || self.cache_read_input_tokens > 0
            || self.cache_creation_input_tokens > 0
            || self.credits > 0.0
    }

    fn merge_fallback(&mut self, other: AssistantUsage) {
        if self.input_tokens <= 0 {
            self.input_tokens = other.input_tokens;
        }
        if self.output_tokens <= 0 {
            self.output_tokens = other.output_tokens;
        }
        if self.cache_read_input_tokens <= 0 {
            self.cache_read_input_tokens = other.cache_read_input_tokens;
        }
        if self.cache_creation_input_tokens <= 0 {
            self.cache_creation_input_tokens = other.cache_creation_input_tokens;
        }
        if self.model.is_none() {
            self.model = other.model;
        }
        if self.credits <= 0.0 {
            self.credits = other.credits;
        }
    }
}

/// Extract assistant usage trying, in order: `metadata.usage`,
/// `metadata.codebuff.usage`, and the stashed RunState message history (which
/// is where OpenRouter-routed calls land their final token counts).
fn extract_assistant_usage(msg: &Value) -> AssistantUsage {
    let metadata = msg.get("metadata");

    let mut usage = AssistantUsage::default();

    if let Some(meta) = metadata {
        if let Some(model) = meta.get("model").and_then(|v| v.as_str()) {
            usage.model = Some(model.to_string());
        }
        if let Some(u) = meta.get("usage") {
            usage.merge_fallback(parse_usage_object(u));
        }
        if let Some(u) = meta.get("codebuff").and_then(|c| c.get("usage")) {
            usage.merge_fallback(parse_usage_object(u));
        }
        if let Some(run_state_usage) = extract_usage_from_run_state(meta) {
            usage.merge_fallback(run_state_usage);
        }
    }

    if let Some(credits) = msg.get("credits").and_then(|v| v.as_f64()) {
        if credits > 0.0 && usage.credits <= 0.0 {
            usage.credits = credits;
        }
    }

    usage
}

/// Find the last assistant entry in `metadata.runState.sessionState.
/// mainAgentState.messageHistory` and pull `providerOptions.usage` (or
/// `providerOptions.codebuff.usage`) plus any model hint it carries.
fn extract_usage_from_run_state(metadata: &Value) -> Option<AssistantUsage> {
    let history = metadata
        .get("runState")
        .and_then(|rs| rs.get("sessionState"))
        .and_then(|ss| ss.get("mainAgentState"))
        .and_then(|mas| mas.get("messageHistory"))
        .and_then(|v| v.as_array())?;

    for entry in history.iter().rev() {
        let role = entry.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role != "assistant" {
            continue;
        }
        let Some(provider_options) = entry.get("providerOptions") else {
            continue;
        };
        let mut usage = AssistantUsage::default();
        if let Some(u) = provider_options.get("usage") {
            usage.merge_fallback(parse_usage_object(u));
        }
        if let Some(u) = provider_options
            .get("codebuff")
            .and_then(|c| c.get("usage"))
        {
            usage.merge_fallback(parse_usage_object(u));
        }
        if let Some(model) = provider_options
            .get("codebuff")
            .and_then(|c| c.get("model"))
            .and_then(|v| v.as_str())
        {
            usage.model = Some(model.to_string());
        }
        if usage.has_signal() || usage.model.is_some() {
            return Some(usage);
        }
    }
    None
}

/// Accept both camelCase and snake_case shapes, matching the @ccusage/codebuff
/// valibot schema (different upstreams ship different casings).
fn parse_usage_object(value: &Value) -> AssistantUsage {
    let mut usage = AssistantUsage::default();

    let input = pick_number(
        value,
        &[
            "inputTokens",
            "input_tokens",
            "promptTokens",
            "prompt_tokens",
        ],
    );
    let output = pick_number(
        value,
        &[
            "outputTokens",
            "output_tokens",
            "completionTokens",
            "completion_tokens",
        ],
    );
    let cache_read = pick_number(
        value,
        &[
            "cacheReadInputTokens",
            "cache_read_input_tokens",
            "cachedTokensCreated",
            "cached_tokens_created",
        ],
    )
    .or_else(|| {
        value
            .get("promptTokensDetails")
            .or_else(|| value.get("prompt_tokens_details"))
            .and_then(|d| {
                d.get("cachedTokens")
                    .or_else(|| d.get("cached_tokens"))
                    .and_then(|v| v.as_i64())
            })
    });
    let cache_write = pick_number(
        value,
        &[
            "cacheCreationInputTokens",
            "cache_creation_input_tokens",
            "cacheCreationTokens",
            "cache_creation_tokens",
        ],
    );

    usage.input_tokens = input.unwrap_or(0);
    usage.output_tokens = output.unwrap_or(0);
    usage.cache_read_input_tokens = cache_read.unwrap_or(0);
    usage.cache_creation_input_tokens = cache_write.unwrap_or(0);

    if let Some(credits) = value.get("credits").and_then(|v| v.as_f64()) {
        usage.credits = credits;
    }
    if let Some(model) = value.get("model").and_then(|v| v.as_str()) {
        usage.model = Some(model.to_string());
    }

    usage
}

fn pick_number(value: &Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(v) = value.get(*key) {
            if let Some(n) = v
                .as_i64()
                .or_else(|| v.as_u64().map(|v| v as i64))
                .or_else(|| v.as_f64().map(|f| f as i64))
            {
                if n > 0 {
                    return Some(n);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_derive_context_from_path_extracts_channel_project_and_chat_id() {
        let p = PathBuf::from(
            "/tmp/home/.config/manicode-dev/projects/sandbox/chats/2025-12-14T10-00-00.000Z/chat-messages.json",
        );
        let (channel, project, chat_id) = derive_context_from_path(&p);
        assert_eq!(channel, "manicode-dev");
        assert_eq!(project, "sandbox");
        assert_eq!(chat_id, "2025-12-14T10-00-00.000Z");
    }

    #[test]
    fn test_extract_assistant_usage_from_metadata_usage() {
        let msg: Value = serde_json::from_str(
            r#"{
                "role": "assistant",
                "metadata": {
                    "model": "claude-sonnet-4-20250514",
                    "usage": {
                        "inputTokens": 1000,
                        "outputTokens": 400,
                        "cacheReadInputTokens": 200,
                        "cacheCreationInputTokens": 50
                    }
                },
                "credits": 1.5
            }"#,
        )
        .unwrap();

        let usage = extract_assistant_usage(&msg);
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 400);
        assert_eq!(usage.cache_read_input_tokens, 200);
        assert_eq!(usage.cache_creation_input_tokens, 50);
        assert_eq!(usage.credits, 1.5);
        assert_eq!(usage.model.as_deref(), Some("claude-sonnet-4-20250514"));
    }

    #[test]
    fn test_extract_usage_snake_case_shape() {
        let msg: Value = serde_json::from_str(
            r#"{
                "role": "assistant",
                "metadata": {
                    "codebuff": {
                        "usage": {
                            "prompt_tokens": 750,
                            "completion_tokens": 120,
                            "prompt_tokens_details": { "cached_tokens": 100 }
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let usage = extract_assistant_usage(&msg);
        assert_eq!(usage.input_tokens, 750);
        assert_eq!(usage.output_tokens, 120);
        assert_eq!(usage.cache_read_input_tokens, 100);
    }

    #[test]
    fn test_extract_usage_falls_back_to_run_state_message_history() {
        let msg: Value = serde_json::from_str(
            r#"{
                "role": "assistant",
                "metadata": {
                    "runState": {
                        "sessionState": {
                            "mainAgentState": {
                                "messageHistory": [
                                    { "role": "user", "providerOptions": {} },
                                    {
                                        "role": "assistant",
                                        "providerOptions": {
                                            "codebuff": {
                                                "model": "openai/gpt-5",
                                                "usage": {
                                                    "inputTokens": 2000,
                                                    "outputTokens": 800,
                                                    "cacheReadInputTokens": 400
                                                }
                                            }
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let usage = extract_assistant_usage(&msg);
        assert_eq!(usage.input_tokens, 2000);
        assert_eq!(usage.output_tokens, 800);
        assert_eq!(usage.cache_read_input_tokens, 400);
        assert_eq!(usage.model.as_deref(), Some("openai/gpt-5"));
    }

    #[test]
    fn test_is_assistant_role_accepts_variant_and_role() {
        let ai: Value = serde_json::from_str(r#"{"variant":"ai"}"#).unwrap();
        let assistant: Value = serde_json::from_str(r#"{"role":"assistant"}"#).unwrap();
        let user: Value = serde_json::from_str(r#"{"role":"user"}"#).unwrap();
        assert!(is_assistant_role(&ai));
        assert!(is_assistant_role(&assistant));
        assert!(!is_assistant_role(&user));
    }

    #[test]
    fn test_parse_chat_id_to_millis_restores_time_separators_without_touching_date() {
        // 2025-12-14T10:00:00.000Z == 1 765 706 400 000 ms
        let expected = 1_765_706_400_000_i64;
        let parsed = parse_chat_id_to_millis("2025-12-14T10-00-00.000Z").unwrap();
        assert_eq!(parsed, expected);

        // A global `-`→`:` replace would corrupt this to "2025:12:14T..." and
        // return None. Guarding against that regression here.
        let broken = "2025-12-14T10-00-00.000Z".replace('-', ":");
        assert!(parse_timestamp_str(&broken).is_none());
    }

    #[test]
    fn test_parse_chat_id_to_millis_returns_none_for_garbage() {
        assert!(parse_chat_id_to_millis("not-a-chat-id").is_none());
        assert!(parse_chat_id_to_millis("").is_none());
    }

    #[test]
    fn test_parse_codebuff_file_skips_messages_without_token_signal() {
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let chat_dir = dir
            .path()
            .join("manicode")
            .join("projects")
            .join("proj")
            .join("chats")
            .join("2025-12-20T12-00-00.000Z");
        fs::create_dir_all(&chat_dir).unwrap();
        let msgs_path = chat_dir.join("chat-messages.json");
        fs::write(
            &msgs_path,
            r#"[
                { "variant": "user", "content": "hi" },
                { "variant": "ai",
                  "timestamp": "2025-12-20T12:00:05.000Z",
                  "metadata": {
                    "model": "claude-sonnet-4-20250514",
                    "usage": { "inputTokens": 10, "outputTokens": 5 }
                  }
                },
                { "variant": "ai",
                  "timestamp": "2025-12-20T12:00:06.000Z",
                  "metadata": { "model": "claude-sonnet-4-20250514" }
                }
            ]"#,
        )
        .unwrap();

        let messages = parse_codebuff_file(&msgs_path);
        assert_eq!(messages.len(), 1);
        let only = &messages[0];
        assert_eq!(only.client, "codebuff");
        assert_eq!(only.model_id, "claude-sonnet-4-20250514");
        assert_eq!(only.provider_id, "anthropic");
        assert!(only.session_id.ends_with("/proj/2025-12-20T12-00-00.000Z"));
        assert_eq!(only.tokens.input, 10);
        assert_eq!(only.tokens.output, 5);
    }
}
