//! Kiro session parser
//!
//! Parses session data from four sources:
//! 1. File-based (Kiro CLI): ~/.kiro/sessions/cli/*.json + *.jsonl
//! 2. Kiro IDE globalStorage snapshots
//! 3. SQLite-based: ~/Library/Application Support/kiro-cli/data.sqlite3
//!    (conversations_v2 table with `history[*].request_metadata`)
//! 4. File-based (Kiro IDE): `~/.kiro/sessions/<workspace>/sess_<uuid>/`
//!    session.json (metadata) + messages.jsonl (conversation). This is the
//!    VS Code-based Kiro IDE layout, distinct from the CLI's cli/*.json layout.
//!
//! Real per-turn token counts win when present and > 0. They are commonly
//! zero (the Auto agent persists zeros), so usage is then ESTIMATED with a
//! shared hybrid model across all three paths: fresh `input` is the turn's own
//! new content — the user prompt bytes PLUS this turn's tool_result bytes (a
//! tool_result is fresh input on the turn it first appears; on later turns it
//! becomes resent context) — `cache_read` is the cumulative context
//! (context_usage_percentage/100 * context_window) minus that fresh input
//! clamped at zero, and `output` is estimated from response_size / 4.

use super::utils::{
    back_anchor_timestamp, estimate_tokens, file_modified_timestamp_ms, for_each_json_line,
    session_id_from_path, sqlite_for_each_row,
};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CLIENT_ID: &str = "kiro";
const PROVIDER_ID: &str = "amazon-bedrock";
const UNKNOWN_MODEL: &str = "auto";

/// USD value of one Kiro credit. Kiro's pay-as-you-go overage rate published on
/// [kiro.dev](https://kiro.dev) is 1 credit = $0.04. Conversations whose
/// `user_turn_metadata.usage_info` carries a credit total use this to derive a
/// provider-reported cost, bypassing the token-based pricing estimate.
/// Content was rephrased for compliance with licensing restrictions.
const CREDIT_TO_USD: f64 = 0.04;

/// Fallback context window (tokens) used by the hybrid estimation when a Kiro
/// source omits an explicit `context_window_tokens`. Kiro's Auto agent runs on
/// a 200K-token window.
const DEFAULT_CONTEXT_WINDOW: i64 = 200_000;

#[derive(Debug, Deserialize)]
struct KiroSessionHeader {
    session_id: Option<String>,
    cwd: Option<String>,
    session_state: Option<KiroSessionState>,
}

#[derive(Debug, Deserialize)]
struct KiroSessionState {
    rts_model_state: Option<KiroRtsModelState>,
    conversation_metadata: Option<KiroConversationMetadata>,
}

#[derive(Debug, Deserialize)]
struct KiroRtsModelState {
    model_info: Option<KiroModelInfo>,
}

#[derive(Debug, Deserialize)]
struct KiroModelInfo {
    model_id: Option<String>,
    context_window_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct KiroConversationMetadata {
    user_turn_metadatas: Option<Vec<KiroTurnMetadata>>,
}

#[derive(Debug, Deserialize)]
struct KiroTurnMetadata {
    input_token_count: Option<i64>,
    output_token_count: Option<i64>,
    end_timestamp: Option<serde_json::Value>,
    total_request_count: Option<i32>,
    message_ids: Option<Vec<Option<String>>>,
    context_usage_percentage: Option<f64>,
    // Richer per-turn fields the CLI (`user_turn_metadatas`) path persists.
    // These were previously dropped by serde because the struct did not
    // deserialize them (the same incomplete-struct defect the SQLite fix
    // addressed). `assistant_response_length` / `user_prompt_length` are byte
    // lengths driving the hybrid output/fresh-input estimates; the
    // `*_token_count` fields carry real counts (0 for the Auto agent);
    // `metering_usage` carries provider-reported credits.
    assistant_response_length: Option<i64>,
    user_prompt_length: Option<i64>,
    cache_read_input_token_count: Option<i64>,
    cache_write_input_token_count: Option<i64>,
    // Deserialized to complete the on-disk turn shape, but not read by the
    // mapping: `context_usage_percentage` (cumulative) already drives
    // `cache_read`, and request-count comes from `total_request_count`.
    #[allow(dead_code)]
    final_context_usage_percentage: Option<f64>,
    #[allow(dead_code)]
    request_attempts: Option<i32>,
    metering_usage: Option<Vec<KiroMeteringEntry>>,
}

/// A single `metering_usage` entry from a CLI `user_turn_metadatas` turn,
/// shaped like `{ "value": 0.0313, "unit": "credit" }`. The credit-unit values
/// drive the provider-reported cost via `CREDIT_TO_USD`.
#[derive(Debug, Deserialize)]
struct KiroMeteringEntry {
    value: Option<f64>,
    unit: Option<String>,
}

impl KiroTurnMetadata {
    /// Sum of `value` across `metering_usage` entries whose `unit == "credit"`.
    /// Returns `0.0` when the array is absent or carries no credit entries.
    fn credit_sum(&self) -> f64 {
        self.metering_usage
            .as_ref()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e.unit.as_deref() == Some("credit"))
                    .filter_map(|e| e.value)
                    .sum()
            })
            .unwrap_or(0.0)
    }
}

#[derive(Debug, Deserialize)]
struct KiroJsonlEntry {
    kind: String,
    data: Option<KiroJsonlData>,
}

#[derive(Debug, Deserialize)]
struct KiroJsonlData {
    message_id: Option<String>,
    content: Option<Vec<KiroContentPart>>,
    meta: Option<KiroEntryMeta>,
}

#[derive(Debug, Deserialize)]
struct KiroContentPart {
    kind: Option<String>,
    data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KiroEntryMeta {
    timestamp: Option<f64>,
}

#[derive(Debug, Clone, Default)]
struct KiroMessageContent {
    prompt_chars: usize,
    assistant_chars: usize,
    // Tool-execution result text fed back to the model this turn. A
    // tool_result is fresh input on the turn it first appears (the model reads
    // it for the first time then); it is added to `input` alongside
    // `prompt_chars`. On later turns it becomes resent context absorbed into
    // `cache_read` via the cumulative context percentage. Populated from
    // `ToolResults` sibling-jsonl entries, attributed order-based to the
    // following `AssistantMessage` (see the parse loop).
    tool_result_chars: usize,
    prompt_timestamp_ms: Option<i64>,
}

/// Metadata half of the Kiro IDE session layout (`session.json`, schemaVersion
/// 1.0.0). The conversation itself lives in the sibling `messages.jsonl`.
#[derive(Debug, Deserialize)]
struct KiroIdeSession {
    id: Option<String>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
    #[serde(rename = "workspacePaths")]
    workspace_paths: Option<Vec<String>>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
    #[serde(rename = "lastModifiedAt")]
    last_modified_at: Option<String>,
}

pub fn parse_kiro_file(path: &Path) -> Vec<UnifiedMessage> {
    if is_kiro_ide_session_path(path) {
        return parse_kiro_ide_session_file(path);
    }

    if is_kiro_global_storage_path(path) || is_kiro_chat_path(path) {
        return parse_kiro_global_storage_file(path);
    }

    let fallback_timestamp = file_modified_timestamp_ms(path);

    let mut json_bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };

    let header = match simd_json::from_slice::<KiroSessionHeader>(&mut json_bytes) {
        Ok(header) => header,
        Err(_) => return Vec::new(),
    };

    let session_id = header
        .session_id
        .unwrap_or_else(|| session_id_from_path(path));
    let model_id = header
        .session_state
        .as_ref()
        .and_then(|state| state.rts_model_state.as_ref())
        .and_then(|state| state.model_info.as_ref())
        .and_then(|info| info.model_id.as_deref())
        .filter(|model| !model.trim().is_empty())
        .unwrap_or(UNKNOWN_MODEL)
        .to_string();
    let workspace_key = header.cwd.as_deref().and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
    let context_window = header
        .session_state
        .as_ref()
        .and_then(|state| state.rts_model_state.as_ref())
        .and_then(|state| state.model_info.as_ref())
        .and_then(|info| info.context_window_tokens)
        .unwrap_or(0);
    let turns = header
        .session_state
        .and_then(|state| state.conversation_metadata)
        .and_then(|metadata| metadata.user_turn_metadatas)
        .unwrap_or_default();

    let Some(jsonl_path) = kiro_related_messages_path(path) else {
        return Vec::new();
    };
    let mut content_by_message_id: HashMap<String, KiroMessageContent> = HashMap::new();

    let mut pending_prompt: Option<(usize, Option<i64>)> = None;
    // Tool-result bytes seen since the last `AssistantMessage`, flushed into
    // the NEXT assistant turn (order-based attribution, mirroring
    // `pending_prompt`). The `message_id` on a `ToolResults` entry does not
    // reliably match the header turn's `message_ids`, so we deliberately do
    // NOT key on it here.
    let mut pending_tool_result_chars: usize = 0;

    for_each_json_line(&jsonl_path, &mut |_index, trimmed| {
        // `ToolResults` entries carry a distinct content shape (an array of
        // `toolResult` parts, each wrapping its own `content` array of
        // text/json payloads) that `KiroJsonlData`'s flat `content`
        // (`data: Option<String>`) does not model — the typed
        // `KiroJsonlEntry` deserialization would FAIL and drop the line. So
        // detect the kind from the raw JSON first and walk the raw value for
        // `ToolResults`, only falling through to the typed parse for the other
        // kinds.
        let raw = match serde_json::from_str::<Value>(trimmed) {
            Ok(value) => value,
            Err(_) => return,
        };

        if raw.get("kind").and_then(Value::as_str) == Some("ToolResults") {
            pending_tool_result_chars +=
                tool_results_text_len(raw.get("data").and_then(|d| d.get("content")));
            return;
        }

        let mut bytes = trimmed.as_bytes().to_vec();
        let entry = match simd_json::from_slice::<KiroJsonlEntry>(&mut bytes) {
            Ok(entry) => entry,
            Err(_) => return,
        };

        let Some(data) = entry.data else {
            return;
        };
        let Some(message_id) = data.message_id else {
            return;
        };

        let text_chars = text_char_count(data.content.as_deref());

        match entry.kind.as_str() {
            "Prompt" => {
                let timestamp_ms = data
                    .meta
                    .and_then(|meta| meta.timestamp)
                    .map(seconds_to_millis);
                pending_prompt = Some((text_chars, timestamp_ms));
            }
            "AssistantMessage" => {
                let message = content_by_message_id.entry(message_id).or_default();
                if let Some((prompt_chars, prompt_ts)) = pending_prompt.take() {
                    message.prompt_chars += prompt_chars;
                    if message.prompt_timestamp_ms.is_none() {
                        message.prompt_timestamp_ms = prompt_ts;
                    }
                }
                message.assistant_chars += text_chars;
                // Flush tool-result bytes accumulated since the previous
                // assistant turn into this one (order-based, like the prompt).
                message.tool_result_chars += pending_tool_result_chars;
                pending_tool_result_chars = 0;
            }
            _ => {}
        }
    });

    // The CLI context window falls back to 200_000 when the header omits it,
    // mirroring the shared hybrid model (design "Amended Design").
    let context_window = if context_window > 0 {
        context_window
    } else {
        DEFAULT_CONTEXT_WINDOW
    };

    // Sum the credit-unit metering entries across every turn: the conversation
    // credit is provider-reported and gets assigned to the first turn actually
    // emitted (skipped zero-token turns are not "emitted").
    let credit_sum: f64 = turns.iter().map(KiroTurnMetadata::credit_sum).sum();

    let mut messages: Vec<UnifiedMessage> = turns
        .into_iter()
        .enumerate()
        .filter_map(|(index, turn)| {
            let message_ids = turn.message_ids.unwrap_or_default();
            let mut prompt_chars = 0;
            let mut assistant_chars = 0;
            let mut tool_result_chars = 0usize;
            let mut prompt_timestamp_ms = None;

            for message_id in message_ids.iter().flatten() {
                let Some(content) = content_by_message_id.get(message_id) else {
                    continue;
                };
                prompt_chars += content.prompt_chars;
                assistant_chars += content.assistant_chars;
                tool_result_chars += content.tool_result_chars;
                if prompt_timestamp_ms.is_none() {
                    prompt_timestamp_ms = content.prompt_timestamp_ms;
                }
            }

            // Shared hybrid model (design "Amended Design — Unify All Three
            // Kiro Parse Paths"):
            //
            // 1. Real per-turn counts win when present and > 0.
            // 2. Otherwise fresh `input` is the turn's OWN new content: the
            //    prompt bytes PLUS this turn's tool_result bytes (a tool_result
            //    is fresh input on the turn it first appears; on later turns it
            //    becomes resent context absorbed into `cache_read`). The
            //    cumulative `context_usage_percentage` becomes `cache_read`,
            //    not `input`, via `max(total_context - fresh_input, 0)`.
            //
            // NOTE: when real counts are absent (the common case — the Auto
            // agent persists zeros), these values are ESTIMATED, not measured.
            //
            // CLI tool_result bytes: the sibling `.jsonl` emits a `ToolResults`
            // entry for each tool-execution result fed back to the model. Its
            // `message_id` does not reliably match the header turn's
            // `message_ids`, so — exactly like the `Prompt` text — those bytes
            // are attributed order-based to the FOLLOWING `AssistantMessage`
            // (accumulated into `KiroMessageContent::tool_result_chars` during
            // the parse loop). Summing them here per matched message_id gives
            // this turn's fresh tool input, matching the SQLite path
            // (`KiroDbUserContent::ToolUseResults`) and the IDE path
            // (`tool_result` payload lines).
            let real_input = turn.input_token_count.unwrap_or(0).max(0);
            let real_output = turn.output_token_count.unwrap_or(0).max(0);
            let real_cache_read = turn.cache_read_input_token_count.unwrap_or(0).max(0);
            let real_cache_write = turn.cache_write_input_token_count.unwrap_or(0).max(0);

            // Fresh input: the turn's own new prompt bytes plus this turn's
            // tool_result bytes (extracted from `ToolResults` jsonl entries and
            // attributed order-based to this assistant turn above).
            let fresh_input = {
                let prompt_bytes = turn.user_prompt_length.unwrap_or(0).max(0);
                let prompt_part = if prompt_bytes > 0 {
                    prompt_bytes as usize
                } else {
                    prompt_chars
                };
                estimate_tokens(prompt_part + tool_result_chars)
            };

            let input = if real_input > 0 {
                real_input
            } else {
                fresh_input
            };

            let cache_read = if real_cache_read > 0 {
                real_cache_read
            } else {
                // Cumulative context resent this turn, minus the fresh input.
                let ctx_pct = turn.context_usage_percentage.unwrap_or(0.0);
                let total_context = if ctx_pct > 0.0 {
                    ((context_window as f64) * ctx_pct / 100.0).floor() as i64
                } else {
                    0
                };
                (total_context - input).max(0)
            };

            let cache_write = real_cache_write;

            let output = if real_output > 0 {
                real_output
            } else {
                let response_bytes = turn.assistant_response_length.unwrap_or(0).max(0);
                if response_bytes > 0 {
                    estimate_tokens(response_bytes as usize)
                } else {
                    estimate_tokens(assistant_chars)
                }
            };

            if input + output == 0 {
                return None;
            }

            let end_timestamp_ms = parse_timestamp_value(turn.end_timestamp.as_ref());
            let duration_ms = duration_between_ms(prompt_timestamp_ms, end_timestamp_ms);
            let timestamp = prompt_timestamp_ms
                .or(end_timestamp_ms)
                .unwrap_or(fallback_timestamp);

            let mut message = UnifiedMessage::new_with_dedup(
                CLIENT_ID,
                model_id.clone(),
                PROVIDER_ID,
                session_id.clone(),
                timestamp,
                TokenBreakdown {
                    input,
                    output,
                    cache_read,
                    cache_write,
                    reasoning: 0,
                },
                0.0,
                Some(format!("{}:{}", session_id, index)),
            );
            message.message_count = turn.total_request_count.unwrap_or(1).max(1);
            message.duration_ms = duration_ms;
            message.is_turn_start = true;
            message.set_workspace(workspace_key.clone(), workspace_label.clone());
            Some(message)
        })
        .collect();

    // Credit-based provider-reported cost (design Property 6): when the
    // conversation's `metering_usage` credit sum is > 0, assign
    // `credit_sum * CREDIT_TO_USD` to the FIRST emitted turn and mark it
    // provider-reported so the pricing dispatch does not override it. When no
    // credit is present, turns stay at cost 0.0 / CostSource::Unknown.
    if credit_sum > 0.0 {
        if let Some(message) = messages.first_mut() {
            message.cost = credit_sum * CREDIT_TO_USD;
            message.mark_provider_reported_cost();
        }
    }

    messages
}

/// Byte length of the tool-execution result payloads in a `ToolResults`
/// entry's `data.content` array. Walks each `toolResult` part's inner
/// `data.content[]` payloads: a string payload contributes its byte length; a
/// structured JSON payload contributes the length of its compact serialization
/// (`serde_json::to_string`) so a non-empty structured result never counts as
/// zero. Unknown shapes contribute 0 and never panic.
fn tool_results_text_len(content: Option<&Value>) -> usize {
    let Some(Value::Array(parts)) = content else {
        return 0;
    };
    parts
        .iter()
        .filter(|part| part.get("kind").and_then(Value::as_str) == Some("toolResult"))
        .filter_map(|part| part.get("data").and_then(|d| d.get("content")))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|inner| inner.get("data"))
        .map(json_value_text_len)
        .sum()
}

/// Byte length of a tool-result payload `data` value: strings count their own
/// bytes; other JSON values count their compact serialization. Absent/null
/// serialization contributes 0.
fn json_value_text_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        other => serde_json::to_string(other).map(|s| s.len()).unwrap_or(0),
    }
}

fn text_char_count(content: Option<&[KiroContentPart]>) -> usize {
    content
        .unwrap_or_default()
        .iter()
        .filter(|part| part.kind.as_deref().is_none_or(|kind| kind == "text"))
        .filter_map(|part| part.data.as_deref())
        .map(str::chars)
        .map(Iterator::count)
        .sum()
}

fn seconds_to_millis(seconds: f64) -> i64 {
    // Scale fractional seconds to milliseconds (preserving sub-second
    // precision), then clamp into i64 range. The `f64 as i64` cast saturates
    // rather than wrapping on out-of-range/garbage timestamps, so the
    // seconds->ms conversion cannot overflow.
    let millis = seconds * 1000.0;
    if millis.is_nan() {
        0
    } else {
        millis.clamp(i64::MIN as f64, i64::MAX as f64) as i64
    }
}

fn duration_between_ms(start_ms: Option<i64>, end_ms: Option<i64>) -> Option<i64> {
    let duration = end_ms?.saturating_sub(start_ms?);
    (duration > 0).then_some(duration)
}

fn parse_timestamp_value(value: Option<&serde_json::Value>) -> Option<i64> {
    match value? {
        serde_json::Value::Number(number) => number.as_f64().map(|timestamp| {
            if timestamp.abs() < 1_000_000_000_000.0 {
                seconds_to_millis(timestamp)
            } else {
                timestamp as i64
            }
        }),
        serde_json::Value::String(timestamp) => chrono::DateTime::parse_from_rfc3339(timestamp)
            .ok()
            .map(|dt| dt.timestamp_millis())
            .or_else(|| timestamp.parse::<f64>().ok().map(seconds_to_millis)),
        _ => None,
    }
}

fn is_kiro_global_storage_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains("globalStorage") && path_str.contains("kiro.kiroagent")
}

fn is_kiro_chat_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("chat"))
}

/// Return the conversation sidecar consumed by `parse_kiro_file`, if this
/// Kiro source format has one. IDE sessions use `messages.jsonl`; CLI session
/// headers use the same stem with a `.jsonl` extension. Global-storage and
/// `.chat` snapshots are self-contained.
pub(crate) fn kiro_related_messages_path(path: &Path) -> Option<PathBuf> {
    if is_kiro_ide_session_path(path) {
        return Some(path.with_file_name("messages.jsonl"));
    }
    if is_kiro_global_storage_path(path) || is_kiro_chat_path(path) {
        return None;
    }
    Some(path.with_extension("jsonl"))
}

/// A Kiro IDE session file is `session.json` sitting inside a `sess_<uuid>`
/// directory (`~/.kiro/sessions/<workspace>/sess_<uuid>/session.json`). The
/// `sess_` parent requirement keeps this from matching the CLI layout, whose
/// arbitrary `~/.kiro/sessions/cli/*.json` files share the same tree.
pub(crate) fn is_kiro_ide_session_path(path: &Path) -> bool {
    let is_session_json = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == "session.json")
        .unwrap_or(false);
    if !is_session_json {
        return false;
    }
    path.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with("sess_"))
        .unwrap_or(false)
}

