//! Kiro session parser
//!
//! Parses session data from four sources:
//! 1. File-based (Kiro CLI): ~/.kiro/sessions/cli/*.json + *.jsonl
//! 2. Kiro IDE globalStorage snapshots
//! 3. SQLite-based: ~/Library/Application Support/kiro-cli/data.sqlite3
//!    (conversations_v2 table with history[*].request_metadata)
//! 4. File-based (Kiro IDE): ~/.kiro/sessions/<workspace>/sess_<uuid>/
//!    session.json (metadata) + messages.jsonl (conversation). This is the
//!    VS Code-based Kiro IDE layout, distinct from the CLI's cli/*.json layout.
//!
//! Turn-level token counts are currently zero in both sources, so usage is
//! estimated from context_usage_percentage * context_window (input) and
//! response_size / 4 (output).

use super::utils::file_modified_timestamp_ms;
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use tracing::warn;

const CLIENT_ID: &str = "kiro";
const PROVIDER_ID: &str = "amazon-bedrock";
const UNKNOWN_MODEL: &str = "auto";

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

    if is_kiro_global_storage_path(path)
        || path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("chat"))
            .unwrap_or(false)
    {
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

    let jsonl_path = path.with_extension("jsonl");
    let mut content_by_message_id: HashMap<String, KiroMessageContent> = HashMap::new();

    if let Ok(jsonl_file) = std::fs::File::open(&jsonl_path) {
        let reader = BufReader::new(jsonl_file);
        let mut pending_prompt: Option<(usize, Option<i64>)> = None;

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let mut bytes = trimmed.as_bytes().to_vec();
            let entry = match simd_json::from_slice::<KiroJsonlEntry>(&mut bytes) {
                Ok(entry) => entry,
                Err(_) => continue,
            };

            let Some(data) = entry.data else {
                continue;
            };
            let Some(message_id) = data.message_id else {
                continue;
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
                }
                _ => {}
            }
        }
    }

    turns
        .into_iter()
        .enumerate()
        .filter_map(|(index, turn)| {
            let message_ids = turn.message_ids.unwrap_or_default();
            let mut prompt_chars = 0;
            let mut assistant_chars = 0;
            let mut prompt_timestamp_ms = None;

            for message_id in message_ids.iter().flatten() {
                let Some(content) = content_by_message_id.get(message_id) else {
                    continue;
                };
                prompt_chars += content.prompt_chars;
                assistant_chars += content.assistant_chars;
                if prompt_timestamp_ms.is_none() {
                    prompt_timestamp_ms = content.prompt_timestamp_ms;
                }
            }

            // NOTE: when explicit per-turn counts are absent (the common case —
            // Kiro currently reports zero), input/output below are ESTIMATED, not
            // measured: input is derived from context_usage_percentage *
            // context_window and output from char_count / 4. Downstream must not
            // treat these as exact token counts.
            let explicit_input = turn.input_token_count.unwrap_or(0).max(0);
            let explicit_output = turn.output_token_count.unwrap_or(0).max(0);
            let input = if explicit_input > 0 {
                explicit_input
            } else if context_window > 0 {
                let ctx_pct = turn.context_usage_percentage.unwrap_or(0.0);
                if ctx_pct > 0.0 {
                    ((context_window as f64) * ctx_pct / 100.0) as i64
                } else {
                    estimate_tokens(prompt_chars)
                }
            } else {
                estimate_tokens(prompt_chars)
            };
            let output = if explicit_output > 0 {
                explicit_output
            } else {
                estimate_tokens(assistant_chars)
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
                    cache_read: 0,
                    cache_write: 0,
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
        .collect()
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

fn estimate_tokens(chars: usize) -> i64 {
    chars.div_ceil(4) as i64
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

fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn is_kiro_global_storage_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains("globalStorage") && path_str.contains("kiro.kiroagent")
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

    let jsonl_file = match std::fs::File::open(&messages_path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let reader = BufReader::new(jsonl_file);

    const DEFAULT_CONTEXT_WINDOW: i64 = 200_000;

    #[derive(Default)]
    struct IdeTurn {
        prompt_chars: usize,
        assistant_chars: usize,
        prompt_timestamp_ms: Option<i64>,
        end_timestamp_ms: Option<i64>,
        context_usage_percentage: f64,
        elapsed_ms: Option<i64>,
    }

    let mut turns: Vec<IdeTurn> = Vec::new();
    let mut current_turn: Option<IdeTurn> = None;
    let mut has_structured_format = false;

    // Fallback accumulators for flat-JSON messages.jsonl (no payload wrapper)
    let mut flat_counts = KiroSnapshotTextCounts::default();
    let mut flat_model_id: Option<String> = None;
    let mut flat_assistant_turns: i32 = 0;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let entry: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
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
                        let args_chars = payload
                            .get("args")
                            .map(|v| match v {
                                Value::String(s) => s.chars().count(),
                                other => other.to_string().chars().count(),
                            })
                            .unwrap_or(0);
                        if let Some(turn) = current_turn.as_mut() {
                            turn.assistant_chars += args_chars;
                        }
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
                    }
                    "turn_end" => {
                        if let Some(turn) = current_turn.as_mut() {
                            turn.end_timestamp_ms = timestamp_ms;
                        }
                        if let Some(turn) = current_turn.take() {
                            if turn.prompt_chars > 0 || turn.assistant_chars > 0 {
                                turns.push(turn);
                            }
                        }
                    }
                    _ => {}
                }
                continue;
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
    }

    // Flush any in-flight structured turn
    if let Some(turn) = current_turn.take() {
        if turn.prompt_chars > 0 || turn.assistant_chars > 0 {
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
                let input = if turn.context_usage_percentage > 0.0 {
                    ((DEFAULT_CONTEXT_WINDOW as f64) * turn.context_usage_percentage / 100.0) as i64
                } else {
                    estimate_tokens(turn.prompt_chars)
                };
                let output = estimate_tokens(turn.assistant_chars);

                if input + output == 0 {
                    return None;
                }

                let timestamp = turn
                    .prompt_timestamp_ms
                    .or(turn.end_timestamp_ms)
                    .unwrap_or(fallback_timestamp);
                let duration_ms = turn.elapsed_ms.or_else(|| {
                    duration_between_ms(turn.prompt_timestamp_ms, turn.end_timestamp_ms)
                });

                let mut message = UnifiedMessage::new_with_dedup(
                    CLIENT_ID,
                    model_id.clone(),
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
                    Some(format!("{}:ide:{}", session_id, index)),
                );
                message.message_count = 1;
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
    let conn = match Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open Kiro CLI database"
            );
            return Vec::new();
        }
    };

    let query = "SELECT key, conversation_id, value FROM conversations_v2";
    let mut stmt = match conn.prepare(query) {
        Ok(s) => s,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to prepare Kiro conversations query"
            );
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) {
        Ok(r) => r,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to execute Kiro conversations query"
            );
            return Vec::new();
        }
    };

    let mut messages = Vec::new();

    for row in rows.flatten() {
        let (cwd, conversation_id, json_str) = row;
        let parsed = match serde_json::from_str::<KiroDbConversation>(&json_str) {
            Ok(p) => p,
            Err(_) => continue,
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

        let history = parsed.history.unwrap_or_default();
        for (index, turn) in history.into_iter().enumerate() {
            let Some(meta) = turn.request_metadata else {
                continue;
            };

            // NOTE: these are ESTIMATED, not measured token counts. Kiro's
            // conversations_v2 does not record real per-turn token usage, so
            // input is derived from context_usage_percentage * context_window
            // and output from response_size (char_count) / 4. Downstream must
            // not treat these as exact.
            let ctx_pct = meta.context_usage_percentage.unwrap_or(0.0);
            let response_size = meta.response_size.unwrap_or(0);

            let input = if context_window > 0 && ctx_pct > 0.0 {
                ((context_window as f64) * ctx_pct / 100.0) as i64
            } else {
                0
            };
            let output = estimate_tokens(response_size);

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
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.0,
                Some(format!("{}:{}", conversation_id, index)),
            );
            message.message_count = 1;
            message.duration_ms = duration_ms;
            message.is_turn_start = true;
            message.set_workspace(workspace_key.clone(), workspace_label.clone());
            messages.push(message);
        }
    }

    messages
}

#[derive(Debug, Deserialize)]
struct KiroDbConversation {
    history: Option<Vec<KiroDbTurn>>,
    model_info: Option<KiroModelInfo>,
}

#[derive(Debug, Deserialize)]
struct KiroDbTurn {
    request_metadata: Option<KiroDbRequestMetadata>,
}

#[derive(Debug, Deserialize)]
struct KiroDbRequestMetadata {
    context_usage_percentage: Option<f64>,
    response_size: Option<usize>,
    request_start_timestamp_ms: Option<i64>,
    stream_end_timestamp_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

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
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 10);
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
}
