//! Command Code session parser
//!
//! Parses JSONL transcripts from `~/.commandcode/projects/<slug>/<session>.jsonl`.
//!
//! Unlike most sources, Command Code does NOT persist token usage locally: the
//! CLI computes per-request usage in memory and ships it to its backend
//! (`api.commandcode.ai`, surfaced in the web Usage dashboard). The on-disk
//! transcript only contains message text (one JSON object per line with
//! `role`/`content`/`timestamp`/`sessionId`), so token counts are ESTIMATED
//! from message text at ~4 characters per token, consistent with tokscale's
//! other estimated sources (see Kiro).
//!
//! These estimates approximate tokens processed; they will not match Command
//! Code's server-reported usage, which reflects tool-output truncation and
//! auxiliary model runs (e.g. tool-desc, taste-1) absent from the transcript.
//!
//! To mirror how Command Code itself reports usage (and how tokscale already
//! counts re-sent context for Claude via cache reads), input is estimated from
//! the cumulative conversation context preceding each assistant response, and
//! output from the assistant message's own content. The model id is not stored
//! per message, so it is read from `~/.commandcode/config.json` (the configured
//! agent model), falling back to "unknown".

use super::utils::file_modified_timestamp_ms;
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::Path;

const CLIENT_ID: &str = "commandcode";
const PROVIDER_ID: &str = "command-code";
const UNKNOWN_MODEL: &str = "unknown";

#[derive(Debug, Deserialize)]
struct CommandCodeEntry {
    role: Option<String>,
    content: Option<serde_json::Value>,
    timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CommandCodeConfig {
    model: Option<String>,
}

pub fn parse_commandcode_file(path: &Path) -> Vec<UnifiedMessage> {
    // The `*.jsonl` glob also matches the per-session checkpoint log
    // (`<session>.checkpoints.jsonl`), which is a snapshot stream, not a
    // transcript. Skip it explicitly rather than relying on schema mismatch.
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".checkpoints.jsonl"))
    {
        return Vec::new();
    }

    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };

    let fallback_timestamp = file_modified_timestamp_ms(path);
    let model_id = model_from_config(path)
        .map(|model| canonicalize_model(&model))
        .unwrap_or_else(|| UNKNOWN_MODEL.to_string());
    let session_id_from_path = session_id_from_path(path);
    let workspace_key = workspace_key_from_path(path);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

    let mut messages = Vec::new();
    let mut session_id: Option<String> = None;
    // Running char count of the conversation context seen so far. This stands in
    // for the input (prompt) tokens sent on each request, which Command Code
    // re-sends in full every turn.
    let mut context_chars: usize = 0;
    // The first assistant message after a user message starts a new turn.
    let mut pending_turn_start = false;
    let mut assistant_index = 0usize;

    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let entry = match serde_json::from_str::<CommandCodeEntry>(trimmed) {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        if session_id.is_none() {
            if let Some(id) = entry.session_id.as_deref().filter(|id| !id.is_empty()) {
                session_id = Some(id.to_string());
            }
        }

        let chars = entry.content.as_ref().map(content_chars).unwrap_or(0);

        match entry.role.as_deref() {
            Some("assistant") => {
                let input = estimate_tokens(context_chars);
                let output = estimate_tokens(chars);
                // Context the model received to produce this response includes
                // everything before it, not its own output.
                context_chars += chars;

                if input + output == 0 {
                    pending_turn_start = false;
                    continue;
                }

                let resolved_session = session_id
                    .clone()
                    .unwrap_or_else(|| session_id_from_path.clone());
                let timestamp = entry
                    .timestamp
                    .as_deref()
                    .and_then(parse_rfc3339_ms)
                    .unwrap_or(fallback_timestamp);

                let mut message = UnifiedMessage::new_with_dedup(
                    CLIENT_ID,
                    model_id.clone(),
                    PROVIDER_ID,
                    resolved_session.clone(),
                    timestamp,
                    TokenBreakdown {
                        input,
                        output,
                        cache_read: 0,
                        cache_write: 0,
                        reasoning: 0,
                    },
                    0.0,
                    Some(format!("{}:{}", resolved_session, assistant_index)),
                );
                message.message_count = 1;
                message.is_turn_start = pending_turn_start;
                message.set_workspace(workspace_key.clone(), workspace_label.clone());
                messages.push(message);

                assistant_index += 1;
                pending_turn_start = false;
            }
            Some("user") => {
                pending_turn_start = true;
                context_chars += chars;
            }
            // Tool results (and any other roles) are part of the context the
            // model sees on subsequent turns.
            _ => {
                context_chars += chars;
            }
        }
    }

    messages
}