/// Parse the Kiro IDE session layout: `session.json` (metadata) plus the
/// sibling `messages.jsonl` (conversation).
///
/// The IDE does NOT record per-turn token usage in these files (confirmed
/// against issue #813's sample, which carries only session metadata), so — like
/// every other Kiro path — token counts here are ESTIMATED from message text
/// (chars / 4), never measured. `messages.jsonl`'s exact schema is not
/// documented, so each line is parsed as generic JSON and fed through the
/// role-tolerant snapshot text collector (user/assistant/human/bot/prompt/
/// response). Lines with no role-tagged text contribute nothing rather than
/// being guessed at, so a session with no recognizable content is dropped
/// instead of fabricating usage.
fn parse_kiro_ide_session_file(path: &Path) -> Vec<UnifiedMessage> {
    let fallback_timestamp = file_modified_timestamp_ms(path);

    let session_json = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return Vec::new(),
    };
    let session: KiroIdeSession = match serde_json::from_str(&session_json) {
        Ok(session) => session,
        Err(_) => return Vec::new(),
    };

    let sess_dir = path.parent();
    let sess_dir_name = sess_dir
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str());
    let session_id = session
        .id
        .filter(|id| !id.trim().is_empty())
        .or_else(|| sess_dir_name.map(|name| name.to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    let session_model_id = session.model_id.filter(|m| !m.trim().is_empty());

    let workspace_path = session
        .workspace_paths
        .as_ref()
        .and_then(|paths| paths.first())
        .map(|s| s.as_str());
    let workspace_from_dir = sess_dir
        .and_then(|dir| dir.parent())
        .and_then(|ws_dir| ws_dir.file_name())
        .and_then(|name| name.to_str())
        .map(|name| name.to_string());
    let ws_str = workspace_path.map(|s| s.to_string()).or(workspace_from_dir);
    let workspace_key = ws_str.as_deref().and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

    let messages_path = path.with_file_name("messages.jsonl");
    if !messages_path.is_file() {
        return Vec::new();
    }

    // `DEFAULT_CONTEXT_WINDOW` is now a module-level const (see top of file);
    // upstream lifted it out of this function, so no local re-declaration.
    //
    // Upstream also switched this IDE path to `for_each_json_line(&messages_path, ..)`
    // below, which opens the file internally and returns silently on I/O error.
    // That subsumes the manual `File::open`/`BufReader` guard we previously added
    // here (the `if !messages_path.is_file()` check above plus the shared driver's
    // own open-failure early-return already cover the unreadable-file case), so no
    // separate reader is created.

    #[derive(Default)]
    struct IdeTurn {
        prompt_chars: usize,
        assistant_chars: usize,
        // Tool-execution result text fed back to the model this turn. A
        // tool_result is fresh input on the turn it first appears (the model
        // reads it for the first time then); it is added to `input` alongside
        // `prompt_chars`. On later turns it becomes resent context absorbed
        // into `cache_read` via the cumulative context percentage.
        tool_result_chars: usize,
        prompt_timestamp_ms: Option<i64>,
        end_timestamp_ms: Option<i64>,
        context_usage_percentage: f64,
        elapsed_ms: Option<i64>,
        request_count: i32,
    }

    let mut turns: Vec<IdeTurn> = Vec::new();
    let mut current_turn: Option<IdeTurn> = None;
    let mut has_structured_format = false;

    // Fallback accumulators for flat-JSON messages.jsonl (no payload wrapper)
    let mut flat_counts = KiroSnapshotTextCounts::default();
    let mut flat_model_id: Option<String> = None;
    let mut flat_assistant_turns: i32 = 0;

    for_each_json_line(&messages_path, &mut |_index, trimmed| {
        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => return,
        };

        // Structured format: each line has `payload.type`
        if let Some(payload) = entry.get("payload") {
            if let Some(msg_type) = payload.get("type").and_then(|v| v.as_str()) {
                has_structured_format = true;

                let timestamp_ms = entry
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.timestamp_millis());

                match msg_type {
                    "user" => {
                        let chars = payload
                            .get("content")
                            .and_then(|v| v.as_str())
                            .map(|s| s.chars().count())
                            .unwrap_or(0);
                        let turn = current_turn.get_or_insert_with(IdeTurn::default);
                        turn.prompt_chars += chars;
                        if turn.prompt_timestamp_ms.is_none() {
                            turn.prompt_timestamp_ms = timestamp_ms;
                        }
                    }
                    "assistant" => {
                        let chars = payload
                            .get("content")
                            .and_then(|v| v.as_str())
                            .map(|s| s.chars().count())
                            .unwrap_or(0);
                        if let Some(turn) = current_turn.as_mut() {
                            turn.assistant_chars += chars;
                        }
                    }
                    "tool_call" => {
                        // Tool-call args are resent context, not fresh
                        // assistant output — they must NOT inflate
                        // `assistant_chars`. Deliberately contribute no
                        // characters here.
                    }
                    "tool_result" => {
                        // Tool-execution results fed back to the model. These
                        // ARE fresh input on the turn they first appear (the
                        // model reads them for the first time then), so they
                        // accumulate into `tool_result_chars` and are added to
                        // `input`. On later turns they fold into `cache_read`
                        // via the cumulative context.
                        let chars = payload
                            .get("content")
                            .and_then(|v| v.as_str())
                            .map(|s| s.chars().count())
                            .unwrap_or(0);
                        let turn = current_turn.get_or_insert_with(IdeTurn::default);
                        turn.tool_result_chars += chars;
                    }
                    "session_metadata"
                        if payload.get("key").and_then(|v| v.as_str()) == Some("contextUsage") =>
                    {
                        if let Some(pct) = payload
                            .get("value")
                            .and_then(|v| v.get("usagePercentage"))
                            .and_then(|v| v.as_f64())
                        {
                            if let Some(turn) = current_turn.as_mut() {
                                turn.context_usage_percentage = pct;
                            }
                        }
                    }
                    "usage_summary" => {
                        if let Some(elapsed) = payload.get("elapsedTime").and_then(|v| v.as_i64()) {
                            if let Some(turn) = current_turn.as_mut() {
                                turn.elapsed_ms = Some(elapsed);
                            }
                        }
                        // `requestIds.len()` is the per-turn request-count
                        // signal (maps to message_count). It carries no
                        // tokens and no credit.
                        if let Some(request_count) = payload
                            .get("requestIds")
                            .and_then(|v| v.as_array())
                            .map(|a| a.len())
                        {
                            if let Some(turn) = current_turn.as_mut() {
                                turn.request_count = request_count as i32;
                            }
                        }
                    }
                    "turn_end" => {
                        if let Some(turn) = current_turn.as_mut() {
                            turn.end_timestamp_ms = timestamp_ms;
                        }
                        if let Some(turn) = current_turn.take() {
                            if turn.prompt_chars > 0
                                || turn.assistant_chars > 0
                                || turn.tool_result_chars > 0
                            {
                                turns.push(turn);
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }
        }

        // Flat format fallback: lines like {"role":"user","content":"..."}
        if flat_model_id.is_none() {
            flat_model_id = find_kiro_snapshot_model_id(&entry);
        }
        let assistant_before = flat_counts.assistant_chars;
        collect_kiro_snapshot_text(&entry, &mut flat_counts, None);
        if flat_counts.assistant_chars > assistant_before {
            flat_assistant_turns += 1;
        }
    });

    // Flush any in-flight structured turn
    if let Some(turn) = current_turn.take() {
        if turn.prompt_chars > 0 || turn.assistant_chars > 0 || turn.tool_result_chars > 0 {
            turns.push(turn);
        }
    }

    if has_structured_format && !turns.is_empty() {
        // Per-turn structured output
        let model_id = session_model_id.unwrap_or_else(|| UNKNOWN_MODEL.to_string());
        return turns
            .into_iter()
            .enumerate()
            .filter_map(|(index, turn)| {
                // Property 7 hybrid model: `input` is the turn's OWN new
                // content — the user prompt bytes PLUS this turn's tool_result
                // bytes (a tool_result is fresh input on the turn it first
                // appears; on later turns it folds into `cache_read`). The
                // cumulative `contextUsage.usagePercentage` becomes
                // `cache_read` via `max(total_context - input, 0)`, and
                // `output` is assistant text only.
                let input = estimate_tokens(turn.prompt_chars + turn.tool_result_chars);
                let total_context = if turn.context_usage_percentage > 0.0 {
                    ((DEFAULT_CONTEXT_WINDOW as f64) * turn.context_usage_percentage / 100.0)
                        .floor() as i64
                } else {
                    0
                };
                let cache_read = (total_context - input).max(0);
                let output = estimate_tokens(turn.assistant_chars);

                if input + output == 0 {
                    return None;
                }

                let duration_ms = turn.elapsed_ms.or_else(|| {
                    duration_between_ms(turn.prompt_timestamp_ms, turn.end_timestamp_ms)
                });
                // Prefer the user prompt's own timestamp. When it's absent or
                // unparseable (e.g. `usage_summary.elapsedTime` supplied
                // `duration_ms` but the prompt timestamp couldn't be
                // resolved), back-calculate the start anchor from
                // `turn_end - elapsed` instead of falling through to
                // `end_timestamp_ms` directly — otherwise sessionize()'s
                // `[timestamp, timestamp + duration_ms]` span would project
                // forward past the turn's actual end into phantom idle time.
                // The back-calculation is guarded against a non-positive
                // result (which sessionize() silently drops) by falling back
                // to the unadjusted `end_timestamp_ms`.
                let timestamp = turn
                    .prompt_timestamp_ms
                    .or_else(|| match (turn.end_timestamp_ms, duration_ms) {
                        (Some(end), Some(elapsed)) => Some(back_anchor_timestamp(end, elapsed)),
                        _ => None,
                    })
                    .or(turn.end_timestamp_ms)
                    .unwrap_or(fallback_timestamp);

                let mut message = UnifiedMessage::new_with_dedup(
                    CLIENT_ID,
                    model_id.clone(),
                    PROVIDER_ID,
                    session_id.clone(),
                    timestamp,
                    TokenBreakdown {
                        input,
                        output,
                        cache_read,
                        cache_write: 0,
                        reasoning: 0,
                    },
                    0.0,
                    Some(format!("{}:ide:{}", session_id, index)),
                );
                message.message_count = turn.request_count.max(1);
                message.is_turn_start = true;
                message.duration_ms = duration_ms;
                message.set_workspace(workspace_key.clone(), workspace_label.clone());
                Some(message)
            })
            .collect();
    }

    // Flat format fallback: single aggregated message (original behavior)
    let input = estimate_tokens(flat_counts.prompt_chars);
    let output = estimate_tokens(flat_counts.assistant_chars);
    if input + output == 0 {
        return Vec::new();
    }

    let created_value = session
        .created_at
        .as_deref()
        .map(|s| Value::String(s.to_string()));
    let created_ms = parse_timestamp_value(created_value.as_ref());
    let modified_value = session
        .last_modified_at
        .as_deref()
        .map(|s| Value::String(s.to_string()));
    let modified_ms = parse_timestamp_value(modified_value.as_ref());

    let timestamp = created_ms.or(modified_ms).unwrap_or(fallback_timestamp);
    let duration_ms = duration_between_ms(created_ms, modified_ms);
    let model_id = session_model_id
        .or(flat_model_id)
        .filter(|model| !model.trim().is_empty())
        .unwrap_or_else(|| UNKNOWN_MODEL.to_string());

    let mut message = UnifiedMessage::new_with_dedup(
        CLIENT_ID,
        model_id,
        PROVIDER_ID,
        session_id.clone(),
        timestamp,
        TokenBreakdown {
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        },
        0.0,
        Some(format!("{}:ide-session", session_id)),
    );
    message.message_count = flat_assistant_turns.max(1);
    message.duration_ms = duration_ms;
    message.is_turn_start = true;
    message.set_workspace(workspace_key, workspace_label);
    vec![message]
}

/// Extract the workspace folder name from a Kiro globalStorage path.
///
/// Snapshots live under `.../globalStorage/kiro.kiroagent/<workspace>/...`,
/// so the workspace folder is the path segment immediately following the
/// `kiro.kiroagent` component. Returns `None` when no such segment exists.
fn kiro_global_storage_workspace(path: &Path) -> Option<String> {
    let mut components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned());
    while let Some(component) = components.next() {
        if component == "kiro.kiroagent" {
            return components.next();
        }
    }
    None
}

#[derive(Debug, Default)]
struct KiroSnapshotTextCounts {
    prompt_chars: usize,
    assistant_chars: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KiroSnapshotRole {
    Prompt,
    Assistant,
}

fn collect_kiro_snapshot_text(
    value: &Value,
    counts: &mut KiroSnapshotTextCounts,
    mut role: Option<KiroSnapshotRole>,
) {
    match value {
        Value::Object(map) => {
            // Real IDE-private `.chat` files use "human"/"bot" (with "tool" for
            // injected context, deliberately left unmatched); other snapshot
            // shapes use "user"/"assistant" or "prompt"/"response".
            if let Some(kind) = map.get("role").and_then(|v| v.as_str()) {
                role = match kind {
                    "user" | "prompt" | "human" => Some(KiroSnapshotRole::Prompt),
                    "assistant" | "response" | "bot" => Some(KiroSnapshotRole::Assistant),
                    _ => role,
                };
            }
            if let Some(kind) = map.get("type").and_then(|v| v.as_str()) {
                role = match kind {
                    "user" | "prompt" | "human" => Some(KiroSnapshotRole::Prompt),
                    "assistant" | "response" | "bot" => Some(KiroSnapshotRole::Assistant),
                    _ => role,
                };
            }

            // Each group below is an ordered list of *aliases* for the same
            // logical payload (text body, conversation list, sub-parts). Kiro
            // snapshots frequently store the identical text under more than one
            // alias in a single object (e.g. both `content` and `text`, or both
            // `messages` and `entries`). Descending into every present alias
            // would count that text once per alias and inflate token totals.
            //
            // However, an object may also legitimately hold *distinct* payloads
            // under several keys of the same group (e.g. a turn with both
            // `prompt` and `response`, or a chat with both `messages` and
            // `history` pointing at different subtrees). Visiting only the first
            // present key would silently drop those, undercounting tokens.
            //
            // So we descend into every present key in the group but de-duplicate
            // by VALUE: subtrees structurally equal to one already visited in the
            // same group are skipped. Distinct subtrees are all counted; repeated
            // (aliased) subtrees are counted once.
            for group in [
                // Inline text body of a single message.
                &["prompt", "response", "content", "text", "message"][..],
                // Container holding a list of messages/turns.
                &[
                    "messages",
                    "conversation",
                    "chat",
                    "transcript",
                    "entries",
                    "events",
                    "history",
                ][..],
                // Sub-parts of a single message.
                &["parts", "items", "nodes"][..],
            ] {
                let mut visited: Vec<&Value> = Vec::new();
                for key in group {
                    if let Some(item) = map.get(*key) {
                        if visited.contains(&item) {
                            continue;
                        }
                        visited.push(item);
                        collect_kiro_snapshot_text(item, counts, role);
                    }
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_kiro_snapshot_text(item, counts, role);
            }
        }
        Value::String(text) => match role {
            Some(KiroSnapshotRole::Assistant) => counts.assistant_chars += text.chars().count(),
            Some(KiroSnapshotRole::Prompt) => counts.prompt_chars += text.chars().count(),
            None => {}
        },
        _ => {}
    }
}

fn find_kiro_snapshot_model_id(value: &Value) -> Option<String> {
    static KIRO_INTERNAL_MODELS: &[&str] = &["agent", "auto", "qdev"];

    match value {
        Value::Object(map) => {
            for key in ["model_id", "modelId", "model"] {
                if let Some(model) = map.get(key).and_then(|v| v.as_str()) {
                    let model = model.trim();
                    if !model.is_empty()
                        && !KIRO_INTERNAL_MODELS.contains(&model.to_lowercase().as_str())
                    {
                        return Some(model.to_string());
                    }
                }
            }

            for key in [
                "messages",
                "conversation",
                "chat",
                "transcript",
                "entries",
                "events",
                "history",
                "prompt",
                "response",
                "content",
                "text",
                "message",
                "parts",
                "items",
                "nodes",
                "promptLogs",
                "completionOptions",
            ] {
                if let Some(item) = map.get(key) {
                    if let Some(model) = find_kiro_snapshot_model_id(item) {
                        return Some(model);
                    }
                }
            }

            None
        }
        Value::Array(items) => items.iter().find_map(find_kiro_snapshot_model_id),
        _ => None,
    }
}

fn parse_kiro_global_storage_file(path: &Path) -> Vec<UnifiedMessage> {
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let json = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => return Vec::new(),
    };

    let value: Value = match serde_json::from_str(&json) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    if let Some(messages) = try_parse_kiro_execution_file(&value, path) {
        return messages;
    }

    if value.get("executions").is_some() && value.get("version").is_some() {
        return Vec::new();
    }

    if let Some(messages) = try_parse_kiro_workspace_session(&value, path, fallback_timestamp) {
        return messages;
    }

    let file_stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    let workspace = kiro_global_storage_workspace(path);
    let workspace_key = workspace.as_deref().and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
    let session_id = match workspace.as_deref() {
        Some(ws) => format!("{}/{}", ws, file_stem),
        None => file_stem.to_string(),
    };
    let model_id = find_kiro_snapshot_model_id(&value).unwrap_or_else(|| "auto".to_string());

    let mut counts = KiroSnapshotTextCounts::default();
    collect_kiro_snapshot_text(&value, &mut counts, None);

    let input = estimate_tokens(counts.prompt_chars);
    let output = estimate_tokens(counts.assistant_chars);
    if input + output == 0 {
        return Vec::new();
    }

    let snapshot_timestamp = fallback_timestamp;

    // IDE-private `.chat` files carry a top-level executionId referencing the
    // execution record stored under the sibling execution-store directory
    // (verified against real globalStorage trees: the same UUID appears as the
    // `.chat`'s executionId and the execution file's executionId). Tag the
    // dedup key with it so suppress_snapshots_covered_by_executions can drop
    // this snapshot when its execution is counted. `try_parse_kiro_execution_file`
    // already returned above for files that have `actions`, so this only tags
    // action-less chat/validation artifacts.
    let dedup_key = match value.get("executionId").and_then(|id| id.as_str()) {
        Some(execution_id) => format!("{}:globalstorage:exec:{}", session_id, execution_id),
        None => format!("{}:globalstorage", session_id),
    };

    let mut message = UnifiedMessage::new_with_dedup(
        CLIENT_ID,
        model_id,
        PROVIDER_ID,
        session_id.clone(),
        snapshot_timestamp,
        TokenBreakdown {
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        },
        0.0,
        Some(dedup_key),
    );
    message.message_count = 1;
    message.is_turn_start = true;
    message.set_workspace(workspace_key, workspace_label);
    vec![message]
}

fn try_parse_kiro_execution_file(value: &Value, path: &Path) -> Option<Vec<UnifiedMessage>> {
    let obj = value.as_object()?;
    let execution_id = obj.get("executionId")?.as_str()?;
    let actions = obj.get("actions")?.as_array()?;
    let status = obj.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status != "succeed" {
        return Some(Vec::new());
    }

    let session_id = obj
        .get("chatSessionId")
        .and_then(|v| v.as_str())
        .unwrap_or(execution_id)
        .to_string();
    // Reuse the shared timestamp parser so epoch-seconds, epoch-millis, RFC3339
    // strings, and float values are all bucketed to the correct day (raw
    // `as_i64` silently mis-buckets everything except integer milliseconds).
    let start_time = parse_timestamp_value(obj.get("startTime"));
    let timestamp = start_time.unwrap_or_else(|| file_modified_timestamp_ms(path));
    let end_time = parse_timestamp_value(obj.get("endTime"));
    let duration_ms = duration_between_ms(start_time.or(Some(timestamp)), end_time);

    let mut output_chars = 0usize;
    for action in actions {
        let action_type = action
            .get("actionType")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !matches!(action_type, "say" | "reasoning") {
            continue;
        }
        let msg = action
            .get("output")
            .and_then(|o| {
                if let Some(s) = o.as_str() {
                    Some(s.to_string())
                } else {
                    o.get("message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                }
            })
            .unwrap_or_default();
        output_chars += msg.chars().count();
    }

    let input_chars = obj
        .get("context")
        .and_then(|ctx| ctx.get("messages"))
        .and_then(|msgs| msgs.as_array())
        .map(|msgs| {
            msgs.iter()
                .map(|m| {
                    m.get("entries")
                        .and_then(|e| e.as_array())
                        .map(|entries| {
                            entries
                                .iter()
                                .filter_map(|entry| {
                                    if entry.get("type").and_then(|t| t.as_str()) == Some("text") {
                                        entry
                                            .get("text")
                                            .and_then(|t| t.as_str())
                                            .map(|s| s.chars().count())
                                    } else {
                                        None
                                    }
                                })
                                .sum::<usize>()
                        })
                        .unwrap_or(0)
                })
                .sum::<usize>()
        })
        .unwrap_or(0)
        + obj
            .get("input")
            .and_then(|inp| inp.get("data"))
            .and_then(|data| data.get("messages"))
            .and_then(|msgs| msgs.as_array())
            .map(|msgs| {
                msgs.iter()
                    .map(|msg| {
                        if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                            content
                                .iter()
                                .filter_map(|part| {
                                    if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                                        part.get("text")
                                            .and_then(|t| t.as_str())
                                            .map(|s| s.chars().count())
                                    } else {
                                        None
                                    }
                                })
                                .sum::<usize>()
                        } else if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                            text.chars().count()
                        } else {
                            0
                        }
                    })
                    .sum::<usize>()
            })
            .unwrap_or(0);

    let input = estimate_tokens(input_chars);
    let output = estimate_tokens(output_chars);
    if input + output == 0 {
        return Some(Vec::new());
    }

    // Prefer a real model id from the execution payload (context/completionOptions),
    // skipping Kiro-internal placeholders, and fall back to "auto" — mirroring the
    // snapshot path so pricing can resolve these messages.
    let model_id = find_kiro_snapshot_model_id(value).unwrap_or_else(|| "auto".to_string());

    // Attribute execution usage to its workspace, matching every other
    // globalStorage Kiro message.
    let workspace = kiro_global_storage_workspace(path);
    let workspace_key = workspace.as_deref().and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

    let mut message = UnifiedMessage::new_with_dedup(
        CLIENT_ID,
        model_id,
        PROVIDER_ID,
        session_id,
        timestamp,
        TokenBreakdown {
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        },
        0.0,
        Some(format!("execution:{}", execution_id)),
    );
    message.message_count = 1;
    message.is_turn_start = true;
    message.duration_ms = duration_ms;
    message.set_workspace(workspace_key, workspace_label);
    Some(vec![message])
}

fn try_parse_kiro_workspace_session(
    value: &Value,
    path: &Path,
    fallback_timestamp: i64,
) -> Option<Vec<UnifiedMessage>> {
    let history = value.get("history")?.as_array()?;
    if value.get("sessionId").is_none() && value.get("selectedModel").is_none() {
        return None;
    }

    let file_stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    let workspace = kiro_global_storage_workspace(path);
    let workspace_key = workspace.as_deref().and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
    let session_id = value
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| match workspace.as_deref() {
            Some(ws) => format!("{}/{}", ws, file_stem),
            None => file_stem.to_string(),
        });

    let model_id = value
        .get("selectedModel")
        .and_then(|v| v.as_str())
        .filter(|m| !m.is_empty())
        .unwrap_or("auto")
        .to_string();

    let mut total_prompt_chars: usize = 0;
    let mut prompt_log_count: i32 = 0;
    let mut assistant_chars: usize = 0;

    for entry in history {
        if let Some(prompt_logs) = entry.get("promptLogs").and_then(|v| v.as_array()) {
            for pl in prompt_logs {
                if let Some(prompt) = pl.get("prompt").and_then(|v| v.as_str()) {
                    total_prompt_chars += prompt.chars().count();
                    prompt_log_count += 1;
                }
            }
        }
        if let Some(msg) = entry.get("message") {
            if msg.get("role").and_then(|v| v.as_str()) == Some("assistant") {
                if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
                    assistant_chars += content.chars().count();
                }
            }
        }
    }

    if total_prompt_chars == 0 {
        return None;
    }

    let input = estimate_tokens(total_prompt_chars);
    let output = estimate_tokens(assistant_chars);

    if input + output == 0 {
        return Some(Vec::new());
    }

    let mut message = UnifiedMessage::new_with_dedup(
        CLIENT_ID,
        model_id,
        PROVIDER_ID,
        session_id.clone(),
        fallback_timestamp,
        TokenBreakdown {
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        },
        0.0,
        Some(format!("{}:workspace-session", session_id)),
    );
    message.message_count = prompt_log_count.max(1);
    message.is_turn_start = true;
    message.set_workspace(workspace_key, workspace_label);
    Some(vec![message])
}

/// Drop globalStorage snapshot messages whose execution is already counted.
///
/// Kiro IDE's globalStorage (verified against real trees) holds, per workspace
/// hash directory: `<hash>.chat` artifacts carrying a top-level `executionId`
/// plus chat/context text, and extensionless execution records (in a nested
/// store directory) carrying the same `executionId` with the full `context`
/// history and `actions`. Counting both counts the same conversation text
/// twice; the execution record's input is a superset of the `.chat` content,
/// so the `.chat` message is redundant once its execution is present.
///
/// Matching is exact and workspace-scoped on the shared `executionId` (with a
/// legacy fallback matching an execution's `chatSessionId` against a snapshot
/// file stem). Workspace-session promptLogs snapshots are matched globally on
/// the session UUID instead, because they live under a different
/// `kiro.kiroagent` subdirectory than executions and so never share a
/// workspace key. Anything unmatched is kept — the pass can only remove
/// verified duplicates, never unrelated usage.
pub(crate) fn suppress_snapshots_covered_by_executions(
    messages: Vec<UnifiedMessage>,
) -> Vec<UnifiedMessage> {
    let mut executed_sessions: std::collections::HashSet<(Option<String>, String)> =
        std::collections::HashSet::new();
    let mut executed_ids: std::collections::HashSet<(Option<String>, String)> =
        std::collections::HashSet::new();
    let mut executed_session_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for message in &messages {
        let Some(execution_id) = message
            .dedup_key
            .as_deref()
            .and_then(|key| key.strip_prefix("execution:"))
        else {
            continue;
        };
        executed_sessions.insert((message.workspace_key.clone(), message.session_id.clone()));
        executed_ids.insert((message.workspace_key.clone(), execution_id.to_string()));
        executed_session_ids.insert(message.session_id.clone());
    }
    if executed_ids.is_empty() {
        return messages;
    }

    messages
        .into_iter()
        .filter(|message| {
            let Some(key) = message.dedup_key.as_deref() else {
                return true;
            };
            // `.chat` artifacts tagged with the execution they belong to.
            if let Some((_, execution_id)) = key.split_once(":globalstorage:exec:") {
                return !executed_ids
                    .contains(&(message.workspace_key.clone(), execution_id.to_string()));
            }
            // Workspace-session promptLogs snapshots duplicate the cumulative
            // request payloads already captured by that session's execution
            // records. They live under `kiro.kiroagent/workspace-sessions/`
            // while executions live under `kiro.kiroagent/<workspace-hash>/`,
            // so their workspace keys can never agree — match globally on the
            // session UUID (execution `chatSessionId` == workspace-session
            // `sessionId`). Sessions with no counted execution are kept.
            if key.ends_with(":workspace-session") {
                return !executed_session_ids.contains(&message.session_id);
            }
            if !key.ends_with(":globalstorage") {
                return true;
            }
            // Legacy fallback: snapshot session ids are `<workspace>/<file-stem>`
            // (or bare stem); match the stem against execution chatSessionIds.
            let stem = message
                .session_id
                .rsplit('/')
                .next()
                .unwrap_or(&message.session_id);
            !executed_sessions.contains(&(message.workspace_key.clone(), stem.to_string()))
        })
        .collect()
}

pub fn parse_kiro_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let query = "SELECT key, conversation_id, value FROM conversations_v2";
    let mut messages = Vec::new();

    sqlite_for_each_row(db_path, query, Some("Kiro conversation"), &mut |row| {
        let cwd: String = row.get(0)?;
        let conversation_id: String = row.get(1)?;
        let json_str: String = row.get(2)?;
        let parsed = match serde_json::from_str::<KiroDbConversation>(&json_str) {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };

        let context_window = parsed
            .model_info
            .as_ref()
            .and_then(|info| info.context_window_tokens)
            .unwrap_or(0);
        let model_id = parsed
            .model_info
            .as_ref()
            .and_then(|info| info.model_id.as_deref())
            .filter(|m| !m.trim().is_empty())
            .unwrap_or(UNKNOWN_MODEL)
            .to_string();
        let workspace_key = normalize_workspace_key(&cwd);
        let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

        // Provider-reported credit cost (task 9). Computed before the turn
        // loop so we can assign it to the first turn actually emitted for this
        // conversation (skipped zero-token turns are not "emitted").
        let credit_sum = parsed.credit_sum();

        let history = parsed.history.unwrap_or_default();
        let mut first_emitted_index: Option<usize> = None;
        for (index, turn) in history.into_iter().enumerate() {
            let Some(ref meta) = turn.request_metadata else {
                continue;
            };

            // Resolve each real count with flat-over-nested precedence: prefer
            // the flat KiroDbRequestMetadata field, else fall back to the nested
            // token_usage field. Estimates below are ESTIMATED, not measured
            // token counts, and are only used when no real count is present.
            let nested = meta.token_usage.as_ref();
            let flat_or_nested_i64 =
                |flat: Option<i64>, pick: fn(&KiroDbTokenUsage) -> Option<i64>| {
                    flat.or_else(|| nested.and_then(pick))
                };

            let real_input =
                flat_or_nested_i64(meta.input_tokens, |u| u.input_tokens).filter(|&v| v > 0);
            let real_output =
                flat_or_nested_i64(meta.output_tokens, |u| u.output_tokens).filter(|&v| v > 0);
            let cache_read_count =
                flat_or_nested_i64(meta.cache_read_input_tokens, |u| u.cache_read_input_tokens);
            let cache_write_count = flat_or_nested_i64(meta.cache_write_input_tokens, |u| {
                u.cache_write_input_tokens
            });
            let reasoning_count = flat_or_nested_i64(meta.reasoning_tokens, |u| u.reasoning_tokens);
            let request_count = meta
                .request_count
                .or_else(|| nested.and_then(|u| u.request_count));

            let ctx_pct = meta.context_usage_percentage.unwrap_or(0.0);
            let response_size = meta.response_size.unwrap_or(0);

            // Hybrid content/metadata estimation (task 8), used ONLY when no
            // real input count is present. Fresh input is the turn's own new
            // content: the user prompt bytes PLUS the ToolUseResults bytes
            // recorded for this turn. A tool_result is FRESH input on the turn
            // it first appears — the model reads it for the first time then;
            // only on SUBSEQUENT turns is it resent context, and it then falls
            // naturally into `cache_read` via the growing cumulative context.
            // Because `cache_read = max(total_context - fresh_input, 0)` and
            // `total_context` is the ctx%-derived ceiling, adding this-turn
            // tool_result bytes to fresh input does NOT double count: it simply
            // moves those bytes from cache_read into input for the turn they
            // first appear. Prefer the persisted
            // `request_metadata.user_prompt_length` for the prompt-bytes part
            // (falling back to the user `Prompt` text length), then ADD this
            // turn's ToolUseResults bytes.
            let fresh_input_tokens = || {
                let prompt_bytes = meta.user_prompt_length.unwrap_or(0).max(0) as usize;
                let prompt_part = if prompt_bytes > 0 {
                    prompt_bytes
                } else {
                    turn.prompt_text_len()
                };
                let tool_results_part = turn.tool_results_text_len();
                estimate_tokens(prompt_part + tool_results_part)
            };
            // Full input resent to the model this turn, derived from the
            // cumulative context percentage. Guarded on window > 0 and
            // ctx_pct > 0, else 0.
            let total_context_tokens = if context_window > 0 && ctx_pct > 0.0 {
                ((context_window as f64) * ctx_pct / 100.0).floor() as i64
            } else {
                0
            };

            // Prefer the real input count. When absent, use the fresh-input
            // estimate and derive cache_read from the cumulative context.
            let (input, hybrid_cache_read) = match real_input {
                Some(real) => (real, None),
                None => {
                    let fresh = fresh_input_tokens();
                    let cache_read = (total_context_tokens - fresh).max(0);
                    (fresh, Some(cache_read))
                }
            };
            // Prefer the real output count; fall back to the response_size / 4
            // estimate when absent.
            let output = real_output.unwrap_or_else(|| estimate_tokens(response_size));

            // A real cache_read count takes precedence over the hybrid
            // estimate, but only when it is a genuine positive value; a
            // zero/negative recorded count is treated as absent (mirroring the
            // input/output `> 0` filter) so the hybrid cumulative-context
            // estimate fills in. The hybrid is itself None when a real input
            // count was present.
            let cache_read = cache_read_count
                .filter(|&v| v > 0)
                .or(hybrid_cache_read)
                .unwrap_or(0)
                .max(0);
            let cache_write = cache_write_count.unwrap_or(0).max(0);
            let reasoning = reasoning_count.unwrap_or(0).max(0);

            // Cache/reasoning do NOT gate the skip — only resolved input/output.
            if input + output == 0 {
                continue;
            }

            let duration_ms = duration_between_ms(
                meta.request_start_timestamp_ms,
                meta.stream_end_timestamp_ms,
            );
            let timestamp = meta
                .request_start_timestamp_ms
                .or(meta.stream_end_timestamp_ms)
                .unwrap_or(0);

            let mut message = UnifiedMessage::new_with_dedup(
                CLIENT_ID,
                model_id.clone(),
                PROVIDER_ID,
                conversation_id.clone(),
                timestamp,
                TokenBreakdown {
                    input,
                    output,
                    cache_read,
                    cache_write,
                    reasoning,
                },
                0.0,
                Some(format!("{}:{}", conversation_id, index)),
            );
            message.message_count = request_count.unwrap_or(1).max(1);
            message.duration_ms = duration_ms;
            message.is_turn_start = true;
            message.set_workspace(workspace_key.clone(), workspace_label.clone());
            if first_emitted_index.is_none() {
                first_emitted_index = Some(messages.len());
            }
            messages.push(message);
        }

        // When the conversation carries a nonzero credit total, its true cost
        // is provider-reported: assign `credit_sum * CREDIT_TO_USD` to the
        // first emitted turn and mark it provider-reported so the pricing
        // dispatch (which respects `has_authoritative_cost`) does not override
        // it. Other turns keep cost 0.0. When no credit is present the turns
        // stay at cost 0.0 / CostSource::Unknown for downstream estimation.
        if credit_sum > 0.0 {
            if let Some(idx) = first_emitted_index {
                let message = &mut messages[idx];
                message.cost = credit_sum * CREDIT_TO_USD;
                message.mark_provider_reported_cost();
            }
        }

        Ok(())
    });

    messages
}

#[derive(Debug, Deserialize)]
struct KiroDbConversation {
    history: Option<Vec<KiroDbTurn>>,
    model_info: Option<KiroModelInfo>,
    // Conversation-level metadata. Its `usage_info` array carries the
    // provider-reported credit cost (task 9): entries shaped like
    // `{"value": 0.0313, "unit": "credit"}`.
    user_turn_metadata: Option<KiroDbUserTurnMetadata>,
}

#[derive(Debug, Deserialize)]
struct KiroDbTurn {
    // The user side of the turn. `content` is an internally-tagged-ish enum:
    // real DB data holds `Prompt` (user text) and `ToolUseResults`
    // (tool-execution results fed back to the model as input). Other variants
    // (e.g. CancelledToolUses) and unknown shapes contribute no text.
    user: Option<KiroDbUser>,
    // The assistant side of the turn. Output tokens are estimated from
    // `request_metadata.response_size`, so this is deserialized only for
    // potential text cross-checks; it does not gate the mapping.
    #[allow(dead_code)]
    assistant: Option<KiroDbAssistant>,
    request_metadata: Option<KiroDbRequestMetadata>,
}

