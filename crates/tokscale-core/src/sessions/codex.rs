//! Codex CLI session parser
//!
//! Parses JSONL files from ~/.codex/sessions/
//! Note: This parser has stateful logic to track model and delta calculations.

use super::utils::{
    extract_i64, extract_string, file_modified_timestamp_ms, parse_timestamp_value,
};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

/// Codex entry structure (from JSONL files)
#[derive(Debug, Deserialize)]
pub struct CodexEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub timestamp: Option<String>,
    pub payload: Option<CodexPayload>,
}

#[derive(Debug, Deserialize)]
pub struct CodexPayload {
    pub id: Option<String>,
    pub forked_from_id: Option<String>,
    #[serde(rename = "type")]
    pub payload_type: Option<String>,
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub model_info: Option<CodexModelInfo>,
    pub info: Option<CodexInfo>,
    pub turn_id: Option<String>,
    pub source: Option<Value>,
    /// Thread origin from session_meta. `"user"` marks a human-initiated fork
    /// (e.g. a VS Code "fork conversation"), which replays parent history but
    /// never emits a `task_started` for the child's own turn.
    pub thread_source: Option<String>,
    /// Current working directory from session_meta.
    pub cwd: Option<String>,
    /// Provider identity from session_meta (e.g. "openai", "azure")
    pub model_provider: Option<String>,
    /// Agent name from session_meta
    pub agent_nickname: Option<String>,
    /// Free-text body of an `event_msg` `user_message` payload. Used to detect
    /// human turn boundaries: real human input is plain text, whereas
    /// system-injected context (`<environment_context>`, `<system-reminder>`,
    /// `<user_instructions>`, …) begins with `<`.
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodexModelInfo {
    pub slug: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodexInfo {
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub last_token_usage: Option<CodexTokenUsage>,
    pub total_token_usage: Option<CodexTokenUsage>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CodexTokenUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CodexTotals {
    input: i64,
    output: i64,
    cached: i64,
    reasoning: i64,
}

impl CodexTotals {
    fn from_usage(usage: &CodexTokenUsage) -> Self {
        Self {
            input: usage.input_tokens.unwrap_or(0).max(0),
            output: usage.output_tokens.unwrap_or(0).max(0),
            cached: usage
                .cached_input_tokens
                .unwrap_or(0)
                .max(usage.cache_read_input_tokens.unwrap_or(0))
                .max(0),
            reasoning: usage.reasoning_output_tokens.unwrap_or(0).max(0),
        }
    }

    fn delta_from(self, previous: Self) -> Option<Self> {
        if self.input < previous.input
            || self.output < previous.output
            || self.cached < previous.cached
            || self.reasoning < previous.reasoning
        {
            return None;
        }

        Some(Self {
            input: self.input - previous.input,
            output: self.output - previous.output,
            cached: self.cached - previous.cached,
            reasoning: self.reasoning - previous.reasoning,
        })
    }

    fn saturating_add(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            output: self.output.saturating_add(other.output),
            cached: self.cached.saturating_add(other.cached),
            reasoning: self.reasoning.saturating_add(other.reasoning),
        }
    }

    fn total(self) -> i64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cached)
            .saturating_add(self.reasoning)
    }

    fn is_within(self, baseline: Self) -> bool {
        self.input <= baseline.input
            && self.output <= baseline.output
            && self.cached <= baseline.cached
            && self.reasoning <= baseline.reasoning
    }

    fn looks_like_stale_regression(self, previous: Self, last: Self) -> bool {
        let previous_total = previous.total();
        let current_total = self.total();
        let last_total = last.total();

        if previous_total <= 0 || current_total <= 0 || last_total <= 0 {
            return false;
        }

        // Some Codex token_count snapshots arrive slightly out of order: the cumulative
        // total regresses by roughly one recent increment, then resumes from the true
        // higher watermark on the next row. Treat those as stale snapshots rather than
        // hard resets so we do not count `last_token_usage` twice.
        current_total.saturating_mul(100) >= previous_total.saturating_mul(98)
            || current_total.saturating_add(last_total.saturating_mul(2)) >= previous_total
    }

