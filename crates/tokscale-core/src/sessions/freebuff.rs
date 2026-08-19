//! Freebuff session parser
//!
//! Freebuff (https://github.com/CodebuffAI/freebuff) is not a separate program
//! from Codebuff: it is the same CLI compiled with `FREEBUFF_MODE=true`, a
//! free-only build that strips paid features. It therefore resolves the same
//! config dir (`~/.config/manicode[-dev|-staging]`) and writes the same
//! `projects/<project>/chats/<chatId>/chat-messages.json` layout as Codebuff.
//! The two products can share one tree on one machine, so location cannot
//! attribute a chat and neither can "this chat records no usage" — plenty of
//! ordinary Codebuff turns record none either (interrupted runs, errors).
//!
//! The discriminator used here is the **root agent id** the run actually used.
//! Freebuff hardcodes FREE mode and maps every model its picker offers onto a
//! `base2-free*` root agent (`base2-free`, `base2-free-deepseek-flash`,
//! `base2-free-minimax-m3`, `base2-free-luna`, ...), while Codebuff's modes map
//! to `base2`, `base2-lite`, `base2-max` and `base2-plan`. On turn completion
//! the CLI persists the run state onto the assistant message, so that id is
//! readable per chat at
//! `metadata.runState.sessionState.mainAgentState.agentType`.
//!
//! A chat is treated as Freebuff only when it carries that positive marker.
//! An unmarked chat (no completed turn, or a CLI old enough not to persist run
//! state) is left alone rather than guessed at, because the client id flows
//! into the TUI, report grouping and the submitted payload — a wrong guess is
//! misattributed submitted data, not just a display quirk.
//!
//! Freebuff does not persist token usage locally (`ChatMessageMetadata` has no
//! usage field; only `credits`, which is 0 in free mode), so token counts are
//! ESTIMATED from message text at ~4 characters per token, consistent with
//! tokscale's other estimated sources (see CommandCode, Kiro, ZCode).
//!
//! The model is not stored per message, so it is read from the channel root's
//! `settings.json` (`freebuffModel`, written by Freebuff's model picker),
//! falling back to "freebuff-unknown".

use super::codebuff::{
    derive_context_from_path, extract_assistant_usage, is_assistant_role, message_timestamp,
    parse_chat_id_to_millis,
};
use super::utils::{estimate_tokens, file_modified_timestamp_ms, read_file_or_none};
use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use serde_json::Value;
use std::path::Path;

const CLIENT_ID: &str = "freebuff";
const DEFAULT_MODEL: &str = "freebuff-unknown";

/// Root agent ids Freebuff runs are prefixed with. Codebuff's own roots
/// (`base2`, `base2-lite`, `base2-max`, `base2-plan`) never match, so this is a
/// positive marker rather than an absence test.
const FREEBUFF_ROOT_AGENT_PREFIX: &str = "base2-free";

/// Read the root agent id a message's persisted run state recorded, if any.
/// The CLI attaches the whole `RunState` to the assistant message when a turn
/// completes, and the runtime stamps the resolved root agent id onto
/// `sessionState.mainAgentState.agentType`.
fn root_agent_id(msg: &Value) -> Option<&str> {
    msg.get("metadata")?
        .get("runState")?
        .get("sessionState")?
        .get("mainAgentState")?
        .get("agentType")?
        .as_str()
        .filter(|s| !s.trim().is_empty())
}

/// Whether any message in the chat proves the chat was produced by Freebuff.
fn is_freebuff_chat(messages: &[Value]) -> bool {
    messages
        .iter()
        .filter_map(root_agent_id)
        .any(|agent| agent.starts_with(FREEBUFF_ROOT_AGENT_PREFIX))
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
/// Returns an empty vec unless the chat carries a `base2-free*` root agent id,
/// so Codebuff chats sharing this directory are never claimed as Freebuff.
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

    // Attribute only on a positive Freebuff marker. Codebuff shares this
    // directory, so an unmarked chat is left to the codebuff parser rather
    // than estimated here on the strength of what it lacks.
    if !is_freebuff_chat(messages) {
        return Vec::new();
    }

    // Belt and braces: should a Freebuff-marked chat ever carry authoritative
    // usage, it is the codebuff parser's to emit, so the two never double
    // count the shared scan.
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