#[derive(Debug, Deserialize)]
struct KiroDbUser {
    content: Option<KiroDbUserContent>,
}

/// The `user.content` variants observed in real `conversations_v2` data.
///
/// Variants are externally tagged (the JSON object has a single key naming the
/// variant, e.g. `{"Prompt": {"prompt": "..."}}`). Only `Prompt` and
/// `ToolUseResults` carry input text we care about for fresh-input estimation;
/// `CancelledToolUses` and any future/unknown variant deserialize to `Other`
/// and contribute empty text.
#[derive(Debug, Deserialize)]
enum KiroDbUserContent {
    Prompt(KiroDbPrompt),
    // Tool-execution results fed back to the model. Captured as raw JSON so
    // this content deserializes into its own variant (distinct from `Prompt`
    // and not swallowed by `Other`). Under the current fresh-input model these
    // results ARE fresh input on the turn they first appear (the model reads
    // them for the first time that turn); on later turns they become resent
    // context absorbed into `cache_read` via the cumulative context. The
    // captured payload's text length is read by `tool_results_len`.
    ToolUseResults(Value),
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct KiroDbPrompt {
    prompt: Option<String>,
}

/// The `assistant` variants observed in real `conversations_v2` data.
///
/// Externally tagged like [`KiroDbUserContent`]. Deserialized for completeness
/// / potential text cross-checks; assistant output stays estimated from
/// `response_size`. Unknown variants deserialize to `Other`.
// Retained for potential assistant-text cross-checks; assistant output stays
// estimated from `response_size`, so these fields are not read yet.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
enum KiroDbAssistant {
    Response(KiroDbAssistantResponse),
    ToolUse(KiroDbAssistantToolUse),
    #[serde(other)]
    Other,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct KiroDbAssistantResponse {
    content: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct KiroDbAssistantToolUse {
    content: Option<String>,
    tool_uses: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
struct KiroDbUserTurnMetadata {
    usage_info: Option<Vec<KiroDbUsageInfo>>,
}

#[derive(Debug, Deserialize)]
struct KiroDbUsageInfo {
    value: Option<f64>,
    unit: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KiroDbRequestMetadata {
    context_usage_percentage: Option<f64>,
    response_size: Option<usize>,
    // Byte length of the turn's user prompt text. Persisted by Kiro's Auto
    // agent (unlike the real token counts, which are all zero on that schema)
    // and used, together with this turn's ToolUseResults text size, for
    // fresh-input estimation (task 8). NOT a token count.
    user_prompt_length: Option<i64>,
    request_start_timestamp_ms: Option<i64>,
    stream_end_timestamp_ms: Option<i64>,
    #[serde(alias = "uncached_input_tokens", alias = "input_token_count")]
    input_tokens: Option<i64>,
    #[serde(alias = "output_token_count")]
    output_tokens: Option<i64>,
    #[serde(alias = "cache_read_tokens", alias = "cache_read")]
    cache_read_input_tokens: Option<i64>,
    #[serde(
        alias = "cache_write_tokens",
        alias = "cache_creation_input_tokens",
        alias = "cache_write"
    )]
    cache_write_input_tokens: Option<i64>,
    #[serde(alias = "reasoning_token_count", alias = "thinking_tokens")]
    reasoning_tokens: Option<i64>,
    #[serde(alias = "user_turn_request_count", alias = "total_request_count")]
    request_count: Option<i32>,
    #[serde(alias = "token_usage", alias = "usage")]
    token_usage: Option<KiroDbTokenUsage>,
}

#[derive(Debug, Deserialize)]
struct KiroDbTokenUsage {
    #[serde(alias = "uncached_input_tokens", alias = "input_token_count")]
    input_tokens: Option<i64>,
    #[serde(alias = "output_token_count")]
    output_tokens: Option<i64>,
    #[serde(alias = "cache_read_tokens", alias = "cache_read")]
    cache_read_input_tokens: Option<i64>,
    #[serde(
        alias = "cache_write_tokens",
        alias = "cache_creation_input_tokens",
        alias = "cache_write"
    )]
    cache_write_input_tokens: Option<i64>,
    #[serde(alias = "reasoning_token_count", alias = "thinking_tokens")]
    reasoning_tokens: Option<i64>,
    #[serde(alias = "user_turn_request_count", alias = "total_request_count")]
    request_count: Option<i32>,
}

impl KiroDbUserContent {
    /// Byte length of only the `Prompt` text. Zero for every other variant.
    /// This is the turn's own new user prompt; it is the fallback prompt-bytes
    /// source (task 8) when `request_metadata.user_prompt_length` is
    /// absent/zero.
    fn prompt_len(&self) -> usize {
        match self {
            KiroDbUserContent::Prompt(prompt) => {
                prompt.prompt.as_deref().map(str::len).unwrap_or(0)
            }
            _ => 0,
        }
    }

    /// Byte length of the human-readable text carried by a `ToolUseResults`
    /// payload. Zero for every other variant. These bytes are the turn's own
    /// fresh tool input (the model reads the tool_result for the first time on
    /// this turn), so they are added to fresh input alongside the prompt bytes.
    ///
    /// The observed shape is an array of entries, each with a `content` array
    /// of `{ "Text": "..." }` (also tolerating a lowercase `text` key). We sum
    /// the lengths of those text strings. If the shape is unexpected and no
    /// text fields are found, fall back to the length of the compact JSON
    /// serialization so a non-empty tool_result never contributes zero.
    fn tool_results_len(&self) -> usize {
        let KiroDbUserContent::ToolUseResults(value) = self else {
            return 0;
        };

        fn text_len_of(value: &Value) -> usize {
            match value {
                Value::Array(items) => items.iter().map(text_len_of).sum(),
                Value::Object(map) => map
                    .iter()
                    .map(|(key, v)| match (key.as_str(), v) {
                        ("Text" | "text", Value::String(s)) => s.len(),
                        _ => text_len_of(v),
                    })
                    .sum(),
                _ => 0,
            }
        }

        let extracted = text_len_of(value);
        if extracted > 0 {
            extracted
        } else {
            serde_json::to_string(value).map(|s| s.len()).unwrap_or(0)
        }
    }
}

impl KiroDbTurn {
    /// Byte length of only the user `Prompt` text for this turn — the turn's
    /// own new user prompt. Returns `0` for ToolUseResults/Other or when there
    /// is no user content. Used as the prompt-bytes fallback (task 8) when
    /// `request_metadata.user_prompt_length` is absent/zero.
    fn prompt_text_len(&self) -> usize {
        self.user
            .as_ref()
            .and_then(|u| u.content.as_ref())
            .map(KiroDbUserContent::prompt_len)
            .unwrap_or(0)
    }