    fn into_tokens(self) -> TokenBreakdown {
        // Clamp cached to not exceed input to prevent inflated totals when
        // malformed data reports more cached tokens than input tokens.
        let clamped_cached = self.cached.min(self.input).max(0);
        TokenBreakdown {
            input: (self.input - clamped_cached).max(0),
            output: self.output.max(0),
            cache_read: clamped_cached,
            cache_write: 0,
            reasoning: self.reasoning.max(0),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct CodexParseState {
    pub current_model: Option<String>,
    #[serde(default)]
    pub current_turn_start_ms: Option<i64>,
    pub previous_totals: Option<CodexTotals>,
    pub session_is_headless: bool,
    pub session_id_from_meta: Option<String>,
    pub session_forked_from_id: Option<String>,
    pub forked_child_session_id: Option<String>,
    pub forked_child_replay_session_id: Option<String>,
    pub session_provider: Option<String>,
    pub session_agent: Option<String>,
    pub session_workspace_key: Option<String>,
    pub session_workspace_label: Option<String>,
    pub forked_child_waiting_for_turn_context: bool,
    pub forked_child_inherited_baseline: Option<CodexTotals>,
    pub forked_child_inherited_reported_total: Option<i64>,
    /// Set when a human `user_message` event is seen; consumed by the next
    /// token_count-derived message to mark it as a turn start. `#[serde(default)]`
    /// keeps a pending turn alive across incremental re-parses of appended chunks.
    #[serde(default)]
    pub pending_turn_start: bool,
    /// `turn_id`s announced by a `task_started` event while a forked child is
    /// still skipping its replayed parent history. The child's own turn is
    /// preceded by `task_started`; replayed parent turns are not. Used only to
    /// disambiguate a same-millisecond turn, where the UUID v7 timestamp ties
    /// and the random tail is meaningless — there, only a task-started turn_id
    /// ends the skip. Cleared when the skip ends or a new fork begins.
    /// `#[serde(default)]` keeps it across incremental re-parses.
    #[serde(default)]
    pub forked_child_task_started_turn_ids: std::collections::HashSet<String>,
    /// Set when the active forked child is a human-initiated (`thread_source:
    /// "user"`) fork. Such forks replay parent history but never emit a
    /// `task_started`, so the same-millisecond gate cannot lean on
    /// `task_started` to recognize the child's own turn — there the millisecond
    /// prefix tie is enough (a user fork's replayed parent turns carry the
    /// parent's millisecond prefix, not the child's). `#[serde(default)]` keeps
    /// it across incremental re-parses.
    #[serde(default)]
    pub forked_child_is_user_fork: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedCodexFile {
    pub messages: Vec<UnifiedMessage>,
    pub fallback_timestamp_indices: Vec<usize>,
    pub consumed_offset: u64,
    pub parse_succeeded: bool,
    /// True when model-less token_count rows were emitted without a later model.
    pub unresolved_model_events: bool,
    pub state: CodexParseState,
}

fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn codex_workspace_from_cwd(cwd: &str) -> (Option<String>, Option<String>) {
    let workspace_key = normalize_codex_workspace_key(cwd);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

    if workspace_label.is_none() {
        return (None, None);
    }

    (workspace_key, workspace_label)
}

fn normalize_codex_workspace_key(raw: &str) -> Option<String> {
    let normalized = normalize_workspace_key(raw)?;
    if normalized.chars().any(char::is_control) {
        return None;
    }

    if looks_like_explicit_workspace_path(&normalized) {
        Some(normalized)
    } else {
        None
    }
}

fn looks_like_explicit_workspace_path(path: &str) -> bool {
    if path.starts_with("//") || path.starts_with('/') {
        return true;
    }

    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

fn parse_codex_reader<R: BufRead>(
    mut reader: R,
    session_id: &str,
    fallback_timestamp: i64,
    start_offset: u64,
    mut state: CodexParseState,
) -> ParsedCodexFile {
    let mut messages = Vec::with_capacity(64);
    let mut fallback_timestamp_indices = Vec::new();
    let mut buffer = Vec::with_capacity(4096);
    let mut line = String::with_capacity(4096);
    let mut consumed_offset = start_offset;
    let mut parse_succeeded = true;
    let mut pending_model_messages = Vec::new();
    let mut unresolved_model_events = false;

    loop {
        line.clear();
        let bytes_read = match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(bytes_read) => bytes_read,
            Err(_) => {
                parse_succeeded = false;
                break;
            }
        };
        consumed_offset += bytes_read as u64;

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut handled = false;
        buffer.clear();
        buffer.extend_from_slice(trimmed.as_bytes());
        if let Ok(entry) = simd_json::from_slice::<CodexEntry>(&mut buffer) {
            if let Some(payload) = entry.payload {
                let payload_model = extract_model(&payload);
                let is_token_count = entry.entry_type == "event_msg"
                    && payload.payload_type.as_deref() == Some("token_count");
                let info_model = if is_token_count {
                    payload.info.as_ref().and_then(extract_model_from_info)
                } else {
                    None
                };
                let event_model = payload_model.clone().or(info_model.clone());

                if state.forked_child_waiting_for_turn_context {
                    if entry.entry_type == "turn_context"
                        && forked_child_turn_starts_own_session(&state, payload.turn_id.as_deref())
                    {
                        state.forked_child_waiting_for_turn_context = false;
                        state.forked_child_replay_session_id = None;
                        state.forked_child_task_started_turn_ids.clear();
                        state.forked_child_is_user_fork = false;
                        if let Some(ref id) = state.forked_child_session_id {
                            state.session_id_from_meta = Some(id.clone());
                        }
                        state.current_model = payload_model.clone();
                        handled = true;
                    } else {
                        if entry.entry_type == "event_msg"
                            && payload.payload_type.as_deref() == Some("task_started")
                        {
                            // The child's own turn is introduced by a
                            // `task_started`; remember its turn_id so the gate can
                            // recognize the child's own same-millisecond turn.
                            if let Some(turn_id) = payload.turn_id.as_deref() {
                                state
                                    .forked_child_task_started_turn_ids
                                    .insert(turn_id.to_string());
                            }
                        }
                        if entry.entry_type == "session_meta" {
                            if let Some(ref id) = payload.id {
                                if state
                                    .forked_child_session_id
                                    .as_deref()
                                    .is_some_and(|child_id| child_id != id)
                                {
                                    // Newer Codex fork logs can embed the
                                    // parent session metadata before replaying
                                    // parent token_count history. Keep
                                    // skipping while that copied upstream
                                    // transcript is active.
                                    state.forked_child_replay_session_id = Some(id.clone());
                                }
                            }
                        }
                        if is_token_count {
                            if let Some(info) = payload.info.as_ref() {
                                remember_forked_child_inherited_baseline(&mut state, info);
                            }
                        }
                        continue;
                    }
                }

                if !pending_model_messages.is_empty()
                    && event_model.is_none()
                    && !is_token_count
                    && entry.entry_type != "session_meta"
                {
                    flush_pending_model_messages_as_unknown(
                        &mut pending_model_messages,
                        &mut messages,
                        &mut fallback_timestamp_indices,
                        &mut unresolved_model_events,
                    );
                }

                if entry.entry_type == "session_meta" {
                    if codex_source_is_exec(payload.source.as_ref()) {
                        state.session_is_headless = true;
                    }
                    if let Some(ref id) = payload.id {
                        state.session_id_from_meta = Some(id.clone());
                    }
                    let forked_from_id = payload
                        .forked_from_id
                        .as_deref()
                        .filter(|id| !id.is_empty())
                        .or_else(|| forked_from_id_from_source(payload.source.as_ref()));
                    if let Some(forked_from_id) = forked_from_id {
                        let repeated_active_child_meta = !state
                            .forked_child_waiting_for_turn_context
                            && payload.id.as_deref().is_some()
                            && state.forked_child_session_id.as_deref() == payload.id.as_deref();
                        state.session_forked_from_id = Some(forked_from_id.to_string());
                        state.forked_child_session_id = payload.id.clone();
                        if !repeated_active_child_meta {
                            state.forked_child_waiting_for_turn_context = true;
                            state.forked_child_replay_session_id = None;
                            state.forked_child_inherited_baseline = None;
                            state.forked_child_inherited_reported_total = None;
                            state.forked_child_task_started_turn_ids.clear();
                            state.forked_child_is_user_fork =
                                payload.thread_source.as_deref() == Some("user");
                        }
                    }
                    if let Some(ref provider) = payload.model_provider {
                        state.session_provider = Some(provider.clone());
                    }
                    if let Some(ref nickname) = payload.agent_nickname {
                        state.session_agent = Some(nickname.clone());
                    }
                    if let Some(ref cwd) = payload.cwd {
                        let (workspace_key, workspace_label) = codex_workspace_from_cwd(cwd);
                        state.session_workspace_key = workspace_key;
                        state.session_workspace_label = workspace_label;
                    }
                }
                // Extract model from turn_context
                if entry.entry_type == "turn_context" {
                    state.current_model = payload_model.clone();
                    state.current_turn_start_ms =
                        parse_codex_entry_timestamp(entry.timestamp.as_deref());
                    if let Some(model) = state.current_model.clone() {
                        flush_pending_model_messages(
                            &mut pending_model_messages,
                            &mut messages,
                            &mut fallback_timestamp_indices,
                            &model,
                        );
                    }
                    handled = true;
                }

                // A human `user_message` event starts a new turn. The event
                // itself carries no tokens, so we defer the flag to the next
                // token_count-derived message (the assistant's reply). This
                // counts `codex exec` one-shots too: they are headless but still
                // carry a real human prompt, so each is one turn. Only
                // system-injected messages (leading `<`, e.g.
                // <environment_context>, <system-reminder>) are excluded as
                // non-human input. Forked-child replays of the parent prompt
                // arrive before turn_context and are skipped by the
                // `forked_child_waiting_for_turn_context` branch above, so they
                // never reach here.
                if entry.entry_type == "event_msg"
                    && payload.payload_type.as_deref() == Some("user_message")
                {
                    if codex_message_is_human_turn(payload.message.as_deref()) {
                        state.pending_turn_start = true;
                    }
                    handled = true;
                }

                // Process token_count events
                if is_token_count {
                    let info = match payload.info {
                        Some(i) => i,
                        None => continue,
                    };

                    let model = payload_model
                        .or(info_model)
                        .or_else(|| state.current_model.clone());
                    if let Some(ref model) = model {
                        state.current_model = Some(model.clone());
                        flush_pending_model_messages(
                            &mut pending_model_messages,
                            &mut messages,
                            &mut fallback_timestamp_indices,
                            model,
                        );
                    }

                    // Use last_token_usage as the primary increment source.
                    // Upstream totals are mutable snapshots (compaction, context-window
                    // capping can rewrite them), so we only use total_token_usage for
                    // dedup and monotonicity checks — never as a direct delta source.
                    let total_usage = info.total_token_usage.as_ref().map(CodexTotals::from_usage);
                    let last_usage = info.last_token_usage.as_ref().map(CodexTotals::from_usage);

                    // Forked child logs can replay more than one parent
                    // token_count row after the first child turn_context,
                    // often with child-local timestamps. Keep the inherited
                    // baseline active until totals move beyond it.
                    if forked_child_should_skip_inherited_snapshot(
                        &state,
                        info.total_token_usage.as_ref(),
                        total_usage,
                    ) {
                        continue;
                    }
                    state.forked_child_inherited_baseline = None;
                    state.forked_child_inherited_reported_total = None;

                    let (tokens, next_totals) =
                        match (total_usage, last_usage, state.previous_totals) {
                            // Both present with previous baseline (standard path)
                            (Some(total), Some(last), Some(previous)) => {
                                if total == previous {
                                    continue;
                                }
                                if total.delta_from(previous).is_none()
                                    && total.looks_like_stale_regression(previous, last)
                                {
                                    continue;
                                }
                                (last.into_tokens(), Some(total))
                            }
                            // Both present, first event — use last (NOT full total) to
                            // avoid overcounting tokens carried from a resumed session.
                            (Some(total), Some(last), None) => (last.into_tokens(), Some(total)),
                            // Only total, have previous (defensive — upstream schema
                            // requires both when info is present)
                            (Some(total), None, Some(previous)) => {
                                if total == previous {
                                    continue;
                                }
                                if let Some(delta) = total.delta_from(previous) {
                                    (delta.into_tokens(), Some(total))
                                } else {
                                    state.previous_totals = Some(total);
                                    continue;
                                }
                            }
                            // Only total, first event, no last — legacy/degraded path
                            (Some(total), None, None) => (total.into_tokens(), Some(total)),
                            // Only last, have previous
                            (None, Some(last), Some(previous)) => {
                                (last.into_tokens(), Some(previous.saturating_add(last)))
                            }
                            // Only last, no previous
                            (None, Some(last), None) => (last.into_tokens(), None),
                            // Neither
                            (None, None, _) => continue,
                        };

                    // Skip zero-token snapshots without advancing the baseline so
                    // that post-compaction zero totals don't inflate later deltas.
                    if tokens.input == 0
                        && tokens.output == 0
                        && tokens.cache_read == 0
                        && tokens.reasoning == 0
                    {
                        continue;
                    }

                    state.previous_totals = next_totals;

                    let parsed_timestamp = parse_codex_entry_timestamp(entry.timestamp.as_deref());
                    let timestamp = parsed_timestamp.unwrap_or(fallback_timestamp);
                    let duration_ms =
                        duration_between_ms(state.current_turn_start_ms, parsed_timestamp);

                    let agent = if state.session_is_headless {
                        Some("headless".to_string())
                    } else {
                        state.session_agent.clone()
                    };

                    let provider = state.session_provider.as_deref().unwrap_or("openai");

                    let mut message = UnifiedMessage::new_with_agent(
                        "codex",
                        model.clone().unwrap_or_else(|| "unknown".to_string()),
                        provider,
                        session_id.to_string(),
                        timestamp,
                        tokens,
                        0.0,
                        agent,
                    );
                    message.duration_ms = duration_ms;
                    // Apply a deferred human-turn marker from a preceding
                    // user_message to this assistant reply — the first
                    // token-bearing message after the human input.
                    if state.pending_turn_start {
                        message.is_turn_start = true;
                        state.pending_turn_start = false;
                    }
                    if parsed_timestamp.is_some() || total_usage.is_some() {
                        // Fork/subagent children replay the same upstream
                        // token_count history into many sibling files. Those
                        // replays carry identical cumulative totals but a
                        // distinct per-file session id, so a session-scoped key
                        // never collapses them and the totals get counted once
                        // per sibling. Scope the key to the fork parent instead
                        // so sibling replays share one key. Unrelated sessions
                        // keep their own id and never merge.
                        let dedup_scope_id = state
                            .session_forked_from_id
                            .as_deref()
                            .or(state.session_id_from_meta.as_deref())
                            .unwrap_or(session_id);
                        set_codex_dedup_key(
                            &mut message,
                            model.as_deref().unwrap_or("unknown"),
                            dedup_scope_id,
                            total_usage,
                        );
                    }
                    message.set_workspace(
                        state.session_workspace_key.clone(),
                        state.session_workspace_label.clone(),
                    );
                    if model.is_some() {
                        messages.push(message);
                        if parsed_timestamp.is_none() {
                            fallback_timestamp_indices.push(messages.len() - 1);
                        }
                    } else {
                        pending_model_messages.push((message, parsed_timestamp.is_none()));
                    }
                    handled = true;
                }
            }

            // Mark session_meta as handled (even if payload was processed above)
            if entry.entry_type == "session_meta" {
                handled = true;
            }
        }

        if handled {
            continue;
        }

        if state.forked_child_waiting_for_turn_context {
            let mut json_probe = trimmed.as_bytes().to_vec();
            if simd_json::from_slice::<Value>(&mut json_probe).is_ok() {
                continue;
            }
        }

        let headless_message = parse_codex_headless_line(
            trimmed,
            session_id,
            &mut state.current_model,
            fallback_timestamp,
            state.session_provider.as_deref(),
            &state.session_agent,
            state.session_is_headless,
        );
        if !pending_model_messages.is_empty() {
            if let Some(model) = state.current_model.clone() {
                flush_pending_model_messages(
                    &mut pending_model_messages,
                    &mut messages,
                    &mut fallback_timestamp_indices,
                    &model,
                );
            } else {
                flush_pending_model_messages_as_unknown(
                    &mut pending_model_messages,
                    &mut messages,
                    &mut fallback_timestamp_indices,
                    &mut unresolved_model_events,
                );
            }
        }

        if let Some((mut msg, used_fallback_timestamp)) = headless_message {
            msg.set_workspace(
                state.session_workspace_key.clone(),
                state.session_workspace_label.clone(),
            );
            messages.push(msg);
            if used_fallback_timestamp {
                fallback_timestamp_indices.push(messages.len() - 1);
            }
            continue;
        }

        let mut json_probe = trimmed.as_bytes().to_vec();
        if simd_json::from_slice::<Value>(&mut json_probe).is_err() {
            parse_succeeded = false;
            continue;
        }
    }

    flush_pending_model_messages_as_unknown(
        &mut pending_model_messages,
        &mut messages,
        &mut fallback_timestamp_indices,
        &mut unresolved_model_events,
    );

    ParsedCodexFile {
        messages,
        fallback_timestamp_indices,
        consumed_offset,
        parse_succeeded,
        unresolved_model_events,
        state,
    }
}

fn codex_source_is_exec(source: Option<&Value>) -> bool {
    source.and_then(Value::as_str) == Some("exec")
}

fn forked_from_id_from_source(source: Option<&Value>) -> Option<&str> {
    source?
        .get("subagent")?
        .get("thread_spawn")?
        .get("parent_thread_id")?
        .as_str()
        .filter(|id| !id.is_empty())
}

fn forked_child_turn_starts_own_session(state: &CodexParseState, turn_id: Option<&str>) -> bool {
    if state.forked_child_replay_session_id.is_none() {
        return true;
    }

    let Some(child_session_id) = state.forked_child_session_id.as_deref() else {
        return true;
    };

    match (turn_id, codex_uuid_v7_order_key(child_session_id)) {
        (Some(turn_id), Some(child_key)) => {
            let Some(turn_key) = codex_uuid_v7_order_key(turn_id) else {
                return true;
            };
            // Compare only the UUID v7 48-bit millisecond timestamp (the first
            // 12 hex of the order key), not the full id. The child's own turn is
            // minted at or after its session_meta and the replayed parent turns
            // strictly earlier, so the millisecond prefix is the causal signal;
            // the version nibble + random tail of two independently-minted v7
            // UUIDs is a coin flip.
            match turn_key[..12].cmp(&child_key[..12]) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Less => false,
                // Same millisecond: the timestamp ties and the random tail
                // cannot order the child's own turn against a replayed parent
                // turn that happens to share the fork's millisecond. A subagent
                // fork announces its own turn with a `task_started` event while
                // replayed parent turns are not, so only a task-started turn_id
                // ends the skip there — otherwise an equal-prefix replayed parent
                // turn would be miscounted as child-local. A human (`thread_source:
                // "user"`) fork never emits `task_started`, but its replayed
                // parent turns carry the *parent's* millisecond prefix rather
                // than the child's, so reaching this equal-prefix branch already
                // means the turn shares the child's fork millisecond and is the
                // child's own turn — end the skip.
                //
                // Residual (accepted): for a user fork this resolves on
                // millisecond-prefix equality alone. If a replayed parent turn
                // were itself minted within the exact same 1ms as the child's
                // fork (so it shares the *child's* prefix, not the parent's),
                // this branch would end the skip one turn early. That requires a
                // sub-millisecond, human-paced fork coincidence and is accepted;
                // subagent forks are hardened separately via `task_started`,
                // which user forks do not emit.
                std::cmp::Ordering::Equal => {
                    state.forked_child_is_user_fork
                        || state.forked_child_task_started_turn_ids.contains(turn_id)
                }
            }
        }
        _ => true,
    }
}

fn codex_uuid_v7_order_key(id: &str) -> Option<String> {
    let mut parts = id.split('-');
    let first = parts.next()?;
    let second = parts.next()?;
    let third = parts.next()?;
    let fourth = parts.next()?;
    let fifth = parts.next()?;

    if parts.next().is_some()
        || first.len() != 8
        || second.len() != 4
        || third.len() != 4
        || fourth.len() != 4
        || fifth.len() != 12
        || !third.starts_with('7')
    {
        return None;
    }

    let mut key = String::with_capacity(32);
    for part in [first, second, third, fourth, fifth] {
        if !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        key.push_str(&part.to_ascii_lowercase());
    }
    Some(key)
}

fn parse_codex_entry_timestamp(timestamp: Option<&str>) -> Option<i64> {
    timestamp
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.timestamp_millis())
}

fn duration_between_ms(start_ms: Option<i64>, end_ms: Option<i64>) -> Option<i64> {
    let duration = end_ms?.saturating_sub(start_ms?);
    (duration > 0).then_some(duration)
}

fn codex_token_count_dedup_key(
    message: &UnifiedMessage,
    model: &str,
    upstream_session_id: &str,
    total_usage: Option<CodexTotals>,
) -> String {
    if let Some(total) = total_usage {
        // Codex fork/subagent logs can replay the same upstream token_count
        // history into many child files with child-local timestamps. The
        // cumulative total is the stable upstream identity; timestamp is only
        // a fallback when older rows do not carry totals.
        return format!(
            "codex:token_count-total:{}:{}:{}:{}:{}:{}:{}",
            upstream_session_id,
            message.provider_id,
            model,
            total.input,
            total.output,
            total.cached,
            total.reasoning
        );
    }

    format!(
        "codex:token_count:{}:{}:{}:{}:{}:{}:{}:{}",
        message.timestamp,
        message.provider_id,
        model,
        message.tokens.input,
        message.tokens.output,
        message.tokens.cache_read,
        message.tokens.cache_write,
        message.tokens.reasoning
    )
}

fn set_codex_dedup_key(
    message: &mut UnifiedMessage,
    model: &str,
    upstream_session_id: &str,
    total_usage: Option<CodexTotals>,
) {
    if message.dedup_key.is_none() {
        message.dedup_key = Some(codex_token_count_dedup_key(
            message,
            model,
            upstream_session_id,
            total_usage,
        ));
    }
}

fn flush_pending_model_messages(
    pending_model_messages: &mut Vec<(UnifiedMessage, bool)>,
    messages: &mut Vec<UnifiedMessage>,
    fallback_timestamp_indices: &mut Vec<usize>,
    model: &str,
) {
    for (mut message, used_fallback_timestamp) in pending_model_messages.drain(..) {
        if !used_fallback_timestamp {
            let upstream_session_id = message.session_id.clone();
            set_codex_dedup_key(&mut message, model, &upstream_session_id, None);
        }
        message.model_id = model.to_string();
        messages.push(message);
        if used_fallback_timestamp {
            fallback_timestamp_indices.push(messages.len() - 1);
        }
    }
}

fn flush_pending_model_messages_as_unknown(
    pending_model_messages: &mut Vec<(UnifiedMessage, bool)>,
    messages: &mut Vec<UnifiedMessage>,
    fallback_timestamp_indices: &mut Vec<usize>,
    unresolved_model_events: &mut bool,
) {
    if pending_model_messages.is_empty() {
        return;
    }

    *unresolved_model_events = true;
    flush_pending_model_messages(
        pending_model_messages,
        messages,
        fallback_timestamp_indices,
        "unknown",
    );
}

/// Parse a Codex JSONL file with stateful tracking
pub fn parse_codex_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let session_id = session_id_from_path(path);
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let reader = BufReader::new(file);
    let parsed = parse_codex_reader(
        reader,
        &session_id,
        fallback_timestamp,
        0,
        CodexParseState::default(),
    );
    parsed.messages
}

fn reported_total_tokens(usage: &CodexTokenUsage) -> Option<i64> {
    usage.total_tokens.filter(|total| *total >= 0)
}

fn remember_forked_child_inherited_baseline(state: &mut CodexParseState, info: &CodexInfo) {
    let Some(total_usage) = info.total_token_usage.as_ref() else {
        return;
    };

    let totals = CodexTotals::from_usage(total_usage);
    state.previous_totals = Some(totals);
    state.forked_child_inherited_baseline = Some(totals);
    state.forked_child_inherited_reported_total = reported_total_tokens(total_usage);
}

fn forked_child_should_skip_inherited_snapshot(
    state: &CodexParseState,
    total_usage: Option<&CodexTokenUsage>,
    totals: Option<CodexTotals>,
) -> bool {
    if let (Some(usage), Some(baseline)) =
        (total_usage, state.forked_child_inherited_reported_total)
    {
        if reported_total_tokens(usage).is_some_and(|total| total <= baseline) {
            return true;
        }
    }

    if let (Some(totals), Some(baseline)) = (totals, state.forked_child_inherited_baseline) {
        return totals.is_within(baseline);
    }

    false
}

pub(crate) fn parse_codex_file_incremental(
    path: &Path,
    start_offset: u64,
    state: CodexParseState,
) -> ParsedCodexFile {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => {
            return ParsedCodexFile {
                messages: Vec::new(),
                fallback_timestamp_indices: Vec::new(),
                consumed_offset: start_offset,
                parse_succeeded: false,
                unresolved_model_events: false,
                state,
            };
        }
    };