/// Char count of a message's `content` for token estimation, measured from its
/// canonical JSON serialization. Counting the serialized form keeps every
/// prompt-bearing byte the model receives — object keys (`command`, `path`, …),
/// tool-call arguments, tool-result payloads, and numeric/boolean values — and
/// avoids guessing which fields are structural versus content.
///
/// Genuinely empty content (null, `[]`, `{}`) counts as zero so that contentless
/// turns are not charged for their structural brackets.
fn content_chars(content: &serde_json::Value) -> usize {
    match content {
        serde_json::Value::Null => 0,
        serde_json::Value::Array(items) if items.is_empty() => 0,
        serde_json::Value::Object(map) if map.is_empty() => 0,
        _ => serde_json::to_string(content)
            .map(|serialized| serialized.chars().count())
            .unwrap_or(0),
    }
}

fn estimate_tokens(chars: usize) -> i64 {
    chars.div_ceil(4) as i64
}

/// Canonicalize the configured model id for pricing. Command Code reports
/// gateway ids such as `MiniMaxAI/MiniMax-M3-Free`; the `MiniMaxAI/` org prefix
/// steers tokscale's pricing resolver to the wrong model and the `-Free` suffix
/// is a temporary promo. Stripping both yields the real paid model (e.g.
/// `MiniMax-M3`), so the cost estimate reflects what the tokens actually cost.
fn canonicalize_model(model: &str) -> String {
    let base = model.rsplit('/').next().unwrap_or(model);
    const PROMO_SUFFIX: &str = "-free";
    if base.len() > PROMO_SUFFIX.len()
        && base[base.len() - PROMO_SUFFIX.len()..].eq_ignore_ascii_case(PROMO_SUFFIX)
    {
        base[..base.len() - PROMO_SUFFIX.len()].to_string()
    } else {
        base.to_string()
    }
}