    /// Byte length of the `ToolUseResults` text recorded for this turn — the
    /// tool-execution results fed back to the model. Returns `0` for
    /// Prompt/Other or when there is no user content. Every tool_result is
    /// recorded exactly once, in the turn it belongs to, so these bytes ARE
    /// this turn's fresh tool input; they are added to fresh input alongside
    /// the prompt bytes.
    fn tool_results_text_len(&self) -> usize {
        self.user
            .as_ref()
            .and_then(|u| u.content.as_ref())
            .map(KiroDbUserContent::tool_results_len)
            .unwrap_or(0)
    }
}

impl KiroDbConversation {
    /// Sum of `value` across `user_turn_metadata.usage_info` entries whose
    /// `unit == "credit"`. Returns `0.0` when the metadata or array is absent.
    /// This credit total drives provider-reported cost in task 9.
    fn credit_sum(&self) -> f64 {
        self.user_turn_metadata
            .as_ref()
            .and_then(|m| m.usage_info.as_ref())
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e.unit.as_deref() == Some("credit"))
                    .filter_map(|e| e.value)
                    .sum()
            })
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::CostSource;
    use rusqlite::Connection;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn parse_kiro_sqlite_returns_empty_for_missing_database() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.db");
        assert!(parse_kiro_sqlite(&missing).is_empty());
    }

    fn create_session_files(
        dir: &TempDir,
        stem: &str,
        json: &str,
        jsonl: &str,
    ) -> std::path::PathBuf {
        let json_path = dir.path().join(format!("{}.json", stem));
        let jsonl_path = dir.path().join(format!("{}.jsonl", stem));
        let mut f = std::fs::File::create(&json_path).unwrap();
        f.write_all(json.as_bytes()).unwrap();
        let mut f = std::fs::File::create(&jsonl_path).unwrap();
        f.write_all(jsonl.as_bytes()).unwrap();
        json_path
    }

    #[test]
    fn test_parse_kiro_estimates_tokens_from_jsonl_content() {
        let dir = TempDir::new().unwrap();
        let json = r#"{"session_id":"session-1","cwd":"/tmp/project","session_state":{"rts_model_state":{"model_info":{"model_id":"claude-sonnet-4-5"}},"conversation_metadata":{"user_turn_metadatas":[{"input_token_count":0,"output_token_count":0,"turn_duration":123,"end_timestamp":1770983427,"total_request_count":2,"message_ids":["prompt-1","assistant-1"]}]}}}"#;
        let jsonl = r#"{"version":"v1","kind":"Prompt","data":{"message_id":"prompt-1","content":[{"kind":"text","data":"hello world"}],"meta":{"timestamp":1770983426.420942}}}
{"version":"v1","kind":"AssistantMessage","data":{"message_id":"assistant-1","content":[{"kind":"text","data":"response text"}]}}"#;
        let path = create_session_files(&dir, "session-1", json, jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "kiro");
        assert_eq!(messages[0].provider_id, "amazon-bedrock");
        assert_eq!(messages[0].model_id, "claude-sonnet-4-5");
        assert_eq!(messages[0].session_id, "session-1");
        assert_eq!(messages[0].tokens.input, 3);
        assert_eq!(messages[0].tokens.output, 4);
        assert_eq!(messages[0].message_count, 2);
        assert!(messages[0].is_turn_start);
        assert_eq!(messages[0].timestamp, 1770983426420);
        assert_eq!(messages[0].duration_ms, Some(580));
        assert_eq!(messages[0].workspace_key, Some("/tmp/project".to_string()));
        assert_eq!(messages[0].workspace_label, Some("project".to_string()));
    }

    #[test]
    fn test_parse_kiro_skips_zero_content_turns() {
        let dir = TempDir::new().unwrap();
        let json = r#"{"session_id":"session-2","cwd":"/tmp","session_state":{"rts_model_state":{"model_info":{"model_id":"model"}},"conversation_metadata":{"user_turn_metadatas":[{"input_token_count":0,"output_token_count":0,"message_ids":["missing"]}]}}}"#;
        let jsonl = "";
        let path = create_session_files(&dir, "session-2", json, jsonl);

        let messages = parse_kiro_file(&path);

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_kiro_skips_malformed_jsonl_lines() {
        let dir = TempDir::new().unwrap();
        let json = r#"{"session_id":"session-3","cwd":"/tmp/project","session_state":{"rts_model_state":{"model_info":{"model_id":"claude-sonnet-4-5"}},"conversation_metadata":{"user_turn_metadatas":[{"input_token_count":0,"output_token_count":0,"turn_duration":100,"end_timestamp":1770983427,"total_request_count":2,"message_ids":["prompt-3","assistant-3"]}]}}}"#;
        let jsonl = r#"{"version":"v1","kind":"Prompt","data":{"message_id":"prompt-3","content":[{"kind":"text","data":"hello world"}],"meta":{"timestamp":1770983426.420942}}}
not valid json at all
{"version":"v1","kind":"AssistantMessage","data":{"message_id":"assistant-3","content":[{"kind":"text","data":"response text"}]}}"#;
        let path = create_session_files(&dir, "session-3", json, jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].tokens.input > 0 || messages[0].tokens.output > 0);
    }

    // A CLI session whose sibling jsonl carries a `ToolResults` entry (a text
    // payload) between the `Prompt` and the `AssistantMessage`. The tool-result
    // bytes are fresh input on this turn, so
    //   input      = estimate_tokens(prompt_bytes + tool_result_bytes)
    //   cache_read = max(total_context - input, 0)
    // with total_context = floor(context_window * ctx_pct / 100).
    #[test]
    fn test_parse_kiro_cli_tool_result_text_adds_to_fresh_input() {
        let dir = TempDir::new().unwrap();
        // context_window_tokens = 100_000, ctx_pct = 5 -> total_context = 5000.
        // user_prompt_length = 20 (prompt-bytes path); all real counts 0.
        let json = r#"{"session_id":"session-tr","cwd":"/tmp/project","session_state":{"rts_model_state":{"model_info":{"model_id":"auto","context_window_tokens":100000}},"conversation_metadata":{"user_turn_metadatas":[{"input_token_count":0,"output_token_count":0,"user_prompt_length":20,"context_usage_percentage":5.0,"total_request_count":1,"message_ids":["assistant-tr"]}]}}}"#;
        // The tool-result text payload is exactly 40 chars.
        let tool_text = "0123456789012345678901234567890123456789";
        assert_eq!(tool_text.len(), 40);
        let prompt_line = r#"{"version":"v1","kind":"Prompt","data":{"message_id":"prompt-tr","content":[{"kind":"text","data":"hello"}],"meta":{"timestamp":1770983426.0}}}"#;
        let tool_line = format!(
            r#"{{"version":"v1","kind":"ToolResults","data":{{"message_id":"toolresults-tr","content":[{{"kind":"toolResult","data":{{"toolUseId":"t1","content":[{{"kind":"text","data":"{}"}}],"status":"success"}}}}]}}}}"#,
            tool_text
        );
        let assistant_line = r#"{"version":"v1","kind":"AssistantMessage","data":{"message_id":"assistant-tr","content":[{"kind":"text","data":"response text"}]}}"#;
        let jsonl = format!("{}\n{}\n{}", prompt_line, tool_line, assistant_line);
        let path = create_session_files(&dir, "session-tr", json, &jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        // prompt_bytes = user_prompt_length = 20; tool_result_bytes = 40.
        let expected_input = estimate_tokens(20 + 40);
        assert_eq!(
            messages[0].tokens.input, expected_input,
            "tool_result bytes are fresh input on the turn they first appear"
        );
        let total_context = 5000i64;
        assert_eq!(
            messages[0].tokens.cache_read,
            (total_context - expected_input).max(0)
        );
    }

    // Real input_token_count > 0 overrides the fresh-input estimate even when
    // tool_result bytes are present (real-count precedence preserved).
    #[test]
    fn test_parse_kiro_cli_real_input_overrides_tool_result_estimate() {
        let dir = TempDir::new().unwrap();
        let json = r#"{"session_id":"session-tr2","cwd":"/tmp/project","session_state":{"rts_model_state":{"model_info":{"model_id":"auto","context_window_tokens":100000}},"conversation_metadata":{"user_turn_metadatas":[{"input_token_count":777,"output_token_count":0,"user_prompt_length":20,"context_usage_percentage":5.0,"total_request_count":1,"message_ids":["assistant-tr2"]}]}}}"#;
        let tool_text = "0123456789012345678901234567890123456789";
        let prompt_line = r#"{"version":"v1","kind":"Prompt","data":{"message_id":"prompt-tr2","content":[{"kind":"text","data":"hello"}],"meta":{"timestamp":1770983426.0}}}"#;
        let tool_line = format!(
            r#"{{"version":"v1","kind":"ToolResults","data":{{"message_id":"toolresults-tr2","content":[{{"kind":"toolResult","data":{{"toolUseId":"t1","content":[{{"kind":"text","data":"{}"}}],"status":"success"}}}}]}}}}"#,
            tool_text
        );
        let assistant_line = r#"{"version":"v1","kind":"AssistantMessage","data":{"message_id":"assistant-tr2","content":[{"kind":"text","data":"response text"}]}}"#;
        let jsonl = format!("{}\n{}\n{}", prompt_line, tool_line, assistant_line);
        let path = create_session_files(&dir, "session-tr2", json, &jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].tokens.input, 777,
            "recorded input_token_count must override the tool_result estimate"
        );
    }

    // A `ToolResults` entry carrying a nested `json` payload: its length is the
    // compact JSON serialization, and the walk must not panic.
    #[test]
    fn test_parse_kiro_cli_tool_result_json_payload_length() {
        let dir = TempDir::new().unwrap();
        let json = r#"{"session_id":"session-tr3","cwd":"/tmp/project","session_state":{"rts_model_state":{"model_info":{"model_id":"auto","context_window_tokens":100000}},"conversation_metadata":{"user_turn_metadatas":[{"input_token_count":0,"output_token_count":0,"user_prompt_length":0,"context_usage_percentage":5.0,"total_request_count":1,"message_ids":["assistant-tr3"]}]}}}"#;
        // Nested json payload. serde compact serialization of this object:
        let payload = serde_json::json!({"stdout": "hi", "exit_status": "exit status: 0"});
        let compact = serde_json::to_string(&payload).unwrap();
        let tool_len = compact.len();
        let prompt_line = r#"{"version":"v1","kind":"Prompt","data":{"message_id":"prompt-tr3","content":[{"kind":"text","data":"hi"}],"meta":{"timestamp":1770983426.0}}}"#;
        let tool_line = format!(
            r#"{{"version":"v1","kind":"ToolResults","data":{{"message_id":"toolresults-tr3","content":[{{"kind":"toolResult","data":{{"toolUseId":"t1","content":[{{"kind":"json","data":{}}}],"status":"success"}}}}]}}}}"#,
            compact
        );
        let assistant_line = r#"{"version":"v1","kind":"AssistantMessage","data":{"message_id":"assistant-tr3","content":[{"kind":"text","data":"ok"}]}}"#;
        let jsonl = format!("{}\n{}\n{}", prompt_line, tool_line, assistant_line);
        let path = create_session_files(&dir, "session-tr3", json, &jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        // user_prompt_length = 0 -> prompt_part falls back to prompt text "hi"
        // = 2 chars. fresh input = estimate_tokens(2 + tool_len).
        let expected_input = estimate_tokens(2 + tool_len);
        assert_eq!(
            messages[0].tokens.input, expected_input,
            "nested json tool_result contributes its compact serialization length"
        );
    }

    #[test]
    fn test_parse_kiro_sqlite_sets_duration_from_request_metadata() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("data.sqlite3");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE conversations_v2 (key TEXT, conversation_id TEXT, value TEXT)",
            [],
        )
        .unwrap();
        let value = r#"{
            "model_info": {
                "model_id": "auto",
                "context_window_tokens": 1000
            },
            "history": [{
                "request_metadata": {
                    "context_usage_percentage": 10,
                    "response_size": 40,
                    "request_start_timestamp_ms": 1770983426000,
                    "stream_end_timestamp_ms": 1770983427500
                }
            }]
        }"#;
        conn.execute(
            "INSERT INTO conversations_v2 (key, conversation_id, value) VALUES (?1, ?2, ?3)",
            (&"/tmp/project", &"conv-1", &value),
        )
        .unwrap();
        drop(conn);

        let messages = parse_kiro_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "auto");
        assert_eq!(messages[0].timestamp, 1770983426000);
        assert_eq!(messages[0].duration_ms, Some(1500));
        // Amended hybrid estimate: no user content -> input 0, cumulative
        // context (1000 * 10 / 100 = 100) -> cache_read, output = 40 / 4.
        assert_eq!(messages[0].tokens.input, 0);
        assert_eq!(messages[0].tokens.cache_read, 100);
        assert_eq!(messages[0].tokens.output, 10);
    }

    // Helper for the bug-condition exploration tests below: inserts a single
    // conversations_v2 row whose history[0].request_metadata is `meta_json`,
    // then runs the parser and returns the resulting messages. Mirrors the
    // TempDir SQLite setup used by
    // test_parse_kiro_sqlite_sets_duration_from_request_metadata.
    fn parse_single_kiro_sqlite_turn(meta_json: &str) -> Vec<UnifiedMessage> {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("data.sqlite3");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE conversations_v2 (key TEXT, conversation_id TEXT, value TEXT)",
            [],
        )
        .unwrap();
        let value = format!(
            r#"{{
                "model_info": {{
                    "model_id": "auto",
                    "context_window_tokens": 1000
                }},
                "history": [{{
                    "request_metadata": {}
                }}]
            }}"#,
            meta_json
        );
        conn.execute(
            "INSERT INTO conversations_v2 (key, conversation_id, value) VALUES (?1, ?2, ?3)",
            (&"/tmp/project", &"conv-1", &value),
        )
        .unwrap();
        drop(conn);

        // Keep the TempDir alive by leaking it into the returned messages'
        // lifetime is unnecessary: parse_kiro_sqlite fully reads the DB before
        // returning, so the dir can drop here.
        parse_kiro_sqlite(&db_path)
    }

    // ---------------------------------------------------------------------
    // Bug condition exploration tests (Task 1).
    //
    // These tests encode the EXPECTED (fixed) behavior from Property 1 in the
    // design: when Kiro persists real token/cache/reasoning counts and a real
    // request count in request_metadata, the parser must surface them in the
    // correct TokenBreakdown fields and set message_count = MAX(count, 1).
    //
    // They are EXPECTED TO FAIL on the unfixed code, because
    // KiroDbRequestMetadata does not deserialize these fields (serde drops
    // them), so the parser reports the byte estimate for output, 0 for
    // cache/reasoning, and message_count = 1. Each failure is a counterexample
    // that confirms the root cause.
    // ---------------------------------------------------------------------

    // Case 1 (real output dropped): output_tokens: 512, response_size: 40.
    // Unfixed reports output = 40 / 4 = 10; fixed must report output = 512.
    #[test]
    fn test_kiro_sqlite_bug_real_output_count_surfaced() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "input_tokens": 300,
            "output_tokens": 512,
            "request_count": 1,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        // Expected (fixed): recorded output count surfaced.
        assert_eq!(
            messages[0].tokens.output, 512,
            "expected recorded output_tokens (512) to surface, not the response_size/4 estimate (10)"
        );
    }

    // Case 2 (cache read dropped): cache_read_input_tokens: 1920.
    // Unfixed reports cacheRead = 0; fixed must report cacheRead = 1920.
    #[test]
    fn test_kiro_sqlite_bug_cache_read_count_surfaced() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "cache_read_input_tokens": 1920,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        // Expected (fixed): recorded cache-read count surfaced.
        assert_eq!(
            messages[0].tokens.cache_read, 1920,
            "expected recorded cache_read_input_tokens (1920) to surface, not 0"
        );
    }

    // Case 3 (reasoning dropped): reasoning_tokens: 40.
    // Unfixed reports reasoning = 0; fixed must report reasoning = 40.
    #[test]
    fn test_kiro_sqlite_bug_reasoning_count_surfaced() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "reasoning_tokens": 40,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        // Expected (fixed): recorded reasoning count surfaced.
        assert_eq!(
            messages[0].tokens.reasoning, 40,
            "expected recorded reasoning_tokens (40) to surface, not 0"
        );
    }

    // Case 4 (request count dropped): request_count: 12.
    // Unfixed reports messageCount = 1; fixed must report message_count = 12.
    #[test]
    fn test_kiro_sqlite_bug_request_count_surfaced() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "request_count": 12,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        // Expected (fixed): message_count = MAX(request_count, 1) = 12.
        assert_eq!(
            messages[0].message_count, 12,
            "expected message_count = MAX(request_count, 1) = 12, not 1"
        );
    }

    // Case 5 (edge — nested token_usage): all real counts nested under
    // token_usage { ... }. Unfixed drops all of them; fixed must surface each
    // one in its correct field and MAX(request_count, 1) into message_count.
    #[test]
    fn test_kiro_sqlite_bug_nested_token_usage_surfaced() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "token_usage": {
                "input_tokens": 300,
                "output_tokens": 512,
                "cache_read_input_tokens": 1920,
                "cache_write_input_tokens": 64,
                "reasoning_tokens": 40,
                "request_count": 12
            },
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        // Expected (fixed): every nested count surfaced in its correct field.
        assert_eq!(
            messages[0].tokens.input, 300,
            "expected nested input_tokens (300) to surface, not the context-percentage estimate (100)"
        );
        assert_eq!(
            messages[0].tokens.output, 512,
            "expected nested output_tokens (512) to surface, not the response_size/4 estimate (10)"
        );
        assert_eq!(
            messages[0].tokens.cache_read, 1920,
            "expected nested cache_read_input_tokens (1920) to surface, not 0"
        );
        assert_eq!(
            messages[0].tokens.cache_write, 64,
            "expected nested cache_write_input_tokens (64) to surface, not 0"
        );
        assert_eq!(
            messages[0].tokens.reasoning, 40,
            "expected nested reasoning_tokens (40) to surface, not 0"
        );
        assert_eq!(
            messages[0].message_count, 12,
            "expected message_count = MAX(request_count, 1) = 12, not 1"
        );
    }

    // ---------------------------------------------------------------------
    // Preservation property tests (Task 2).
    //
    // Property 2 (design): for every turn whose persisted metadata carries NO
    // real token/request counts (only the four legacy fields
    // context_usage_percentage, response_size, request_start_timestamp_ms,
    // stream_end_timestamp_ms), the parser must produce exactly today's
    // estimate/skip/message_count = 1 behavior.
    //
    // Methodology is observation-first: each assertion below encodes a value
    // recorded by RUNNING THE UNFIXED PARSER, not a hoped-for value. These
    // tests therefore PASS on the current (unfixed) code and lock in the
    // baseline the fix must preserve.
    //
    // No property-based-testing crate (proptest/quickcheck) is a
    // dev-dependency of tokscale-core, and the task scopes this work to "prefer
    // scoped/table-driven tests over adding a new dependency". The
    // domain-spanning "for all legacy-only metadata" property is therefore
    // exercised by a deterministic table-driven generator
    // (`legacy_only_metadata_cases`) that enumerates the four-field input space
    // (absent / zero / positive for each field, plus edge values), rather than
    // by a randomized PBT harness.
    // ---------------------------------------------------------------------

    // The parser's context-window estimate uses context_window_tokens = 1000
    // (set by parse_single_kiro_sqlite_turn), so input = (1000 * pct / 100).
    const PRESERVATION_CONTEXT_WINDOW: i64 = 1000;

    // Reproduce the amended (data-driven) hybrid estimate for a legacy-only
    // turn — one that carries no `user` content, so fresh input text is empty:
    //   input      = ceil(fresh_input_bytes / 4) = 0 (no prompt / tool results)
    //   cache_read = max(floor(window * ctx_pct / 100) - input, 0)
    //   output     = ceil(response_size / 4)
    // Returns None when the turn contributes zero input and zero output (the
    // parser skips such turns; cache_read/reasoning do not gate the skip).
    fn expected_legacy_estimate(
        ctx_pct: Option<f64>,
        response_size: Option<usize>,
    ) -> Option<(i64, i64, i64)> {
        let pct = ctx_pct.unwrap_or(0.0);
        // Legacy-only fixtures carry no user prompt / tool-result text, so the
        // fresh-input estimate is zero and the cumulative context becomes
        // cache_read.
        let input = 0;
        let total_context = if PRESERVATION_CONTEXT_WINDOW > 0 && pct > 0.0 {
            ((PRESERVATION_CONTEXT_WINDOW as f64) * pct / 100.0).floor() as i64
        } else {
            0
        };
        let cache_read = (total_context - input).max(0);
        let output = {
            let size = response_size.unwrap_or(0);
            size.div_ceil(4) as i64
        };
        if input + output == 0 {
            None
        } else {
            Some((input, output, cache_read))
        }
    }

    // Build a request_metadata JSON object carrying ONLY the four legacy
    // fields, omitting any field whose value is None so the generated shapes
    // span "absent" as well as "present" for each field.
    fn legacy_only_meta_json(
        ctx_pct: Option<f64>,
        response_size: Option<usize>,
        start_ms: Option<i64>,
        end_ms: Option<i64>,
    ) -> String {
        let mut fields: Vec<String> = Vec::new();
        if let Some(p) = ctx_pct {
            fields.push(format!("\"context_usage_percentage\": {}", p));
        }
        if let Some(r) = response_size {
            fields.push(format!("\"response_size\": {}", r));
        }
        if let Some(s) = start_ms {
            fields.push(format!("\"request_start_timestamp_ms\": {}", s));
        }
        if let Some(e) = end_ms {
            fields.push(format!("\"stream_end_timestamp_ms\": {}", e));
        }
        format!("{{ {} }}", fields.join(", "))
    }

    type LegacyMetaCase = (Option<f64>, Option<usize>, Option<i64>, Option<i64>);

    // Deterministic enumeration of the legacy-only input domain: every
    // combination of {absent, zero, positive} for context_usage_percentage and
    // response_size, crossed with {absent, present} timestamps. This is the
    // table-driven stand-in for "FOR ALL legacy-only metadata".
    fn legacy_only_metadata_cases() -> Vec<LegacyMetaCase> {
        let pcts: [Option<f64>; 4] = [None, Some(0.0), Some(10.0), Some(37.5)];
        let sizes: [Option<usize>; 4] = [None, Some(0), Some(40), Some(4001)];
        let start_ms: [Option<i64>; 2] = [None, Some(1_770_983_426_000)];
        let end_ms: [Option<i64>; 2] = [None, Some(1_770_983_427_500)];

        let mut cases = Vec::new();
        for &p in &pcts {
            for &s in &sizes {
                for &st in &start_ms {
                    for &en in &end_ms {
                        cases.push((p, s, st, en));
                    }
                }
            }
        }
        cases
    }

    // Preservation Test 1 + 2 + 3 (property-style over the legacy-only domain):
    //   - output estimate preserved (response_size / 4),
    //   - input estimate preserved (context_window * ctx_pct / 100),
    //   - zero-token skip preserved (no entry when input + output == 0),
    //   - message_count = 1 preserved (no real request count present).
    //
    // Runs against the UNFIXED parser: assertions mirror expected_legacy_estimate,
    // which recomputes today's estimate. All cases must pass on unfixed code.
    #[test]
    fn test_kiro_sqlite_preservation_legacy_only_estimates_and_skip() {
        for (pct, size, start_ms, end_ms) in legacy_only_metadata_cases() {
            let meta = legacy_only_meta_json(pct, size, start_ms, end_ms);
            let messages = parse_single_kiro_sqlite_turn(&meta);

            match expected_legacy_estimate(pct, size) {
                None => {
                    // Zero-token turn: parser skips it, emitting no entry.
                    assert!(
                        messages.is_empty(),
                        "zero-token legacy turn should be skipped (no entry); meta = {meta}"
                    );
                }
                Some((expected_input, expected_output, expected_cache_read)) => {
                    assert_eq!(
                        messages.len(),
                        1,
                        "non-zero legacy turn should emit exactly one entry; meta = {meta}"
                    );
                    let m = &messages[0];
                    // Hybrid estimate: input is fresh-only (0 for content-less
                    // fixtures), cumulative context becomes cache_read, output
                    // still from response_size.
                    assert_eq!(
                        m.tokens.input, expected_input,
                        "input estimate not preserved; meta = {meta}"
                    );
                    assert_eq!(
                        m.tokens.output, expected_output,
                        "output estimate not preserved; meta = {meta}"
                    );
                    // Cumulative context routes into cache_read under the
                    // amended data-driven estimate.
                    assert_eq!(
                        m.tokens.cache_read, expected_cache_read,
                        "cache_read must equal cumulative context; meta = {meta}"
                    );
                    assert_eq!(
                        m.tokens.cache_write, 0,
                        "cache_write must stay 0; meta = {meta}"
                    );
                    assert_eq!(
                        m.tokens.reasoning, 0,
                        "reasoning must stay 0; meta = {meta}"
                    );
                    // message_count = 1 when no real request count is present.
                    assert_eq!(
                        m.message_count, 1,
                        "message_count must be 1 for legacy-only turns; meta = {meta}"
                    );
                }
            }
        }
    }

    // Preservation Test 4 (metadata preserved): timestamp, duration_ms, model
    // "auto" fallback, provider amazon-bedrock, workspace key/label, the
    // "{conversation_id}:{index}" dedup key, and is_turn_start.
    //
    // parse_single_kiro_sqlite_turn inserts one row with cwd "/tmp/project",
    // conversation_id "conv-1", model_id "auto", so index 0 -> dedup key
    // "conv-1:0". Values here are observed on the UNFIXED parser.
    #[test]
    fn test_kiro_sqlite_preservation_metadata_fields() {
        let meta = legacy_only_meta_json(
            Some(10.0),
            Some(40),
            Some(1_770_983_426_000),
            Some(1_770_983_427_500),
        );
        let messages = parse_single_kiro_sqlite_turn(&meta);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.client, CLIENT_ID, "client preserved");
        assert_eq!(
            m.provider_id, PROVIDER_ID,
            "provider amazon-bedrock preserved"
        );
        assert_eq!(m.model_id, "auto", "model \"auto\" fallback preserved");
        // timestamp = request_start_timestamp_ms.
        assert_eq!(m.timestamp, 1_770_983_426_000, "timestamp preserved");
        // duration_ms = stream_end - request_start.
        assert_eq!(m.duration_ms, Some(1500), "duration_ms preserved");
        assert_eq!(
            m.dedup_key,
            Some("conv-1:0".to_string()),
            "\"{{conversation_id}}:{{index}}\" dedup key preserved"
        );
        assert!(m.is_turn_start, "is_turn_start preserved");
        assert_eq!(m.message_count, 1, "message_count = 1 preserved");
        // Amended hybrid estimate for this case: no user content -> input 0,
        // cumulative context (1000 * 10 / 100 = 100) -> cache_read, output 40/4.
        assert_eq!(m.tokens.input, 0, "input is fresh-only (no user content)");
        assert_eq!(
            m.tokens.cache_read, 100,
            "cache_read = cumulative context (1000 * 10 / 100)"
        );
        assert_eq!(m.tokens.output, 10, "output estimate (40 / 4) preserved");
    }

    // Preservation Test: timestamp falls back to stream_end_timestamp_ms when
    // request_start is absent, and duration_ms is None (no positive delta).
    // Observed on the UNFIXED parser.
    #[test]
    fn test_kiro_sqlite_preservation_timestamp_fallback_no_duration() {
        let meta = legacy_only_meta_json(Some(10.0), Some(40), None, Some(1_770_983_427_500));
        let messages = parse_single_kiro_sqlite_turn(&meta);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(
            m.timestamp, 1_770_983_427_500,
            "timestamp falls back to stream_end_timestamp_ms"
        );
        assert_eq!(
            m.duration_ms, None,
            "duration_ms is None when request_start is absent"
        );
    }

    // Preservation Test: the regression fixture
    // (test_parse_kiro_sqlite_sets_duration_from_request_metadata) exact shape,
    // asserted through the shared helper — locks the observed baseline for a
    // legacy-only turn end to end.
    #[test]
    fn test_kiro_sqlite_preservation_regression_fixture_shape() {
        let meta = legacy_only_meta_json(
            Some(10.0),
            Some(40),
            Some(1_770_983_426_000),
            Some(1_770_983_427_500),
        );
        let messages = parse_single_kiro_sqlite_turn(&meta);
        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.model_id, "auto");
        assert_eq!(m.timestamp, 1_770_983_426_000);
        assert_eq!(m.duration_ms, Some(1500));
        assert_eq!(m.tokens.input, 0);
        assert_eq!(m.tokens.output, 10);
        assert_eq!(m.tokens.cache_read, 100);
        assert_eq!(m.tokens.cache_write, 0);
        assert_eq!(m.tokens.reasoning, 0);
        assert_eq!(m.message_count, 1);
    }

    // Preservation Test 5 (non-SQLite sources preserved, Requirement 3.5):
    // the file-based path (globalStorage `.chat`) is handled by parse_kiro_file
    // and never touches KiroDbRequestMetadata, so this fix cannot change its
    // aggregation. Observed on the UNFIXED parser: one entry, non-zero
    // input/output, cache/reasoning zero, workspace/dedup attribution intact.
    // The broader IDE/CLI/snapshot paths are covered by the other tests in this
    // module, which continue to pass unchanged.
    #[test]
    fn test_kiro_non_sqlite_file_source_aggregation_preserved() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-a/execution.chat",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            r#"{
                "model": "auto",
                "messages": [
                    {"role": "user", "content": "hello world"},
                    {"role": "assistant", "content": "response text"}
                ]
            }"#,
        )
        .unwrap();

        let messages = parse_kiro_file(&file_path);

        assert_eq!(messages.len(), 1, "file-based Kiro source unchanged");
        let m = &messages[0];
        assert_eq!(m.client, CLIENT_ID);
        assert_eq!(m.provider_id, PROVIDER_ID);
        assert_eq!(m.model_id, "auto");
        assert!(m.tokens.input > 0, "file-based input estimate preserved");
        assert!(m.tokens.output > 0, "file-based output estimate preserved");
        assert_eq!(m.tokens.cache_read, 0);
        assert_eq!(m.tokens.cache_write, 0);
        assert_eq!(m.tokens.reasoning, 0);
        assert_eq!(m.workspace_key, Some("workspace-a".to_string()));
        assert_eq!(
            m.dedup_key,
            Some("workspace-a/execution:globalstorage".to_string())
        );
    }

    #[test]
    fn test_parse_kiro_global_storage_chat_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-a/execution.chat",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            r#"{
                "model": "auto",
                "messages": [
                    {"role": "user", "content": "hello world"},
                    {"role": "assistant", "content": "response text"}
                ]
            }"#,
        )
        .unwrap();

        let messages = parse_kiro_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "kiro");
        assert_eq!(messages[0].model_id, "auto");
        assert!(messages[0].tokens.input > 0);
        assert!(messages[0].tokens.output > 0);
        // (4a) Workspace attribution: the `<workspace>` segment after
        // `kiro.kiroagent/` flows through the same workspace helpers.
        assert_eq!(messages[0].workspace_key, Some("workspace-a".to_string()));
        assert_eq!(messages[0].workspace_label, Some("workspace-a".to_string()));
        assert_eq!(
            messages[0].dedup_key,
            Some("workspace-a/execution:globalstorage".to_string())
        );
    }

    #[test]
    fn test_parse_kiro_execution_file_attributes_workspace_model_and_duration() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-a/execution-123.json",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            r#"{
                "executionId": "exec-123",
                "chatSessionId": "chat-abc",
                "status": "succeed",
                "startTime": 1770983426000,
                "endTime": 1770983427500,
                "completionOptions": {"modelId": "claude-sonnet-4-5"},
                "actions": [
                    {"actionType": "say", "output": "the assistant replied with a full answer"},
                    {"actionType": "reasoning", "output": {"message": "thinking it through"}}
                ],
                "context": {
                    "messages": [
                        {"entries": [{"type": "text", "text": "user asks a reasonably long question"}]}
                    ]
                }
            }"#,
        )
        .unwrap();

        let messages = parse_kiro_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "chat-abc");
        assert_eq!(
            messages[0].dedup_key,
            Some("execution:exec-123".to_string())
        );
        assert!(messages[0].tokens.input > 0);
        assert!(messages[0].tokens.output > 0);
        // Model is extracted from completionOptions, not hardcoded to "auto".
        assert_eq!(messages[0].model_id, "claude-sonnet-4-5");
        // Workspace attribution matches the snapshot path.
        assert_eq!(messages[0].workspace_key, Some("workspace-a".to_string()));
        assert_eq!(messages[0].workspace_label, Some("workspace-a".to_string()));
        // Duration is carried through (endTime - startTime = 1500ms).
        assert_eq!(messages[0].duration_ms, Some(1500));
    }

    #[test]
    fn test_parse_kiro_execution_file_parses_seconds_epoch_start_time() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-a/execution-secs.json",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        // startTime as an epoch-seconds integer must be scaled to ms, not read
        // as a millisecond value (which would file it under 1970).
        fs::write(
            &file_path,
            r#"{
                "executionId": "exec-secs",
                "status": "succeed",
                "startTime": 1770983426,
                "actions": [{"actionType": "say", "output": "answer text here"}],
                "context": {
                    "messages": [
                        {"entries": [{"type": "text", "text": "a question from the user"}]}
                    ]
                }
            }"#,
        )
        .unwrap();

        let messages = parse_kiro_file(&file_path);

        assert_eq!(messages.len(), 1);
        // 1770983426 seconds -> 1770983426000 ms -> 2026, not 1970.
        assert_eq!(messages[0].timestamp, 1770983426000);
        assert!(messages[0].date.starts_with("2026-"));
    }

    fn make_globalstorage_message(
        session_id: &str,
        dedup_key: &str,
        workspace: Option<&str>,
    ) -> UnifiedMessage {
        let mut message = UnifiedMessage::new_with_dedup(
            CLIENT_ID,
            "auto".to_string(),
            PROVIDER_ID,
            session_id.to_string(),
            1_770_983_426_000,
            TokenBreakdown {
                input: 100,
                output: 10,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
            Some(dedup_key.to_string()),
        );
        message.set_workspace(workspace.map(str::to_string), workspace.map(str::to_string));
        message
    }

    #[test]
    fn suppress_snapshots_covered_by_executions_drops_only_exact_matches() {
        let messages = vec![
            // Snapshot for chat-abc in workspace-a: covered by the execution below.
            make_globalstorage_message(
                "workspace-a/chat-abc",
                "workspace-a/chat-abc:globalstorage",
                Some("workspace-a"),
            ),
            // Execution for the same chat session and workspace.
            make_globalstorage_message("chat-abc", "execution:exec-1", Some("workspace-a")),
            // Snapshot with a different stem: kept.
            make_globalstorage_message(
                "workspace-a/other-session",
                "workspace-a/other-session:globalstorage",
                Some("workspace-a"),
            ),
            // Same stem but different workspace: kept.
            make_globalstorage_message(
                "workspace-b/chat-abc",
                "workspace-b/chat-abc:globalstorage",
                Some("workspace-b"),
            ),
        ];

        let kept = suppress_snapshots_covered_by_executions(messages);

        let keys: Vec<&str> = kept
            .iter()
            .filter_map(|message| message.dedup_key.as_deref())
            .collect();
        assert_eq!(kept.len(), 3);
        assert!(keys.contains(&"execution:exec-1"));
        assert!(keys.contains(&"workspace-a/other-session:globalstorage"));
        assert!(keys.contains(&"workspace-b/chat-abc:globalstorage"));
        assert!(!keys.contains(&"workspace-a/chat-abc:globalstorage"));
    }

    #[test]
    fn test_parse_kiro_workspace_session_promptlogs() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-sessions/d29ya3NwYWNl/sess-uuid-1.json",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            r#"{
                "sessionId": "sess-uuid-1",
                "selectedModel": "claude-sonnet-4",
                "history": [
                    {
                        "message": {"role": "user", "content": "hello"},
                        "promptLogs": [{"prompt": "0123456789012345", "completion": "hi"}]
                    },
                    {
                        "message": {"role": "assistant", "content": "On it."},
                        "promptLogs": [{"prompt": "01234567890123456789012345678901", "completion": "done"}]
                    }
                ]
            }"#,
        )
        .unwrap();

        let messages = parse_kiro_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "sess-uuid-1");
        assert_eq!(messages[0].model_id, "claude-sonnet-4");
        // 16 + 32 prompt chars -> ceil(48 / 4) = 12 estimated input tokens.
        assert_eq!(messages[0].tokens.input, 12);
        // "On it." -> ceil(6 / 4) = 2 estimated output tokens.
        assert_eq!(messages[0].tokens.output, 2);
        assert_eq!(messages[0].message_count, 2);
        assert_eq!(
            messages[0].dedup_key,
            Some("sess-uuid-1:workspace-session".to_string())
        );
    }

    #[test]
    fn suppress_drops_workspace_session_covered_by_execution() {
        let messages = vec![
            // Workspace-session promptLogs snapshot for sess-1: covered by the
            // execution below even though the workspace keys differ (the two
            // stores live under different kiro.kiroagent subdirectories).
            make_globalstorage_message(
                "sess-1",
                "sess-1:workspace-session",
                Some("workspace-sessions"),
            ),
            // Execution whose chatSessionId is the same session UUID.
            make_globalstorage_message("sess-1", "execution:exec-9", Some("abc080c47e826767")),
            // Workspace-session for a session with no counted execution: kept.
            make_globalstorage_message(
                "sess-2",
                "sess-2:workspace-session",
                Some("workspace-sessions"),
            ),
        ];

        let kept = suppress_snapshots_covered_by_executions(messages);

        let keys: Vec<&str> = kept
            .iter()
            .filter_map(|message| message.dedup_key.as_deref())
            .collect();
        assert_eq!(kept.len(), 2);
        assert!(keys.contains(&"execution:exec-9"));
        assert!(keys.contains(&"sess-2:workspace-session"));
        assert!(!keys.contains(&"sess-1:workspace-session"));
    }

    #[test]
    fn suppress_snapshots_is_noop_without_executions() {
        let messages = vec![make_globalstorage_message(
            "workspace-a/chat-abc",
            "workspace-a/chat-abc:globalstorage",
            Some("workspace-a"),
        )];

        let kept = suppress_snapshots_covered_by_executions(messages);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn parse_kiro_chat_artifact_counts_human_and_bot_roles() {
        // Real IDE-private .chat files use human/bot/tool roles; tool context
        // is intentionally not counted.
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Kiro/User/globalStorage/kiro.kiroagent/workspace-a/0c433dc89e4c1803dd6fe838634ed7fc.chat",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            r#"{
                "executionId": "5b40545a-2539-4334-9411-23df0bfea51b",
                "actionId": "act",
                "chat": [
                    {"role": "human", "content": "please refactor the loader"},
                    {"role": "tool", "content": "You are operating in a workspace"},
                    {"role": "bot", "content": "Done, refactored."}
                ],
                "metadata": {}
            }"#,
        )
        .unwrap();

        let messages = parse_kiro_file(&file_path);

        assert_eq!(messages.len(), 1);
        // human: 26 chars -> ceil(26/4) = 7; bot: 17 chars -> ceil(17/4) = 5.
        // The 32-char tool line is excluded from both.
        assert_eq!(messages[0].tokens.input, 7);
        assert_eq!(messages[0].tokens.output, 5);
    }

    #[test]
    fn parse_kiro_chat_artifact_tags_dedup_key_with_execution_id() {
        // Shape observed in real globalStorage trees: `<hash>.chat` carries a
        // top-level executionId (and NO `actions`, so it must not be parsed as
        // an execution record).
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Kiro/User/globalStorage/kiro.kiroagent/abc080c47e826767f65b27677d791c66/006924fffc3bc58648f10379cdfd77a6.chat",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            r#"{
                "executionId": "3067e447-2cda-47c9-a476-536a72d92f31",
                "actionId": "act",
                "context": {},
                "chat": [
                    {"role": "user", "content": "please refactor the config loader"},
                    {"role": "assistant", "content": "On it."}
                ],
                "metadata": {"workflowId": "3e445aa7-f59c-4bf4-a471-c655dad734f5"}
            }"#,
        )
        .unwrap();

        let messages = parse_kiro_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some(
                "abc080c47e826767f65b27677d791c66/006924fffc3bc58648f10379cdfd77a6:globalstorage:exec:3067e447-2cda-47c9-a476-536a72d92f31"
            )
        );
    }

    #[test]
    fn suppress_snapshots_drops_chat_artifacts_matching_execution_id() {
        // Real-world id shapes: `.chat` stems are opaque 32-hex hashes, while
        // executionId/chatSessionId are dashed UUIDs — so only the executionId
        // tag can link the two.
        let ws = "abc080c47e826767f65b27677d791c66";
        let messages = vec![
            // Two .chat artifacts for the same execution: both covered.
            make_globalstorage_message(
                "abc080c47e826767f65b27677d791c66/006924fffc3bc58648f10379cdfd77a6",
                "abc080c47e826767f65b27677d791c66/006924fffc3bc58648f10379cdfd77a6:globalstorage:exec:3067e447-2cda-47c9-a476-536a72d92f31",
                Some(ws),
            ),
            make_globalstorage_message(
                "abc080c47e826767f65b27677d791c66/01e341965ac1caf00a9ecb9cc1635d62",
                "abc080c47e826767f65b27677d791c66/01e341965ac1caf00a9ecb9cc1635d62:globalstorage:exec:3067e447-2cda-47c9-a476-536a72d92f31",
                Some(ws),
            ),
            // The execution record itself (session id = chatSessionId).
            make_globalstorage_message(
                "efddf80a-eab9-4f1c-8a13-877eaac72736",
                "execution:3067e447-2cda-47c9-a476-536a72d92f31",
                Some(ws),
            ),
            // .chat artifact for an execution that is NOT counted (e.g. failed):
            // kept.
            make_globalstorage_message(
                "abc080c47e826767f65b27677d791c66/0681d950923f98601e198293ca2040fd",
                "abc080c47e826767f65b27677d791c66/0681d950923f98601e198293ca2040fd:globalstorage:exec:5b40545a-2539-4334-9411-23df0bfea51b",
                Some(ws),
            ),
            // Same execution id but a different workspace: kept.
            make_globalstorage_message(
                "other-ws/aaaa",
                "other-ws/aaaa:globalstorage:exec:3067e447-2cda-47c9-a476-536a72d92f31",
                Some("other-ws"),
            ),
        ];

        let kept = suppress_snapshots_covered_by_executions(messages);

        let keys: Vec<&str> = kept
            .iter()
            .filter_map(|message| message.dedup_key.as_deref())
            .collect();
        assert_eq!(kept.len(), 3);
        assert!(keys.contains(&"execution:3067e447-2cda-47c9-a476-536a72d92f31"));
        assert!(keys.iter().any(|key| key.contains("0681d950")));
        assert!(keys.iter().any(|key| key.starts_with("other-ws/aaaa")));
        assert!(!keys.iter().any(|key| key.contains("006924ff")));
        assert!(!keys.iter().any(|key| key.contains("01e34196")));
    }

    #[test]
    fn test_parse_kiro_global_storage_dedup_keys_differ_across_workspaces() {
        let dir = TempDir::new().unwrap();
        let payload = r#"{
                "model": "auto",
                "messages": [
                    {"role": "user", "content": "hello world"},
                    {"role": "assistant", "content": "response text"}
                ]
            }"#;

        // Two `execution.chat` snapshots under DIFFERENT workspaces.
        let path_a = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-a/execution.chat",
        );
        let path_b = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-b/execution.chat",
        );
        fs::create_dir_all(path_a.parent().unwrap()).unwrap();
        fs::create_dir_all(path_b.parent().unwrap()).unwrap();
        fs::write(&path_a, payload).unwrap();
        fs::write(&path_b, payload).unwrap();

        let messages_a = parse_kiro_file(&path_a);
        let messages_b = parse_kiro_file(&path_b);

        assert_eq!(messages_a.len(), 1);
        assert_eq!(messages_b.len(), 1);
        assert_ne!(messages_a[0].dedup_key, messages_b[0].dedup_key);
        assert_eq!(
            messages_a[0].dedup_key,
            Some("workspace-a/execution:globalstorage".to_string())
        );
        assert_eq!(
            messages_b[0].dedup_key,
            Some("workspace-b/execution:globalstorage".to_string())
        );
    }

    #[test]
    fn test_parse_kiro_global_storage_ignores_unknown_roles() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-a/execution.chat",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            r#"{
                "model": "auto",
                "messages": [
                    {"role": "mystery", "content": "mystery text"},
                    {"role": "assistant", "content": "response text"}
                ]
            }"#,
        )
        .unwrap();

        let messages = parse_kiro_file(&file_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.output, 4);
    }

    #[test]
    fn test_collect_kiro_snapshot_text_does_not_double_count_aliased_keys() {
        // (a) A single message object that stores the SAME assistant body under
        // two aliased text keys (`content` and `text`). Before the fix, the
        // traversal descended into every present alias and counted "response
        // text" twice (8 assistant chars -> output 2). After the fix it descends
        // into only the first present alias in the group, counting once.
        let value: Value = serde_json::from_str(
            r#"{
                "messages": [
                    {"role": "assistant", "content": "abcd", "text": "abcd"}
                ]
            }"#,
        )
        .unwrap();

        let mut counts = KiroSnapshotTextCounts::default();
        collect_kiro_snapshot_text(&value, &mut counts, None);

        // "abcd" counted once = 4 chars, not 8.
        assert_eq!(counts.assistant_chars, 4);
        assert_eq!(counts.prompt_chars, 0);
    }

    #[test]
    fn test_collect_kiro_snapshot_text_does_not_double_count_aliased_containers() {
        // (a) An object that stores the SAME conversation list under two aliased
        // container keys (`messages` and `entries`). Before the fix both were
        // traversed and the text was counted twice.
        let value: Value = serde_json::from_str(
            r#"{
                "messages": [{"role": "user", "content": "hello"}],
                "entries": [{"role": "user", "content": "hello"}]
            }"#,
        )
        .unwrap();

        let mut counts = KiroSnapshotTextCounts::default();
        collect_kiro_snapshot_text(&value, &mut counts, None);

        // "hello" counted once = 5 chars, not 10.
        assert_eq!(counts.prompt_chars, 5);
        assert_eq!(counts.assistant_chars, 0);
    }

    #[test]
    fn test_collect_kiro_snapshot_text_counts_distinct_alias_subtrees() {
        // A single turn that stores DISTINCT payloads under two keys of the same
        // alias group: `prompt` (user text) and `response` (assistant text).
        // These are different subtrees, so both must be counted. A first-key-only
        // traversal would drop the `response` body and undercount.
        let value: Value = serde_json::from_str(
            r#"{
                "prompt": {"role": "user", "text": "hi there"},
                "response": {"role": "assistant", "text": "hello back"}
            }"#,
        )
        .unwrap();

        let mut counts = KiroSnapshotTextCounts::default();
        collect_kiro_snapshot_text(&value, &mut counts, None);

        // "hi there" = 8 prompt chars, "hello back" = 10 assistant chars.
        assert_eq!(counts.prompt_chars, 8);
        assert_eq!(counts.assistant_chars, 10);
    }

    #[test]
    fn test_collect_kiro_snapshot_text_counts_distinct_container_subtrees() {
        // A chat object holding DISTINCT conversation lists under two container
        // aliases (`messages` and `history`). Both must be counted; the
        // value-based de-dup only skips structurally identical subtrees.
        let value: Value = serde_json::from_str(
            r#"{
                "messages": [{"role": "user", "content": "alpha"}],
                "history": [{"role": "user", "content": "bravo"}]
            }"#,
        )
        .unwrap();

        let mut counts = KiroSnapshotTextCounts::default();
        collect_kiro_snapshot_text(&value, &mut counts, None);

        // "alpha" (5) + "bravo" (5) = 10 prompt chars; nothing dropped.
        assert_eq!(counts.prompt_chars, 10);
        assert_eq!(counts.assistant_chars, 0);
    }

    #[test]
    fn test_find_kiro_snapshot_model_id_descends_into_aliased_text_keys() {
        // (b) Model id nested under `parts` / `prompt` — keys that
        // `collect_kiro_snapshot_text` descends into but the model-id finder
        // previously omitted, causing the model to fall back to `unknown`.
        let parts_value: Value = serde_json::from_str(
            r#"{
                "messages": [
                    {"parts": [{"model_id": "claude-sonnet-4-5"}]}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(
            find_kiro_snapshot_model_id(&parts_value),
            Some("claude-sonnet-4-5".to_string())
        );

        let prompt_value: Value =
            serde_json::from_str(r#"{"prompt": {"model": "claude-sonnet-4"}}"#).unwrap();
        assert_eq!(
            find_kiro_snapshot_model_id(&prompt_value),
            Some("claude-sonnet-4".to_string())
        );
    }

    /// Build the Kiro IDE session layout on disk:
    /// `<base>/.kiro/sessions/<workspace>/<sess_dir>/{session.json,messages.jsonl}`
    /// and return the path to `session.json`.
    fn create_ide_session_files(
        dir: &TempDir,
        workspace: &str,
        sess_dir: &str,
        session_json: &str,
        messages_jsonl: &str,
    ) -> std::path::PathBuf {
        let sess_path = dir
            .path()
            .join(".kiro/sessions")
            .join(workspace)
            .join(sess_dir);
        fs::create_dir_all(&sess_path).unwrap();
        let session_path = sess_path.join("session.json");
        fs::write(&session_path, session_json).unwrap();
        fs::write(sess_path.join("messages.jsonl"), messages_jsonl).unwrap();
        session_path
    }

    #[test]
    fn test_parse_kiro_ide_session_estimates_tokens_from_messages_jsonl() {
        // session.json is the schemaVersion 1.0.0 sample from issue #813.
        let session_json = r#"{
            "schemaVersion": "1.0.0",
            "dataModelVersion": 1,
            "id": "sess_02f1c107-37e8-4398-8b95-c3847bf59335",
            "title": "Writing README docs for projects",
            "agentMode": "vibe",
            "createdAt": "2026-06-30T12:57:10.991Z",
            "lastModifiedAt": "2026-06-30T12:57:12.991Z",
            "status": "completed"
        }"#;
        let messages_jsonl = "{\"role\":\"user\",\"content\":\"hello world\"}\n{\"role\":\"assistant\",\"content\":\"response text\"}\n";

        let dir = TempDir::new().unwrap();
        let path = create_ide_session_files(
            &dir,
            "my-project",
            "sess_02f1c107-37e8-4398-8b95-c3847bf59335",
            session_json,
            messages_jsonl,
        );

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "kiro");
        assert_eq!(messages[0].provider_id, "amazon-bedrock");
        assert_eq!(
            messages[0].session_id,
            "sess_02f1c107-37e8-4398-8b95-c3847bf59335"
        );
        // "hello world" = 11 chars -> ceil(11/4) = 3; "response text" = 13 -> 4.
        assert_eq!(messages[0].tokens.input, 3);
        assert_eq!(messages[0].tokens.output, 4);
        assert!(messages[0].is_turn_start);
        // Workspace is the folder holding the sess_* dir.
        assert_eq!(messages[0].workspace_key, Some("my-project".to_string()));
        assert_eq!(messages[0].workspace_label, Some("my-project".to_string()));
        // createdAt -> ms; duration = lastModifiedAt - createdAt = 2000ms.
        assert_eq!(messages[0].timestamp, 1782824230991);
        assert_eq!(messages[0].duration_ms, Some(2000));
        assert!(messages[0].date.starts_with("2026-"));
        // No model in session.json/messages.jsonl -> "auto" so pricing can resolve.
        assert_eq!(messages[0].model_id, "auto");
        // Dedup key is IDE-session-scoped and survives execution suppression.
        assert_eq!(
            messages[0].dedup_key,
            Some("sess_02f1c107-37e8-4398-8b95-c3847bf59335:ide-session".to_string())
        );
        // One assistant response -> message_count 1.
        assert_eq!(messages[0].message_count, 1);
    }

    #[test]
    fn test_parse_kiro_ide_session_extracts_model_and_counts_turns() {
        let session_json = r#"{
            "schemaVersion": "1.0.0",
            "id": "sess_abc",
            "createdAt": "2026-06-30T12:57:10.000Z",
            "lastModifiedAt": "2026-06-30T12:57:10.000Z"
        }"#;
        // Two assistant turns and a Kiro IDE model codename embedded in a line.
        let messages_jsonl = concat!(
            "{\"role\":\"user\",\"content\":\"first question here\"}\n",
            "{\"role\":\"assistant\",\"model\":\"big-pickle\",\"content\":\"first answer\"}\n",
            "{\"role\":\"user\",\"content\":\"second question\"}\n",
            "{\"role\":\"assistant\",\"content\":\"second answer\"}\n"
        );

        let dir = TempDir::new().unwrap();
        let path = create_ide_session_files(&dir, "ws", "sess_abc", session_json, messages_jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        // Model codename is picked up from messages.jsonl (not a Kiro-internal id).
        assert_eq!(messages[0].model_id, "big-pickle");
        // Two assistant responses -> message_count 2.
        assert_eq!(messages[0].message_count, 2);
        assert!(messages[0].tokens.input > 0);
        assert!(messages[0].tokens.output > 0);
    }

    #[test]
    fn test_structured_turn_missing_prompt_timestamp_back_calculates_from_elapsed_time() {
        // Second-round review fix: in the structured `messages.jsonl` layout,
        // `usage_summary.elapsedTime` can supply `duration_ms` while the user
        // prompt's own timestamp is absent (or unparseable). Previously the
        // message timestamp fell back to the `turn_end` event's own
        // timestamp, leaving the message end-anchored — sessionize()'s
        // `[timestamp, timestamp + duration_ms]` span would then project
        // forward past the turn's actual end into phantom idle time. The
        // parser must back-calculate `turn_end - elapsedTime` as the anchor
        // instead.
        let session_json = r#"{
            "schemaVersion": "1.0.0",
            "id": "sess_structured"
        }"#;
        let messages_jsonl = concat!(
            "{\"payload\":{\"type\":\"user\",\"content\":\"hello world\"}}\n",
            "{\"payload\":{\"type\":\"assistant\",\"content\":\"response text\"}}\n",
            "{\"payload\":{\"type\":\"usage_summary\",\"elapsedTime\":5000}}\n",
            "{\"payload\":{\"type\":\"turn_end\"},\"timestamp\":\"2026-06-20T10:00:05Z\"}\n",
        );

        let dir = TempDir::new().unwrap();
        let path =
            create_ide_session_files(&dir, "ws", "sess_structured", session_json, messages_jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        let expected_end = chrono::DateTime::parse_from_rfc3339("2026-06-20T10:00:05Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            messages[0].timestamp,
            expected_end - 5000,
            "timestamp must be back-calculated from turn_end - elapsedTime when the prompt timestamp is missing"
        );
        assert_eq!(messages[0].duration_ms, Some(5000));
    }

    #[test]
    fn test_parse_kiro_ide_session_dropped_when_no_recognizable_content() {
        let session_json = r#"{"schemaVersion":"1.0.0","id":"sess_empty"}"#;
        // Only tool/system noise with no role-tagged conversation text.
        let messages_jsonl = "{\"kind\":\"toolCall\",\"name\":\"read_file\"}\n";

        let dir = TempDir::new().unwrap();
        let path = create_ide_session_files(&dir, "ws", "sess_empty", session_json, messages_jsonl);

        let messages = parse_kiro_file(&path);

        // No estimable usage -> no fabricated message.
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_kiro_ide_session_falls_back_to_dir_name_and_mtime() {
        // session.json with no id and no timestamps: session id falls back to the
        // sess_* directory name and timestamp to the file mtime.
        let session_json = r#"{"schemaVersion":"1.0.0"}"#;
        let messages_jsonl = "{\"role\":\"user\",\"content\":\"hello\"}\n";

        let dir = TempDir::new().unwrap();
        let path =
            create_ide_session_files(&dir, "ws", "sess_no_meta", session_json, messages_jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "sess_no_meta");
        assert_eq!(
            messages[0].dedup_key,
            Some("sess_no_meta:ide-session".to_string())
        );
        assert!(messages[0].timestamp > 0);
    }

    #[test]
    fn suppress_snapshots_leaves_ide_sessions_untouched() {
        // An IDE-session message must never be dropped by execution suppression,
        // even when a globalStorage execution is present in the same batch.
        let messages = vec![
            make_globalstorage_message("chat-abc", "execution:exec-1", Some("ws")),
            make_globalstorage_message("sess_abc", "sess_abc:ide-session", Some("ws")),
        ];

        let kept = suppress_snapshots_covered_by_executions(messages);

        let keys: Vec<&str> = kept
            .iter()
            .filter_map(|message| message.dedup_key.as_deref())
            .collect();
        assert_eq!(kept.len(), 2);
        assert!(keys.contains(&"sess_abc:ide-session"));
        assert!(keys.contains(&"execution:exec-1"));
    }

    // ---------------------------------------------------------------------
    // Unit tests for the mapping changes (Task 3.3).
    //
    // These cover the gaps NOT already exercised by the Task 1 exploration
    // tests (which cover real-output/cache-read/reasoning/request-count
    // surfacing and the nested-token_usage-when-flat-absent case):
    //   - flat fields take precedence over nested token_usage when both present
    //   - negative / zero counts fall back to the estimate/zero and never
    //     produce negative tokens
    //   - cache_write_input_tokens maps into cacheWrite in isolation
    //   - real input_tokens overrides the context_usage_percentage estimate in
    //     isolation
    //   - real output_tokens overrides the response_size estimate in isolation
    //   - request_count applies a .max(1) floor
    // All run against the FIXED parser and must PASS.
    // ---------------------------------------------------------------------

    // Requirement 2.1 / 3.1: a real output_tokens count overrides the
    // response_size / 4 estimate. In isolation (no other real counts), input
    // still comes from the context-percentage estimate.
    #[test]
    fn test_kiro_sqlite_real_output_overrides_response_size_estimate() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "output_tokens": 512,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(
            m.tokens.output, 512,
            "real output_tokens overrides response_size/4 (10)"
        );
        // No real input count -> hybrid estimate: fresh input is 0 (no user
        // content) and the cumulative context (1000 * 10 / 100 = 100) becomes
        // cache_read.
        assert_eq!(
            m.tokens.input, 0,
            "input is fresh-only when no real count present"
        );
        assert_eq!(
            m.tokens.cache_read, 100,
            "cumulative context becomes cache_read"
        );
        assert_eq!(m.tokens.cache_write, 0);
        assert_eq!(m.tokens.reasoning, 0);
        assert_eq!(m.message_count, 1);
    }

    // Requirement 2.6 / 3.2: a real input_tokens count overrides the
    // context_usage_percentage * context_window estimate. In isolation, output
    // still comes from the response_size estimate.
    #[test]
    fn test_kiro_sqlite_real_input_overrides_context_percentage_estimate() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "input_tokens": 7777,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(
            m.tokens.input, 7777,
            "real input_tokens overrides context-percentage estimate (100)"
        );
        // Output still estimated: 40 / 4 = 10.
        assert_eq!(
            m.tokens.output, 10,
            "output still comes from response_size estimate"
        );
        assert_eq!(m.message_count, 1);
    }

    // Requirement 2.3: cache_write_input_tokens maps into cacheWrite in
    // isolation (Task 1 covers cache_read and reasoning individually, but not
    // cache_write on its own).
    #[test]
    fn test_kiro_sqlite_cache_write_count_surfaced_in_isolation() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "cache_write_input_tokens": 256,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(
            m.tokens.cache_write, 256,
            "recorded cache_write_input_tokens surfaced"
        );
        // No real cache_read count -> hybrid cache_read from cumulative context
        // (1000 * 10 / 100 = 100, with fresh input 0 for content-less fixture).
        assert_eq!(
            m.tokens.cache_read, 100,
            "cache_read falls back to cumulative context when absent"
        );
        assert_eq!(m.tokens.reasoning, 0, "reasoning stays 0 when absent");
    }

    // Requirement 2.5: message_count applies a .max(1) floor. A recorded
    // request_count of 0 must not drop message_count below 1.
    #[test]
    fn test_kiro_sqlite_request_count_zero_floored_to_one() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "request_count": 0,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].message_count, 1,
            "message_count = MAX(request_count, 1) = 1 when request_count is 0"
        );
    }

    // Flat fields take precedence over nested token_usage when BOTH are present
    // (design change 1: "Flat fields take precedence when both are present").
    #[test]
    fn test_kiro_sqlite_flat_fields_take_precedence_over_nested() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "input_tokens": 111,
            "output_tokens": 222,
            "cache_read_input_tokens": 333,
            "cache_write_input_tokens": 444,
            "reasoning_tokens": 555,
            "request_count": 6,
            "token_usage": {
                "input_tokens": 1,
                "output_tokens": 2,
                "cache_read_input_tokens": 3,
                "cache_write_input_tokens": 4,
                "reasoning_tokens": 5,
                "request_count": 99
            },
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.tokens.input, 111, "flat input_tokens wins over nested");
        assert_eq!(m.tokens.output, 222, "flat output_tokens wins over nested");
        assert_eq!(m.tokens.cache_read, 333, "flat cache_read wins over nested");
        assert_eq!(
            m.tokens.cache_write, 444,
            "flat cache_write wins over nested"
        );
        assert_eq!(m.tokens.reasoning, 555, "flat reasoning wins over nested");
        assert_eq!(m.message_count, 6, "flat request_count wins over nested");
    }

    // Edge (Requirement 3.1/3.2 fallback + non-negative clamp): zero real
    // counts are treated as absent (filtered to > 0), so input/output fall back
    // to the estimate and cache/reasoning stay 0.
    #[test]
    fn test_kiro_sqlite_zero_counts_fall_back_to_estimate_and_zero() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "input_tokens": 0,
            "output_tokens": 0,
            "cache_read_input_tokens": 0,
            "cache_write_input_tokens": 0,
            "reasoning_tokens": 0,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        // Zero real counts are ignored -> hybrid estimate: fresh input 0 (no
        // user content), cumulative context (100) -> cache_read, output 40/4.
        assert_eq!(
            m.tokens.input, 0,
            "zero input_tokens falls back to fresh estimate (0)"
        );
        assert_eq!(
            m.tokens.output, 10,
            "zero output_tokens falls back to estimate (10)"
        );
        assert_eq!(
            m.tokens.cache_read, 100,
            "cumulative context becomes cache_read"
        );
        assert_eq!(m.tokens.cache_write, 0);
        assert_eq!(m.tokens.reasoning, 0);
    }

    // Edge (non-negative clamp): negative real counts must never produce
    // negative tokens. Negative input/output are filtered out (> 0 guard) so
    // they fall back to the estimate; negative cache/reasoning clamp to 0.
    #[test]
    fn test_kiro_sqlite_negative_counts_never_produce_negative_tokens() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "input_tokens": -5,
            "output_tokens": -9,
            "cache_read_input_tokens": -100,
            "cache_write_input_tokens": -200,
            "reasoning_tokens": -300,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let messages = parse_single_kiro_sqlite_turn(meta);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        // Negative input/output are not > 0, so the hybrid estimate is used:
        // fresh input 0 (no user content), output 40/4.
        assert_eq!(
            m.tokens.input, 0,
            "negative input_tokens falls back to fresh estimate"
        );
        assert_eq!(
            m.tokens.output, 10,
            "negative output_tokens falls back to estimate"
        );
        // Tokens are never negative. Negative real cache_read is treated as
        // absent, so cache_read falls back to the cumulative context (100);
        // negative cache_write/reasoning clamp to 0.
        assert!(m.tokens.cache_read >= 0, "cache_read never negative");
        assert!(m.tokens.cache_write >= 0, "cache_write never negative");
        assert!(m.tokens.reasoning >= 0, "reasoning never negative");
        assert_eq!(
            m.tokens.cache_read, 100,
            "negative cache_read falls back to cumulative context"
        );
        assert_eq!(m.tokens.cache_write, 0, "negative cache_write clamped to 0");
        assert_eq!(m.tokens.reasoning, 0, "negative reasoning clamped to 0");
    }

    // ---------------------------------------------------------------------
    // Fix Checking property test (Task 4.1).
    //
    // **Property 1: Bug Condition** - Full token breakdown and request count
    // surfaced.
    //
    // Encodes the design Fix Checking pseudocode `FOR ALL X WHERE
    // isBugCondition(X)`: for every turn whose persisted metadata carries a
    // real count (or a request count > 1), the fixed parser must surface each
    // recorded real count in its correct TokenBreakdown field and set
    // message_count = MAX(request_count, 1). When a particular real count is
    // absent, that field alone must fall back to the estimate/zero behavior
    // (per-field independence).
    //
    // Consistent with Task 2's Preservation property test, no proptest/
    // quickcheck crate is a dev-dependency and the project prefers a
    // deterministic table-driven generator over adding one. The domain-spanning
    // "for all buggy metadata" property is therefore exercised by a
    // deterministic generator (`bug_condition_metadata_cases`) that enumerates
    // and combines real-count presence and values — spanning present-positive,
    // absent, and mixed-presence — rather than a randomized PBT harness.
    // ---------------------------------------------------------------------

    // Context window used by parse_single_kiro_sqlite_turn (mirrors the
    // preservation tests' PRESERVATION_CONTEXT_WINDOW).
    const FIX_CONTEXT_WINDOW: i64 = 1000;

    // A single generated buggy-turn shape: each real count is Some(value) when
    // present, None when absent. The legacy estimate inputs (ctx_pct,
    // response_size) are fixed to positive values so a per-field fallback is
    // observable and the resolved turn is never skipped.
    #[derive(Clone, Copy, Debug)]
    struct BugCase {
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        cache_read: Option<i64>,
        cache_write: Option<i64>,
        reasoning: Option<i64>,
        request_count: Option<i32>,
    }

    // Fixed legacy estimate inputs for every generated case:
    //   ctx_pct = 10  -> input estimate  = 1000 * 10 / 100 = 100
    //   response_size = 40 -> output estimate = ceil(40 / 4) = 10
    const FIX_CTX_PCT: f64 = 10.0;
    const FIX_RESPONSE_SIZE: usize = 40;

    fn fix_input_estimate() -> i64 {
        ((FIX_CONTEXT_WINDOW as f64) * FIX_CTX_PCT / 100.0) as i64
    }

    fn fix_output_estimate() -> i64 {
        estimate_tokens(FIX_RESPONSE_SIZE)
    }

    // Build a request_metadata JSON object carrying the two legacy estimate
    // fields plus every real count that is Some(...) in `case`. Absent fields
    // are omitted so serde sees them as missing (None), exercising the
    // per-field estimate/zero fallback.
    fn bug_condition_meta_json(case: BugCase) -> String {
        let mut fields: Vec<String> = vec![
            format!("\"context_usage_percentage\": {}", FIX_CTX_PCT),
            format!("\"response_size\": {}", FIX_RESPONSE_SIZE),
            "\"request_start_timestamp_ms\": 1770983426000".to_string(),
            "\"stream_end_timestamp_ms\": 1770983427500".to_string(),
        ];
        if let Some(v) = case.input_tokens {
            fields.push(format!("\"input_tokens\": {}", v));
        }
        if let Some(v) = case.output_tokens {
            fields.push(format!("\"output_tokens\": {}", v));
        }
        if let Some(v) = case.cache_read {
            fields.push(format!("\"cache_read_input_tokens\": {}", v));
        }
        if let Some(v) = case.cache_write {
            fields.push(format!("\"cache_write_input_tokens\": {}", v));
        }
        if let Some(v) = case.reasoning {
            fields.push(format!("\"reasoning_tokens\": {}", v));
        }
        if let Some(v) = case.request_count {
            fields.push(format!("\"request_count\": {}", v));
        }
        format!("{{ {} }}", fields.join(", "))
    }

    // isBugCondition(input): true when any real count is present-positive, or
    // the recorded request count is > 1 (mirrors the design predicate). Used to
    // scope the generated domain to `FOR ALL X WHERE isBugCondition(X)`.
    fn is_bug_condition(case: BugCase) -> bool {
        let pos = |v: Option<i64>| v.is_some_and(|x| x > 0);
        pos(case.input_tokens)
            || pos(case.output_tokens)
            || pos(case.cache_read)
            || pos(case.cache_write)
            || pos(case.reasoning)
            || case.request_count.is_some_and(|c| c > 1)
    }

    // Deterministic enumeration of the buggy-turn input domain. Combines:
    //   - an all-present case (every real count set, request_count > 1),
    //   - mixed-presence cases (each real count present in isolation while the
    //     rest are absent), and
    //   - request_count-only cases (> 1 and the .max(1) floor edge at 1),
    // spanning present-positive / absent for each field. This is the
    // table-driven stand-in for "FOR ALL buggy metadata".
    fn bug_condition_metadata_cases() -> Vec<BugCase> {
        let none = BugCase {
            input_tokens: None,
            output_tokens: None,
            cache_read: None,
            cache_write: None,
            reasoning: None,
            request_count: None,
        };

        // All real counts present together, with a multi-request count.
        let mut cases = vec![BugCase {
            input_tokens: Some(7777),
            output_tokens: Some(512),
            cache_read: Some(1920),
            cache_write: Some(64),
            reasoning: Some(40),
            request_count: Some(12),
        }];

        // Each real count present in isolation (the rest absent), asserting
        // per-field independence: the present field uses the real value, every
        // absent field uses its estimate/zero fallback.
        cases.push(BugCase {
            input_tokens: Some(654),
            ..none
        });
        cases.push(BugCase {
            output_tokens: Some(321),
            ..none
        });
        cases.push(BugCase {
            cache_read: Some(1920),
            ..none
        });
        cases.push(BugCase {
            cache_write: Some(256),
            ..none
        });
        cases.push(BugCase {
            reasoning: Some(48),
            ..none
        });
        cases.push(BugCase {
            request_count: Some(9),
            ..none
        });

        // Mixed presence: two fields present, the rest absent.
        cases.push(BugCase {
            output_tokens: Some(200),
            reasoning: Some(15),
            ..none
        });
        cases.push(BugCase {
            input_tokens: Some(1000),
            cache_write: Some(300),
            ..none
        });
        cases.push(BugCase {
            cache_read: Some(500),
            request_count: Some(3),
            ..none
        });

        // request_count = 1 is the .max(1) floor edge; combine with a real
        // count so the turn is still a bug condition and is not skipped.
        cases.push(BugCase {
            output_tokens: Some(77),
            request_count: Some(1),
            ..none
        });

        cases
    }

    // Property 1 (Fix Checking) over the buggy-turn domain: for every generated
    // case where isBugCondition holds, each present real count surfaces in its
    // TokenBreakdown field, each absent count falls back to the estimate/zero
    // behavior for that field only, and message_count = MAX(request_count, 1).
    //
    // Validates: Requirements 2.1, 2.2, 2.3, 2.4, 2.5, 2.6
    #[test]
    fn test_kiro_sqlite_fix_checking_property_bug_condition_surfaced() {
        for case in bug_condition_metadata_cases() {
            // Scope to the design's FOR ALL X WHERE isBugCondition(X).
            assert!(
                is_bug_condition(case),
                "generated case must be a bug condition; case = {case:?}"
            );

            let meta = bug_condition_meta_json(case);
            let messages = parse_single_kiro_sqlite_turn(&meta);

            // Every generated case resolves to non-zero input+output (the legacy
            // estimates are positive and real input/output only override them),
            // so exactly one entry is emitted.
            assert_eq!(
                messages.len(),
                1,
                "buggy turn should emit exactly one entry; case = {case:?}"
            );
            let m = &messages[0];

            // Per-field independence: present real input surfaces exactly.
            // When absent, input falls back to the hybrid fresh-input estimate,
            // which is 0 for these content-less fixtures.
            let expected_input = match case.input_tokens {
                Some(v) if v > 0 => v,
                _ => 0,
            };
            assert_eq!(
                m.tokens.input, expected_input,
                "input: present real count must surface, absent must be fresh estimate; case = {case:?}"
            );

            // Present real output overrides the response_size estimate; absent
            // output falls back to it.
            let expected_output = match case.output_tokens {
                Some(v) if v > 0 => v,
                _ => fix_output_estimate(),
            };
            assert_eq!(
                m.tokens.output, expected_output,
                "output: present real count must surface, absent must estimate; case = {case:?}"
            );

            // cache_read surfaces the recorded count when present-positive.
            // When absent AND no real input count is present, it falls back to
            // the hybrid cumulative context (max(total_context - fresh_input,
            // 0)); with a real input count present, no hybrid runs so it is 0.
            let hybrid_cache_read = (fix_input_estimate() - expected_input).max(0);
            let real_input_present = matches!(case.input_tokens, Some(v) if v > 0);
            let expected_cache_read = match case.cache_read.filter(|&v| v > 0) {
                Some(v) => v.max(0),
                None if real_input_present => 0,
                None => hybrid_cache_read,
            };
            assert_eq!(
                m.tokens.cache_read, expected_cache_read,
                "cache_read: present count surfaces, else hybrid cumulative context; case = {case:?}"
            );
            let expected_cache_write = case.cache_write.filter(|&v| v > 0).unwrap_or(0).max(0);
            assert_eq!(
                m.tokens.cache_write, expected_cache_write,
                "cache_write: present real count must surface, absent must be 0; case = {case:?}"
            );
            let expected_reasoning = case.reasoning.filter(|&v| v > 0).unwrap_or(0).max(0);
            assert_eq!(
                m.tokens.reasoning, expected_reasoning,
                "reasoning: present real count must surface, absent must be 0; case = {case:?}"
            );

            // message_count = MAX(recordedRequestCount, 1).
            let expected_message_count: i32 = case.request_count.unwrap_or(1).max(1);
            assert_eq!(
                m.message_count, expected_message_count,
                "message_count must equal MAX(request_count, 1); case = {case:?}"
            );
        }
    }

    // ---------------------------------------------------------------------
    // Preservation Checking property test (Task 4.2).
    //
    // **Property 2: Preservation** - Estimates, skips, and metadata unchanged.
    //
    // Encodes the design Preservation Checking pseudocode `FOR ALL X WHERE NOT
    // isBugCondition(X)`: for every turn whose persisted metadata carries NO
    // real token/request counts (only the four legacy fields
    // context_usage_percentage, response_size, request_start_timestamp_ms,
    // stream_end_timestamp_ms, with arbitrary values), the FIXED parser must
    // reproduce today's estimate/skip/message_count = 1 output exactly —
    // including cache_read = cache_write = reasoning = 0.
    //
    // Relationship to Task 2: Task 2's
    // `test_kiro_sqlite_preservation_legacy_only_estimates_and_skip` established
    // the same baseline by RUNNING THE UNFIXED PARSER over the legacy-only
    // domain (observation-first). That test locked the values the fix must
    // preserve. This Task 4.2 test intentionally reuses the identical
    // generator infrastructure (`legacy_only_metadata_cases`,
    // `legacy_only_meta_json`, `expected_legacy_estimate`) but formalizes the
    // FOR ALL NOT isBugCondition(X) framing against the NOW-FIXED parser: it
    // asserts the fix did not regress any legacy-only turn. Together the two
    // tests bracket the property before and after the fix. No new dependency is
    // added; the deterministic table-driven generator remains the domain-
    // spanning stand-in for "FOR ALL legacy-only metadata", consistent with the
    // rest of this module.
    // ---------------------------------------------------------------------

    // NOT isBugCondition for a legacy-only shape: these cases carry only the
    // four legacy fields, so no real input/output/cache/reasoning count and no
    // request count are ever present. isBugCondition is therefore vacuously
    // false for the entire generated domain — exactly the NOT-bug-condition
    // half of the input space the design's Preservation Checking quantifies
    // over. Asserted explicitly below to scope the property.
    fn legacy_only_is_not_bug_condition() -> bool {
        // A legacy-only metadata JSON carries none of the real-count fields, so
        // by construction the bug predicate cannot hold. Encoded as a constant
        // to document the scoping precondition of this property.
        true
    }

    // Property 2 (Preservation Checking) over the NOT-isBugCondition domain: for
    // every legacy-only metadata shape (arbitrary values for the four legacy
    // fields), the fixed parser reproduces the original estimate/skip behavior
    // exactly — same input/output estimates, same zero-token skip, cache =
    // reasoning = 0, and message_count = 1.
    //
    // Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5
    #[test]
    fn test_kiro_sqlite_preservation_checking_property_not_bug_condition() {
        // Scope: the design's FOR ALL X WHERE NOT isBugCondition(X). Every
        // legacy-only shape is outside the bug condition by construction.
        assert!(
            legacy_only_is_not_bug_condition(),
            "legacy-only metadata must never satisfy isBugCondition"
        );

        for (pct, size, start_ms, end_ms) in legacy_only_metadata_cases() {
            let meta = legacy_only_meta_json(pct, size, start_ms, end_ms);
            let messages = parse_single_kiro_sqlite_turn(&meta);

            match expected_legacy_estimate(pct, size) {
                // Requirement 3.3: zero-token turn is still skipped, no entry.
                None => {
                    assert!(
                        messages.is_empty(),
                        "NOT-bug-condition zero-token turn must still be skipped by the fixed \
                         parser; meta = {meta}"
                    );
                }
                Some((expected_input, expected_output, expected_cache_read)) => {
                    assert_eq!(
                        messages.len(),
                        1,
                        "NOT-bug-condition non-zero turn must emit exactly one entry; meta = {meta}"
                    );
                    let m = &messages[0];

                    // Hybrid input estimate (fresh-only, 0 for content-less
                    // fixtures) is applied when no real input count is present.
                    assert_eq!(
                        m.tokens.input, expected_input,
                        "fixed parser must apply the hybrid input estimate; meta = {meta}"
                    );
                    // Requirement 3.1: response_size / 4 output estimate preserved.
                    assert_eq!(
                        m.tokens.output, expected_output,
                        "fixed parser must preserve the output estimate; meta = {meta}"
                    );
                    // Cumulative context routes into cache_read under the
                    // amended data-driven estimate.
                    assert_eq!(
                        m.tokens.cache_read, expected_cache_read,
                        "cache_read must equal cumulative context for legacy-only turns; meta = {meta}"
                    );
                    assert_eq!(
                        m.tokens.cache_write, 0,
                        "cache_write must stay 0 for legacy-only turns; meta = {meta}"
                    );
                    assert_eq!(
                        m.tokens.reasoning, 0,
                        "reasoning must stay 0 for legacy-only turns; meta = {meta}"
                    );
                    // Requirement 3.4: message_count = 1 when no real request count.
                    assert_eq!(
                        m.message_count, 1,
                        "message_count must remain 1 for legacy-only turns; meta = {meta}"
                    );
                }
            }
        }
    }

    // ---------------------------------------------------------------------
    // Integration tests over the full parse_kiro_sqlite (Task 5).
    //
    // These exercise parse_kiro_sqlite end to end over a real TempDir SQLite
    // database with MULTIPLE conversations_v2 rows, each carrying MULTIPLE
    // history turns that mix buggy (real counts present) and non-buggy
    // (legacy-only estimate) shapes, and assert the AGGREGATE token breakdown
    // and message_count totals across the returned UnifiedMessage vec. They
    // complement the per-turn unit/property tests above (which run a single
    // turn through parse_single_kiro_sqlite_turn) by covering the full read
    // loop over many rows and turns.
    //
    // No new crate dependency is introduced: the fixture is built with the
    // existing rusqlite Connection and tempfile::TempDir already used
    // throughout this module.
    // _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 3.5_
    // ---------------------------------------------------------------------

    // Insert `rows` into a fresh TempDir conversations_v2 database and run the
    // full parse_kiro_sqlite over it. Each row is (cwd, conversation_id,
    // value_json). The TempDir is dropped once parse_kiro_sqlite returns
    // (it reads the whole DB eagerly, exactly like parse_single_kiro_sqlite_turn).
    fn parse_kiro_sqlite_rows(rows: &[(&str, &str, &str)]) -> Vec<UnifiedMessage> {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("data.sqlite3");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE conversations_v2 (key TEXT, conversation_id TEXT, value TEXT)",
            [],
        )
        .unwrap();
        for (cwd, conversation_id, value) in rows {
            conn.execute(
                "INSERT INTO conversations_v2 (key, conversation_id, value) VALUES (?1, ?2, ?3)",
                (cwd, conversation_id, value),
            )
            .unwrap();
        }
        drop(conn);
        parse_kiro_sqlite(&db_path)
    }

    // Wrap a slice of request_metadata JSON objects into a conversations_v2
    // `value` payload with the shared model_info (context_window_tokens = 1000,
    // model_id = "auto") used across this module's SQLite fixtures.
    fn conversation_value_with_turns(request_metadatas: &[&str]) -> String {
        let turns: Vec<String> = request_metadatas
            .iter()
            .map(|meta| format!("{{ \"request_metadata\": {} }}", meta))
            .collect();
        format!(
            r#"{{
                "model_info": {{ "model_id": "auto", "context_window_tokens": 1000 }},
                "history": [ {} ]
            }}"#,
            turns.join(", ")
        )
    }

    // Full multi-row, multi-turn aggregation: two conversations_v2 rows, each
    // with several history turns mixing buggy (real counts) and non-buggy
    // (legacy-only estimate) shapes, plus one zero-token turn that must be
    // skipped. Asserts the SUMMED input/output/cacheRead/cacheWrite/reasoning
    // and message_count totals across every emitted UnifiedMessage.
    #[test]
    fn test_kiro_sqlite_integration_multi_row_multi_turn_aggregate_totals() {
        // --- Row 1 (conv-1) --------------------------------------------------
        // Turn 1a: fully-buggy turn (every real count present, request_count 12).
        //   input 7777, output 512, cacheRead 1920, cacheWrite 64, reasoning 40,
        //   message_count 12.
        let t1a = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "input_tokens": 7777,
            "output_tokens": 512,
            "cache_read_input_tokens": 1920,
            "cache_write_input_tokens": 64,
            "reasoning_tokens": 40,
            "request_count": 12,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        // Turn 1b: legacy-only estimate turn (no real counts). Under the
        //   amended hybrid estimate, with no user content: input 0, cumulative
        //   context (1000 * 10 / 100 = 100) -> cacheRead, output estimate
        //   40 / 4 = 10, cacheWrite/reasoning 0, message_count 1.
        let t1b = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        // Turn 1c: zero-token turn (no context pct, no response_size, no real
        // counts) -> skipped, contributes nothing.
        let t1c = r#"{
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let conv1 = conversation_value_with_turns(&[t1a, t1b, t1c]);

        // --- Row 2 (conv-2) --------------------------------------------------
        // Turn 2a: nested token_usage buggy turn (flat fields absent).
        //   input 300, output 256, cacheRead 900, cacheWrite 32, reasoning 8,
        //   message_count = MAX(4, 1) = 4.
        let t2a = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "token_usage": {
                "input_tokens": 300,
                "output_tokens": 256,
                "cache_read_input_tokens": 900,
                "cache_write_input_tokens": 32,
                "reasoning_tokens": 8,
                "request_count": 4
            },
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        // Turn 2b: partial-presence turn: real output only. No real input, so
        // the hybrid estimate applies: input 0 (no user content), cumulative
        // context (100) -> cacheRead, reasoning 0, message_count 1.
        let t2b = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "output_tokens": 999,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let conv2 = conversation_value_with_turns(&[t2a, t2b]);

        let messages = parse_kiro_sqlite_rows(&[
            ("/tmp/project-1", "conv-1", &conv1),
            ("/tmp/project-2", "conv-2", &conv2),
        ]);

        // Four turns survive the zero-token skip (1a, 1b, 2a, 2b); 1c is dropped.
        assert_eq!(
            messages.len(),
            4,
            "one zero-token turn (1c) must be skipped; four entries expected"
        );

        // Aggregate the returned breakdown and message_count totals.
        let total_input: i64 = messages.iter().map(|m| m.tokens.input).sum();
        let total_output: i64 = messages.iter().map(|m| m.tokens.output).sum();
        let total_cache_read: i64 = messages.iter().map(|m| m.tokens.cache_read).sum();
        let total_cache_write: i64 = messages.iter().map(|m| m.tokens.cache_write).sum();
        let total_reasoning: i64 = messages.iter().map(|m| m.tokens.reasoning).sum();
        let total_message_count: i32 = messages.iter().map(|m| m.message_count).sum();

        // input:  7777 (1a real) + 0 (1b hybrid fresh) + 300 (2a nested) + 0 (2b hybrid fresh) = 8077
        assert_eq!(total_input, 7777 + 300, "aggregate input total");
        // output: 512 (1a) + 10 (1b est) + 256 (2a nested) + 999 (2b real) = 1777
        assert_eq!(total_output, 512 + 10 + 256 + 999, "aggregate output total");
        // cacheRead: 1920 (1a real) + 100 (1b hybrid) + 900 (2a real) + 100 (2b hybrid) = 3020
        assert_eq!(
            total_cache_read,
            1920 + 100 + 900 + 100,
            "aggregate cacheRead total"
        );
        // cacheWrite: 64 (1a) + 0 + 32 (2a) + 0 = 96
        assert_eq!(total_cache_write, 64 + 32, "aggregate cacheWrite total");
        // reasoning: 40 (1a) + 0 + 8 (2a) + 0 = 48
        assert_eq!(total_reasoning, 40 + 8, "aggregate reasoning total");
        // message_count: 12 (1a) + 1 (1b) + 4 (2a) + 1 (2b) = 18
        assert_eq!(
            total_message_count,
            12 + 1 + 4 + 1,
            "aggregate message_count total"
        );
    }

    // End-to-end shape check: a Kiro entry whose metadata provides real counts
    // reports non-zero output/cache/reasoning and message_count > 1, matching
    // the `tokscale --json --today -c kiro` report shape the counterexample was
    // drawn from. Asserted directly on the parsed UnifiedMessage (the same data
    // the JSON report aggregates) rather than by shelling out to the CLI, so the
    // test stays hermetic and fast; per AGENTS.md any CLI invocation would pass
    // --no-spinner, but none is needed here.
    #[test]
    fn test_kiro_sqlite_integration_entry_reports_nonzero_breakdown_and_multi_request() {
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "input_tokens": 4096,
            "output_tokens": 512,
            "cache_read_input_tokens": 1920,
            "cache_write_input_tokens": 64,
            "reasoning_tokens": 40,
            "request_count": 12,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let conv = conversation_value_with_turns(&[meta]);
        let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-kiro", &conv)]);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        // Client/provider shape matches the `-c kiro` report rows.
        assert_eq!(m.client, CLIENT_ID);
        assert_eq!(m.provider_id, PROVIDER_ID);
        // Non-zero output/cache/reasoning — the exact fields the bug reported as
        // stuck at 0 in the --json --today -c kiro output.
        assert!(m.tokens.output > 0, "output must be non-zero");
        assert!(m.tokens.cache_read > 0, "cacheRead must be non-zero");
        assert!(m.tokens.cache_write > 0, "cacheWrite must be non-zero");
        assert!(m.tokens.reasoning > 0, "reasoning must be non-zero");
        assert!(m.tokens.input > 0, "input must be non-zero");
        // messageCount > 1 when the metadata records a multi-request turn.
        assert!(
            m.message_count > 1,
            "messageCount must exceed 1 when request_count records multiple requests"
        );
        // Exact recorded values (the counterexample's expected behavior).
        assert_eq!(m.tokens.input, 4096);
        assert_eq!(m.tokens.output, 512);
        assert_eq!(m.tokens.cache_read, 1920);
        assert_eq!(m.tokens.cache_write, 64);
        assert_eq!(m.tokens.reasoning, 40);
        assert_eq!(m.message_count, 12);
    }

    // No regression when Kiro SQLite and file-based Kiro sources are BOTH
    // present: the SQLite path (parse_kiro_sqlite) and the file-based
    // globalStorage `.chat` path (parse_kiro_file) are independent readers, and
    // their combined output must aggregate cleanly. Reuses the existing
    // globalStorage `.chat` fixture shape used by the preservation tests.
    #[test]
    fn test_kiro_sqlite_and_file_based_sources_aggregate_without_regression() {
        // --- SQLite source: one buggy turn with real counts. ----------------
        let meta = r#"{
            "context_usage_percentage": 10,
            "response_size": 40,
            "input_tokens": 4096,
            "output_tokens": 512,
            "cache_read_input_tokens": 1920,
            "cache_write_input_tokens": 64,
            "reasoning_tokens": 40,
            "request_count": 3,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let conv = conversation_value_with_turns(&[meta]);
        let sqlite_messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-kiro", &conv)]);
        assert_eq!(sqlite_messages.len(), 1, "SQLite source yields one entry");

        // --- File-based source: globalStorage `.chat` snapshot. -------------
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-a/execution.chat",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            r#"{
                "model": "auto",
                "messages": [
                    {"role": "user", "content": "hello world"},
                    {"role": "assistant", "content": "response text"}
                ]
            }"#,
        )
        .unwrap();
        let file_messages = parse_kiro_file(&file_path);
        assert_eq!(file_messages.len(), 1, "file-based source yields one entry");

        // Capture each source's per-source breakdown so we can prove the
        // combined totals equal the sum of the independent sources (no
        // cross-source interference or double counting).
        let sqlite_only = &sqlite_messages[0];
        let file_only = &file_messages[0];
        assert!(
            file_only.tokens.input > 0,
            "file-based input estimate preserved"
        );
        assert!(
            file_only.tokens.output > 0,
            "file-based output estimate preserved"
        );
        // File-based source never carries cache/reasoning (unchanged by the fix).
        assert_eq!(file_only.tokens.cache_read, 0);
        assert_eq!(file_only.tokens.cache_write, 0);
        assert_eq!(file_only.tokens.reasoning, 0);

        // Combined multi-source aggregation.
        let mut combined = Vec::new();
        combined.extend(sqlite_messages.iter().cloned());
        combined.extend(file_messages.iter().cloned());
        assert_eq!(combined.len(), 2, "combined batch holds both sources");

        let total_input: i64 = combined.iter().map(|m| m.tokens.input).sum();
        let total_output: i64 = combined.iter().map(|m| m.tokens.output).sum();
        let total_cache_read: i64 = combined.iter().map(|m| m.tokens.cache_read).sum();
        let total_cache_write: i64 = combined.iter().map(|m| m.tokens.cache_write).sum();
        let total_reasoning: i64 = combined.iter().map(|m| m.tokens.reasoning).sum();
        let total_message_count: i32 = combined.iter().map(|m| m.message_count).sum();

        // Combined == SQLite source + file-based source, field by field.
        assert_eq!(
            total_input,
            sqlite_only.tokens.input + file_only.tokens.input
        );
        assert_eq!(
            total_output,
            sqlite_only.tokens.output + file_only.tokens.output
        );
        assert_eq!(
            total_cache_read,
            sqlite_only.tokens.cache_read + file_only.tokens.cache_read
        );
        assert_eq!(
            total_cache_write,
            sqlite_only.tokens.cache_write + file_only.tokens.cache_write
        );
        assert_eq!(
            total_reasoning,
            sqlite_only.tokens.reasoning + file_only.tokens.reasoning
        );
        assert_eq!(
            total_message_count,
            sqlite_only.message_count + file_only.message_count
        );

        // The SQLite source's real counts survive combination (regression guard
        // on the fix), and both entries stay attributed to the kiro client.
        assert_eq!(sqlite_only.tokens.cache_read, 1920);
        assert_eq!(sqlite_only.tokens.reasoning, 40);
        assert_eq!(sqlite_only.message_count, 3);
        assert!(combined.iter().all(|m| m.client == CLIENT_ID));
    }

    // =====================================================================
    // Task 10: data-driven estimation (Property 3) and credit-based cost
    // (Property 4) tests.
    //
    // These exercise the amended fallback path (tasks 7-9): when no real token
    // count is present, `input` is estimated from the turn's fresh content
    // (user Prompt bytes + ToolUseResults JSON bytes), `cache_read` is the
    // cumulative context (context_usage_percentage/100 * context_window) minus
    // that fresh input clamped at zero, and conversation-level
    // `user_turn_metadata.usage_info` credit drives a provider-reported cost
    // on the first emitted turn.
    //
    // No PBT crate is a dev-dependency of tokscale-core (see the note on the
    // preservation tests), so the "for all" properties are exercised by
    // deterministic table-driven generators rather than a randomized harness,
    // matching the existing style in this module.
    // =====================================================================

    // Shared context window used by conversation_value_with_turns and the
    // task-10 builders below (model_info.context_window_tokens = 1000).
    const TASK10_CONTEXT_WINDOW: i64 = 1000;

    // The ceil(chars / 4) token estimate, mirroring `estimate_tokens`, for use
    // in test expectations.
    fn est_tokens(bytes: usize) -> i64 {
        bytes.div_ceil(4) as i64
    }

    // Cumulative-context tokens for a given percentage against
    // TASK10_CONTEXT_WINDOW: floor(window * pct / 100), guarded on pct > 0.
    fn total_context_tokens(ctx_pct: f64) -> i64 {
        if TASK10_CONTEXT_WINDOW > 0 && ctx_pct > 0.0 {
            ((TASK10_CONTEXT_WINDOW as f64) * ctx_pct / 100.0).floor() as i64
        } else {
            0
        }
    }

    // Epsilon for f64 cost comparisons. Credit costs are small (order 1e-3), so
    // a tight absolute tolerance is plenty to distinguish the credit-derived
    // value from a token-based estimate or zero.
    const COST_EPSILON: f64 = 1e-9;

    fn assert_cost_approx(actual: f64, expected: f64, ctx: &str) {
        assert!(
            (actual - expected).abs() < COST_EPSILON,
            "{ctx}: cost {actual} not within {COST_EPSILON} of expected {expected}"
        );
    }

    // JSON-escape a string for embedding inside a `"prompt"` value. The test
    // prompts here are plain ASCII, but escape quotes/backslashes defensively.
    fn json_escape(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }

    // Build a single history turn JSON object carrying an optional user Prompt,
    // an optional ToolUseResults value, and a request_metadata object.
    //
    // - `prompt`: when Some, emitted as user.content = {"Prompt": {"prompt": ...}}
    //   (externally tagged, matching KiroDbUserContent). When both prompt and
    //   tool_results are None, no `user` field is emitted at all.
    // - `tool_results_json`: when Some, emitted as
    //   user.content = {"ToolUseResults": <raw json>}. Prompt takes precedence
    //   for the `user.content` slot when both are given (a single turn's
    //   user.content is ONE variant), so pass them on separate turns to
    //   exercise both.
    // - `request_metadata`: the raw request_metadata JSON object.
    fn turn_with_user_content(
        prompt: Option<&str>,
        tool_results_json: Option<&str>,
        request_metadata: &str,
    ) -> String {
        let user_field = match (prompt, tool_results_json) {
            (Some(p), _) => Some(format!(
                r#""user": {{ "content": {{ "Prompt": {{ "prompt": "{}" }} }} }}"#,
                json_escape(p)
            )),
            (None, Some(tr)) => Some(format!(
                r#""user": {{ "content": {{ "ToolUseResults": {} }} }}"#,
                tr
            )),
            (None, None) => None,
        };
        match user_field {
            Some(user) => format!(
                r#"{{ {}, "request_metadata": {} }}"#,
                user, request_metadata
            ),
            None => format!(r#"{{ "request_metadata": {} }}"#, request_metadata),
        }
    }

    // Wrap history turns into a conversations_v2 `value` payload with the shared
    // model_info and an optional `user_turn_metadata.usage_info` credit array.
    fn conversation_value_with_turns_and_credits(turns: &[String], credits: &[f64]) -> String {
        let history = turns.join(", ");
        let usage_info = credits
            .iter()
            .map(|c| format!(r#"{{ "value": {}, "unit": "credit" }}"#, c))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            r#"{{
                "model_info": {{ "model_id": "auto", "context_window_tokens": {window} }},
                "user_turn_metadata": {{ "usage_info": [ {usage_info} ] }},
                "history": [ {history} ]
            }}"#,
            window = TASK10_CONTEXT_WINDOW,
            usage_info = usage_info,
            history = history,
        )
    }

    // ---------------------------------------------------------------------
    // Property 3: Data-Driven Estimation (Validates: Requirements 2.1, 2.2, 2.6)
    // ---------------------------------------------------------------------

    // Unit: a turn WITH a real user Prompt. fresh input = ceil(user_prompt_length
    // bytes / 4); cache_read = max(cumulative context - fresh input, 0);
    // output = ceil(response_size / 4); reasoning = 0.
    #[test]
    fn test_kiro_sqlite_data_driven_prompt_turn_input_and_cache_read() {
        let prompt = "Reply KIRO_CLI_OK only";
        // user_prompt_length is the byte length Kiro persists for the prompt.
        let prompt_len = prompt.len() as i64;
        let ctx_pct = 20.0_f64;
        let response_size = 40usize;
        let meta = format!(
            r#"{{
                "context_usage_percentage": {ctx_pct},
                "response_size": {response_size},
                "user_prompt_length": {prompt_len},
                "request_start_timestamp_ms": 1770983426000,
                "stream_end_timestamp_ms": 1770983427500
            }}"#,
        );
        let turn = turn_with_user_content(Some(prompt), None, &meta);
        let conv = conversation_value_with_turns_and_credits(&[turn], &[]);
        let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-prompt", &conv)]);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        let fresh_input = est_tokens(prompt_len as usize);
        let expected_cache_read = (total_context_tokens(ctx_pct) - fresh_input).max(0);
        assert_eq!(
            m.tokens.input, fresh_input,
            "input = ceil(user_prompt_length / 4)"
        );
        assert_eq!(
            m.tokens.cache_read, expected_cache_read,
            "cache_read = max(cumulative context - fresh input, 0)"
        );
        assert_eq!(
            m.tokens.output,
            est_tokens(response_size),
            "output = ceil(response_size / 4)"
        );
        assert_eq!(m.tokens.reasoning, 0, "reasoning = 0");
        assert_eq!(m.tokens.cache_write, 0, "cache_write = 0");
    }

    // Unit: a turn WITH ToolUseResults content and no prompt bytes. A
    // tool_result is FRESH input on the turn it first appears, so fresh input
    // = est_tokens(tool_result_text_bytes), and cache_read =
    // max(floor(window * ctx% / 100) - fresh_input, 0).
    #[test]
    fn test_kiro_sqlite_data_driven_tool_use_results_input() {
        // A ToolUseResults value; its `Text` is this turn's fresh tool input.
        let tool_text = "file contents here that came back from a tool";
        let tool_results = format!(
            r#"[ {{ "tool_use_id": "t1", "content": [ {{ "Text": "{tool_text}" }} ] }} ]"#,
        );

        let ctx_pct = 30.0_f64;
        let response_size = 80usize;
        let meta = format!(
            r#"{{
                "context_usage_percentage": {ctx_pct},
                "response_size": {response_size},
                "request_start_timestamp_ms": 1770983426000,
                "stream_end_timestamp_ms": 1770983427500
            }}"#,
        );
        let turn = turn_with_user_content(None, Some(&tool_results), &meta);
        let conv = conversation_value_with_turns_and_credits(&[turn], &[]);
        let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-tur", &conv)]);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        // Fresh input = est_tokens(tool_result text bytes) = ceil(45/4) = 12.
        let expected_input = est_tokens(tool_text.len());
        assert_eq!(expected_input, 12, "sanity: 45-byte tool text -> 12 tokens");
        let expected_cache_read = (total_context_tokens(ctx_pct) - expected_input).max(0);
        assert_eq!(
            m.tokens.input, expected_input,
            "ToolUseResults are fresh input on the turn they first appear"
        );
        assert_eq!(
            m.tokens.cache_read, expected_cache_read,
            "cache_read = max(floor(window * ctx% / 100) - fresh_input, 0)"
        );
        assert_eq!(m.tokens.output, est_tokens(response_size));
        assert_eq!(m.tokens.reasoning, 0);
    }

    // Unit: fresh input across two turns. Turn 1 carries user_prompt_length
    // (prompt-bytes path); turn 2 is ToolUseResults only, whose text IS this
    // turn's fresh input (a tool_result is fresh input on the turn it first
    // appears).
    #[test]
    fn test_kiro_sqlite_data_driven_prompt_and_tool_results_two_turns() {
        let prompt = "run the tests and summarize failures";
        let prompt_len = prompt.len() as i64;
        let tool_text = "42 passed, 0 failed";
        let tool_results = format!(
            r#"[ {{ "tool_use_id": "t9", "content": [ {{ "Text": "{tool_text}" }} ] }} ]"#,
        );

        let ctx1 = 15.0_f64;
        let ctx2 = 25.0_f64;
        let meta1 = format!(
            r#"{{ "context_usage_percentage": {ctx1}, "response_size": 40, "user_prompt_length": {prompt_len},
                 "request_start_timestamp_ms": 1770983426000, "stream_end_timestamp_ms": 1770983427500 }}"#,
        );
        let meta2 = format!(
            r#"{{ "context_usage_percentage": {ctx2}, "response_size": 40,
                 "request_start_timestamp_ms": 1770983428000, "stream_end_timestamp_ms": 1770983429500 }}"#,
        );
        let turn1 = turn_with_user_content(Some(prompt), None, &meta1);
        let turn2 = turn_with_user_content(None, Some(&tool_results), &meta2);
        let conv = conversation_value_with_turns_and_credits(&[turn1, turn2], &[]);
        let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-2turn", &conv)]);

        assert_eq!(messages.len(), 2);
        // Turn 1: prompt bytes only (no tool_result this turn).
        let fresh1 = est_tokens(prompt_len as usize);
        assert_eq!(messages[0].tokens.input, fresh1);
        assert_eq!(
            messages[0].tokens.cache_read,
            (total_context_tokens(ctx1) - fresh1).max(0)
        );
        // Turn 2: ToolUseResults only -> fresh input = est_tokens(tool text
        // bytes) (the model reads the result for the first time this turn);
        // cache_read = max(cumulative context - that fresh input, 0).
        let fresh2 = est_tokens(tool_text.len());
        assert_eq!(fresh2, 5, "sanity: 19-byte tool text -> 5 tokens");
        assert_eq!(messages[1].tokens.input, fresh2);
        assert_eq!(
            messages[1].tokens.cache_read,
            (total_context_tokens(ctx2) - fresh2).max(0)
        );
    }

    // Unit: no double count across turns. Turn 1 carries a tool_result of
    // known bytes; those bytes appear as `input` on turn 1. Turn 2 (prompt
    // only) has a LARGER cumulative context (the turn-1 tool_result is now
    // resent context), and turn 2's fresh input does NOT include the turn-1
    // tool bytes — they have been absorbed into turn 2's cache_read growth.
    #[test]
    fn test_kiro_sqlite_tool_result_input_turn1_absorbed_into_cache_read_turn2() {
        let tool_text = "big tool output that the model reads for the first time now";
        let tool_results = format!(
            r#"[ {{ "tool_use_id": "t1", "content": [ {{ "Text": "{tool_text}" }} ] }} ]"#,
        );
        let prompt2 = "and now a short follow-up prompt";
        let prompt2_len = prompt2.len() as i64;

        // Turn 2's context percentage is higher: the turn-1 tool_result is now
        // part of the resent cumulative context.
        let ctx1 = 10.0_f64;
        let ctx2 = 30.0_f64;
        let meta1 = format!(
            r#"{{ "context_usage_percentage": {ctx1}, "response_size": 40,
                 "request_start_timestamp_ms": 1770983426000, "stream_end_timestamp_ms": 1770983427500 }}"#,
        );
        let meta2 = format!(
            r#"{{ "context_usage_percentage": {ctx2}, "response_size": 40, "user_prompt_length": {prompt2_len},
                 "request_start_timestamp_ms": 1770983428000, "stream_end_timestamp_ms": 1770983429500 }}"#,
        );
        let turn1 = turn_with_user_content(None, Some(&tool_results), &meta1);
        let turn2 = turn_with_user_content(Some(prompt2), None, &meta2);
        let conv = conversation_value_with_turns_and_credits(&[turn1, turn2], &[]);
        let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-nodup", &conv)]);

        assert_eq!(messages.len(), 2);

        // Turn 1: the tool_result bytes ARE the fresh input.
        let tool_tokens = est_tokens(tool_text.len());
        assert_eq!(
            messages[0].tokens.input, tool_tokens,
            "turn 1: tool_result is fresh input on the turn it first appears"
        );
        assert_eq!(
            messages[0].tokens.cache_read,
            (total_context_tokens(ctx1) - tool_tokens).max(0)
        );

        // Turn 2: fresh input is the new prompt only — it does NOT re-count the
        // turn-1 tool bytes. Those bytes are now inside the (larger) cumulative
        // context and show up in turn 2's cache_read, which is strictly larger
        // than turn 1's.
        let fresh2 = est_tokens(prompt2_len as usize);
        assert_eq!(
            messages[1].tokens.input, fresh2,
            "turn 2: fresh input excludes the turn-1 tool bytes (no double count)"
        );
        assert_eq!(
            messages[1].tokens.cache_read,
            (total_context_tokens(ctx2) - fresh2).max(0)
        );
        assert!(
            messages[1].tokens.cache_read > messages[0].tokens.cache_read,
            "turn-1 tool bytes are absorbed into turn 2's grown cache_read"
        );
    }

    // Unit: edge case — when fresh_input (a large tool_result) exceeds the
    // ctx%-derived cumulative context, `input` is kept as-is (NOT clamped to
    // total_context) and cache_read = 0.
    #[test]
    fn test_kiro_sqlite_tool_result_exceeds_context_input_not_clamped() {
        // ~200 bytes of tool text; at a tiny ctx% the cumulative context is 0.
        let tool_text = "z".repeat(200);
        let tool_results = format!(
            r#"[ {{ "tool_use_id": "t1", "content": [ {{ "Text": "{tool_text}" }} ] }} ]"#,
        );
        let ctx_pct = 1.0_f64; // floor(1000 * 1 / 100) = 10 tokens
        let meta = format!(
            r#"{{ "context_usage_percentage": {ctx_pct}, "response_size": 40,
                 "request_start_timestamp_ms": 1770983426000, "stream_end_timestamp_ms": 1770983427500 }}"#,
        );
        let turn = turn_with_user_content(None, Some(&tool_results), &meta);
        let conv = conversation_value_with_turns_and_credits(&[turn], &[]);
        let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-big-tool", &conv)]);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        let fresh = est_tokens(tool_text.len()); // ceil(200/4) = 50
        assert_eq!(fresh, 50);
        assert!(
            fresh > total_context_tokens(ctx_pct),
            "sanity: fresh input exceeds the ctx%-derived context"
        );
        assert_eq!(
            m.tokens.input, fresh,
            "input kept as-is (not clamped to total_context)"
        );
        assert_eq!(
            m.tokens.cache_read, 0,
            "cache_read = max(total_context - fresh_input, 0) = 0"
        );
    }

    // Unit: skip guard preserved for the data-driven path — a turn with no
    // prompt/tool content, no context percentage, and no response_size
    // contributes input 0 + output 0 and is dropped.
    #[test]
    fn test_kiro_sqlite_data_driven_skip_guard_zero_input_output() {
        let meta = r#"{
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let turn = turn_with_user_content(None, None, meta);
        let conv = conversation_value_with_turns_and_credits(&[turn], &[]);
        let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-skip", &conv)]);
        assert!(messages.is_empty(), "input + output == 0 turn is skipped");
    }

    // Property-based (deterministic table): across a range of
    // (user_prompt_length, tool_results_bytes, ctx_pct, window) tuples, assert
    // cache_read is never negative and equals
    //   max(floor(window * ctx% / 100) - fresh_input, 0)
    // where fresh_input = ceil((prompt_bytes + tool_result_bytes) / 4). A
    // tool_result is FRESH input on the turn it first appears, so its bytes are
    // added to fresh input alongside the prompt bytes; on later turns they fold
    // into cache_read via the growing cumulative context (no double count,
    // since cache_read = max(total_context - fresh_input, 0)).
    //
    // Since a single turn's user.content is ONE variant, the prompt+tool rows
    // are realized by emitting ToolUseResults of `tbytes` AND setting
    // user_prompt_length = pbytes on the same turn (the tool bytes come from
    // the content variant, the prompt bytes from the metadata field). So fresh
    // input = pbytes + tbytes.
    //
    // Validates: Requirements 2.1, 2.2, 2.6
    #[test]
    fn test_kiro_sqlite_data_driven_cache_read_never_negative_property() {
        // (prompt_bytes, tool_bytes, ctx_pct)
        let prompt_byte_opts: [usize; 4] = [0, 10, 200, 5000];
        let tool_byte_opts: [usize; 4] = [0, 24, 512, 8000];
        let ctx_pcts: [f64; 5] = [0.0, 1.7308, 20.0, 95.0, 100.0];

        for &pbytes in &prompt_byte_opts {
            for &tbytes in &tool_byte_opts {
                for &ctx_pct in &ctx_pcts {
                    // Build a Prompt of exactly `pbytes` ASCII chars and a
                    // ToolUseResults whose JSON serialization is exactly
                    // `tbytes` long. Under the current model fresh-input bytes
                    // = prompt bytes + this-turn tool_result bytes (the tool
                    // result is fresh input on the turn it first appears).
                    let prompt = "x".repeat(pbytes);
                    // A ToolUseResults value that is a JSON string of an exact
                    // length: serde serializes "aaa" as "\"aaa\"" (len + 2), so
                    // build an inner string of tbytes-2 chars for tbytes >= 2.
                    // A bare JSON string has no Text/text field, so
                    // `tool_results_len` falls back to the compact JSON
                    // serialization length = tbytes.
                    let (tool_json, tool_contributed) = if tbytes == 0 {
                        (None, 0usize)
                    } else if tbytes >= 2 {
                        let inner = "y".repeat(tbytes - 2);
                        (Some(format!(r#""{}""#, inner)), tbytes)
                    } else {
                        // tbytes == 1 is not representable as a JSON string;
                        // skip it (not in the table anyway).
                        (None, 0usize)
                    };

                    // A single turn's user.content is one variant. Fresh input
                    // now tracks prompt bytes + this-turn tool_result bytes:
                    //  - when pbytes > 0 and tbytes > 0: emit ToolUseResults as
                    //    the content AND set user_prompt_length = pbytes; the
                    //    tool bytes come from the content variant and the prompt
                    //    bytes from the metadata field -> fresh = pbytes+tbytes.
                    //  - when only pbytes > 0: emit the Prompt; fresh = pbytes.
                    //  - when only tbytes > 0: emit ToolUseResults; fresh=tbytes.
                    //  - when both 0: fresh input is 0.
                    let (fresh_bytes, turn) = if pbytes > 0 && tool_contributed > 0 {
                        // ToolUseResults content + user_prompt_length = pbytes;
                        // fresh input is prompt bytes + tool bytes.
                        let meta = format!(
                            r#"{{ "context_usage_percentage": {ctx_pct}, "response_size": 40, "user_prompt_length": {} }}"#,
                            pbytes as i64
                        );
                        (
                            pbytes + tool_contributed,
                            turn_with_user_content(None, tool_json.as_deref(), &meta),
                        )
                    } else if pbytes > 0 {
                        let meta = format!(
                            r#"{{ "context_usage_percentage": {ctx_pct}, "response_size": 40, "user_prompt_length": {} }}"#,
                            pbytes as i64
                        );
                        (pbytes, turn_with_user_content(Some(&prompt), None, &meta))
                    } else {
                        // pbytes == 0: fresh input = this-turn tool bytes (the
                        // tool result is fresh input on its turn). When tbytes
                        // is also 0, no `user` field is emitted and fresh = 0.
                        let meta = format!(
                            r#"{{ "context_usage_percentage": {ctx_pct}, "response_size": 40 }}"#,
                        );
                        (
                            tool_contributed,
                            turn_with_user_content(None, tool_json.as_deref(), &meta),
                        )
                    };

                    let conv = conversation_value_with_turns_and_credits(&[turn], &[]);
                    let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-prop", &conv)]);

                    // output = ceil(40/4) = 10 > 0, so the turn always emits.
                    assert_eq!(messages.len(), 1, "table row should emit one turn");
                    let m = &messages[0];
                    let fresh_input = est_tokens(fresh_bytes);
                    let expected_cache_read = (total_context_tokens(ctx_pct) - fresh_input).max(0);
                    assert!(
                        m.tokens.cache_read >= 0,
                        "cache_read must never be negative (pbytes={pbytes}, tbytes={tbytes}, ctx={ctx_pct})"
                    );
                    assert_eq!(
                        m.tokens.cache_read, expected_cache_read,
                        "cache_read = max(cumulative context - fresh input, 0) (pbytes={pbytes}, tbytes={tbytes}, ctx={ctx_pct})"
                    );
                    assert_eq!(
                        m.tokens.input, fresh_input,
                        "input = ceil(fresh bytes / 4) (pbytes={pbytes}, tbytes={tbytes}, ctx={ctx_pct})"
                    );
                }
            }
        }
    }

    // ---------------------------------------------------------------------
    // Property 4: Credit-Based Cost (Validates: Requirements 2.1, 3.4)
    // ---------------------------------------------------------------------

    // Unit: a conversation with a single credit entry and at least one emitting
    // turn assigns credit_sum * CREDIT_TO_USD to the FIRST emitted message and
    // marks it provider-reported; later turns keep cost 0.0 / Unknown.
    #[test]
    fn test_kiro_sqlite_credit_cost_on_first_emitted_turn() {
        let credit = 0.03132_f64;
        let meta1 = r#"{ "context_usage_percentage": 10, "response_size": 40,
            "request_start_timestamp_ms": 1770983426000, "stream_end_timestamp_ms": 1770983427500 }"#;
        let meta2 = r#"{ "context_usage_percentage": 12, "response_size": 40,
            "request_start_timestamp_ms": 1770983428000, "stream_end_timestamp_ms": 1770983429500 }"#;
        let turn1 = turn_with_user_content(Some("first prompt"), None, meta1);
        let turn2 = turn_with_user_content(Some("second prompt"), None, meta2);
        let conv = conversation_value_with_turns_and_credits(&[turn1, turn2], &[credit]);
        let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-credit", &conv)]);

        assert_eq!(messages.len(), 2);
        assert_cost_approx(
            messages[0].cost,
            credit * CREDIT_TO_USD,
            "first emitted turn",
        );
        assert_eq!(
            messages[0].cost_source,
            CostSource::ProviderReported,
            "first emitted turn is provider-reported"
        );
        assert_eq!(messages[1].cost, 0.0, "later turns keep cost 0.0");
        assert_eq!(
            messages[1].cost_source,
            CostSource::Unknown,
            "later turns stay Unknown"
        );
    }

    // Unit: real-ping fixture mirroring the inspected conversation — a single
    // turn with a Prompt of "Reply KIRO_CLI_OK only", request_metadata
    // { context_usage_percentage: 4.916, response_size: 11, user_prompt_length:
    // 182, timestamps }, model_info context_window 200000, and usage_info credit
    // 0.03131873533129692. Asserts cost ~= $0.001253 and provider-reported.
    #[test]
    fn test_kiro_sqlite_credit_cost_real_ping_fixture() {
        let credit = 0.03131873533129692_f64;
        let meta = r#"{
            "context_usage_percentage": 4.916,
            "response_size": 11,
            "user_prompt_length": 182,
            "request_start_timestamp_ms": 1770983426000,
            "stream_end_timestamp_ms": 1770983427500
        }"#;
        let turn = turn_with_user_content(Some("Reply KIRO_CLI_OK only"), None, meta);
        // Real ping used a 200k context window; build the conversation value
        // explicitly with that window rather than the shared 1000 default.
        let conv = format!(
            r#"{{
                "model_info": {{ "model_id": "auto", "context_window_tokens": 200000 }},
                "user_turn_metadata": {{ "usage_info": [ {{ "value": {credit}, "unit": "credit" }} ] }},
                "history": [ {turn} ]
            }}"#,
        );
        let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-ping", &conv)]);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        let expected_cost = credit * CREDIT_TO_USD; // 0.03131873533129692 * 0.04
        assert_cost_approx(m.cost, expected_cost, "real ping fixture");
        assert_eq!(m.cost_source, CostSource::ProviderReported);
        // Document the asserted value: ~$0.001253.
        assert!(
            (m.cost - 0.0012527494132518767).abs() < 1e-12,
            "ping cost should equal 0.03131873533129692 * 0.04 = 0.0012527494132518767, got {}",
            m.cost
        );
    }

    // Unit: a conversation with NO credit-unit entries leaves cost 0.0 /
    // CostSource::Unknown so downstream pricing can estimate from tokens.
    #[test]
    fn test_kiro_sqlite_no_credit_leaves_cost_unknown() {
        let meta = r#"{ "context_usage_percentage": 10, "response_size": 40,
            "request_start_timestamp_ms": 1770983426000, "stream_end_timestamp_ms": 1770983427500 }"#;
        let turn = turn_with_user_content(Some("hello"), None, meta);
        // No usage_info credit entries.
        let conv = conversation_value_with_turns_and_credits(&[turn], &[]);
        let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-nocredit", &conv)]);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].cost, 0.0,
            "no-credit conversation keeps cost 0.0"
        );
        assert_eq!(
            messages[0].cost_source,
            CostSource::Unknown,
            "no-credit conversation stays Unknown"
        );
    }

    // Property-based (deterministic table): for a set of nonnegative credit
    // arrays, cost on the first emitted turn == sum(values where unit==credit)
    // * CREDIT_TO_USD; conversations without any credit-unit entries never mark
    // provider-reported.
    //
    // Validates: Requirements 2.1, 3.4
    #[test]
    fn test_kiro_sqlite_credit_cost_property_table() {
        let credit_arrays: [&[f64]; 6] = [
            &[],
            &[0.0],
            &[0.03132],
            &[0.01, 0.02, 0.03],
            &[0.03131873533129692],
            &[1.5, 2.5, 4.0],
        ];

        for credits in credit_arrays {
            // One emitting turn per conversation.
            let meta = r#"{ "context_usage_percentage": 10, "response_size": 40,
                "request_start_timestamp_ms": 1770983426000, "stream_end_timestamp_ms": 1770983427500 }"#;
            let turn = turn_with_user_content(Some("prompt text"), None, meta);
            let conv = conversation_value_with_turns_and_credits(&[turn], credits);
            let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-cprop", &conv)]);

            assert_eq!(
                messages.len(),
                1,
                "credits={credits:?} should emit one turn"
            );
            let m = &messages[0];
            let credit_sum: f64 = credits.iter().sum();
            if credit_sum > 0.0 {
                assert_cost_approx(
                    m.cost,
                    credit_sum * CREDIT_TO_USD,
                    &format!("credits={credits:?}"),
                );
                assert_eq!(
                    m.cost_source,
                    CostSource::ProviderReported,
                    "credits={credits:?} with positive sum must be provider-reported"
                );
            } else {
                assert_eq!(
                    m.cost, 0.0,
                    "credits={credits:?} with zero sum keeps cost 0.0"
                );
                assert_eq!(
                    m.cost_source,
                    CostSource::Unknown,
                    "credits={credits:?} without positive credit never provider-reported"
                );
            }
        }
    }

    // ---------------------------------------------------------------------
    // 16-turn cache_read integration case: monotonically increasing
    // context_usage_percentage drives growing cache_read. Asserts per-turn
    // cache_read == max(floor(window * ctx% / 100) - fresh_input, 0) and that
    // cache_read grows across turns.
    // ---------------------------------------------------------------------
    #[test]
    fn test_kiro_sqlite_16_turn_cache_read_grows_with_context() {
        // Real monotonically increasing percentages spanning 1.7308..2.9482.
        let ctx_pcts: [f64; 16] = [
            1.7308, 1.8100, 1.8955, 1.9800, 2.0650, 2.1500, 2.2380, 2.3260, 2.4150, 2.5040, 2.5930,
            2.6820, 2.7710, 2.8600, 2.9040, 2.9482,
        ];
        // Use a realistic 200k window so cumulative context dominates the small
        // fresh input and cache_read is positive and growing.
        let window: i64 = 200_000;
        let prompt = "small user content";
        let prompt_len = prompt.len() as i64;

        let turns: Vec<String> = ctx_pcts
            .iter()
            .enumerate()
            .map(|(i, pct)| {
                let start = 1770983426000i64 + (i as i64) * 2000;
                let end = start + 1500;
                let meta = format!(
                    r#"{{ "context_usage_percentage": {pct}, "response_size": 40, "user_prompt_length": {prompt_len},
                         "request_start_timestamp_ms": {start}, "stream_end_timestamp_ms": {end} }}"#,
                );
                turn_with_user_content(Some(prompt), None, &meta)
            })
            .collect();

        let conv = format!(
            r#"{{
                "model_info": {{ "model_id": "auto", "context_window_tokens": {window} }},
                "history": [ {} ]
            }}"#,
            turns.join(", ")
        );
        let messages = parse_kiro_sqlite_rows(&[("/tmp/project", "conv-16", &conv)]);

        assert_eq!(messages.len(), 16, "all 16 turns emit (output > 0)");

        let fresh_input = est_tokens(prompt_len as usize);
        let mut prev_cache_read = i64::MIN;
        for (i, m) in messages.iter().enumerate() {
            let expected_total = if window > 0 && ctx_pcts[i] > 0.0 {
                ((window as f64) * ctx_pcts[i] / 100.0).floor() as i64
            } else {
                0
            };
            let expected_cache_read = (expected_total - fresh_input).max(0);
            assert_eq!(
                m.tokens.input, fresh_input,
                "turn {i}: input = ceil(user_prompt_length / 4)"
            );
            assert_eq!(
                m.tokens.cache_read, expected_cache_read,
                "turn {i}: cache_read = max(floor(window * ctx% / 100) - fresh_input, 0)"
            );
            assert!(
                m.tokens.cache_read > prev_cache_read,
                "turn {i}: cache_read must grow across turns (prev {prev_cache_read}, now {})",
                m.tokens.cache_read
            );
            prev_cache_read = m.tokens.cache_read;
        }
    }

    // ---------------------------------------------------------------------
    // Task 12 — Bug condition exploration tests for the file-based paths.
    //
    // Property 5 (CLI, parse_kiro_file) and Property 7 (IDE,
    // parse_kiro_ide_session_file) from design "Amended Design — Unify All
    // Three Kiro Parse Paths".
    //
    // CRITICAL: these two tests encode the FIXED (expected) behavior and are
    // EXPECTED TO FAIL on the current unfixed file-based paths. The failure is
    // the confirmed counterexample proving the bug — DO NOT fix the code here.
    //
    // On unfixed code:
    //   - CLI: context_usage_percentage (3.9788%) * context_window (200000) is
    //     mapped straight into `input` = 7957, and `output` = 0 because the
    //     real `assistant_response_length` (398) is ignored in favor of the
    //     empty sibling .jsonl / char estimate. cache_read stays 0.
    //   - IDE: usagePercentage (40.0%) * DEFAULT_CONTEXT_WINDOW (200000) is
    //     mapped into `input` = 80000, tool_call args fold into
    //     assistant_chars inflating `output`, and message_count is forced to 1.
    // ---------------------------------------------------------------------

    // Property 5: CLI cumulative-context input over-count.
    //
    // Mirrors the real bug report: user_prompt_length=2,
    // context_usage_percentage=3.9788, context_window_tokens=200000,
    // assistant_response_length=398, all *_token_count = 0. Expected (fixed):
    //   input      = ceil(user_prompt_length / 4) = ceil(2/4)  = 1  (fresh)
    //   cache_read = max(floor(3.9788/100 * 200000) - 1, 0)
    //              = max(7957 - 1, 0)                           = 7956
    //   output     = ceil(assistant_response_length / 4)
    //              = ceil(398/4)                                = 100
    //
    // EXPECTED OUTCOME: FAILS on unfixed code (unfixed gives input=7957,
    // output=0, cache_read=0).
    #[test]
    fn test_parse_kiro_cli_bug_condition_cumulative_context_input_overcount() {
        let dir = TempDir::new().unwrap();
        // Real-bug-data turn: richer per-turn fields present, all real token
        // counts zero (Auto agent does not persist them).
        let json = r#"{
            "session_id": "sess-cli-bug",
            "cwd": "/tmp/project",
            "session_state": {
                "rts_model_state": {
                    "model_info": {
                        "model_id": "auto",
                        "context_window_tokens": 200000
                    }
                },
                "conversation_metadata": {
                    "user_turn_metadatas": [{
                        "user_prompt_length": 2,
                        "context_usage_percentage": 3.9788,
                        "input_token_count": 0,
                        "output_token_count": 0,
                        "cache_read_input_token_count": 0,
                        "cache_write_input_token_count": 0,
                        "assistant_response_length": 398,
                        "total_request_count": 1,
                        "message_ids": ["m1", "m2"]
                    }]
                }
            }
        }"#;
        // Minimal sibling .jsonl — deliberately empty of the turn's message ids
        // so the fix must rely on the real user_prompt_length /
        // assistant_response_length fields, not char estimates from here.
        let jsonl = "";
        let path = create_session_files(&dir, "sess-cli-bug", json, jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1, "the single turn must emit one message");
        let m = &messages[0];

        // Expected (fixed) Property 5 behavior. FAILS on unfixed code.
        assert_eq!(
            m.tokens.input, 1,
            "input = ceil(user_prompt_length / 4) = ceil(2/4) = 1 (fresh input only)"
        );
        assert_eq!(
            m.tokens.cache_read, 7956,
            "cache_read = max(floor(3.9788/100 * 200000) - 1, 0) = 7957 - 1 = 7956"
        );
        assert_eq!(
            m.tokens.output, 100,
            "output = ceil(assistant_response_length / 4) = ceil(398/4) = 100"
        );
        assert_eq!(m.tokens.reasoning, 0, "reasoning = 0");
        assert_eq!(
            m.message_count, 1,
            "message_count = max(total_request_count, 1) = 1"
        );
    }

    // ---------------------------------------------------------------------
    // Task 14.4 — CLI hybrid estimation + metering credit unit/property tests
    // ---------------------------------------------------------------------

    // Builds a CLI session `.json` header carrying a single
    // `user_turn_metadatas` turn with the given per-turn metadata fields.
    fn cli_session_json(turn_fields: &str) -> String {
        format!(
            r#"{{
                "session_id": "sess-cli",
                "cwd": "/tmp/project",
                "session_state": {{
                    "rts_model_state": {{
                        "model_info": {{
                            "model_id": "auto",
                            "context_window_tokens": 200000
                        }}
                    }},
                    "conversation_metadata": {{
                        "user_turn_metadatas": [{{ {turn_fields} }}]
                    }}
                }}
            }}"#
        )
    }

    // Unit real-data fixture (design "CLI real-data fixture"): user_prompt_length
    // = 2, context_usage_percentage = 3.9788, context_window = 200000,
    // assistant_response_length = 398, all real *_token_count = 0, empty sibling
    // jsonl. Asserts the fresh-input hybrid: input = ceil(2/4) = 1, cache_read =
    // max(floor(3.9788/100 * 200000) - 1, 0) = 7956, output = ceil(398/4) = 100.
    #[test]
    fn test_parse_kiro_cli_hybrid_real_data_fixture() {
        let dir = TempDir::new().unwrap();
        let json = cli_session_json(
            r#""user_prompt_length": 2,
               "context_usage_percentage": 3.9788,
               "input_token_count": 0,
               "output_token_count": 0,
               "cache_read_input_token_count": 0,
               "cache_write_input_token_count": 0,
               "assistant_response_length": 398,
               "total_request_count": 1,
               "message_ids": ["m1"]"#,
        );
        let path = create_session_files(&dir, "sess-cli", &json, "");

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.tokens.input, 1, "fresh input = ceil(2/4)");
        assert_eq!(
            m.tokens.cache_read, 7956,
            "cache_read = max(floor(3.9788/100 * 200000) - 1, 0)"
        );
        assert_eq!(m.tokens.output, 100, "output = ceil(398/4)");
        assert_eq!(m.tokens.reasoning, 0);
        assert_eq!(m.tokens.cache_write, 0);
        assert_eq!(m.message_count, 1);
        assert_eq!(m.cost, 0.0);
        assert_eq!(m.cost_source, CostSource::Unknown);
    }

    // Unit: real per-turn token counts win over the hybrid estimate. A turn with
    // nonzero input_token_count / output_token_count / cache_read /
    // cache_write must surface those exact values, ignoring the cumulative
    // context and byte-length fields.
    #[test]
    fn test_parse_kiro_cli_real_count_precedence() {
        let dir = TempDir::new().unwrap();
        let json = cli_session_json(
            r#""user_prompt_length": 2,
               "context_usage_percentage": 3.9788,
               "input_token_count": 321,
               "output_token_count": 654,
               "cache_read_input_token_count": 111,
               "cache_write_input_token_count": 22,
               "assistant_response_length": 398,
               "total_request_count": 3,
               "message_ids": ["m1"]"#,
        );
        let path = create_session_files(&dir, "sess-cli", &json, "");

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.tokens.input, 321, "real input_token_count wins");
        assert_eq!(m.tokens.output, 654, "real output_token_count wins");
        assert_eq!(m.tokens.cache_read, 111, "real cache_read count wins");
        assert_eq!(m.tokens.cache_write, 22, "real cache_write count wins");
        assert_eq!(m.message_count, 3);
    }

    // Unit: metering_usage credit sum → credit_sum * CREDIT_TO_USD assigned to
    // the FIRST emitted turn and marked provider-reported. A skipped
    // (zero-token) first turn does not consume the credit.
    #[test]
    fn test_parse_kiro_cli_metering_credit_on_first_emitted_turn() {
        let dir = TempDir::new().unwrap();
        let credit = 0.03132_f64;
        // Two turns: turn 0 is zero-token (skipped), turn 1 emits and carries
        // the metering credit. The credit still lands on the first EMITTED turn.
        let turn0 = r#"{ "input_token_count": 0, "output_token_count": 0,
            "user_prompt_length": 0, "assistant_response_length": 0,
            "context_usage_percentage": 0, "total_request_count": 1,
            "message_ids": ["z"] }"#;
        let turn1 = format!(
            r#"{{ "user_prompt_length": 2, "assistant_response_length": 398,
                "context_usage_percentage": 3.9788, "input_token_count": 0,
                "output_token_count": 0, "total_request_count": 1,
                "message_ids": ["m1"],
                "metering_usage": [{{ "value": {credit}, "unit": "credit" }}] }}"#
        );
        let json = format!(
            r#"{{
                "session_id": "sess-cli",
                "cwd": "/tmp/project",
                "session_state": {{
                    "rts_model_state": {{
                        "model_info": {{ "model_id": "auto", "context_window_tokens": 200000 }}
                    }},
                    "conversation_metadata": {{
                        "user_turn_metadatas": [{turn0}, {turn1}]
                    }}
                }}
            }}"#
        );
        let path = create_session_files(&dir, "sess-cli", &json, "");

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1, "only the nonzero turn emits");
        let m = &messages[0];
        assert_cost_approx(m.cost, credit * CREDIT_TO_USD, "first emitted turn");
        assert_eq!(m.cost_source, CostSource::ProviderReported);
    }

    // Unit: empty / absent metering_usage leaves cost 0.0 / Unknown so the
    // downstream pricing service estimates from the hybrid breakdown.
    #[test]
    fn test_parse_kiro_cli_no_metering_leaves_cost_unknown() {
        let dir = TempDir::new().unwrap();
        let json = cli_session_json(
            r#""user_prompt_length": 2,
               "context_usage_percentage": 3.9788,
               "assistant_response_length": 398,
               "total_request_count": 1,
               "message_ids": ["m1"],
               "metering_usage": []"#,
        );
        let path = create_session_files(&dir, "sess-cli", &json, "");

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.cost, 0.0);
        assert_eq!(m.cost_source, CostSource::Unknown);
    }

    // Property-based (deterministic table): for arbitrary (user_prompt_length,
    // context_usage_percentage, context_window), the CLI cache_read is never
    // negative and equals max(floor(ctx% / 100 * window) - fresh_input, 0),
    // where fresh_input = ceil(user_prompt_length / 4).
    //
    // Validates: Requirements 2.1, 2.2, 2.6, 3.1, 3.3, 3.4
    #[test]
    fn test_parse_kiro_cli_cache_read_property_table() {
        // (user_prompt_length bytes, ctx_pct, context_window)
        let cases: [(i64, f64, i64); 7] = [
            (2, 3.9788, 200_000),
            (0, 0.0, 200_000),
            (4000, 50.0, 200_000),
            (10, 0.001, 200_000),
            (2, 99.9, 100_000),
            (1_000_000, 10.0, 200_000), // fresh input exceeds cumulative context
            (8, 65.69, 200_000),
        ];

        for (prompt_len, ctx_pct, window) in cases {
            let dir = TempDir::new().unwrap();
            let json = format!(
                r#"{{
                    "session_id": "sess-cli",
                    "cwd": "/tmp/project",
                    "session_state": {{
                        "rts_model_state": {{
                            "model_info": {{ "model_id": "auto", "context_window_tokens": {window} }}
                        }},
                        "conversation_metadata": {{
                            "user_turn_metadatas": [{{
                                "user_prompt_length": {prompt_len},
                                "context_usage_percentage": {ctx_pct},
                                "assistant_response_length": 400,
                                "input_token_count": 0,
                                "output_token_count": 0,
                                "total_request_count": 1,
                                "message_ids": ["m1"]
                            }}]
                        }}
                    }}
                }}"#
            );
            let path = create_session_files(&dir, "sess-cli", &json, "");

            let messages = parse_kiro_file(&path);
            assert_eq!(messages.len(), 1, "case {prompt_len}/{ctx_pct}/{window}");
            let m = &messages[0];

            let fresh_input = (prompt_len.max(0) as usize).div_ceil(4) as i64;
            let total_context = if ctx_pct > 0.0 {
                ((window as f64) * ctx_pct / 100.0).floor() as i64
            } else {
                0
            };
            let expected_cache_read = (total_context - fresh_input).max(0);

            assert!(
                m.tokens.cache_read >= 0,
                "cache_read never negative (case {prompt_len}/{ctx_pct}/{window})"
            );
            assert_eq!(
                m.tokens.cache_read, expected_cache_read,
                "cache_read = max(floor(ctx%/100 * window) - fresh, 0) (case {prompt_len}/{ctx_pct}/{window})"
            );
            assert_eq!(
                m.tokens.input, fresh_input,
                "input = fresh (case {prompt_len}/{ctx_pct}/{window})"
            );
        }
    }

    // Property 7: IDE cumulative-context input over-count with tool inflation.
    //
    // Structured messages.jsonl with a user payload, an assistant payload, a
    // tool_call payload with large args, a tool_result payload with large
    // content, a session_metadata contextUsage payload (usagePercentage=40.0),
    // a usage_summary payload with requestIds of length 3, and a turn_end.
    //
    // Expected Property 7 behavior (current model — tool_result IS fresh input
    // on the turn it first appears; tool_call args are NOT):
    //   input      = ceil((user_content_chars + tool_result_chars) / 4)
    //   output     = ceil(assistant_content_chars / 4) (assistant text only)
    //   cache_read = max(floor(40.0/100 * 200000) - input, 0)
    //   message_count = requestIds.len() = 3
    //
    // tool_call args still NEVER inflate input/output.
    #[test]
    fn test_parse_kiro_ide_bug_condition_cumulative_context_and_tool_inflation() {
        // Known content strings. Byte length == char count (ASCII).
        let user_prompt = "please summarize the recent build failures"; // 42 chars
        let assistant_text = "the build failed in three test modules today"; // 44 chars
        let user_chars = user_prompt.chars().count();
        let assistant_chars = assistant_text.chars().count();

        // tool_call args must NOT be folded into input/output; tool_result
        // content IS this turn's fresh input.
        let tool_args = "x".repeat(5000);
        let tool_result_content = "y".repeat(6000);
        let tool_result_chars = tool_result_content.chars().count();

        let session_json = r#"{
            "schemaVersion": "1.0.0",
            "id": "sess_ide_bug"
        }"#;

        let messages_jsonl = format!(
            concat!(
                "{{\"payload\":{{\"type\":\"user\",\"content\":\"{user}\"}}}}\n",
                "{{\"payload\":{{\"type\":\"assistant\",\"content\":\"{assistant}\"}}}}\n",
                "{{\"payload\":{{\"type\":\"tool_call\",\"args\":\"{tool_args}\"}}}}\n",
                "{{\"payload\":{{\"type\":\"tool_result\",\"content\":\"{tool_result}\"}}}}\n",
                "{{\"payload\":{{\"type\":\"session_metadata\",\"key\":\"contextUsage\",\"value\":{{\"usagePercentage\":40.0}}}}}}\n",
                "{{\"payload\":{{\"type\":\"usage_summary\",\"elapsedTime\":5000,\"requestIds\":[\"r1\",\"r2\",\"r3\"]}}}}\n",
                "{{\"payload\":{{\"type\":\"turn_end\"}},\"timestamp\":\"2026-06-20T10:00:05Z\"}}\n",
            ),
            user = user_prompt,
            assistant = assistant_text,
            tool_args = tool_args,
            tool_result = tool_result_content,
        );

        let dir = TempDir::new().unwrap();
        let path =
            create_ide_session_files(&dir, "ws", "sess_ide_bug", session_json, &messages_jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1, "the single structured turn emits once");
        let m = &messages[0];

        let expected_input = est_tokens(user_chars + tool_result_chars);
        let expected_output = est_tokens(assistant_chars);
        let total_context = (200_000f64 * 40.0 / 100.0).floor() as i64; // 80000
        let expected_cache_read = (total_context - expected_input).max(0);

        assert_eq!(
            m.tokens.input, expected_input,
            "input = ceil((user_content_chars + tool_result_chars) / 4) — tool_result IS fresh input"
        );
        assert_eq!(
            m.tokens.output, expected_output,
            "output = ceil(assistant_content_chars / 4) — assistant text only, NOT tool_call args"
        );
        assert_eq!(
            m.tokens.cache_read, expected_cache_read,
            "cache_read = max(floor(40.0/100 * 200000) - input, 0)"
        );
        assert_eq!(m.message_count, 3, "message_count = requestIds.len() = 3");
    }

    // =====================================================================
    // Task 15.3 — IDE hybrid estimation unit + property tests (Property 7)
    // =====================================================================

    // Unit (real-data-shaped): a structured messages.jsonl carrying a
    // contextUsage progression, a large tool_call args line, a large
    // tool_result content line, user/assistant content, a usage_summary with
    // requestIds length 3, and a turn_end. The hybrid model must set:
    //   input      = ceil((user_content_chars + tool_result_chars) / 4)
    //   output     = ceil(assistant_content_chars / 4) (assistant only — tool
    //                                                    _call args excluded)
    //   cache_read = max(floor(40/100 * 200000) - input, 0)
    //   message_count = requestIds.len() = 3
    //
    // Validates: Requirements 2.1, 2.2, 2.6, 3.1, 3.3
    #[test]
    fn test_parse_kiro_ide_hybrid_estimation_tool_result_is_fresh_input() {
        let user_prompt = "please summarize the recent build failures"; // 42 chars
        let assistant_text = "the build failed in three test modules today"; // 44 chars
        let user_chars = user_prompt.chars().count();
        let assistant_chars = assistant_text.chars().count();

        // tool_call args must NOT be folded into input/output; tool_result
        // content IS this turn's fresh input.
        let tool_args = "x".repeat(5000);
        let tool_result_content = "y".repeat(6000);
        let tool_result_chars = tool_result_content.chars().count();

        let session_json = r#"{
            "schemaVersion": "1.0.0",
            "id": "sess_ide_hybrid"
        }"#;

        let messages_jsonl = format!(
            concat!(
                "{{\"payload\":{{\"type\":\"user\",\"content\":\"{user}\"}}}}\n",
                "{{\"payload\":{{\"type\":\"assistant\",\"content\":\"{assistant}\"}}}}\n",
                "{{\"payload\":{{\"type\":\"tool_call\",\"args\":\"{tool_args}\"}}}}\n",
                "{{\"payload\":{{\"type\":\"tool_result\",\"content\":\"{tool_result}\"}}}}\n",
                "{{\"payload\":{{\"type\":\"session_metadata\",\"key\":\"contextUsage\",\"value\":{{\"usagePercentage\":40.0}}}}}}\n",
                "{{\"payload\":{{\"type\":\"usage_summary\",\"elapsedTime\":5000,\"requestIds\":[\"r1\",\"r2\",\"r3\"]}}}}\n",
                "{{\"payload\":{{\"type\":\"turn_end\"}},\"timestamp\":\"2026-06-20T10:00:05Z\"}}\n",
            ),
            user = user_prompt,
            assistant = assistant_text,
            tool_args = tool_args,
            tool_result = tool_result_content,
        );

        let dir = TempDir::new().unwrap();
        let path =
            create_ide_session_files(&dir, "ws", "sess_ide_hybrid", session_json, &messages_jsonl);

        let messages = parse_kiro_file(&path);
        assert_eq!(messages.len(), 1);
        let m = &messages[0];

        let expected_input = est_tokens(user_chars + tool_result_chars);
        let expected_output = est_tokens(assistant_chars);
        let total_context = (200_000f64 * 40.0 / 100.0).floor() as i64; // 80000
        let expected_cache_read = (total_context - expected_input).max(0);

        assert_eq!(
            m.tokens.input, expected_input,
            "input = user content + tool_result content"
        );
        assert_eq!(
            m.tokens.output, expected_output,
            "output = assistant content only (tool_call args excluded)"
        );
        assert_eq!(
            m.tokens.cache_read, expected_cache_read,
            "cache_read = max(floor(40/100 * 200000) - input, 0)"
        );
        assert_eq!(m.tokens.cache_write, 0, "no cache_write in IDE files");
        assert_eq!(m.tokens.reasoning, 0, "no reasoning in IDE files");
        assert_eq!(m.message_count, 3, "message_count = requestIds.len()");
    }

    // Unit: message_count = requestIds.len().max(1) when usage_summary carries
    // requestIds; and message_count = 1 when usage_summary / requestIds are
    // absent.
    //
    // Validates: Requirements 2.6, 3.3
    #[test]
    fn test_parse_kiro_ide_message_count_from_request_ids() {
        let session_json = r#"{"schemaVersion":"1.0.0","id":"sess_mc"}"#;

        // Case A: requestIds length 5 -> message_count = 5.
        let with_ids = concat!(
            "{\"payload\":{\"type\":\"user\",\"content\":\"hi there friend\"}}\n",
            "{\"payload\":{\"type\":\"assistant\",\"content\":\"hello back now\"}}\n",
            "{\"payload\":{\"type\":\"usage_summary\",\"requestIds\":[\"a\",\"b\",\"c\",\"d\",\"e\"]}}\n",
            "{\"payload\":{\"type\":\"turn_end\"},\"timestamp\":\"2026-06-20T10:00:05Z\"}\n",
        );
        let dir_a = TempDir::new().unwrap();
        let path_a = create_ide_session_files(&dir_a, "ws", "sess_mc", session_json, with_ids);
        let messages_a = parse_kiro_file(&path_a);
        assert_eq!(messages_a.len(), 1);
        assert_eq!(
            messages_a[0].message_count, 5,
            "message_count = requestIds.len() = 5"
        );

        // Case B: no usage_summary / requestIds -> message_count = 1.
        let without_ids = concat!(
            "{\"payload\":{\"type\":\"user\",\"content\":\"hi there friend\"}}\n",
            "{\"payload\":{\"type\":\"assistant\",\"content\":\"hello back now\"}}\n",
            "{\"payload\":{\"type\":\"turn_end\"},\"timestamp\":\"2026-06-20T10:00:05Z\"}\n",
        );
        let dir_b = TempDir::new().unwrap();
        let path_b = create_ide_session_files(&dir_b, "ws", "sess_mc", session_json, without_ids);
        let messages_b = parse_kiro_file(&path_b);
        assert_eq!(messages_b.len(), 1);
        assert_eq!(
            messages_b[0].message_count, 1,
            "message_count = 1 when requestIds absent"
        );
    }

    // Unit: IDE no double count across two structured turns. Turn 1 carries a
    // large tool_result; those chars appear as `input` on turn 1. Turn 2
    // (prompt only) has a larger contextUsage (the turn-1 tool_result is now
    // resent context) and its fresh input excludes the turn-1 tool chars —
    // they are absorbed into turn 2's grown cache_read.
    #[test]
    fn test_parse_kiro_ide_tool_result_input_turn1_absorbed_turn2() {
        let user1 = "read the config file for me"; // 27 chars
        let user1_chars = user1.chars().count();
        let tool_result_content = "y".repeat(4000);
        let tool_result_chars = tool_result_content.chars().count();
        let user2 = "thanks, now summarize it"; // 24 chars
        let user2_chars = user2.chars().count();

        let session_json = r#"{"schemaVersion":"1.0.0","id":"sess_nodup"}"#;
        // Turn 2's contextUsage is higher: the turn-1 tool_result is now resent.
        let messages_jsonl = format!(
            concat!(
                "{{\"payload\":{{\"type\":\"user\",\"content\":\"{user1}\"}}}}\n",
                "{{\"payload\":{{\"type\":\"assistant\",\"content\":\"ok\"}}}}\n",
                "{{\"payload\":{{\"type\":\"tool_result\",\"content\":\"{tool_result}\"}}}}\n",
                "{{\"payload\":{{\"type\":\"session_metadata\",\"key\":\"contextUsage\",\"value\":{{\"usagePercentage\":5.0}}}}}}\n",
                "{{\"payload\":{{\"type\":\"turn_end\"}},\"timestamp\":\"2026-06-20T10:00:05Z\"}}\n",
                "{{\"payload\":{{\"type\":\"user\",\"content\":\"{user2}\"}}}}\n",
                "{{\"payload\":{{\"type\":\"assistant\",\"content\":\"ok\"}}}}\n",
                "{{\"payload\":{{\"type\":\"session_metadata\",\"key\":\"contextUsage\",\"value\":{{\"usagePercentage\":20.0}}}}}}\n",
                "{{\"payload\":{{\"type\":\"turn_end\"}},\"timestamp\":\"2026-06-20T10:00:10Z\"}}\n",
            ),
            user1 = user1,
            tool_result = tool_result_content,
            user2 = user2,
        );

        let dir = TempDir::new().unwrap();
        let path =
            create_ide_session_files(&dir, "ws", "sess_nodup", session_json, &messages_jsonl);
        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 2);

        // Turn 1: fresh input = user1 chars + tool_result chars.
        let input1 = est_tokens(user1_chars + tool_result_chars);
        let ctx1 = (200_000f64 * 5.0 / 100.0).floor() as i64;
        assert_eq!(
            messages[0].tokens.input, input1,
            "turn 1: tool_result is fresh input on the turn it first appears"
        );
        assert_eq!(messages[0].tokens.cache_read, (ctx1 - input1).max(0));

        // Turn 2: fresh input excludes the turn-1 tool chars (no double count).
        let input2 = est_tokens(user2_chars);
        let ctx2 = (200_000f64 * 20.0 / 100.0).floor() as i64;
        assert_eq!(
            messages[1].tokens.input, input2,
            "turn 2: fresh input excludes the turn-1 tool chars"
        );
        assert_eq!(messages[1].tokens.cache_read, (ctx2 - input2).max(0));
        assert!(
            messages[1].tokens.cache_read > messages[0].tokens.cache_read,
            "turn-1 tool chars absorbed into turn 2's grown cache_read"
        );
    }

    // Property table: across arbitrary (user_content_chars, usagePercentage),
    // cache_read is never negative and equals
    // max(floor(ctx%/100 * DEFAULT_CONTEXT_WINDOW) - input, 0), where
    // input = ceil(user_content_chars / 4).
    //
    // Validates: Requirements 2.1, 2.2, 2.6, 3.1, 3.3
    #[test]
    fn test_parse_kiro_ide_cache_read_property_table() {
        // (user_content_chars, usagePercentage)
        let cases: [(usize, f64); 7] = [
            (0, 40.0),
            (42, 40.0),
            (4000, 50.0),
            (10, 0.001),
            (8, 65.69),
            (1_000_000, 10.0), // fresh input exceeds cumulative context
            (100, 99.9),
        ];

        for (user_chars, ctx_pct) in cases {
            let user_content = "u".repeat(user_chars);
            let session_json = r#"{"schemaVersion":"1.0.0","id":"sess_prop"}"#;
            let messages_jsonl = format!(
                concat!(
                    "{{\"payload\":{{\"type\":\"user\",\"content\":\"{user}\"}}}}\n",
                    "{{\"payload\":{{\"type\":\"assistant\",\"content\":\"a\"}}}}\n",
                    "{{\"payload\":{{\"type\":\"session_metadata\",\"key\":\"contextUsage\",\"value\":{{\"usagePercentage\":{ctx_pct}}}}}}}\n",
                    "{{\"payload\":{{\"type\":\"turn_end\"}},\"timestamp\":\"2026-06-20T10:00:05Z\"}}\n",
                ),
                user = user_content,
                ctx_pct = ctx_pct,
            );

            let dir = TempDir::new().unwrap();
            let path =
                create_ide_session_files(&dir, "ws", "sess_prop", session_json, &messages_jsonl);
            let messages = parse_kiro_file(&path);
            assert_eq!(messages.len(), 1, "case {user_chars}/{ctx_pct}");
            let m = &messages[0];

            let input = est_tokens(user_chars);
            let total_context = if ctx_pct > 0.0 {
                (200_000f64 * ctx_pct / 100.0).floor() as i64
            } else {
                0
            };
            let expected_cache_read = (total_context - input).max(0);

            assert!(
                m.tokens.cache_read >= 0,
                "cache_read never negative (case {user_chars}/{ctx_pct})"
            );
            assert_eq!(
                m.tokens.cache_read, expected_cache_read,
                "cache_read = max(floor(ctx%/100 * 200000) - input, 0) (case {user_chars}/{ctx_pct})"
            );
            assert_eq!(
                m.tokens.input, input,
                "input = ceil(user_content_chars / 4) (case {user_chars}/{ctx_pct})"
            );
        }
    }

    // =====================================================================
    // Task 13 — Property 8: Cross-Path Preservation
    //
    // Property 8 (design "Amended correctness properties"): for any turn in
    // any of the three Kiro paths, the fixed parsers SHALL preserve
    // timestamp / duration_ms, model / provider / workspace attribution, the
    // per-path dedup key, and the real-token-field precedence; and SHALL leave
    // the SQLite path's already-fixed behavior, the IDE flat-format fallback,
    // globalStorage/.chat snapshots, and every non-Kiro source unchanged.
    //
    // Methodology is OBSERVATION-FIRST: every assertion below encodes a value
    // recorded by running the UNFIXED parser (the code shipping before the
    // tasks 14/15 hybrid fix). These tests therefore PASS on the current
    // (unfixed) code and pin the baseline the CLI/IDE fix must NOT regress.
    //
    // Fields whose value the fix WILL change (input / output / cache_read on
    // the fallback estimate paths) are deliberately NOT asserted with a
    // specific number here — asserting them would either fail now or fail
    // after the fix. Only the preserved fields (metadata, dedup, model,
    // provider, workspace, message_count, and real-count precedence) are
    // pinned. Token values are asserted only where the fix keeps them
    // (real-count-present turns where precedence wins).
    // =====================================================================

    // --- Invariant 1: CLI path metadata / dedup / message_count ----------
    //
    // A normal CLI turn: assert only the metadata/dedup/message_count/model/
    // provider/workspace invariants the fix preserves. Token counts are NOT
    // asserted here because the fallback estimate changes under the fix. The
    // dedup key is "{session_id}:{index}", message_count is
    // total_request_count.max(1), timestamp/duration derive from the prompt
    // timestamp and end_timestamp.
    #[test]
    fn test_kiro_cli_preservation_metadata_dedup_message_count() {
        let dir = TempDir::new().unwrap();
        let json = r#"{
            "session_id": "sess-preserve-cli",
            "cwd": "/tmp/project",
            "session_state": {
                "rts_model_state": {
                    "model_info": {
                        "model_id": "claude-sonnet-4-5",
                        "context_window_tokens": 200000
                    }
                },
                "conversation_metadata": {
                    "user_turn_metadatas": [{
                        "input_token_count": 0,
                        "output_token_count": 0,
                        "context_usage_percentage": 3.9788,
                        "total_request_count": 4,
                        "end_timestamp": 1770983427,
                        "message_ids": ["prompt-1", "assistant-1"]
                    }]
                }
            }
        }"#;
        let jsonl = r#"{"version":"v1","kind":"Prompt","data":{"message_id":"prompt-1","content":[{"kind":"text","data":"hello world"}],"meta":{"timestamp":1770983426.420942}}}
{"version":"v1","kind":"AssistantMessage","data":{"message_id":"assistant-1","content":[{"kind":"text","data":"response text"}]}}"#;
        let path = create_session_files(&dir, "sess-preserve-cli", json, jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1, "the single turn emits one message");
        let m = &messages[0];

        // Attribution (preserved by the fix).
        assert_eq!(m.client, CLIENT_ID);
        assert_eq!(m.provider_id, PROVIDER_ID);
        assert_eq!(m.model_id, "claude-sonnet-4-5");
        assert_eq!(m.session_id, "sess-preserve-cli");
        assert_eq!(m.workspace_key, Some("/tmp/project".to_string()));
        assert_eq!(m.workspace_label, Some("project".to_string()));

        // Dedup key = "{session_id}:{index}" (index 0 for the first turn).
        assert_eq!(m.dedup_key, Some("sess-preserve-cli:0".to_string()));

        // message_count = total_request_count.max(1) = 4.
        assert_eq!(m.message_count, 4);

        // Timestamp/duration derivation from prompt + end timestamps.
        assert_eq!(m.timestamp, 1_770_983_426_420);
        assert_eq!(m.duration_ms, Some(580));

        assert!(m.is_turn_start);
    }

    // --- Invariant 2: CLI real-count precedence --------------------------
    //
    // A CLI turn carrying REAL nonzero input_token_count / output_token_count.
    // The unfixed parser already prefers explicit_input / explicit_output when
    // > 0 (see the `if explicit_input > 0 { explicit_input } else ...` branch
    // in parse_kiro_file). The fix keeps real counts as the highest-priority
    // branch, so these exact counts must survive — asserting them pins the
    // precedence contract. The cumulative context_usage_percentage here would
    // otherwise estimate input = 3.9788% * 200000 = 7957, so a passing real
    // count of 5000 proves precedence (not the estimate) is in effect.
    #[test]
    fn test_kiro_cli_preservation_real_token_counts_win() {
        let dir = TempDir::new().unwrap();
        let json = r#"{
            "session_id": "sess-real-cli",
            "cwd": "/tmp/project",
            "session_state": {
                "rts_model_state": {
                    "model_info": {
                        "model_id": "auto",
                        "context_window_tokens": 200000
                    }
                },
                "conversation_metadata": {
                    "user_turn_metadatas": [{
                        "input_token_count": 5000,
                        "output_token_count": 250,
                        "context_usage_percentage": 3.9788,
                        "total_request_count": 1,
                        "message_ids": ["p1", "a1"]
                    }]
                }
            }
        }"#;
        let jsonl = "";
        let path = create_session_files(&dir, "sess-real-cli", json, jsonl);

        let messages = parse_kiro_file(&path);

        assert_eq!(messages.len(), 1, "the single turn emits one message");
        let m = &messages[0];

        // Real counts win over the context-percentage / char estimate. This
        // holds both before AND after the fix.
        assert_eq!(
            m.tokens.input, 5000,
            "real input_token_count wins over the 7957 context-percentage estimate"
        );
        assert_eq!(
            m.tokens.output, 250,
            "real output_token_count wins over the char estimate"
        );
        assert_eq!(m.message_count, 1);
    }

    // --- Invariant 3: IDE flat-format fallback ---------------------------
    //
    // A messages.jsonl in the FLAT format (plain {"role":...,"content":...}
    // lines, no "payload" wrapper) exercises the flat-format aggregated-message
    // branch, which the structured-path (tasks 15) fix does not touch.
    // Observed on the unfixed parser: a single aggregated message with the
    // "{session_id}:ide-session" dedup key, char-estimated input/output, and
    // message_count = number of assistant turns.
    #[test]
    fn test_kiro_ide_preservation_flat_format_fallback_unchanged() {
        let session_json = r#"{
            "schemaVersion": "1.0.0",
            "id": "sess_flat_preserve",
            "createdAt": "2026-06-30T12:57:10.000Z",
            "lastModifiedAt": "2026-06-30T12:57:12.000Z"
        }"#;
        // Flat format: no `payload` wrapper. Two assistant turns.
        let messages_jsonl = concat!(
            "{\"role\":\"user\",\"content\":\"first question here\"}\n",
            "{\"role\":\"assistant\",\"content\":\"first answer\"}\n",
            "{\"role\":\"user\",\"content\":\"second question\"}\n",
            "{\"role\":\"assistant\",\"content\":\"second answer\"}\n",
        );

        let dir = TempDir::new().unwrap();
        let path = create_ide_session_files(
            &dir,
            "ws",
            "sess_flat_preserve",
            session_json,
            messages_jsonl,
        );

        let messages = parse_kiro_file(&path);

        assert_eq!(
            messages.len(),
            1,
            "flat format aggregates into a single message"
        );
        let m = &messages[0];

        // Flat-format-specific dedup key and aggregation (preserved).
        assert_eq!(m.client, CLIENT_ID);
        assert_eq!(m.provider_id, PROVIDER_ID);
        assert_eq!(m.session_id, "sess_flat_preserve");
        assert_eq!(
            m.dedup_key,
            Some("sess_flat_preserve:ide-session".to_string())
        );
        // Two assistant turns -> message_count 2 (flat_assistant_turns.max(1)).
        assert_eq!(m.message_count, 2);
        // Char-estimated tokens: this branch is untouched by the structured fix.
        assert!(m.tokens.input > 0 && m.tokens.output > 0);
        assert_eq!(m.tokens.cache_read, 0);
        assert!(m.is_turn_start);
    }

    // --- Invariant 4: IDE structured turn-flush / skip guard -------------
    //
    // The structured-format branch flushes a turn on `turn_end` only when
    // prompt_chars > 0 || assistant_chars > 0, and the per-turn emit skips a
    // resolved input + output == 0 turn. Here two structured turns are present:
    // one with content (flushed and emitted) and one zero-content turn (never
    // flushed). Observed on the unfixed parser: exactly one message emitted.
    // These guards are preserved by the fix.
    #[test]
    fn test_kiro_ide_preservation_structured_turn_flush_and_skip_guard() {
        let session_json = r#"{
            "schemaVersion": "1.0.0",
            "id": "sess_flush_preserve"
        }"#;
        // Turn 1: has user + assistant content -> flushed on turn_end.
        // Turn 2: only a contextUsage metadata line, no user/assistant content
        //         -> prompt_chars == 0 && assistant_chars == 0 -> NOT flushed.
        let messages_jsonl = concat!(
            "{\"payload\":{\"type\":\"user\",\"content\":\"hello world\"}}\n",
            "{\"payload\":{\"type\":\"assistant\",\"content\":\"response text\"}}\n",
            "{\"payload\":{\"type\":\"turn_end\"},\"timestamp\":\"2026-06-20T10:00:05Z\"}\n",
            "{\"payload\":{\"type\":\"session_metadata\",\"key\":\"contextUsage\",\"value\":{\"usagePercentage\":10.0}}}\n",
            "{\"payload\":{\"type\":\"turn_end\"},\"timestamp\":\"2026-06-20T10:00:10Z\"}\n",
        );

        let dir = TempDir::new().unwrap();
        let path = create_ide_session_files(
            &dir,
            "ws",
            "sess_flush_preserve",
            session_json,
            messages_jsonl,
        );

        let messages = parse_kiro_file(&path);

        // Only the content-bearing turn is flushed and emitted; the
        // zero-content turn is skipped by the flush guard.
        assert_eq!(
            messages.len(),
            1,
            "zero-content structured turn is skipped by the flush guard"
        );
        let m = &messages[0];
        assert_eq!(m.client, CLIENT_ID);
        assert_eq!(m.provider_id, PROVIDER_ID);
        assert!(m.is_turn_start);
        // Structured per-turn dedup key shape "{session_id}:ide:{index}".
        assert_eq!(m.dedup_key, Some("sess_flush_preserve:ide:0".to_string()));
    }

    // --- Invariant 5: Non-Kiro / globalStorage snapshot regression -------
    //
    // The globalStorage/.chat snapshot path and the SQLite path are untouched
    // by the CLI/IDE (tasks 14/15) fix. The SQLite path is covered by the
    // already-passing tasks 1-10 tests, and the .chat snapshot path by
    // test_kiro_non_sqlite_file_source_aggregation_preserved and the
    // parse_kiro_chat_artifact_* tests. This adds one explicit assertion that a
    // globalStorage .chat file still parses to the same shape, confirming the
    // cross-path preservation contract for the snapshot source.
    #[test]
    fn test_kiro_globalstorage_chat_snapshot_shape_preserved() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join(
            "Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent/workspace-a/0c433dc89e4c1803dd6fe838634ed7fc.chat",
        );
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(
            &file_path,
            r#"{
                "executionId": "5b40545a-2539-4334-9411-23df0bfea51b",
                "actionId": "act",
                "chat": [
                    {"role": "human", "content": "please refactor the loader"},
                    {"role": "tool", "content": "You are operating in a workspace"},
                    {"role": "bot", "content": "Done, refactored."}
                ],
                "metadata": {}
            }"#,
        )
        .unwrap();

        let messages = parse_kiro_file(&file_path);

        // Same shape as parse_kiro_chat_artifact_counts_human_and_bot_roles —
        // the snapshot path is unchanged by the CLI/IDE fix.
        assert_eq!(messages.len(), 1);
        let m = &messages[0];
        assert_eq!(m.client, CLIENT_ID);
        assert_eq!(m.provider_id, PROVIDER_ID);
        // human: 26 chars -> 7; bot: 17 chars -> 5; tool line excluded.
        assert_eq!(m.tokens.input, 7);
        assert_eq!(m.tokens.output, 5);
        assert_eq!(m.tokens.cache_read, 0);
        assert_eq!(m.tokens.cache_write, 0);
        assert_eq!(m.tokens.reasoning, 0);
        assert_eq!(m.workspace_key, Some("workspace-a".to_string()));
        // CostSource is unused here but referenced to confirm the snapshot path
        // leaves cost handling untouched (no provider-reported credit).
        assert_eq!(m.cost_source, CostSource::Unknown);
    }
}