    if file.seek(SeekFrom::Start(start_offset)).is_err() {
        return ParsedCodexFile {
            messages: Vec::new(),
            fallback_timestamp_indices: Vec::new(),
            consumed_offset: start_offset,
            parse_succeeded: false,
            unresolved_model_events: false,
            state,
        };
    }

    let session_id = session_id_from_path(path);
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let reader = BufReader::new(file);
    parse_codex_reader(reader, &session_id, fallback_timestamp, start_offset, state)
}

fn extract_model(payload: &CodexPayload) -> Option<String> {
    payload
        .model_info
        .as_ref()
        .and_then(|mi| mi.slug.clone())
        .filter(|s| !s.is_empty())
        .or(payload.model.clone().filter(|s| !s.is_empty()))
        .or(payload.model_name.clone().filter(|s| !s.is_empty()))
        .or(payload.info.as_ref().and_then(extract_model_from_info))
}

fn extract_model_from_info(info: &CodexInfo) -> Option<String> {
    info.model
        .clone()
        .filter(|s| !s.is_empty())
        .or(info.model_name.clone().filter(|s| !s.is_empty()))
}

struct CodexHeadlessUsage {
    input: i64,
    output: i64,
    cached: i64,
    model: Option<String>,
    timestamp_ms: Option<i64>,
}

