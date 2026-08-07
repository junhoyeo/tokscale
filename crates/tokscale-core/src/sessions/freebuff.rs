//! Freebuff session parser
//!
//! Freebuff (https://github.com/CodebuffAI/freebuff) is a sibling product to
//! Codebuff and is built on the same underlying runtime, so it persists chat
//! history under the same `~/.config/manicode/projects/<project>/chats/<chatId>/`
//! layout and file schema as Codebuff (`chat-messages.json` etc.).
//!
//! Unlike Codebuff, Freebuff does NOT persist token usage locally: usage is
//! computed in memory and shipped to its backend (Freebuff is a free,
//! ad-supported product whose web Usage dashboard is server-side). The on-disk
//! transcript only contains message text, so token counts are ESTIMATED from
//! message text at ~4 characters per token, consistent with tokscale's other
//! estimated sources (see CommandCode, Kiro, ZCode).
//!
//! Because Freebuff and Codebuff share the same directory and file schema, the
//! two parsers partition the shared scan in `lib.rs`: `parse_codebuff_file`
//! emits only chats carrying authoritative usage metadata (real Codebuff
//! sessions), while `parse_freebuff_file` skips those and emits estimated rows
//! for the rest (Freebuff sessions). This keeps the two products attributed
//! separately without double counting.
//!
//! The model is not stored per message, so it is read from the channel root's
//! `settings.json` (`freebuffModel`), falling back to "freebuff-unknown".

use super::codebuff::{
    derive_context_from_path, extract_assistant_usage, is_assistant_role, message_timestamp,
    parse_chat_id_to_millis,
};
use super::utils::{file_modified_timestamp_ms, read_file_or_none};
use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use serde_json::Value;
use std::path::Path;

const CLIENT_ID: &str = "freebuff";
const DEFAULT_MODEL: &str = "freebuff-unknown";

/// Estimate tokens from character length at ~4 chars/token, matching the other
/// estimated sources (CommandCode, Kiro, ZCode).
fn estimate_tokens(chars: usize) -> i64 {
    chars.div_ceil(4) as i64
}

/// Collect the textual content of a Freebuff message for token estimation.
/// Top-level `content` carries the user prompt; assistant text lives in
/// `blocks[*].content` (mode-divider blocks contribute nothing).
fn message_text_chars(msg: &Value) -> usize {
    let mut chars = 0usize;
    if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
        chars += s.chars().count();
    }
    if let Some(blocks) = msg.get("blocks").and_then(|v| v.as_array()) {
        for block in blocks {
            if let Some(content) = block.get("content").and_then(|v| v.as_str()) {
                chars += content.chars().count();
            }
        }
    }
    chars
}

/// Read the configured agent model from the channel root's `settings.json`
/// (`freebuffModel`), mirroring how CommandCode reads `~/.commandcode/config.json`.
fn model_from_settings(path: &Path) -> Option<String> {
    // chat-messages.json -> chats/<chatId> -> <project> -> projects -> channel root
    let settings_path = path
        .parent()?
        .parent()?
        .parent()?
        .parent()?
        .parent()?
        .join("settings.json");
    let bytes = std::fs::read(settings_path).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("freebuffModel")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

/// Parse a single `chat-messages.json` into estimated Freebuff UnifiedMessages.
///
/// Returns an empty vec when the file carries authoritative usage (a real
/// Codebuff session sharing this directory), so the two parsers never double
/// count the shared scan.
pub fn parse_freebuff_file(path: &Path) -> Vec<UnifiedMessage> {
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

    // Real Codebuff chats carry authoritative usage in this same directory;
    // those belong to the codebuff parser. Defer entirely when any assistant
    // message exposes usage so the two products never double count.
    if messages
        .iter()
        .any(|m| is_assistant_role(m) && extract_assistant_usage(m).has_signal())
    {
        return Vec::new();
    }

    let (channel, project_basename, chat_id) = derive_context_from_path(path);
    let session_id = format!("{}/{}/{}", channel, project_basename, chat_id);

    let chat_id_ts = parse_chat_id_to_millis(&chat_id).unwrap_or(0);
    let file_mtime_ms = file_modified_timestamp_ms(path);

    let model = model_from_settings(path).unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let provider = provider_identity::inferred_provider_from_model(&model).unwrap_or("unknown");

    let mut results = Vec::new();
    // Input estimation is per-turn, not cumulative: only the *new* context
    // introduced since the previous assistant response (user prompt + tool
    // results) is counted, so a session's input sums to its own content exactly
    // once — the same accounting other estimated clients use (see CommandCode).
    let mut turn_input_chars: usize = 0;
    let mut pending_turn_start = false;
    let mut assistant_index = 0usize;

    for msg in messages.iter() {
        let msg_chars = message_text_chars(msg);

        if !is_assistant_role(msg) {
            // user / tool content is new context for the next assistant turn.
            pending_turn_start = true;
            turn_input_chars += msg_chars;
            continue;
        }

        // Assistant messages with no output text (e.g. Freebuff's mode-divider
        // rows) carry no usage to record; skip them and keep the accumulated
        // input for the next real response.
        if msg_chars == 0 {
            continue;
        }

        let input = estimate_tokens(turn_input_chars);
        let output = estimate_tokens(msg_chars);
        turn_input_chars = 0;

        let chat_id_fallback = if chat_id_ts > 0 {
            Some(chat_id_ts)
        } else {
            None
        };
        let ts = message_timestamp(msg)
            .or(chat_id_fallback)
            .unwrap_or(file_mtime_ms);

        let mut message = UnifiedMessage::new_with_dedup(
            CLIENT_ID,
            &model,
            provider,
            &session_id,
            ts,
            TokenBreakdown {
                input,
                output,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
            Some(format!("{}:{}", session_id, assistant_index)),
        );
        message.message_count = 1;
        message.is_turn_start = pending_turn_start;
        results.push(message);

        assistant_index += 1;
        pending_turn_start = false;
    }

    results
}