fn parse_rfc3339_ms(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Read the configured agent model from `~/.commandcode/config.json`.
///
/// `session_path` is `<root>/.commandcode/projects/<slug>/<session>.jsonl`, so
/// the config file lives three directories up.
fn model_from_config(session_path: &Path) -> Option<String> {
    let commandcode_root = session_path.parent()?.parent()?.parent()?;
    let config_path = commandcode_root.join("config.json");
    let bytes = std::fs::read(config_path).ok()?;
    let config: CommandCodeConfig = serde_json::from_slice(&bytes).ok()?;
    config.model.filter(|model| !model.trim().is_empty())
}

fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Command Code names project directories after a slugified working directory
/// (e.g. `users-alice-development-repo`). The original path is not recoverable
/// (lowercased, separators collapsed), so the slug itself is used as the
/// workspace key.
fn workspace_key_from_path(path: &Path) -> Option<String> {
    path.parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .and_then(normalize_workspace_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_session(dir: &TempDir, slug: &str, session: &str, jsonl: &str) -> std::path::PathBuf {
        let project_dir = dir.path().join("projects").join(slug);
        std::fs::create_dir_all(&project_dir).unwrap();
        let path = project_dir.join(format!("{session}.jsonl"));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(jsonl.as_bytes()).unwrap();
        path
    }

    fn write_config(dir: &TempDir, model: &str) {
        let path = dir.path().join("config.json");
        let mut file = std::fs::File::create(&path).unwrap();
        write!(file, r#"{{"provider":"command-code","model":"{model}"}}"#).unwrap();
    }

    #[test]
    fn test_canonicalize_model_strips_org_prefix_and_free_promo_suffix() {
        // "-Free" is a temporary promo; the org prefix mis-resolves pricing.
        assert_eq!(
            canonicalize_model("MiniMaxAI/MiniMax-M3-Free"),
            "MiniMax-M3"
        );
        assert_eq!(
            canonicalize_model("minimaxai/minimax-m3-free"),
            "minimax-m3"
        );
        assert_eq!(canonicalize_model("MiniMaxAI/MiniMax-M2.5"), "MiniMax-M2.5");
        assert_eq!(canonicalize_model("taste-1"), "taste-1");
    }

    #[test]
    fn test_content_chars_counts_keys_numbers_and_nested_payloads() {
        // Structured tool args/results carry meaning in keys and primitive
        // values; a string-only counter would return 0 for numeric content.
        assert!(content_chars(&json!([{"value": 12345}])) > 0);
        let small = content_chars(&json!([{"a": "x"}]));
        let large = content_chars(&json!([{"command": "run", "args": ["a", "b"], "n": 42}]));
        assert!(large > small);
    }

    #[test]
    fn test_parse_canonicalizes_model_and_estimates_tokens() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "MiniMaxAI/MiniMax-M3-Free");
        let user = json!([{"type": "text", "text": "12345678"}]);
        let assistant = json!([{"type": "text", "text": "abcd"}]);
        let jsonl = format!(
            "{}\n{}",
            json!({"role": "user", "sessionId": "sess-1", "timestamp": "2026-06-16T05:58:15.580Z", "content": user.clone()}),
            json!({"role": "assistant", "sessionId": "sess-1", "timestamp": "2026-06-16T05:58:20.332Z", "content": assistant.clone()}),
        );
        let path = write_session(&dir, "users-alice-repo", "sess-1", &jsonl);

        let messages = parse_commandcode_file(&path);

        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.client, "commandcode");
        assert_eq!(msg.provider_id, "command-code");
        // Promo suffix + org prefix stripped so pricing hits the real model.
        assert_eq!(msg.model_id, "MiniMax-M3");
        assert_eq!(msg.session_id, "sess-1");
        // Input = context before this turn (just the user message); output = this
        // assistant message. Computed from the same helper to avoid brittle counts.
        assert_eq!(msg.tokens.input, estimate_tokens(content_chars(&user)));
        assert_eq!(
            msg.tokens.output,
            estimate_tokens(content_chars(&assistant))
        );
        assert!(msg.tokens.input > 0 && msg.tokens.output > 0);
        assert_eq!(msg.message_count, 1);
        assert!(msg.is_turn_start);
        assert_eq!(msg.timestamp, 1781589500332); // 2026-06-16T05:58:20.332Z
        assert_eq!(msg.workspace_key.as_deref(), Some("users-alice-repo"));
        assert_eq!(msg.workspace_label.as_deref(), Some("users-alice-repo"));
    }

    #[test]
    fn test_input_grows_with_cumulative_context() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        // Two turns; the second assistant's input must include all prior text
        // (user1 + assistant1 + tool result + user2), so it exceeds the first.
        let jsonl = concat!(
            r#"{"role":"user","sessionId":"s","content":[{"type":"text","text":"aaaa"}]}"#,
            "\n",
            r#"{"role":"assistant","sessionId":"s","content":[{"type":"text","text":"bbbb"}]}"#,
            "\n",
            r#"{"role":"tool","sessionId":"s","content":[{"type":"tool-result","output":{"type":"text","value":"cccccccc"}}]}"#,
            "\n",
            r#"{"role":"user","sessionId":"s","content":[{"type":"text","text":"dddd"}]}"#,
            "\n",
            r#"{"role":"assistant","sessionId":"s","content":[{"type":"text","text":"e"}]}"#,
        );
        let path = write_session(&dir, "proj", "s", jsonl);

        let messages = parse_commandcode_file(&path);

        assert_eq!(messages.len(), 2);
        assert!(messages[0].tokens.input > 0);
        assert!(messages[0].is_turn_start);
        assert!(messages[1].is_turn_start);
        // Cumulative context strictly grows across turns.
        assert!(messages[1].tokens.input > messages[0].tokens.input);
    }

    #[test]
    fn test_checkpoint_files_are_skipped() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let project_dir = dir.path().join("projects").join("proj");
        std::fs::create_dir_all(&project_dir).unwrap();
        let path = project_dir.join("s.checkpoints.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(
            br#"{"type":"checkpoint","messageId":"m","snapshot":"snap","isSnapshotUpdate":false}"#,
        )
        .unwrap();

        let messages = parse_commandcode_file(&path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_missing_config_falls_back_to_unknown_model() {
        let dir = TempDir::new().unwrap();
        let jsonl = concat!(
            r#"{"role":"user","sessionId":"s","content":[{"type":"text","text":"hello"}]}"#,
            "\n",
            r#"{"role":"assistant","sessionId":"s","content":[{"type":"text","text":"world"}]}"#,
        );
        let path = write_session(&dir, "proj", "s", jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "unknown");
    }

    #[test]
    fn test_skips_malformed_lines_without_panicking() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let jsonl = concat!(
            r#"{"role":"user","sessionId":"s","content":[{"type":"text","text":"hello"}]}"#,
            "\n",
            "not valid json at all",
            "\n",
            r#"{"role":"assistant","sessionId":"s","content":[{"type":"text","text":"response"}]}"#,
        );
        let path = write_session(&dir, "proj", "s", jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].tokens.input > 0 || messages[0].tokens.output > 0);
    }

    #[test]
    fn test_empty_assistant_with_no_context_is_skipped() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        // Assistant with no content and no preceding context -> 0 tokens, skip.
        let jsonl = r#"{"role":"assistant","sessionId":"s","content":[]}"#;
        let path = write_session(&dir, "proj", "s", jsonl);

        let messages = parse_commandcode_file(&path);
        assert!(messages.is_empty());
    }
}