fn parse_codex_headless_line(
    line: &str,
    session_id: &str,
    current_model: &mut Option<String>,
    fallback_timestamp: i64,
    session_provider: Option<&str>,
    session_agent: &Option<String>,
    session_is_headless: bool,
) -> Option<(UnifiedMessage, bool)> {
    let mut bytes = line.as_bytes().to_vec();
    let value: Value = simd_json::from_slice(&mut bytes).ok()?;

    if let Some(model) = extract_model_from_value(&value) {
        *current_model = Some(model);
    }

    let usage = extract_headless_usage(&value)?;
    let model = usage
        .model
        .or_else(|| current_model.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let timestamp = usage.timestamp_ms.unwrap_or(fallback_timestamp);

    if usage.input == 0 && usage.output == 0 && usage.cached == 0 {
        return None;
    }

    let provider = session_provider.unwrap_or("openai");
    let agent = if session_is_headless {
        Some("headless".to_string())
    } else {
        session_agent.clone()
    };

    Some((
        UnifiedMessage::new_with_agent(
            "codex",
            model,
            provider,
            session_id.to_string(),
            timestamp,
            TokenBreakdown {
                input: usage.input.max(0),
                output: usage.output.max(0),
                cache_read: usage.cached.max(0),
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
            agent,
        ),
        usage.timestamp_ms.is_none(),
    ))
}

fn extract_headless_usage(value: &Value) -> Option<CodexHeadlessUsage> {
    let usage = value
        .get("usage")
        .or_else(|| value.get("data").and_then(|data| data.get("usage")))
        .or_else(|| value.get("result").and_then(|data| data.get("usage")))
        .or_else(|| value.get("response").and_then(|data| data.get("usage")))?;

    let input_tokens = extract_i64(usage.get("input_tokens"))
        .or_else(|| extract_i64(usage.get("prompt_tokens")))
        .or_else(|| extract_i64(usage.get("input")))
        .unwrap_or(0);
    let output_tokens = extract_i64(usage.get("output_tokens"))
        .or_else(|| extract_i64(usage.get("completion_tokens")))
        .or_else(|| extract_i64(usage.get("output")))
        .unwrap_or(0);
    let cached_tokens = extract_i64(usage.get("cached_input_tokens"))
        .or_else(|| extract_i64(usage.get("cache_read_input_tokens")))
        .or_else(|| extract_i64(usage.get("cached_tokens")))
        .unwrap_or(0);

    let model = extract_model_from_value(value)
        .or_else(|| value.get("data").and_then(extract_model_from_value));
    let timestamp_ms = extract_timestamp_from_value(value);

    Some(CodexHeadlessUsage {
        input: input_tokens.saturating_sub(cached_tokens),
        output: output_tokens,
        cached: cached_tokens,
        model,
        timestamp_ms,
    })
}

fn extract_model_from_value(value: &Value) -> Option<String> {
    extract_string(value.get("model"))
        .or_else(|| extract_string(value.get("model_name")))
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| extract_string(data.get("model")))
        })
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| extract_string(data.get("model_name")))
        })
        .or_else(|| {
            value
                .get("response")
                .and_then(|data| extract_string(data.get("model")))
        })
}

fn extract_timestamp_from_value(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .or_else(|| value.get("time"))
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("data").and_then(|data| data.get("timestamp")))
        .and_then(parse_timestamp_value)
}

/// Prefixes Codex prepends to context it injects as `user_message` events.
/// These are the bodies that must NOT be counted as human turns.
const CODEX_SYSTEM_INJECTED_PREFIXES: [&str; 3] = [
    "<environment_context>",
    "<system-reminder>",
    "<user_instructions>",
];

/// Returns true when a Codex `user_message` payload represents real human input
/// rather than system-injected context. Codex stores the body as a plain string
/// in `payload.message`; the harness injects context blocks that open with one of
/// the known tags in [`CODEX_SYSTEM_INJECTED_PREFIXES`] after trimming. Matching
/// those specific prefixes — rather than any leading `<` — avoids dropping
/// legitimate human prompts that happen to start with markup (asking about a
/// `<div>`, pasting an XML snippet, etc.). The `kind` field can't be used to
/// distinguish them: both human and injected bodies appear as `kind:"plain"` or
/// with no `kind` at all.
fn codex_message_is_human_turn(message: Option<&str>) -> bool {
    match message {
        Some(text) => {
            let trimmed = text.trim_start();
            !CODEX_SYSTEM_INJECTED_PREFIXES
                .iter()
                .any(|prefix| trimmed.starts_with(prefix))
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Cursor, Error, ErrorKind, Seek, SeekFrom, Write};
    use tempfile::NamedTempFile;

    #[test]
    fn codex_human_turn_matches_only_known_system_tags() {
        // Real human prompts that happen to start with markup must still count.
        assert!(codex_message_is_human_turn(Some(
            "how do I center a <div>?"
        )));
        assert!(codex_message_is_human_turn(Some("<div>hi</div>")));
        assert!(codex_message_is_human_turn(Some("  plain question")));
        // Known system-injected context blocks are not human turns.
        assert!(!codex_message_is_human_turn(Some(
            "<environment_context>cwd=/tmp</environment_context>"
        )));
        assert!(!codex_message_is_human_turn(Some(
            "  <system-reminder>be concise</system-reminder>"
        )));
        assert!(!codex_message_is_human_turn(Some(
            "<user_instructions>do X</user_instructions>"
        )));
        // A missing body is never a human turn.
        assert!(!codex_message_is_human_turn(None));
    }

    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    struct FailAfterFirstLine {
        inner: Cursor<Vec<u8>>,
        fail_next_read: bool,
    }

    impl FailAfterFirstLine {
        fn new(contents: &str) -> Self {
            Self {
                inner: Cursor::new(contents.as_bytes().to_vec()),
                fail_next_read: false,
            }
        }
    }

    impl std::io::Read for FailAfterFirstLine {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl BufRead for FailAfterFirstLine {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            self.inner.fill_buf()
        }

        fn consume(&mut self, amt: usize) {
            self.inner.consume(amt);
        }

        fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
            if self.fail_next_read {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "synthetic line decode failure",
                ));
            }
            let bytes_read = self.inner.read_line(buf)?;
            if bytes_read > 0 {
                self.fail_next_read = true;
            }
            Ok(bytes_read)
        }
    }

    #[test]
    fn test_headless_usage_line() {
        let content = r#"{"type":"turn.completed","model":"gpt-4o-mini","usage":{"input_tokens":120,"cached_input_tokens":20,"output_tokens":30}}"#;
        let file = create_test_file(content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o-mini");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
    }

    #[test]
    fn test_headless_usage_nested_data() {
        let content = r#"{"type":"result","data":{"model_name":"gpt-4o","usage":{"input_tokens":50,"cached_input_tokens":5,"output_tokens":12}}}"#;
        let file = create_test_file(content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o");
        assert_eq!(messages[0].tokens.input, 45);
        assert_eq!(messages[0].tokens.output, 12);
        assert_eq!(messages[0].tokens.cache_read, 5);
    }

    #[test]
    fn test_incremental_parse_matches_full_parse_for_appended_lines() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"chat","model_provider":"openai","agent_nickname":"builder","cwd":"/Users/alice/codex-demo"}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            "\n"
        ));

        let initial_size = file.as_file().metadata().unwrap().len();
        let initial = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert_eq!(initial.messages.len(), 1);
        assert_eq!(initial.consumed_offset, initial_size);
        assert_eq!(
            initial.messages[0].workspace_key.as_deref(),
            Some("/Users/alice/codex-demo")
        );
        assert_eq!(
            initial.messages[0].workspace_label.as_deref(),
            Some("codex-demo")
        );

        let appended = concat!(
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":5},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2}}}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":22,"cached_input_tokens":4,"output_tokens":7},"last_token_usage":{"input_tokens":7,"cached_input_tokens":1,"output_tokens":2}}}}"#,
            "\n"
        );

        let mut reopened = file.reopen().unwrap();
        reopened.seek(SeekFrom::End(0)).unwrap();
        reopened.write_all(appended.as_bytes()).unwrap();
        reopened.flush().unwrap();

        let incremental =
            parse_codex_file_incremental(file.path(), initial_size, initial.state.clone());
        let mut combined = initial.messages.clone();
        combined.extend(incremental.messages);
        assert_eq!(
            incremental.consumed_offset,
            file.as_file().metadata().unwrap().len()
        );

        let full = parse_codex_file(file.path());
        assert_eq!(combined, full);
        assert_eq!(
            combined
                .iter()
                .map(|msg| msg.workspace_key.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("/Users/alice/codex-demo"),
                Some("/Users/alice/codex-demo"),
                Some("/Users/alice/codex-demo")
            ]
        );
    }

    #[test]
    fn test_token_count_before_turn_context_uses_later_model() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"interactive","model_provider":"openai","agent_nickname":"builder","cwd":"/Users/alice/codex-demo"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":5,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":0}}}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:04Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":22,"cached_input_tokens":4,"output_tokens":7,"reasoning_output_tokens":2},"last_token_usage":{"input_tokens":7,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":1}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["gpt-5.5", "gpt-5.5", "gpt-5.5"]
        );
        assert_eq!(
            messages
                .iter()
                .map(|message| message.workspace_key.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("/Users/alice/codex-demo"),
                Some("/Users/alice/codex-demo"),
                Some("/Users/alice/codex-demo")
            ]
        );
        assert_eq!(messages[0].tokens.input, 8);
        assert_eq!(messages[0].tokens.output, 3);
        assert_eq!(messages[0].tokens.cache_read, 2);
        assert_eq!(messages[0].tokens.reasoning, 1);
        assert_eq!(messages[1].tokens.input, 4);
        assert_eq!(messages[1].tokens.output, 2);
        assert_eq!(messages[1].tokens.cache_read, 1);
        assert_eq!(messages[1].tokens.reasoning, 0);
        assert_eq!(messages[2].tokens.input, 6);
        assert_eq!(messages[2].tokens.output, 2);
        assert_eq!(messages[2].tokens.cache_read, 1);
        assert_eq!(messages[2].tokens.reasoning, 1);

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(!parsed.unresolved_model_events);
    }

    #[test]
    fn test_token_count_without_model_stays_unknown_but_is_not_cacheable() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"interactive","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#,
            "\n"
        ));

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(parsed.parse_succeeded);
        assert!(parsed.unresolved_model_events);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].model_id, "unknown");
    }

    #[test]
    fn test_model_only_headless_line_flushes_pending_token_counts() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"interactive","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#,
            "\n",
            r#"{"model":"gpt-5.5","type":"metadata"}"#,
            "\n"
        ));

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(parsed.parse_succeeded);
        assert!(!parsed.unresolved_model_events);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].model_id, "gpt-5.5");
    }

    #[test]
    fn test_parse_reader_marks_failure_on_line_read_error() {
        let reader = FailAfterFirstLine::new(concat!(
            r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            "\n"
        ));

        let parsed = parse_codex_reader(reader, "session", 0, 0, CodexParseState::default());

        assert!(!parsed.parse_succeeded);
        assert!(parsed.messages.is_empty());
    }

    #[test]
    fn test_parse_file_returns_empty_on_invalid_utf8_line_error() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            concat!(
                r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                "\n"
            )
            .as_bytes(),
        )
        .unwrap();
        file.write_all(&[0xff, b'\n']).unwrap();
        file.flush().unwrap();

        let messages = parse_codex_file(file.path());
        assert!(messages.is_empty());

        let incremental = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(!incremental.parse_succeeded);
    }

    #[test]
    fn test_parse_file_preserves_valid_messages_after_late_invalid_utf8_line_error() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            concat!(
                r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                "\n"
            )
            .as_bytes(),
        )
        .unwrap();
        file.write_all(&[0xff, b'\n']).unwrap();
        file.flush().unwrap();

        let messages = parse_codex_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.4");
        assert_eq!(messages[0].tokens.input, 8);
        assert_eq!(messages[0].tokens.output, 3);
        assert_eq!(messages[0].tokens.cache_read, 2);

        let incremental = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(!incremental.parse_succeeded);
        assert_eq!(incremental.messages.len(), 1);
    }

    #[test]
    fn test_session_meta_exec_marks_headless() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"originator":"codex_exec","source":"exec"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#;
        let content = format!("{}\n{}", line1, line2);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent.as_deref(), Some("headless"));
    }

    #[test]
    fn test_token_count_uses_total_deltas_when_totals_repeat() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
    }

    #[test]
    fn test_token_count_falls_back_to_last_usage_when_totals_reset() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 3);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);
    }

    #[test]
    fn test_token_count_advances_baseline_after_missing_total_fallback() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":110,"cached_input_tokens":22,"output_tokens":33,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}\n{}", line1, line2, line3, line4);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 3);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);
    }

    #[test]
    fn test_token_count_skips_regressed_totals_without_last_usage() {
        // When totals regress and last_usage is absent, the row should be
        // skipped entirely to avoid double-counting the full cumulative total.
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        // Totals regress (lower values) and no last_token_usage — should skip
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"cached_input_tokens":10,"output_tokens":15,"reasoning_output_tokens":2}}}}"#;
        // Normal continuation after reset
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":80,"cached_input_tokens":15,"output_tokens":25,"reasoning_output_tokens":4}}}}"#;
        let content = format!("{}\n{}\n{}\n{}", line1, line2, line3, line4);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        // Should produce 2 messages: first from line2 (full total),
        // then delta from line4 relative to line3 (baseline reset).
        assert_eq!(messages.len(), 2);
        // First message: full total
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        // Second message: delta from 50→80
        assert_eq!(messages[1].tokens.input, 25);
        assert_eq!(messages[1].tokens.output, 10);
        assert_eq!(messages[1].tokens.cache_read, 5);
        assert_eq!(messages[1].tokens.reasoning, 2);
    }

    #[test]
    fn test_into_tokens_clamps_cached_to_input() {
        // When cached > input (malformed data), cached should be clamped to input
        // so that input + cache_read never exceeds the raw input value.
        let totals = CodexTotals {
            input: 50,
            output: 30,
            cached: 100, // More than input — malformed
            reasoning: 5,
        };
        let tokens = totals.into_tokens();
        assert_eq!(tokens.cache_read, 50); // Clamped to input
        assert_eq!(tokens.input, 0); // input - clamped_cached = 0
        assert_eq!(tokens.output, 30);
        assert_eq!(tokens.reasoning, 5);
    }

    #[test]
    fn test_token_count_ignores_negative_fallback_usage_in_baseline() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":-10,"cached_input_tokens":-2,"output_tokens":-3,"reasoning_output_tokens":-1}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":110,"cached_input_tokens":22,"output_tokens":33,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}\n{}", line1, line2, line3, line4);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 3);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);
    }

    #[test]
    fn test_token_count_avoids_double_counting_stale_cumulative_regressions() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":110,"cached_input_tokens":22,"output_tokens":33,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":109,"cached_input_tokens":21,"output_tokens":32,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":9,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":0}}}}"#;
        let line5 = r#"{"timestamp":"2026-01-01T00:00:04Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":119,"cached_input_tokens":23,"output_tokens":35,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":0}}}}"#;
        let content = format!("{}\n{}\n{}\n{}\n{}", line1, line2, line3, line4, line5);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);

        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 3);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);

        // Stale snapshot (line4) is now skipped entirely; messages[2]
        // comes from line5's last_token_usage instead.
        assert_eq!(messages[2].tokens.input, 8);
        assert_eq!(messages[2].tokens.output, 3);
        assert_eq!(messages[2].tokens.cache_read, 2);
        assert_eq!(messages[2].tokens.reasoning, 0);
    }

    #[test]
    fn test_token_count_handles_multiple_stale_regressions_before_recovery() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":110,"cached_input_tokens":22,"output_tokens":33,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":109,"cached_input_tokens":21,"output_tokens":32,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":9,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":0}}}}"#;
        let line5 = r#"{"timestamp":"2026-01-01T00:00:04Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":118,"cached_input_tokens":22,"output_tokens":34,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":9,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":0}}}}"#;
        let line6 = r#"{"timestamp":"2026-01-01T00:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":128,"cached_input_tokens":24,"output_tokens":37,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":0}}}}"#;
        let content = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            line1, line2, line3, line4, line5, line6
        );
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        // Stale line4 is skipped; messages come from lines 2, 3, 5, 6.
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[2].tokens.input, 8);
        assert_eq!(messages[2].tokens.output, 2);
        assert_eq!(messages[2].tokens.cache_read, 1);
        assert_eq!(messages[2].tokens.reasoning, 0);
        assert_eq!(messages[3].tokens.input, 8);
        assert_eq!(messages[3].tokens.output, 3);
        assert_eq!(messages[3].tokens.cache_read, 2);
        assert_eq!(messages[3].tokens.reasoning, 0);
    }

    #[test]
    fn test_token_count_treats_large_regressions_as_real_resets() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10000,"cached_input_tokens":1000,"output_tokens":400,"reasoning_output_tokens":50},"last_token_usage":{"input_tokens":10000,"cached_input_tokens":1000,"output_tokens":400,"reasoning_output_tokens":50}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":7600,"cached_input_tokens":800,"output_tokens":280,"reasoning_output_tokens":35},"last_token_usage":{"input_tokens":25,"cached_input_tokens":5,"output_tokens":4,"reasoning_output_tokens":1}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":7625,"cached_input_tokens":805,"output_tokens":284,"reasoning_output_tokens":36},"last_token_usage":{"input_tokens":25,"cached_input_tokens":5,"output_tokens":4,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}\n{}", line1, line2, line3, line4);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].tokens.input, 9000);
        assert_eq!(messages[0].tokens.output, 400);
        assert_eq!(messages[0].tokens.cache_read, 1000);
        assert_eq!(messages[0].tokens.reasoning, 50);

        assert_eq!(messages[1].tokens.input, 20);
        assert_eq!(messages[1].tokens.output, 4);
        assert_eq!(messages[1].tokens.cache_read, 5);
        assert_eq!(messages[1].tokens.reasoning, 1);

        assert_eq!(messages[2].tokens.input, 20);
        assert_eq!(messages[2].tokens.output, 4);
        assert_eq!(messages[2].tokens.cache_read, 5);
        assert_eq!(messages[2].tokens.reasoning, 1);
    }

    #[test]
    fn test_first_event_uses_last_not_total_for_resumed_sessions() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5000,"cached_input_tokens":500,"output_tokens":800,"reasoning_output_tokens":100},"last_token_usage":{"input_tokens":12,"cached_input_tokens":2,"output_tokens":5,"reasoning_output_tokens":1}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5012,"cached_input_tokens":502,"output_tokens":805,"reasoning_output_tokens":101},"last_token_usage":{"input_tokens":12,"cached_input_tokens":2,"output_tokens":5,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 5);
        assert_eq!(messages[0].tokens.cache_read, 2);
        assert_eq!(messages[0].tokens.reasoning, 1);
        assert_eq!(messages[1].tokens.input, 10);
        assert_eq!(messages[1].tokens.output, 5);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);
    }

    #[test]
    fn test_zero_token_snapshot_does_not_inflate_later_deltas() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":500,"cached_input_tokens":50,"output_tokens":80,"reasoning_output_tokens":10},"last_token_usage":{"input_tokens":500,"cached_input_tokens":50,"output_tokens":80,"reasoning_output_tokens":10}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0},"last_token_usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":510,"cached_input_tokens":52,"output_tokens":83,"reasoning_output_tokens":11},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}\n{}", line1, line2, line3, line4);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 450);
        assert_eq!(messages[0].tokens.output, 80);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.reasoning, 10);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 3);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);
    }

    #[test]
    fn test_model_info_slug_from_turn_context() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model_info":{"slug":"o3-pro"}}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}", line1, line2);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "o3-pro");
        assert_eq!(messages[0].duration_ms, Some(1000));
    }

    #[test]
    fn test_session_meta_provider_and_agent() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","model_provider":"azure","agent_nickname":"my-agent"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "azure");
        assert_eq!(messages[0].agent.as_deref(), Some("my-agent"));
    }

    #[test]
    fn test_session_meta_object_source_keeps_provider_agent_and_workspace() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"fork-session","forked_from_id":"parent-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/Users/alice/codex-fork"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].agent.as_deref(), Some("worker"));
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/alice/codex-fork")
        );
        assert!(messages[0].dedup_key.is_some());
    }

    #[test]
    fn test_forked_child_ignores_inherited_records_before_turn_context() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","forked_from_id":"parent-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.992Z","type":"session_meta","payload":{"id":"parent-session","source":"interactive","model_provider":"azure","agent_nickname":"parent","cwd":"/repo-parent"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.993Z","type":"event_msg","payload":{"type":"user_message","message":"parent prompt copied into child log"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":116000,"cached_input_tokens":114000,"output_tokens":1000,"total_tokens":117000},"last_token_usage":{"input_tokens":73000,"cached_input_tokens":72000,"output_tokens":500,"total_tokens":73500}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":116000,"cached_input_tokens":114000,"output_tokens":1000,"total_tokens":117000},"last_token_usage":{"input_tokens":73000,"cached_input_tokens":72000,"output_tokens":500,"total_tokens":73500}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":117500,"cached_input_tokens":115000,"output_tokens":1200,"reasoning_output_tokens":50,"total_tokens":118700},"last_token_usage":{"input_tokens":1500,"cached_input_tokens":1000,"output_tokens":200,"reasoning_output_tokens":50,"total_tokens":1700}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.5");
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].agent.as_deref(), Some("worker"));
        assert_eq!(messages[0].workspace_key.as_deref(), Some("/repo-child"));
        assert_eq!(messages[0].tokens.input, 500);
        assert_eq!(messages[0].tokens.cache_read, 1000);
        assert_eq!(messages[0].tokens.output, 200);
        assert_eq!(messages[0].tokens.reasoning, 50);
    }

    #[test]
    fn test_forked_child_ignores_replayed_parent_rows_after_turn_context() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","forked_from_id":"parent-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55},"last_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.949Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":250,"output_tokens":25,"total_tokens":275}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":310,"output_tokens":32,"total_tokens":342},"last_token_usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.5");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 2);
    }

    #[test]
    fn test_forked_child_submit_cap_regression_skips_large_inherited_cache_replays() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","forked_from_id":"parent-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1,"agent_role":"architect"}}},"model_provider":"openai","agent_nickname":"architect","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1200000000,"cached_input_tokens":1180000000,"output_tokens":1000000,"reasoning_output_tokens":100000,"total_tokens":1201100000},"last_token_usage":{"input_tokens":750000000,"cached_input_tokens":740000000,"output_tokens":500000,"reasoning_output_tokens":50000,"total_tokens":750550000}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1180000000,"cached_input_tokens":1160000000,"output_tokens":900000,"reasoning_output_tokens":90000,"total_tokens":1180990000},"last_token_usage":{"input_tokens":20000000,"cached_input_tokens":20000000,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":20000000}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.949Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1200000000,"cached_input_tokens":1180000000,"output_tokens":1000000,"reasoning_output_tokens":100000,"total_tokens":1201100000},"last_token_usage":{"input_tokens":20000000,"cached_input_tokens":20000000,"output_tokens":100000,"reasoning_output_tokens":10000,"total_tokens":20110000}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1200001500,"cached_input_tokens":1180001000,"output_tokens":1000200,"reasoning_output_tokens":100050,"total_tokens":1201101750},"last_token_usage":{"input_tokens":1500,"cached_input_tokens":1000,"output_tokens":200,"reasoning_output_tokens":50,"total_tokens":1750}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.5");
        assert_eq!(messages[0].tokens.input, 500);
        assert_eq!(messages[0].tokens.cache_read, 1000);
        assert_eq!(messages[0].tokens.output, 200);
        assert_eq!(messages[0].tokens.reasoning, 50);
    }

    #[test]
    fn test_forked_child_detects_thread_spawn_source_without_top_level_fork_id() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55},"last_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":310,"output_tokens":32,"total_tokens":342},"last_token_usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.5");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 2);
    }

    #[test]
    fn test_user_forked_child_counts_own_turn_after_parent_replay() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-01-02T03:10:00.000Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.001Z","type":"session_meta","payload":{"id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.100Z","type":"turn_context","payload":{"turn_id":"11111111-3333-7333-8333-333333333333","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100}}}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:30.100Z","type":"turn_context","payload":{"turn_id":"22222222-4444-7444-8444-444444444444","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:30.200Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:31.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1250,"cached_input_tokens":450,"output_tokens":120,"total_tokens":1370},"last_token_usage":{"input_tokens":250,"cached_input_tokens":50,"output_tokens":20,"total_tokens":270}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 200);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.output, 20);
    }

    #[test]
    fn test_user_forked_child_same_millisecond_own_turn_counts_without_task_started() {
        // Human (`thread_source:"user"`) fork where the child session_meta and
        // the child's own first turn_context are minted in the SAME millisecond,
        // so both UUID v7 ids share the 48-bit prefix (`22222222-2222`). A user
        // fork never emits a `task_started`, so a same-millisecond gate that
        // requires `task_started` would keep skipping forever and drop the
        // child's own turn (0 messages). The replayed parent turn carries the
        // *parent's* millisecond prefix (`11111111`), so it sorts strictly
        // earlier and is still skipped; only the child's own turn — the one that
        // shares the child's fork millisecond — must end the skip and be counted
        // (200/20 delta from the inherited 1000/100 baseline).
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-01-02T03:10:00.000Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.001Z","type":"session_meta","payload":{"id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.100Z","type":"turn_context","payload":{"turn_id":"11111111-3333-7333-8333-333333333333","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100}}}}"#,
            "\n",
            // child's own turn: turn_id shares the child session's millisecond
            // prefix (`22222222-2222`) — same-millisecond tie with the fork.
            r#"{"timestamp":"2026-01-02T03:10:30.100Z","type":"turn_context","payload":{"turn_id":"22222222-2222-7444-8444-444444444444","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:30.200Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:31.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1250,"cached_input_tokens":450,"output_tokens":120,"total_tokens":1370},"last_token_usage":{"input_tokens":250,"cached_input_tokens":50,"output_tokens":20,"total_tokens":270}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 200);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.output, 20);
    }

    #[test]
    fn test_user_fork_replayed_parent_shares_child_ms_ends_skip_early() {
        // Documents (locks) the accepted residual called out at the Equal branch:
        // a human (`thread_source:"user"`) fork resolves a same-millisecond tie on
        // the millisecond prefix alone, because user forks never emit a
        // `task_started` to harden the gate. Here the *replayed parent* turn is
        // (pathologically) minted within the exact same 1ms as the child's fork
        // session_meta, so it shares the *child's* prefix (`22222222-2222`) rather
        // than the parent's. Because the gate cannot distinguish it from the
        // child's own turn, it ends the skip one turn early and counts that
        // replayed parent row (500/50 delta off the 1000/100 baseline) as the
        // child's first turn. This is a sub-millisecond, human-paced coincidence;
        // the test pins the CURRENT behavior so any future change is intentional.
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-01-02T03:10:00.000Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.001Z","type":"session_meta","payload":{"id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100}}}}"#,
            "\n",
            // replayed parent turn whose turn_id coincidentally shares the child's
            // fork millisecond prefix (`22222222-2222`) — equal-prefix tie. With a
            // user fork (no task_started) this ends the skip here, one turn early.
            r#"{"timestamp":"2026-01-02T03:10:00.200Z","type":"turn_context","payload":{"turn_id":"22222222-2222-7333-8333-333333333333","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.300Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":450,"output_tokens":150,"total_tokens":1650},"last_token_usage":{"input_tokens":500,"cached_input_tokens":50,"output_tokens":50,"total_tokens":550}}}}"#,
            "\n",
            // the child's actual own turn (also shares the child's prefix).
            r#"{"timestamp":"2026-01-02T03:10:30.100Z","type":"turn_context","payload":{"turn_id":"22222222-2222-7444-8444-444444444444","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:31.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1750,"cached_input_tokens":500,"output_tokens":170,"total_tokens":1920},"last_token_usage":{"input_tokens":250,"cached_input_tokens":50,"output_tokens":20,"total_tokens":270}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        // CURRENT behavior: the skip ends one turn early at the equal-prefix
        // replayed parent turn, so BOTH that row and the child's own turn are
        // counted (two messages) rather than only the child's own turn. The
        // first message is the replayed parent delta (total 1500-1000=500, of
        // which 50 is cache_read, leaving 450 non-cached input + 50 output); the
        // second is the child's own delta (250-50=200 input + 20 output). A
        // future change that hardened this tie would instead yield a single
        // message with the child's 200/20.
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 450);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[1].tokens.input, 200);
        assert_eq!(messages[1].tokens.cache_read, 50);
        assert_eq!(messages[1].tokens.output, 20);
    }

    #[test]
    fn test_forked_child_skips_nested_parent_replay_until_own_turn() {
        let parent = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"019e5b00-0000-7000-8000-000000000001","source":"vscode","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.000Z","type":"turn_context","payload":{"turn_id":"019e5b00-0001-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n"
        ));
        let child = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5c03-1e99-7000-8000-000000000001","forked_from_id":"019e5b00-0000-7000-8000-000000000001","source":{"subagent":{"thread_spawn":{"parent_thread_id":"019e5b00-0000-7000-8000-000000000001","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5b00-0000-7000-8000-000000000001","source":"vscode","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.100Z","type":"turn_context","payload":{"turn_id":"019e5b00-0001-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.000Z","type":"event_msg","payload":{"type":"task_started","turn_id":"019e5c03-6425-7000-8000-000000000001"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.100Z","type":"turn_context","payload":{"turn_id":"019e5c03-6425-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":320,"output_tokens":32,"total_tokens":352},"last_token_usage":{"input_tokens":20,"output_tokens":2,"total_tokens":22}}}}"#,
            "\n"
        ));

        let parent_messages = parse_codex_file(parent.path());
        let child_messages = parse_codex_file(child.path());

        assert_eq!(parent_messages.len(), 1);
        assert_eq!(child_messages.len(), 1);
        assert_ne!(parent_messages[0].dedup_key, child_messages[0].dedup_key);
        assert_eq!(child_messages[0].tokens.input, 20);
        assert_eq!(child_messages[0].tokens.output, 2);
    }

    #[test]
    fn test_forked_child_same_millisecond_turn_starts_own_session() {
        // Regression: the child's own first turn starts in the SAME millisecond
        // as its fork session_meta, so both UUID v7 ids share the 48-bit ms
        // prefix (`019e5c03-1e99`) and differ only in the random tail
        // (`…0001` vs `…00ff`). Comparing the full id makes the gate fall through
        // to the coin-flip tail (here `0001 < 00ff`), so the replay-skip never
        // ends and the child's own turn is dropped. Comparing only the ms prefix
        // keeps the child's own turn.
        let child = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5c03-1e99-7000-8000-0000000000ff","forked_from_id":"019e5b00-0000-7000-8000-000000000001","source":{"subagent":{"thread_spawn":{"parent_thread_id":"019e5b00-0000-7000-8000-000000000001","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5b00-0000-7000-8000-000000000001","source":"vscode","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.100Z","type":"turn_context","payload":{"turn_id":"019e5b00-0001-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.000Z","type":"event_msg","payload":{"type":"task_started","turn_id":"019e5c03-1e99-7000-8000-000000000001"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.100Z","type":"turn_context","payload":{"turn_id":"019e5c03-1e99-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":320,"output_tokens":32,"total_tokens":352},"last_token_usage":{"input_tokens":20,"output_tokens":2,"total_tokens":22}}}}"#,
            "\n"
        ));

        let child_messages = parse_codex_file(child.path());

        assert_eq!(child_messages.len(), 1);
        assert_eq!(child_messages[0].tokens.input, 20);
        assert_eq!(child_messages[0].tokens.output, 2);
    }

    #[test]
    fn test_forked_child_same_millisecond_replayed_parent_turn_keeps_skipping() {
        // A replayed parent `turn_context` can coincidentally share the child's
        // fork millisecond (here both `019e5c03-1e99`) while NOT being preceded
        // by a `task_started`. A millisecond-prefix-only gate would treat that
        // equal-prefix turn as child-local, end the skip early, and count the
        // inherited replayed row (500/50) as the child's own usage. The child's
        // own turn is the later one announced by `task_started`; only it should
        // end the skip, so only its 20/2 delta is counted.
        let child = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5c03-1e99-7000-8000-0000000000ff","forked_from_id":"019e5b00-0000-7000-8000-000000000001","source":{"subagent":{"thread_spawn":{"parent_thread_id":"019e5b00-0000-7000-8000-000000000001","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5b00-0000-7000-8000-000000000001","source":"vscode","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            // replayed parent turn that shares the child's fork millisecond, with
            // NO task_started — must NOT end the skip.
            r#"{"timestamp":"2026-05-05T21:52:10.100Z","type":"turn_context","payload":{"turn_id":"019e5c03-1e99-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":500,"output_tokens":50,"total_tokens":550},"last_token_usage":{"input_tokens":500,"output_tokens":50,"total_tokens":550}}}}"#,
            "\n",
            // the child's real own turn, announced by task_started.
            r#"{"timestamp":"2026-05-05T21:52:20.000Z","type":"event_msg","payload":{"type":"task_started","turn_id":"019e5c03-1e99-7000-8000-000000000002"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.100Z","type":"turn_context","payload":{"turn_id":"019e5c03-1e99-7000-8000-000000000002","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":520,"output_tokens":52,"total_tokens":572},"last_token_usage":{"input_tokens":20,"output_tokens":2,"total_tokens":22}}}}"#,
            "\n"
        ));

        let child_messages = parse_codex_file(child.path());

        assert_eq!(child_messages.len(), 1);
        assert_eq!(child_messages[0].tokens.input, 20);
        assert_eq!(child_messages[0].tokens.output, 2);
    }

    #[test]
    fn test_forked_child_incremental_state_skips_inherited_prefix() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","forked_from_id":"parent-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.992Z","type":"session_meta","payload":{"id":"parent-session","source":"interactive","model_provider":"azure","agent_nickname":"parent","cwd":"/repo-parent"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":116000,"cached_input_tokens":114000,"output_tokens":1000,"total_tokens":117000},"last_token_usage":{"input_tokens":73000,"cached_input_tokens":72000,"output_tokens":500,"total_tokens":73500}}}}"#,
            "\n"
        ));
        let prefix_size = file.as_file().metadata().unwrap().len();
        let prefix = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(prefix.parse_succeeded);
        assert!(!prefix.unresolved_model_events);
        assert!(prefix.messages.is_empty());

        let appended = concat!(
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":116000,"cached_input_tokens":114000,"output_tokens":1000,"total_tokens":117000},"last_token_usage":{"input_tokens":73000,"cached_input_tokens":72000,"output_tokens":500,"total_tokens":73500}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":117500,"cached_input_tokens":115000,"output_tokens":1200,"reasoning_output_tokens":50,"total_tokens":118700},"last_token_usage":{"input_tokens":1500,"cached_input_tokens":1000,"output_tokens":200,"reasoning_output_tokens":50,"total_tokens":1700}}}}"#,
            "\n"
        );
        let mut reopened = file.reopen().unwrap();
        reopened.seek(SeekFrom::End(0)).unwrap();
        reopened.write_all(appended.as_bytes()).unwrap();
        reopened.flush().unwrap();

        let incremental =
            parse_codex_file_incremental(file.path(), prefix_size, prefix.state.clone());
        let full = parse_codex_file(file.path());

        assert_eq!(incremental.messages, full);
        assert_eq!(incremental.messages.len(), 1);
        assert_eq!(incremental.messages[0].tokens.input, 500);
        assert_eq!(incremental.messages[0].tokens.cache_read, 1000);
        assert_eq!(incremental.messages[0].tokens.output, 200);
        assert_eq!(incremental.messages[0].tokens.reasoning, 50);
    }

    #[test]
    fn test_session_meta_cwd_sets_workspace_metadata() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","cwd":"/Users/alice/demo-repo"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/alice/demo-repo")
        );
        assert_eq!(messages[0].workspace_label.as_deref(), Some("demo-repo"));
    }

    #[test]
    fn test_inaccessible_cwd_still_parses_token_usage() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","cwd":"/path/that/does/not/exist/demo-repo"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 8);
        assert_eq!(messages[0].tokens.output, 3);
        assert_eq!(messages[0].tokens.cache_read, 2);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/path/that/does/not/exist/demo-repo")
        );
        assert_eq!(messages[0].workspace_label.as_deref(), Some("demo-repo"));
    }

    #[test]
    fn test_session_meta_empty_cwd_clears_workspace_metadata() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","cwd":"   "}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key, None);
        assert_eq!(messages[0].workspace_label, None);
        assert_eq!(messages[0].tokens.input, 8);
    }

    #[test]
    fn test_session_meta_malformed_cwd_clears_workspace_metadata() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","cwd":"file:///Users/alice/demo-repo"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key, None);
        assert_eq!(messages[0].workspace_label, None);
        assert_eq!(messages[0].tokens.input, 8);
    }

    #[test]
    fn test_session_meta_path_like_noncanonical_cwd_normalizes_consistently() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","cwd":"//server//share///demo-repo/"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("//server/share/demo-repo")
        );
        assert_eq!(messages[0].workspace_label.as_deref(), Some("demo-repo"));
        assert_eq!(messages[0].tokens.input, 8);
    }

    #[test]
    fn test_cached_tokens_takes_max_of_both_fields() {
        let usage = CodexTokenUsage {
            input_tokens: Some(100),
            output_tokens: Some(30),
            cached_input_tokens: Some(10),
            cache_read_input_tokens: Some(20),
            reasoning_output_tokens: Some(5),
            total_tokens: None,
        };
        let totals = CodexTotals::from_usage(&usage);
        assert_eq!(totals.cached, 20);
    }

    #[test]
    fn test_compaction_total_drop_uses_last_as_increment() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150000,"cached_input_tokens":10000,"output_tokens":20000,"reasoning_output_tokens":5000},"last_token_usage":{"input_tokens":150000,"cached_input_tokens":10000,"output_tokens":20000,"reasoning_output_tokens":5000}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":200000,"cached_input_tokens":15000,"output_tokens":25000,"reasoning_output_tokens":6000},"last_token_usage":{"input_tokens":50,"cached_input_tokens":5,"output_tokens":10,"reasoning_output_tokens":2}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].tokens.input, 45);
        assert_eq!(messages[1].tokens.output, 10);
        assert_eq!(messages[1].tokens.cache_read, 5);
        assert_eq!(messages[1].tokens.reasoning, 2);
    }

    #[test]
    fn test_headless_fallback_uses_session_provider_and_agent() {
        // session_meta sets provider to "azure" and agent to "my-bot",
        // then a line falls through to headless parsing (no structured entry_type)
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"model_provider":"azure","agent_nickname":"my-bot"}}"#;
        let line2 = r#"{"type":"turn.completed","model":"gpt-4o","usage":{"input_tokens":100,"output_tokens":50}}"#;
        let content = format!("{}\n{}", line1, line2);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "azure");
        assert_eq!(messages[0].agent.as_deref(), Some("my-bot"));
    }

    #[test]
    fn test_headless_fallback_defaults_to_openai_without_session_meta() {
        // No session_meta — headless fallback should default to "openai"
        let content = r#"{"type":"turn.completed","model":"gpt-4o-mini","usage":{"input_tokens":120,"cached_input_tokens":20,"output_tokens":30}}"#;
        let file = create_test_file(content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openai");
        assert!(messages[0].agent.is_none());
    }

    #[test]
    fn test_extract_model_skips_empty_slug_falls_through_to_model() {
        // model_info.slug is empty string, but payload.model has a valid value.
        // extract_model should skip the empty slug and return payload.model.
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model_info":{"slug":""},"model":"gpt-4o"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":5}}}}"#;
        let content = format!("{}\n{}", line1, line2);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o");
    }

    #[test]
    fn test_pending_model_messages_do_not_bind_across_unrelated_turns() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"interactive","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:02Z","type":"assistant_message"}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:04Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":5},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2}}}}"#,
            "\n"
        ));

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(parsed.parse_succeeded);
        assert!(parsed.unresolved_model_events);
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].model_id, "unknown");
        assert_eq!(parsed.messages[1].model_id, "gpt-5.5");
    }

    #[test]
    fn test_token_count_ignores_empty_info_model_until_later_valid_model() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"interactive","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"model":"","model_name":"","total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:04Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
            "\n"
        ));

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(parsed.parse_succeeded);
        assert!(!parsed.unresolved_model_events);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].model_id, "gpt-5.5");
    }

    #[test]
    fn test_user_message_marks_next_token_count_as_turn_start() {
        let content = [
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"continue please"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            r#"{"timestamp":"2026-01-01T00:00:04Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"cached_input_tokens":4,"output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
        ]
        .join("\n");
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert!(
            messages[0].is_turn_start,
            "first reply after a human user_message is a turn start"
        );
        assert!(
            !messages[1].is_turn_start,
            "a later reply with no new user_message is not a turn start"
        );
    }

    #[test]
    fn test_xml_user_message_does_not_mark_turn_start() {
        let content = [
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"\n<environment_context>\n  <cwd>/tmp</cwd>\n</environment_context>"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
        ]
        .join("\n");
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert!(
            !messages[0].is_turn_start,
            "a system-injected <...> message is not a human turn"
        );
    }

    #[test]
    fn test_exec_user_message_still_marks_turn_start() {
        // A `codex exec` one-shot is headless but still carries a real human
        // prompt, so it counts as exactly one turn (verified against a real
        // `codex exec` session: 1 user_message -> turn_count 1).
        let content = [
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"exec"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"hello"}}"#,
            // A real `codex exec` interleaves an agent_message between the user
            // prompt and the token_count; the deferred turn flag must survive it.
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"agent_message","message":"hi"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
        ]
        .join("\n");
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].is_turn_start,
            "an exec one-shot with a human prompt counts as one turn"
        );
        assert_eq!(messages[0].agent.as_deref(), Some("headless"));
    }

    #[test]
    fn test_incremental_parse_preserves_pending_turn_start() {
        let content = [
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"hello"}}"#,
            "",
        ]
        .join("\n");
        let file = create_test_file(&content);
        let initial_size = file.as_file().metadata().unwrap().len();

        let initial = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(
            initial.messages.is_empty(),
            "no token_count yet, so no message"
        );
        assert!(
            initial.state.pending_turn_start,
            "a pending turn survives a chunk that ends before the token_count"
        );

        let appended = format!(
            "{}\n",
            r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#
        );
        let mut reopened = file.reopen().unwrap();
        reopened.seek(SeekFrom::End(0)).unwrap();
        reopened.write_all(appended.as_bytes()).unwrap();
        reopened.flush().unwrap();

        let incremental =
            parse_codex_file_incremental(file.path(), initial_size, initial.state.clone());

        assert_eq!(incremental.messages.len(), 1);
        assert!(
            incremental.messages[0].is_turn_start,
            "the deferred turn applies to the message parsed in the next chunk"
        );
        assert!(
            !incremental.state.pending_turn_start,
            "the pending flag is consumed once applied"
        );
    }
}
