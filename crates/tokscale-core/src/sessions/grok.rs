//! Grok Build session parser.
//!
//! Grok Build writes JSON-RPC session updates under
//! `~/.grok/sessions/<urlencoded-workspace>/<session-id>/updates.jsonl`.
//! Session rollups also land in sibling `signals.json` (including
//! `totalTokensBeforeCompaction` and `contextTokensUsed`). Legacy update logs
//! expose cumulative `totalTokens` counters without a stable input/output
//! split, so this parser records per-turn positive total-token deltas as input
//! tokens and reconciles any remaining `signals.json` total so compacted
//! sessions are not under-counted. Recent Grok Build releases additionally
//! write per-inference token breakdowns to `~/.grok/logs/unified.jsonl`.

use super::utils::{
    extract_i64, extract_string, file_modified_timestamp_ms, lossy_lines, parse_timestamp_value,
    read_file_or_none,
};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const CLIENT_ID: &str = "grok";
const PROVIDER_ID: &str = "xai";
const UNKNOWN_MODEL: &str = "grok-unknown";
const UNIFIED_LOG_DEDUP_PREFIX: &str = "grok-unified:";

type UnifiedGeneration = u64;
type UnifiedProcessKey = (i64, UnifiedGeneration);
type UnifiedProcessSessionKey = (i64, UnifiedGeneration, String);
type UnifiedSessionTree = Vec<(PathBuf, Vec<PathBuf>)>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UnifiedChildScope {
    pid: i64,
    generation: UnifiedGeneration,
    session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UnifiedModelEvidence {
    Unique(String),
    Conflict,
}

#[derive(Debug, Default)]
struct UnifiedChildEvidence {
    known_scopes: HashSet<UnifiedChildScope>,
    child_models: HashMap<UnifiedChildScope, UnifiedModelEvidence>,
    terminal_scopes: HashSet<UnifiedChildScope>,
    terminal_models: HashMap<UnifiedChildScope, UnifiedModelEvidence>,
    child_session_ids: HashSet<String>,
}

fn authoritative_model(value: Option<&Value>) -> Option<String> {
    extract_string(value).and_then(|model| {
        let model = model.trim();
        (!model.is_empty() && model != UNKNOWN_MODEL).then(|| model.to_string())
    })
}

fn record_model_evidence(
    evidence: &mut HashMap<UnifiedChildScope, UnifiedModelEvidence>,
    scope: &UnifiedChildScope,
    model: String,
) {
    match evidence.entry(scope.clone()) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(UnifiedModelEvidence::Unique(model));
        }
        std::collections::hash_map::Entry::Occupied(mut entry) => match entry.get() {
            UnifiedModelEvidence::Unique(existing) if existing == &model => {}
            UnifiedModelEvidence::Unique(_) | UnifiedModelEvidence::Conflict => {
                entry.insert(UnifiedModelEvidence::Conflict);
            }
        },
    }
}

fn current_unified_generation(
    generations: &mut HashMap<i64, UnifiedGeneration>,
    pid: i64,
) -> UnifiedGeneration {
    *generations.entry(pid).or_insert(0)
}

fn advance_unified_generation(generations: &mut HashMap<i64, UnifiedGeneration>, pid: i64) {
    let generation = generations.entry(pid).or_insert(0);
    *generation = generation.saturating_add(1);
}

fn unified_subagent_id(value: &Value) -> Option<String> {
    extract_string(value.get("ctx")?.get("subagent_id")).filter(|id| !id.trim().is_empty())
}

fn unified_child_scope(
    value: &Value,
    generations: &mut HashMap<i64, UnifiedGeneration>,
) -> Option<UnifiedChildScope> {
    let pid = required_non_negative_i64(value.get("pid"))?;
    Some(UnifiedChildScope {
        pid,
        generation: current_unified_generation(generations, pid),
        session_id: unified_subagent_id(value)?,
    })
}

fn unified_spawn_model(value: &Value) -> Option<String> {
    let context = value.get("ctx")?;
    authoritative_model(context.get("effective_model"))
        .or_else(|| authoritative_model(context.get("effective_model_raw")))
}

fn unified_terminal_model(value: &Value) -> Option<String> {
    authoritative_model(value.get("ctx")?.get("effective_model"))
}

fn unique_child_model<'a>(
    evidence: &'a UnifiedChildEvidence,
    scope: &UnifiedChildScope,
) -> Option<&'a str> {
    let UnifiedModelEvidence::Unique(model) = evidence.child_models.get(scope)? else {
        return None;
    };
    Some(model)
}

fn unique_terminal_model<'a>(
    evidence: &'a UnifiedChildEvidence,
    scope: &UnifiedChildScope,
) -> Option<&'a str> {
    if !evidence.terminal_scopes.contains(scope) {
        return None;
    }
    let UnifiedModelEvidence::Unique(terminal_model) = evidence.terminal_models.get(scope)? else {
        return None;
    };
    let child_model = unique_child_model(evidence, scope)?;
    (terminal_model == child_model).then_some(child_model)
}

fn has_conflicting_child_evidence(
    evidence: &UnifiedChildEvidence,
    scope: &UnifiedChildScope,
) -> bool {
    matches!(
        evidence.child_models.get(scope),
        Some(UnifiedModelEvidence::Conflict)
    ) || matches!(
        evidence.terminal_models.get(scope),
        Some(UnifiedModelEvidence::Conflict)
    )
}

#[derive(Debug, Clone)]
struct GrokMetadata {
    session_id: String,
    model_id: Option<String>,
    timestamp: i64,
    workspace_key: Option<String>,
    workspace_label: Option<String>,
}

#[derive(Debug, Clone)]
struct ActiveTurn {
    baseline_total: i64,
    max_total: i64,
    timestamp: i64,
    model_id: String,
    turn_index: usize,
}

#[derive(Debug, Clone)]
struct GrokUsage {
    tokens: TokenBreakdown,
}

impl GrokUsage {
    fn from_update(value: &Value) -> Option<Self> {
        let usage = get_path(value, &["params", "update", "usage"])?;
        let raw_input = usage_value(usage, &["inputTokens", "input_tokens", "promptTokens"]);
        let raw_output = usage_value(
            usage,
            &["outputTokens", "output_tokens", "completionTokens"],
        );
        let cache_read = usage_value(
            usage,
            &[
                "cachedReadTokens",
                "cacheReadTokens",
                "cache_read_input_tokens",
            ],
        );
        let cache_write = usage_value(
            usage,
            &[
                "cachedWriteTokens",
                "cacheWriteTokens",
                "cacheCreationTokens",
                "cache_creation_input_tokens",
            ],
        );
        let reasoning = usage_value(
            usage,
            &["reasoningTokens", "thoughtTokens", "thinkingTokens"],
        );
        let reported_total = usage
            .get("totalTokens")
            .or_else(|| usage.get("total_tokens"))
            .and_then(|value| extract_i64(Some(value)))
            .map(|value| value.max(0));

        if raw_input == 0
            && raw_output == 0
            && cache_read == 0
            && cache_write == 0
            && reasoning == 0
        {
            return None;
        }

        // Grok's `inputTokens` includes the `cachedReadTokens` subset, and its
        // `outputTokens` includes the `reasoningTokens` subset. The reported
        // total is input + output, so split those overlaps before handing the
        // values to TokenBreakdown, whose buckets are additive.
        let inclusive_total = raw_input.saturating_add(raw_output);
        // The surrounding Grok usage contract treats input/output as inclusive
        // buckets even when the optional aggregate total is absent. Do not
        // require the redundant total field before removing the nested cache
        // and reasoning buckets.
        let reported_total_is_inclusive =
            reported_total.is_none() || reported_total == Some(inclusive_total);

        Some(Self {
            tokens: TokenBreakdown {
                input: if reported_total_is_inclusive {
                    raw_input.saturating_sub(cache_read)
                } else {
                    raw_input
                },
                output: if reported_total_is_inclusive {
                    raw_output.saturating_sub(reasoning)
                } else {
                    raw_output
                },
                cache_read,
                cache_write,
                reasoning,
            },
        })
    }
}

fn usage_value(value: &Value, keys: &[&str]) -> i64 {
    keys.iter()
        .find_map(|key| extract_i64(value.get(*key)))
        .unwrap_or(0)
        .max(0)
}

fn message_from_tokens(
    metadata: &GrokMetadata,
    model_id: String,
    timestamp: i64,
    tokens: TokenBreakdown,
    dedup_key: String,
    is_turn_start: bool,
) -> UnifiedMessage {
    let mut message = UnifiedMessage::new_with_dedup(
        CLIENT_ID,
        if model_id.trim().is_empty() {
            UNKNOWN_MODEL.to_string()
        } else {
            model_id
        },
        PROVIDER_ID,
        metadata.session_id.clone(),
        timestamp,
        tokens,
        0.0,
        Some(dedup_key),
    );
    message.set_workspace(
        metadata.workspace_key.clone(),
        metadata.workspace_label.clone(),
    );
    message.is_turn_start = is_turn_start;
    message
}

impl ActiveTurn {
    fn new(baseline_total: i64, timestamp: i64, model_id: String, turn_index: usize) -> Self {
        Self {
            baseline_total,
            max_total: baseline_total,
            timestamp,
            model_id,
            turn_index,
        }
    }

    fn observe_total(&mut self, total: i64, timestamp: i64) {
        if total > self.max_total {
            self.max_total = total;
            self.timestamp = timestamp;
        }
    }

    fn into_message(self, metadata: &GrokMetadata) -> Option<UnifiedMessage> {
        let token_delta = self.max_total.saturating_sub(self.baseline_total);
        if token_delta <= 0 {
            return None;
        }

        let model_id = if self.model_id.trim().is_empty() {
            UNKNOWN_MODEL.to_string()
        } else {
            self.model_id
        };

        Some(message_from_tokens(
            metadata,
            model_id,
            self.timestamp,
            TokenBreakdown {
                input: token_delta,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            format!("grok:{}:{}", metadata.session_id, self.turn_index),
            true,
        ))
    }
}

pub fn parse_grok_updates_file(path: &Path) -> Vec<UnifiedMessage> {
    if path.file_name().and_then(|name| name.to_str()) != Some("updates.jsonl") {
        return Vec::new();
    }

    let metadata = read_metadata(path);
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };

    let mut fallback_messages = Vec::new();
    let mut usage_messages = Vec::new();
    let mut current_model = metadata
        .model_id
        .clone()
        .unwrap_or_else(|| UNKNOWN_MODEL.to_string());
    let mut last_total: Option<i64> = None;
    let mut last_total_timestamp = metadata.timestamp;
    let mut active_turn: Option<ActiveTurn> = None;
    let mut turn_index = 0usize;
    let mut usage_index = 0usize;

    for line in lossy_lines(BufReader::new(file)) {
        if line.trim().is_empty() {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        if let Some(model_id) = extract_model_id(&value) {
            current_model = model_id;
            if let Some(turn) = active_turn.as_mut() {
                if turn.model_id == UNKNOWN_MODEL {
                    turn.model_id = current_model.clone();
                }
            }
        }

        let timestamp = extract_timestamp_ms(&value).unwrap_or(metadata.timestamp);
        if is_user_message_chunk(&value) {
            if let Some(turn) = active_turn.take() {
                if let Some(message) = turn.into_message(&metadata) {
                    fallback_messages.push(message);
                }
            }

            active_turn = Some(ActiveTurn::new(
                last_total.unwrap_or(0),
                timestamp,
                current_model.clone(),
                turn_index,
            ));
            turn_index = turn_index.saturating_add(1);
        }

        if let Some(usage) = GrokUsage::from_update(&value) {
            let model_id = if current_model != UNKNOWN_MODEL {
                current_model.clone()
            } else {
                get_path(&value, &["params", "update", "usage", "modelUsage"])
                    .and_then(Value::as_object)
                    .and_then(|models| (models.len() == 1).then(|| models.keys().next().cloned()))
                    .flatten()
                    .or_else(|| metadata.model_id.clone())
                    .unwrap_or_else(|| UNKNOWN_MODEL.to_string())
            };
            let event_id = get_path(&value, &["params", "_meta", "eventId"])
                .and_then(|value| extract_string(Some(value)))
                .unwrap_or_else(|| format!("turn-{usage_index}"));
            // `eventId` is not unique: Grok reuses it across usage records, so
            // keying on it alone gave distinct turns byte-identical keys. The
            // Grok lane in `lib.rs` does not collapse duplicate keys today —
            // it only runs `prefer_unified_log_messages` — so this is not
            // currently load-bearing, but a per-record-unique key is correct on
            // its own merits and cheap insurance against any consumer that does
            // key on it. The position of the record within the file
            // disambiguates them and stays stable across re-parses of an
            // unchanged file, which the on-disk message cache this key feeds
            // requires. Note the key is only unique within one file; it is not
            // a global identity.
            usage_messages.push(message_from_tokens(
                &metadata,
                model_id,
                timestamp,
                usage.tokens,
                format!(
                    "grok:{}:usage:{usage_index}:{event_id}",
                    metadata.session_id
                ),
                true,
            ));
            usage_index = usage_index.saturating_add(1);
        }

        let Some(total_tokens) = extract_total_tokens(&value) else {
            continue;
        };
        if total_tokens < 0 {
            continue;
        }

        match last_total {
            Some(previous) if total_tokens < previous => {
                // Grok sometimes repeats or rewinds intermediate counters while
                // streaming tool updates. Treat cumulative totals as monotonic.
                continue;
            }
            Some(previous) if total_tokens == previous => {
                last_total_timestamp = timestamp;
            }
            Some(previous) => {
                if active_turn.is_none() {
                    active_turn = Some(ActiveTurn::new(
                        previous,
                        timestamp,
                        current_model.clone(),
                        turn_index,
                    ));
                    turn_index = turn_index.saturating_add(1);
                }
                if let Some(turn) = active_turn.as_mut() {
                    turn.observe_total(total_tokens, timestamp);
                }
                last_total_timestamp = timestamp;
                last_total = Some(total_tokens);
            }
            None => {
                if let Some(turn) = active_turn.as_mut() {
                    turn.observe_total(total_tokens, timestamp);
                }
                last_total_timestamp = timestamp;
                last_total = Some(total_tokens);
            }
        }
    }

    if let Some(turn) = active_turn {
        if let Some(message) = turn.into_message(&metadata) {
            fallback_messages.push(message);
        }
    }

    if fallback_messages.is_empty() && usage_messages.is_empty() {
        if let Some(total_tokens) = last_total.filter(|tokens| *tokens > 0) {
            let aggregate_turn = ActiveTurn {
                baseline_total: 0,
                max_total: total_tokens,
                timestamp: last_total_timestamp,
                model_id: current_model.clone(),
                turn_index: 0,
            };
            if let Some(message) = aggregate_turn.into_message(&metadata) {
                fallback_messages.push(message);
            }
        }
    }

    if usage_messages.is_empty() {
        append_signals_reconciliation(path, &metadata, &mut fallback_messages, &current_model);
        return fallback_messages;
    }

    // A usage record is emitted when a turn completes. Keep only cumulative
    // counter activity newer than the latest completed turn as a best-effort
    // representation of a currently running turn; older fallback messages are
    // the same work already covered by authoritative usage records.
    let latest_usage_timestamp = usage_messages
        .iter()
        .map(|message| message.timestamp)
        .max()
        .unwrap_or(0);
    usage_messages.extend(
        fallback_messages
            .into_iter()
            .filter(|message| message.timestamp > latest_usage_timestamp),
    );
    usage_messages
}

/// Parse Grok Build's append-only unified log. Each
/// `shell.turn.inference_done` record reports a prompt total that includes
/// cached prompt tokens and a completion total that includes reasoning tokens.
/// Store the non-overlapping component buckets so the breakdown remains
/// additive and the source totals are preserved.
pub fn parse_grok_unified_log_file(path: &Path) -> Vec<UnifiedMessage> {
    if path.file_name().and_then(|name| name.to_str()) != Some("unified.jsonl") {
        return Vec::new();
    }

    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };
    let prefix_len = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    parse_grok_unified_log_snapshot(path, &mut file, prefix_len)
}

#[cfg(test)]
fn parse_grok_unified_log_file_with_prefix(path: &Path, prefix_len: u64) -> Vec<UnifiedMessage> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };
    parse_grok_unified_log_snapshot(path, &mut file, prefix_len)
}

fn parse_grok_unified_log_snapshot(
    path: &Path,
    file: &mut std::fs::File,
    prefix_len: u64,
) -> Vec<UnifiedMessage> {
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let evidence = collect_unified_child_evidence(file, prefix_len);
    if file.seek(SeekFrom::Start(0)).is_err() {
        return Vec::new();
    }

    let metadata_by_session = read_unified_session_metadata(path);
    let mut generations = HashMap::new();
    let mut fallback_model_by_pid: HashMap<UnifiedProcessKey, String> = HashMap::new();
    let mut model_by_pid_and_session: HashMap<UnifiedProcessSessionKey, String> = HashMap::new();
    let mut model_by_session = HashMap::new();
    let mut seen = HashSet::new();
    let mut messages = Vec::new();

    for line in lossy_lines(BufReader::new(file).take(prefix_len)) {
        if line.trim().is_empty() {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        if let Some(pid) = unified_log_process_start_pid(&value) {
            // The unified log survives process restarts, so an OS-reused PID
            // must not inherit model authority from the previous process.
            advance_unified_generation(&mut generations, pid);
            continue;
        }

        let message_name = value.get("msg").and_then(Value::as_str);
        match message_name {
            Some("subagent read parent config (live)") => {
                if let Some((pid, model_id)) = unified_log_parent_model(&value) {
                    let generation = current_unified_generation(&mut generations, pid);
                    fallback_model_by_pid.insert((pid, generation), model_id);
                }
                continue;
            }
            Some("subagent model resolved") => {
                if let Some((pid, model_id)) = unified_log_parent_model(&value) {
                    let generation = current_unified_generation(&mut generations, pid);
                    fallback_model_by_pid.insert((pid, generation), model_id);
                    continue;
                }
            }
            Some("subagent spawn credentials") => {
                if let Some((pid, model_id)) = unified_log_parent_model(&value) {
                    let generation = current_unified_generation(&mut generations, pid);
                    fallback_model_by_pid.insert((pid, generation), model_id);
                }
                if let Some(scope) = unified_child_scope(&value, &mut generations) {
                    if let Some(model_id) = unified_spawn_model(&value) {
                        if unique_child_model(&evidence, &scope) == Some(model_id.as_str()) {
                            model_by_pid_and_session
                                .entry((scope.pid, scope.generation, scope.session_id))
                                .or_insert(model_id);
                        }
                    }
                }
                continue;
            }
            Some("subagent completed") | Some("subagent failed") => {
                if let Some(scope) = unified_child_scope(&value, &mut generations) {
                    if let Some(model_id) = unified_terminal_model(&value) {
                        if unique_terminal_model(&evidence, &scope) == Some(model_id.as_str()) {
                            // A terminal record is fallback evidence, never a
                            // rewrite of a model established by an earlier
                            // exact event.
                            model_by_pid_and_session
                                .entry((scope.pid, scope.generation, scope.session_id))
                                .or_insert(model_id);
                        }
                    }
                }
                continue;
            }
            _ => {}
        }

        if let Some((pid, session_id, model_id)) = unified_log_model_change(&value) {
            match (pid, session_id) {
                (Some(pid), Some(session_id)) => {
                    let generation = current_unified_generation(&mut generations, pid);
                    model_by_pid_and_session.insert((pid, generation, session_id), model_id);
                }
                (None, Some(session_id)) => {
                    model_by_pid_and_session.retain(|key, _| {
                        key.2 != session_id || evidence.child_session_ids.contains(&key.2)
                    });
                    model_by_session.insert(session_id, model_id);
                }
                (Some(pid), None) => {
                    let generation = current_unified_generation(&mut generations, pid);
                    fallback_model_by_pid.insert((pid, generation), model_id);
                }
                (None, None) => {}
            }
            continue;
        }

        if message_name != Some("shell.turn.inference_done") {
            continue;
        }

        let Some(session_id) =
            extract_string(value.get("sid")).filter(|session_id| !session_id.trim().is_empty())
        else {
            continue;
        };
        let Some(context) = value.get("ctx") else {
            continue;
        };
        let Some(prompt_tokens) = required_non_negative_i64(context.get("prompt_tokens")) else {
            continue;
        };
        let Some(completion_tokens) = required_non_negative_i64(context.get("completion_tokens"))
        else {
            continue;
        };
        let Some(mut cached_prompt_tokens) =
            optional_non_negative_i64(context.get("cached_prompt_tokens"))
        else {
            continue;
        };
        let Some(reasoning_tokens) = optional_non_negative_i64(context.get("reasoning_tokens"))
        else {
            continue;
        };
        cached_prompt_tokens = cached_prompt_tokens.min(prompt_tokens);

        let loop_index = match context.get("loop_index") {
            Some(value) => {
                let Some(loop_index) = required_non_negative_i64(Some(value)) else {
                    continue;
                };
                loop_index
            }
            None => 1,
        };
        let Some(pid) = optional_non_negative_i64(value.get("pid")) else {
            continue;
        };
        let timestamp = value
            .get("ts")
            .and_then(parse_timestamp_value)
            .unwrap_or(fallback_timestamp);
        let reasoning = reasoning_tokens.min(completion_tokens);
        let dedup_key = unified_log_dedup_key(&session_id, &value);
        if !seen.insert(dedup_key.clone()) {
            continue;
        }

        let metadata = metadata_by_session
            .get(&session_id)
            .cloned()
            .unwrap_or_else(|| fallback_unified_metadata(&session_id, fallback_timestamp));
        let generation = current_unified_generation(&mut generations, pid);
        let child_scope = value.get("pid").map(|_| UnifiedChildScope {
            pid,
            generation,
            session_id: session_id.clone(),
        });
        let known_scope = child_scope
            .as_ref()
            .is_some_and(|scope| evidence.known_scopes.contains(scope));
        let model_attribution_conflicted = child_scope
            .as_ref()
            .is_some_and(|scope| has_conflicting_child_evidence(&evidence, scope));
        let known_child_session = evidence.child_session_ids.contains(&session_id);
        let exact_model = model_by_pid_and_session
            .get(&(pid, generation, session_id.clone()))
            .cloned();
        let model_id = if model_attribution_conflicted {
            UNKNOWN_MODEL.to_string()
        } else if let Some(model_id) = exact_model {
            model_id
        } else if known_scope {
            child_scope
                .as_ref()
                .and_then(|scope| unique_terminal_model(&evidence, scope))
                .map(str::to_string)
                .unwrap_or_else(|| UNKNOWN_MODEL.to_string())
        } else if known_child_session {
            UNKNOWN_MODEL.to_string()
        } else {
            model_by_session
                .get(&session_id)
                .or_else(|| fallback_model_by_pid.get(&(pid, generation)))
                .cloned()
                .or_else(|| metadata.model_id.clone())
                .unwrap_or_else(|| UNKNOWN_MODEL.to_string())
        };
        let mut message = message_from_tokens(
            &metadata,
            model_id,
            timestamp,
            TokenBreakdown {
                input: prompt_tokens.saturating_sub(cached_prompt_tokens),
                output: completion_tokens.saturating_sub(reasoning),
                cache_read: cached_prompt_tokens,
                cache_write: 0,
                reasoning,
            },
            dedup_key,
            loop_index == 1,
        );
        message.model_attribution_conflicted = model_attribution_conflicted;
        message.session_id = session_id;
        message.message_count = i32::from(message.is_turn_start);
        messages.push(message);
    }

    messages
}

fn collect_unified_child_evidence(
    file: &mut std::fs::File,
    prefix_len: u64,
) -> UnifiedChildEvidence {
    let mut evidence = UnifiedChildEvidence::default();
    let mut generations = HashMap::new();

    for line in lossy_lines(BufReader::new(file).take(prefix_len)) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(pid) = unified_log_process_start_pid(&value) {
            advance_unified_generation(&mut generations, pid);
            continue;
        }

        let message_name = value.get("msg").and_then(Value::as_str);
        let is_spawn = message_name == Some("subagent spawn credentials");
        let is_terminal = matches!(message_name, Some("subagent completed" | "subagent failed"));
        if !is_spawn && !is_terminal {
            continue;
        }
        let Some(subagent_id) = unified_subagent_id(&value) else {
            continue;
        };
        evidence.child_session_ids.insert(subagent_id);
        let Some(scope) = unified_child_scope(&value, &mut generations) else {
            continue;
        };
        evidence.known_scopes.insert(scope.clone());
        if is_terminal {
            evidence.terminal_scopes.insert(scope.clone());
        }

        let model_id = if is_spawn {
            unified_spawn_model(&value)
        } else {
            unified_terminal_model(&value)
        };
        let Some(model_id) = model_id else {
            continue;
        };
        record_model_evidence(&mut evidence.child_models, &scope, model_id.clone());
        if is_terminal {
            record_model_evidence(&mut evidence.terminal_models, &scope, model_id);
        }
    }

    evidence
}

/// Dispatch between Grok's legacy per-session updates and its newer unified
/// log without accepting unrelated JSONL files under the Grok home directory.
pub fn parse_grok_file(path: &Path) -> Vec<UnifiedMessage> {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("updates.jsonl") => parse_grok_updates_file(path),
        Some("unified.jsonl") => parse_grok_unified_log_file(path),
        _ => Vec::new(),
    }
}

/// Return the files and directories that can affect metadata attached to a
/// unified-log message. The unified parser reads every session under the Grok
/// home, so the root, workspace/session directories, and metadata siblings all
/// participate in its source fingerprint. Legacy update files only need their
/// own sibling metadata.
pub(crate) fn grok_related_paths(path: &Path) -> Vec<(String, PathBuf)> {
    if path.file_name().and_then(|name| name.to_str()) != Some("unified.jsonl") {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        return ["signals.json", "summary.json", "events.jsonl"]
            .into_iter()
            .map(|name| (name.to_string(), parent.join(name)))
            .collect();
    }

    let Some(grok_home) = path.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    let sessions_root = grok_home.join("sessions");
    let mut related = vec![("sessions-directory".to_string(), sessions_root.clone())];

    let Some((_, workspaces)) = unified_session_tree(path) else {
        return related;
    };
    for (workspace_dir, session_dirs) in workspaces {
        let workspace_suffix = cache_path_suffix(grok_home, &workspace_dir);
        related.push((
            format!("sessions-workspace:{workspace_suffix}"),
            workspace_dir.clone(),
        ));
        for session_dir in session_dirs {
            let session_suffix = cache_path_suffix(grok_home, &session_dir);
            related.push((
                format!("sessions-session:{session_suffix}"),
                session_dir.clone(),
            ));
            for file_name in [
                "updates.jsonl",
                "summary.json",
                "events.jsonl",
                "signals.json",
            ] {
                related.push((
                    format!("sessions-file:{session_suffix}/{file_name}"),
                    session_dir.join(file_name),
                ));
            }
        }
    }

    related
}

/// Uses the richer, per-inference unified log for sessions it covers. Legacy
/// updates remain a fallback for sessions absent from that log, avoiding an
/// additive merge of two representations of the same activity.
pub fn prefer_unified_log_messages(mut messages: Vec<UnifiedMessage>) -> Vec<UnifiedMessage> {
    let unified_sessions: HashSet<String> = messages
        .iter()
        .filter(|message| is_unified_log_message(message))
        .map(|message| message.session_id.clone())
        .collect();

    if unified_sessions.is_empty() {
        return messages;
    }

    let mut legacy_models = HashMap::new();
    let mut legacy_workspaces = HashMap::new();
    for message in messages
        .iter()
        .filter(|message| !is_unified_log_message(message))
    {
        if message.model_id != UNKNOWN_MODEL {
            match legacy_models.entry(message.session_id.clone()) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(Some(message.model_id.clone()));
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if entry.get().as_ref() != Some(&message.model_id) {
                        entry.insert(None);
                    }
                }
            }
        }

        let workspace = (
            message.workspace_key.clone(),
            message.workspace_label.clone(),
        );
        if workspace == (None, None) {
            continue;
        }

        match legacy_workspaces.entry(message.session_id.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Some(workspace));
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if entry.get().as_ref() != Some(&workspace) {
                    entry.insert(None);
                }
            }
        }
    }

    for message in messages
        .iter_mut()
        .filter(|message| is_unified_log_message(message))
    {
        if message.model_id == UNKNOWN_MODEL && !message.model_attribution_conflicted {
            if let Some(Some(model_id)) = legacy_models.get(&message.session_id) {
                message.model_id = model_id.clone();
            }
        }
        if message.workspace_key.is_none() && message.workspace_label.is_none() {
            if let Some(Some((workspace_key, workspace_label))) =
                legacy_workspaces.get(&message.session_id)
            {
                message.set_workspace(workspace_key.clone(), workspace_label.clone());
            }
        }
    }

    // A unified row only proves that one legacy activity row is covered when
    // both representations agree on the session, timestamp, and inclusive
    // token total. Retain every unmatched legacy row so a partially migrated
    // session cannot lose its older history.
    let mut covered_activity = HashMap::new();
    let mut covered_fallback_timestamps = HashMap::new();
    for message in messages
        .iter()
        .filter(|message| is_unified_log_message(message))
    {
        *covered_activity
            .entry((
                message.session_id.clone(),
                message.timestamp,
                message.tokens.total(),
            ))
            .or_insert(0usize) += 1;
        *covered_fallback_timestamps
            .entry((message.session_id.clone(), message.timestamp))
            .or_insert(0usize) += 1;
    }

    let mut selected = Vec::with_capacity(messages.len());
    for message in messages {
        if is_unified_log_message(&message) {
            selected.push(message);
            continue;
        }

        let key = (
            message.session_id.clone(),
            message.timestamp,
            message.tokens.total(),
        );
        let covered = covered_activity.get_mut(&key).is_some_and(|count| {
            if *count == 0 {
                false
            } else {
                *count -= 1;
                true
            }
        }) || (is_legacy_fallback_message(&message)
            && covered_fallback_timestamps
                .get_mut(&(message.session_id.clone(), message.timestamp))
                .is_some_and(|count| {
                    if *count == 0 {
                        false
                    } else {
                        *count -= 1;
                        true
                    }
                }));
        if !covered {
            selected.push(message);
        }
    }

    selected
}

fn is_unified_log_message(message: &UnifiedMessage) -> bool {
    message
        .dedup_key
        .as_deref()
        .is_some_and(|key| key.starts_with(UNIFIED_LOG_DEDUP_PREFIX))
}

fn is_legacy_fallback_message(message: &UnifiedMessage) -> bool {
    let Some(key) = message.dedup_key.as_deref() else {
        return false;
    };
    key.starts_with("grok:") && !key.contains(":usage:") && !key.ends_with(":signals")
}

fn unified_log_process_start_pid(value: &Value) -> Option<i64> {
    if value.get("msg").and_then(Value::as_str) != Some("AuthManager::new") {
        return None;
    }
    required_non_negative_i64(value.get("pid"))
}

fn unified_log_parent_model(value: &Value) -> Option<(i64, String)> {
    let pid = required_non_negative_i64(value.get("pid"))?;
    let context = value.get("ctx")?;
    let model_id = match value.get("msg").and_then(Value::as_str)? {
        "subagent read parent config (live)" => {
            authoritative_model(context.get("session_model_id"))
                .or_else(|| authoritative_model(context.get("parent_model")))
                .or_else(|| authoritative_model(context.get("global_model_id")))
        }
        "subagent model resolved" | "subagent spawn credentials" => {
            authoritative_model(context.get("parent_model"))
        }
        _ => None,
    }?;
    Some((pid, model_id))
}

fn unified_log_model_change(value: &Value) -> Option<(Option<i64>, Option<String>, String)> {
    let pid = match value.get("pid") {
        Some(value) => Some(required_non_negative_i64(Some(value))?),
        None => None,
    };
    let context = value.get("ctx")?;
    let model_id = match value.get("msg").and_then(Value::as_str)? {
        "model changed" => authoritative_model(context.get("model")),
        "model catalog: notifying clients" => authoritative_model(context.get("current_model_id")),
        "backend_search: model switch" => authoritative_model(context.get("new_model"))
            .or_else(|| authoritative_model(context.get("model")))
            .or_else(|| authoritative_model(context.get("current_model_id"))),
        "subagent model resolved" => authoritative_model(context.get("model_id"))
            .or_else(|| authoritative_model(context.get("model"))),
        _ => None,
    }?;

    let session_id =
        extract_string(value.get("sid")).filter(|session_id| !session_id.trim().is_empty());
    (pid.is_some() || session_id.is_some()).then_some((pid, session_id, model_id))
}

fn required_non_negative_i64(value: Option<&Value>) -> Option<i64> {
    extract_i64(value).filter(|value| *value >= 0)
}

fn optional_non_negative_i64(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(value) => required_non_negative_i64(Some(value)),
        None => Some(0),
    }
}

fn unified_log_dedup_key(session_id: &str, value: &Value) -> String {
    let event_id = [
        &["event_id"][..],
        &["eventId"][..],
        &["id"][..],
        &["uuid"][..],
        &["ctx", "event_id"][..],
        &["ctx", "eventId"][..],
        &["ctx", "id"][..],
        &["ctx", "uuid"][..],
    ]
    .into_iter()
    .find_map(|path| {
        get_path(value, path)
            .and_then(|value| extract_string(Some(value)))
            .filter(|id| !id.trim().is_empty())
    });

    let identity = event_id.map_or_else(
        || {
            // Without a source event ID, the complete normalized row is the
            // stable discriminator. Exact duplicate rows still collapse, but
            // rows that happen to share timestamp and token fields do not.
            format!(
                "row:{}",
                serde_json::to_string(value).unwrap_or_else(|_| String::new())
            )
        },
        |event_id| format!("id:{event_id}"),
    );
    format!("{UNIFIED_LOG_DEDUP_PREFIX}{session_id}:{identity}")
}

fn fallback_unified_metadata(session_id: &str, timestamp: i64) -> GrokMetadata {
    GrokMetadata {
        session_id: session_id.to_string(),
        model_id: None,
        timestamp,
        workspace_key: None,
        workspace_label: None,
    }
}

fn read_unified_session_metadata(path: &Path) -> HashMap<String, GrokMetadata> {
    let Some((_, workspaces)) = unified_session_tree(path) else {
        return HashMap::new();
    };

    let mut metadata_by_session = HashMap::new();
    for (workspace_dir, session_dirs) in workspaces {
        let workspace_key = workspace_dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(percent_decode_lossy)
            .and_then(|decoded| normalize_workspace_key(&decoded));
        let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

        for session_dir in session_dirs {
            let Some(session_id) = session_dir
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|id| !id.trim().is_empty())
            else {
                continue;
            };

            let updates_path = session_dir.join("updates.jsonl");
            let metadata = if updates_path.is_file() {
                read_metadata(&updates_path)
            } else {
                let mut metadata =
                    fallback_unified_metadata(session_id, file_modified_timestamp_ms(&session_dir));
                metadata.workspace_key = workspace_key.clone();
                metadata.workspace_label = workspace_label.clone();
                read_summary_metadata(&session_dir.join("summary.json"), &mut metadata);
                read_events_metadata(&session_dir.join("events.jsonl"), &mut metadata);
                read_signals_metadata(&session_dir.join("signals.json"), &mut metadata);
                metadata
            };
            metadata_by_session.insert(session_id.to_string(), metadata);
        }
    }

    metadata_by_session
}

fn unified_session_tree(path: &Path) -> Option<(PathBuf, UnifiedSessionTree)> {
    let grok_home = path.parent().and_then(Path::parent)?;
    let sessions_root = grok_home.join("sessions");
    let mut workspaces = Vec::new();
    let Ok(entries) = std::fs::read_dir(&sessions_root) else {
        return Some((sessions_root, workspaces));
    };

    for entry in entries.flatten() {
        let workspace_dir = entry.path();
        if !workspace_dir.is_dir() {
            continue;
        }
        let mut session_dirs = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&workspace_dir) {
            for entry in entries.flatten() {
                let session_dir = entry.path();
                if session_dir.is_dir() {
                    session_dirs.push(session_dir);
                }
            }
        }
        session_dirs.sort_unstable();
        workspaces.push((workspace_dir, session_dirs));
    }
    workspaces.sort_by(|left, right| left.0.cmp(&right.0));

    Some((sessions_root, workspaces))
}

fn cache_path_suffix(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn non_negative_i64(value: Option<&Value>) -> i64 {
    extract_i64(value).unwrap_or(0).max(0)
}

fn effective_total_from_signals(value: &Value) -> i64 {
    let before = non_negative_i64(value.get("totalTokensBeforeCompaction"));
    let total = non_negative_i64(value.get("totalTokens"));
    match value.get("contextTokensUsed") {
        None => before.saturating_add(total),
        Some(ctx) => total.max(before.saturating_add(non_negative_i64(Some(ctx)))),
    }
}

fn model_id_from_signals(value: &Value) -> Option<String> {
    extract_string(value.get("primaryModelId")).or_else(|| {
        value
            .get("modelsUsed")
            .and_then(|models| models.as_array())
            .and_then(|models| models.first())
            .and_then(|model| extract_string(Some(model)))
    })
}

fn append_signals_reconciliation(
    updates_path: &Path,
    metadata: &GrokMetadata,
    messages: &mut Vec<UnifiedMessage>,
    fallback_model: &str,
) {
    let signals_path = match sibling(updates_path, "signals.json") {
        Some(path) => path,
        None => return,
    };
    let data = match read_file_or_none(&signals_path) {
        Some(data) => data,
        None => return,
    };
    let value: Value = match serde_json::from_slice(&data) {
        Ok(value) => value,
        Err(_) => return,
    };

    let signals_total = effective_total_from_signals(&value);
    if signals_total <= 0 {
        return;
    }

    let updates_total: i64 = messages.iter().map(|message| message.tokens.input).sum();
    let extra = signals_total.saturating_sub(updates_total);
    if extra <= 0 {
        return;
    }

    let model_id = model_id_from_signals(&value)
        .filter(|model| !model.trim().is_empty())
        .or_else(|| metadata.model_id.clone())
        .unwrap_or_else(|| fallback_model.to_string());
    // Anchor the reconciliation delta to the last recorded update activity rather
    // than signals.json's mtime. The mtime advances every time Grok rewrites the
    // rollup for a live session, which would migrate this whole (potentially
    // multi-million-token) extra to a new day on each rescan and retroactively
    // shrink the prior day's total. The last update timestamp only moves when
    // genuine new activity is recorded, so the delta stays put across rescans.
    let timestamp = messages
        .iter()
        .map(|message| message.timestamp)
        .max()
        .unwrap_or(metadata.timestamp);

    let mut message = UnifiedMessage::new_with_dedup(
        CLIENT_ID,
        model_id,
        PROVIDER_ID,
        metadata.session_id.clone(),
        timestamp,
        TokenBreakdown {
            input: extra,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        },
        0.0,
        Some(format!("grok:{}:signals", metadata.session_id)),
    );
    message.set_workspace(
        metadata.workspace_key.clone(),
        metadata.workspace_label.clone(),
    );
    messages.push(message);
}

fn read_metadata(path: &Path) -> GrokMetadata {
    let session_dir = path.parent();
    let session_id = session_dir
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .filter(|id| !id.trim().is_empty())
        .unwrap_or("unknown")
        .to_string();

    let workspace_key = session_dir
        .and_then(|dir| dir.parent())
        .and_then(|workspace_dir| workspace_dir.file_name())
        .and_then(|name| name.to_str())
        .map(percent_decode_lossy)
        .and_then(|decoded| normalize_workspace_key(&decoded));
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

    let fallback_timestamp = file_modified_timestamp_ms(path);
    let mut metadata = GrokMetadata {
        session_id,
        model_id: None,
        timestamp: fallback_timestamp,
        workspace_key,
        workspace_label,
    };

    if let Some(summary_path) = sibling(path, "summary.json") {
        read_summary_metadata(&summary_path, &mut metadata);
    }
    if let Some(events_path) = sibling(path, "events.jsonl") {
        read_events_metadata(&events_path, &mut metadata);
    }
    if let Some(signals_path) = sibling(path, "signals.json") {
        read_signals_metadata(&signals_path, &mut metadata);
    }

    metadata
}

fn read_signals_metadata(path: &Path, metadata: &mut GrokMetadata) {
    let Some(data) = read_file_or_none(path) else {
        return;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&data) else {
        return;
    };

    if metadata.model_id.is_none() {
        metadata.model_id = model_id_from_signals(&value);
    }
}

fn read_summary_metadata(path: &Path, metadata: &mut GrokMetadata) {
    let Some(data) = read_file_or_none(path) else {
        return;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&data) else {
        return;
    };

    if metadata.model_id.is_none() {
        metadata.model_id = extract_string(value.get("current_model_id"))
            .or_else(|| extract_string(value.get("model_id")));
    }

    if let Some(timestamp) = value
        .get("updated_at")
        .or_else(|| value.get("created_at"))
        .and_then(parse_timestamp_value)
    {
        metadata.timestamp = timestamp;
    }
}

fn read_events_metadata(path: &Path, metadata: &mut GrokMetadata) {
    let Ok(file) = std::fs::File::open(path) else {
        return;
    };

    for line in lossy_lines(BufReader::new(file)).take(500) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        if metadata.model_id.is_none() {
            metadata.model_id = extract_string(value.get("model_id"));
        }
        if metadata.session_id == "unknown" {
            if let Some(session_id) = extract_string(value.get("session_id")) {
                metadata.session_id = session_id;
            }
        }
        if let Some(timestamp) = value.get("ts").and_then(parse_timestamp_value) {
            metadata.timestamp = timestamp;
        }

        if metadata.model_id.is_some() && metadata.session_id != "unknown" {
            break;
        }
    }
}

fn sibling(path: &Path, file_name: &str) -> Option<PathBuf> {
    Some(path.parent()?.join(file_name))
}

fn extract_model_id(value: &Value) -> Option<String> {
    for path in [
        &["params", "update", "_meta", "modelId"][..],
        &["params", "_meta", "modelId"][..],
        &["params", "modelId"][..],
        &["model_id"][..],
        &["modelId"][..],
        &["model"][..],
    ] {
        if let Some(model_id) = get_path(value, path).and_then(|value| extract_string(Some(value)))
        {
            if !model_id.trim().is_empty() {
                return Some(model_id);
            }
        }
    }
    None
}

fn extract_total_tokens(value: &Value) -> Option<i64> {
    for path in [
        &["params", "_meta", "totalTokens"][..],
        &["params", "update", "_meta", "totalTokens"][..],
        &["params", "update", "totalTokens"][..],
        &["params", "totalTokens"][..],
        &["usage", "totalTokens"][..],
        &["totalTokens"][..],
    ] {
        if let Some(total) = get_path(value, path).and_then(|value| extract_i64(Some(value))) {
            return Some(total);
        }
    }
    None
}

fn extract_timestamp_ms(value: &Value) -> Option<i64> {
    for path in [
        &["params", "_meta", "agentTimestampMs"][..],
        &["params", "update", "_meta", "agentTimestampMs"][..],
        &["params", "timestamp"][..],
        &["timestamp"][..],
        &["ts"][..],
    ] {
        if let Some(timestamp) = get_path(value, path).and_then(parse_timestamp_value) {
            return Some(timestamp);
        }
    }
    None
}

fn is_user_message_chunk(value: &Value) -> bool {
    get_path(value, &["params", "update", "sessionUpdate"]).and_then(|value| value.as_str())
        == Some("user_message_chunk")
}

fn get_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
}

fn percent_decode_lossy(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                decoded.push((high << 4) | low);
                i += 3;
                continue;
            }
        }

        decoded.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `updates_jsonl` is taken as bytes so fixtures can contain sequences a
    /// `&str` cannot hold (undecodable bytes, a UTF-8 BOM); `&str` and `&String`
    /// still pass through unchanged.
    fn write_fixture(
        updates_jsonl: impl AsRef<[u8]>,
        summary_json: Option<&str>,
        signals_json: Option<&str>,
    ) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::TempDir::new().unwrap();
        let session_dir = temp
            .path()
            .join(".grok")
            .join("sessions")
            .join("%2Ftmp%2Fproject")
            .join("session-1");
        std::fs::create_dir_all(&session_dir).unwrap();
        let updates_path = session_dir.join("updates.jsonl");
        std::fs::write(&updates_path, updates_jsonl.as_ref()).unwrap();
        if let Some(summary_json) = summary_json {
            std::fs::write(session_dir.join("summary.json"), summary_json).unwrap();
        }
        if let Some(signals_json) = signals_json {
            std::fs::write(session_dir.join("signals.json"), signals_json).unwrap();
        }
        (temp, updates_path)
    }

    fn usage_line(event_id: &str, timestamp_ms: i64, input: i64, output: i64) -> String {
        format!(
            r#"{{"method":"session/update","params":{{"sessionId":"session-1","update":{{"sessionUpdate":"turn_completed","usage":{{"inputTokens":{input},"outputTokens":{output},"totalTokens":{}}}}},"_meta":{{"eventId":"{event_id}","agentTimestampMs":{timestamp_ms}}}}}}}"#,
            input + output
        )
    }

    fn write_unified_fixture(unified_jsonl: &str) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::TempDir::new().unwrap();
        let logs_dir = temp.path().join(".grok/logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        let path = logs_dir.join("unified.jsonl");
        std::fs::write(&path, unified_jsonl).unwrap();
        (temp, path)
    }

    fn test_message(session_id: &str, dedup_key: &str) -> UnifiedMessage {
        UnifiedMessage::new_with_dedup(
            CLIENT_ID,
            "grok-build",
            PROVIDER_ID,
            session_id,
            1_700_000_000_000,
            TokenBreakdown::default(),
            0.0,
            Some(dedup_key.to_string()),
        )
    }

    #[test]
    fn parses_unified_log_token_breakdown_without_double_counting_reasoning() {
        let (_temp, path) = write_unified_fixture(
            r#"{"ts":"2023-11-14T22:13:19Z","pid":17,"sid":"session-1","msg":"model changed","ctx":{"model":"grok-composer-2.5-fast"}}
{"ts":"2023-11-14T22:13:19Z","pid":17,"msg":"model catalog: notifying clients","ctx":{"current_model_id":"grok-4.5"}}
{"ts":"2023-11-14T22:13:20Z","pid":17,"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":100,"cached_prompt_tokens":60,"completion_tokens":25,"reasoning_tokens":5}}
{"ts":"2023-11-14T22:13:21Z","pid":17,"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"loop_index":2,"prompt_tokens":80,"cached_prompt_tokens":0,"completion_tokens":12,"reasoning_tokens":0}}
{"ts":"2023-11-14T22:13:20Z","pid":17,"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":100,"cached_prompt_tokens":60,"completion_tokens":25,"reasoning_tokens":5}}
{"ts":"2023-11-14T22:13:22Z","pid":17,"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"loop_index":3,"prompt_tokens":10,"cached_prompt_tokens":11,"completion_tokens":1,"reasoning_tokens":0}}
{"ts":"2023-11-14T22:13:23Z","pid":17,"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"loop_index":4,"prompt_tokens":10,"cached_prompt_tokens":0,"completion_tokens":1,"reasoning_tokens":2}}"#,
        );

        let messages = parse_grok_unified_log_file(&path);

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].client, CLIENT_ID);
        assert_eq!(messages[0].model_id, "grok-composer-2.5-fast");
        assert_eq!(messages[0].session_id, "session-1");
        assert_eq!(messages[0].tokens.input, 40);
        assert_eq!(messages[0].tokens.cache_read, 60);
        assert_eq!(messages[0].tokens.output, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[0].tokens.total(), 125);
        assert_eq!(messages[0].message_count, 1);
        assert!(messages[0].is_turn_start);
        assert_eq!(messages[1].tokens.input, 80);
        assert_eq!(messages[1].tokens.output, 12);
        assert_eq!(messages[1].message_count, 0);
        assert!(!messages[1].is_turn_start);
        assert_eq!(messages[2].tokens.input, 0);
        assert_eq!(messages[2].tokens.cache_read, 10);
        assert_eq!(messages[2].tokens.output, 1);
        assert_eq!(messages[2].tokens.total(), 11);
        assert_eq!(messages[2].message_count, 0);
        assert!(!messages[2].is_turn_start);
        assert_eq!(messages[3].tokens.input, 10);
        assert_eq!(messages[3].tokens.output, 0);
        assert_eq!(messages[3].tokens.reasoning, 1);
        assert_eq!(messages[3].tokens.total(), 11);
        assert_eq!(messages[3].message_count, 0);
        assert!(!messages[3].is_turn_start);
    }

    #[test]
    fn unified_log_keeps_distinct_rows_when_fallback_timestamp_and_tokens_repeat() {
        let (_temp, path) = write_unified_fixture(
            r#"{"pid":17,"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":100,"completion_tokens":25,"request_id":"first"}}
{"pid":17,"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":100,"completion_tokens":25,"request_id":"second"}}
{"pid":17,"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":100,"completion_tokens":25,"request_id":"first"}}"#,
        );

        let messages = parse_grok_unified_log_file(&path);

        assert_eq!(messages.len(), 2);
        assert_ne!(messages[0].dedup_key, messages[1].dedup_key);
        assert_eq!(messages[0].timestamp, messages[1].timestamp);
        assert_eq!(messages[0].tokens.total(), messages[1].tokens.total());
    }

    #[test]
    fn unified_log_preserves_session_workspace_metadata() {
        let temp = tempfile::TempDir::new().unwrap();
        let logs_dir = temp.path().join("home/.grok/logs");
        let session_dir = temp
            .path()
            .join("home/.grok/sessions/%2Ftmp%2Fproject/session-1");
        std::fs::create_dir_all(&logs_dir).unwrap();
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(
            session_dir.join("summary.json"),
            r#"{"current_model_id":"grok-4.5","updated_at":"2023-11-14T22:13:20Z"}"#,
        )
        .unwrap();
        let path = logs_dir.join("unified.jsonl");
        std::fs::write(
            &path,
            r#"{"ts":"2023-11-14T22:13:20Z","pid":17,"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":10,"cached_prompt_tokens":2,"completion_tokens":4,"reasoning_tokens":1}}"#,
        )
        .unwrap();

        let messages = parse_grok_unified_log_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "grok-4.5");
        assert_eq!(messages[0].workspace_key.as_deref(), Some("/tmp/project"));
        assert_eq!(messages[0].workspace_label.as_deref(), Some("project"));
    }

    #[test]
    fn unified_log_applies_pidless_session_model_switch() {
        let (_temp, path) = write_unified_fixture(
            r#"{"ts":"2023-11-14T22:13:18Z","pid":17,"msg":"model catalog: notifying clients","ctx":{"current_model_id":"grok-4.5"}}
{"ts":"2023-11-14T22:13:19Z","pid":17,"sid":"session-with-model-event","msg":"model changed","ctx":{"model":"grok-composer-2.5-fast"}}
{"ts":"2023-11-14T22:13:20Z","pid":17,"sid":"session-with-model-event","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":10,"completion_tokens":1}}
{"ts":"2023-11-14T22:13:21Z","sid":"session-with-model-event","msg":"model changed","ctx":{"model":"grok-4.1-fast"}}
{"ts":"2023-11-14T22:13:22Z","pid":17,"sid":"session-with-model-event","msg":"shell.turn.inference_done","ctx":{"loop_index":2,"prompt_tokens":15,"completion_tokens":2}}
{"ts":"2023-11-14T22:13:23Z","pid":17,"sid":"session-without-model-event","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":20,"completion_tokens":2}}"#,
        );

        let messages = parse_grok_unified_log_file(&path);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].model_id, "grok-composer-2.5-fast");
        assert_eq!(messages[1].model_id, "grok-4.1-fast");
        assert_eq!(messages[2].model_id, "grok-4.5");
    }

    #[test]
    fn unified_log_expires_pid_scoped_models_on_process_restart() {
        let (_temp, path) = write_unified_fixture(
            r#"{"ts":"2023-11-14T22:13:17Z","sid":"session-stable","msg":"model changed","ctx":{"model":"grok-session"}}
{"ts":"2023-11-14T22:13:18Z","pid":17,"msg":"model catalog: notifying clients","ctx":{"current_model_id":"grok-old"}}
{"ts":"2023-11-14T22:13:19Z","pid":17,"sid":"session-old","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":10,"completion_tokens":1}}
{"ts":"2023-11-14T22:13:20Z","pid":17,"msg":"AuthManager::new","src":"shell","ctx":{}}
{"ts":"2023-11-14T22:13:21Z","pid":17,"sid":"session-stable","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":15,"completion_tokens":1}}
{"ts":"2023-11-14T22:13:22Z","pid":17,"sid":"session-new","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":20,"completion_tokens":2}}
{"ts":"2023-11-14T22:13:23Z","pid":17,"msg":"model catalog: notifying clients","ctx":{"current_model_id":"grok-new"}}
{"ts":"2023-11-14T22:13:24Z","pid":17,"sid":"session-new","msg":"shell.turn.inference_done","ctx":{"loop_index":2,"prompt_tokens":30,"completion_tokens":3}}"#,
        );

        let messages = parse_grok_unified_log_file(&path);

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].model_id, "grok-old");
        assert_eq!(messages[1].model_id, "grok-session");
        assert_eq!(messages[2].model_id, UNKNOWN_MODEL);
        assert_eq!(messages[3].model_id, "grok-new");
    }

    #[test]
    fn unified_log_attributes_parent_and_child_models_by_exact_scope() {
        let (_temp, path) = write_unified_fixture(
            r#"{"ts":"2026-07-31T00:00:00Z","pid":17,"msg":"subagent read parent config (live)","ctx":{"session_model_id":" grok-4.6 ","parent_model":"grok-4.5","global_model_id":"grok-4.4"}}
{"ts":"2026-07-31T00:00:01Z","pid":17,"sid":"parent","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":10,"completion_tokens":2}}
{"ts":"2026-07-31T00:00:02Z","pid":17,"msg":"subagent spawn credentials","ctx":{"subagent_id":"child-a","effective_model":" grok-4.7 ","effective_model_raw":"raw-a","parent_model":"grok-4.6"}}
{"ts":"2026-07-31T00:00:03Z","pid":17,"sid":"child-a","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":11,"completion_tokens":2}}
{"ts":"2026-07-31T00:00:04Z","pid":17,"msg":"subagent spawn credentials","ctx":{"subagent_id":"child-b","effective_model":"grok-4.8","parent_model":"grok-4.6"}}
{"ts":"2026-07-31T00:00:05Z","pid":17,"sid":"child-b","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":12,"completion_tokens":2}}
{"ts":"2026-07-31T00:00:06Z","sid":"child-a","msg":"model changed","ctx":{"model":"grok-global"}}
{"ts":"2026-07-31T00:00:07Z","pid":17,"sid":"child-a","msg":"shell.turn.inference_done","ctx":{"loop_index":2,"prompt_tokens":13,"completion_tokens":2}}
{"ts":"2026-07-31T00:00:08Z","sid":"ordinary","msg":"model changed","ctx":{"model":" grok-ordinary "}}
{"ts":"2026-07-31T00:00:09Z","pid":17,"sid":"ordinary","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":14,"completion_tokens":2}}"#,
        );

        let messages = parse_grok_unified_log_file(&path);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.model_id.as_str())
                .collect::<Vec<_>>(),
            [
                "grok-4.6",
                "grok-4.7",
                "grok-4.8",
                "grok-4.7",
                "grok-ordinary"
            ]
        );
    }

    #[test]
    fn unified_log_fails_closed_on_conflicting_child_evidence() {
        let (_temp, path) = write_unified_fixture(
            r#"{"ts":"2026-07-31T00:00:00Z","pid":19,"sid":"child","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":10,"completion_tokens":2}}
{"ts":"2026-07-31T00:00:01Z","pid":19,"msg":"subagent spawn credentials","ctx":{"subagent_id":"child","effective_model":"grok-4.8"}}
{"ts":"2026-07-31T00:00:02Z","pid":19,"msg":"subagent failed","ctx":{"subagent_id":"child","effective_model":"grok-4.9"}}
{"ts":"2026-07-31T00:00:03Z","pid":19,"sid":"child","msg":"shell.turn.inference_done","ctx":{"loop_index":2,"prompt_tokens":11,"completion_tokens":2}}
{"ts":"2026-07-31T00:00:04Z","pid":19,"msg":"subagent completed","ctx":{"subagent_id":"missing","effective_model":null}}
{"ts":"2026-07-31T00:00:05Z","pid":19,"sid":"missing","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":12,"completion_tokens":2}}"#,
        );

        let messages = parse_grok_unified_log_file(&path);
        assert_eq!(messages.len(), 3);
        assert!(messages
            .iter()
            .all(|message| message.model_id == UNKNOWN_MODEL));
    }

    #[test]
    fn unified_log_snapshot_ignores_rows_appended_after_scan_start() {
        use std::io::Write;

        let (_temp, path) = write_unified_fixture(
            r#"{"ts":"2026-07-31T00:00:00Z","pid":23,"sid":"first","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":10,"completion_tokens":2}}
"#,
        );
        let prefix_len = std::fs::metadata(&path).unwrap().len();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(
                br#"{"ts":"2026-07-31T00:00:01Z","pid":23,"sid":"second","msg":"shell.turn.inference_done","ctx":{"loop_index":1,"prompt_tokens":11,"completion_tokens":2}}
"#,
            )
            .unwrap();

        assert_eq!(
            parse_grok_unified_log_file_with_prefix(&path, prefix_len).len(),
            1
        );
        assert_eq!(parse_grok_unified_log_file(&path).len(), 2);
    }

    #[test]
    fn selector_recovers_unified_model_and_workspace_from_consistent_legacy_rows() {
        let mut legacy = test_message("covered", "grok:covered:0");
        legacy.model_id = "grok-4.5".to_string();
        legacy.set_workspace(
            Some("/tmp/project".to_string()),
            Some("project".to_string()),
        );
        let mut unified = test_message("covered", "grok-unified:covered:1:1:1");
        unified.model_id = UNKNOWN_MODEL.to_string();
        unified.workspace_key = None;
        unified.workspace_label = None;

        let messages = prefer_unified_log_messages(vec![legacy, unified]);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "grok-4.5");
        assert_eq!(messages[0].workspace_key.as_deref(), Some("/tmp/project"));
        assert_eq!(messages[0].workspace_label.as_deref(), Some("project"));
    }

    #[test]
    fn selector_retains_uncovered_legacy_history_for_partially_unified_session() {
        let mut older_legacy = test_message("covered", "grok:covered:older");
        older_legacy.timestamp = 1_700_000_000_000;
        older_legacy.tokens.input = 10;

        let mut covered_legacy = test_message("covered", "grok:covered:covered");
        covered_legacy.timestamp = 1_700_000_001_000;
        covered_legacy.tokens.input = 20;

        let mut covered_unified = test_message("covered", "grok-unified:covered:event");
        covered_unified.timestamp = covered_legacy.timestamp;
        covered_unified.tokens.input = covered_legacy.tokens.input;

        let messages =
            prefer_unified_log_messages(vec![older_legacy, covered_legacy, covered_unified]);

        assert_eq!(messages.len(), 2);
        assert!(messages
            .iter()
            .any(|message| message.dedup_key.as_deref() == Some("grok:covered:older")));
        assert!(messages.iter().any(is_unified_log_message));
    }

    #[test]
    fn selector_is_order_invariant_for_activity_and_fallback_rows() {
        let legacy_activity = test_message("covered", "grok:covered:usage:turn");
        let mut legacy_fallback = test_message("covered", "grok:covered:fallback");
        legacy_fallback.tokens.input = 10;
        let unified = test_message("covered", "grok-unified:covered:event");

        let first_order = prefer_unified_log_messages(vec![
            legacy_activity.clone(),
            legacy_fallback.clone(),
            unified.clone(),
        ]);
        let second_order =
            prefer_unified_log_messages(vec![legacy_fallback, legacy_activity, unified]);

        assert_eq!(first_order, second_order);
        assert_eq!(
            first_order
                .iter()
                .map(|message| message.dedup_key.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("grok-unified:covered:event")]
        );
    }

    #[test]
    fn prefers_unified_log_messages_only_for_covered_sessions() {
        let covered_legacy = test_message("covered", "grok:covered:0");
        let uncovered_legacy = test_message("fallback", "grok:fallback:0");
        let covered_unified = test_message("covered", "grok-unified:covered:1:1:1");

        let messages =
            prefer_unified_log_messages(vec![covered_legacy, uncovered_legacy, covered_unified]);

        assert_eq!(messages.len(), 2);
        assert!(messages
            .iter()
            .any(|message| { message.session_id == "covered" && is_unified_log_message(message) }));
        assert!(messages
            .iter()
            .any(|message| message.session_id == "fallback"));
    }

    #[test]
    fn keeps_parsing_updates_after_an_undecodable_line() {
        let mut fixture = Vec::new();
        fixture.extend_from_slice(usage_line("turn-1", 1_700_000_001_000, 10, 1).as_bytes());
        fixture.push(b'\n');
        // A lone 0xff can never appear in valid UTF-8, so `BufRead::lines()`
        // reports this line as `InvalidData`.
        fixture.extend_from_slice(b"{\"garbage\":\"\xff\xfe\"}\n");
        for index in 2..=100i64 {
            fixture.extend_from_slice(
                usage_line(
                    &format!("turn-{index}"),
                    1_700_000_001_000 + index * 1000,
                    10,
                    1,
                )
                .as_bytes(),
            );
            fixture.push(b'\n');
        }

        let (_temp, path) = write_fixture(&fixture, None, None);
        let messages = parse_grok_updates_file(&path);

        assert_eq!(messages.len(), 100);
        assert_eq!(messages.last().unwrap().timestamp, 1_700_000_101_000);
    }

    #[test]
    fn parses_first_update_of_a_bom_prefixed_file() {
        let mut fixture = Vec::new();
        fixture.extend_from_slice("\u{feff}".as_bytes());
        fixture.extend_from_slice(usage_line("turn-1", 1_700_000_001_000, 10, 1).as_bytes());
        fixture.push(b'\n');
        fixture.extend_from_slice(usage_line("turn-2", 1_700_000_002_000, 20, 2).as_bytes());
        fixture.push(b'\n');

        let (_temp, path) = write_fixture(&fixture, None, None);
        let messages = parse_grok_updates_file(&path);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].timestamp, 1_700_000_001_000);
        assert_eq!(messages[0].tokens.input, 10);
    }

    #[test]
    fn keeps_repeated_event_ids_in_distinct_dedup_keys() {
        let (_temp, path) = write_fixture(
            format!(
                "{}\n{}\n",
                usage_line("turn-1", 1_700_000_001_000, 10, 1),
                usage_line("turn-1", 1_700_000_002_000, 20, 2),
            ),
            None,
            None,
        );

        let messages = parse_grok_updates_file(&path);

        assert_eq!(messages.len(), 2);
        assert_ne!(messages[0].dedup_key, messages[1].dedup_key);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("grok:session-1:usage:0:turn-1")
        );
        assert_eq!(
            messages[1].dedup_key.as_deref(),
            Some("grok:session-1:usage:1:turn-1")
        );
    }

    #[test]
    fn prefers_authoritative_usage_breakdown_when_available() {
        let (_temp, path) = write_fixture(
            r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"user_message_chunk","_meta":{"modelId":"grok-4.5"}},"_meta":{"agentTimestampMs":1700000001000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":1200,"agentTimestampMs":1700000002000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"turn_completed","usage":{"inputTokens":1000,"outputTokens":100,"reasoningTokens":20,"cachedReadTokens":400,"totalTokens":1100,"modelUsage":{"grok-4.5-build":{"inputTokens":1000,"outputTokens":100,"reasoningTokens":20,"cachedReadTokens":400,"totalTokens":1100}}}},"_meta":{"eventId":"turn-1","agentTimestampMs":1700000003000}}}"#,
            None,
            None,
        );

        let messages = parse_grok_updates_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "grok-4.5");
        assert_eq!(messages[0].tokens.input, 600);
        assert_eq!(messages[0].tokens.output, 80);
        assert_eq!(messages[0].tokens.cache_read, 400);
        assert_eq!(messages[0].tokens.reasoning, 20);
        assert_eq!(messages[0].timestamp, 1700000003000);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("grok:session-1:usage:0:turn-1")
        );
    }

    #[test]
    fn parses_inclusive_usage_buckets_without_total_tokens() {
        let (_temp, path) = write_fixture(
            r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"turn_completed","usage":{"inputTokens":100,"outputTokens":25,"reasoningTokens":5,"cachedReadTokens":60}} ,"_meta":{"agentTimestampMs":1700000003000}}}"#,
            None,
            None,
        );

        let messages = parse_grok_updates_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 40);
        assert_eq!(messages[0].tokens.output, 20);
        assert_eq!(messages[0].tokens.cache_read, 60);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[0].tokens.total(), 125);
    }

    #[test]
    fn parses_grok_total_token_deltas_by_turn() {
        let (_temp, path) = write_fixture(
            r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"available_commands_update"},"_meta":{"totalTokens":100,"agentTimestampMs":1700000000000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"user_message_chunk","_meta":{"modelId":"grok-composer-2.5-fast"}},"_meta":{"agentTimestampMs":1700000001000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_thought_chunk"},"_meta":{"totalTokens":250,"agentTimestampMs":1700000002000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":300,"agentTimestampMs":1700000003000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"user_message_chunk","_meta":{"modelId":"grok-composer-2.5-fast"}},"_meta":{"agentTimestampMs":1700000004000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":450,"agentTimestampMs":1700000005000}}}"#,
            Some(
                r#"{"current_model_id":"grok-composer-2.5-fast","updated_at":"2023-11-14T22:13:20Z"}"#,
            ),
            None,
        );

        let messages = parse_grok_updates_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].client, "grok");
        assert_eq!(messages[0].model_id, "grok-composer-2.5-fast");
        assert_eq!(messages[0].provider_id, "xai");
        assert_eq!(messages[0].session_id, "session-1");
        assert_eq!(messages[0].tokens.input, 200);
        assert_eq!(messages[0].tokens.output, 0);
        assert_eq!(messages[0].timestamp, 1700000003000);
        assert_eq!(messages[0].workspace_key.as_deref(), Some("/tmp/project"));
        assert_eq!(messages[0].workspace_label.as_deref(), Some("project"));
        assert_eq!(messages[1].tokens.input, 150);
        assert_eq!(messages[1].timestamp, 1700000005000);
    }

    #[test]
    fn uses_summary_model_when_update_model_is_missing() {
        let (_temp, path) = write_fixture(
            r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"user_message_chunk"},"_meta":{"agentTimestampMs":1700000000000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":220,"agentTimestampMs":1700000001000}}}"#,
            Some(
                r#"{"current_model_id":"grok-composer-2.5-fast","updated_at":"2023-11-14T22:13:20Z"}"#,
            ),
            None,
        );

        let messages = parse_grok_updates_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "grok-composer-2.5-fast");
        assert_eq!(messages[0].tokens.input, 220);
    }

    #[test]
    fn ignores_repeated_and_decreasing_total_tokens() {
        let (_temp, path) = write_fixture(
            r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"available_commands_update"},"_meta":{"totalTokens":100,"agentTimestampMs":1700000000000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"user_message_chunk","_meta":{"modelId":"grok-composer-2.5-fast"}},"_meta":{"agentTimestampMs":1700000001000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":150,"agentTimestampMs":1700000002000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":150,"agentTimestampMs":1700000003000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":120,"agentTimestampMs":1700000004000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":200,"agentTimestampMs":1700000005000}}}"#,
            None,
            None,
        );

        let messages = parse_grok_updates_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].timestamp, 1700000005000);
    }

    #[test]
    fn preserves_total_tokens_without_model_metadata() {
        let (_temp, path) = write_fixture(
            r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"available_commands_update"},"_meta":{"totalTokens":120,"agentTimestampMs":1700000000000}}}"#,
            None,
            None,
        );

        let messages = parse_grok_updates_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, UNKNOWN_MODEL);
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].timestamp, 1700000000000);
    }

    #[test]
    fn creates_unknown_model_turn_without_model_metadata() {
        let (_temp, path) = write_fixture(
            r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"available_commands_update"},"_meta":{"totalTokens":100,"agentTimestampMs":1700000000000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":250,"agentTimestampMs":1700000002000}}}"#,
            None,
            None,
        );

        let messages = parse_grok_updates_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, UNKNOWN_MODEL);
        assert_eq!(messages[0].tokens.input, 150);
        assert_eq!(messages[0].timestamp, 1700000002000);
    }

    #[test]
    fn adds_signals_reconciliation_when_compaction_exceeds_updates() {
        let (_temp, path) = write_fixture(
            r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"user_message_chunk","_meta":{"modelId":"grok-build"}},"_meta":{"agentTimestampMs":1700000000000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":171056,"agentTimestampMs":1700000001000}}}"#,
            None,
            Some(
                r#"{"primaryModelId":"grok-build","totalTokensBeforeCompaction":3224659,"contextTokensUsed":172309}"#,
            ),
        );

        let messages = parse_grok_updates_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 171056);
        assert_eq!(messages[1].tokens.input, 3225912);
        assert_eq!(messages[1].model_id, "grok-build");
        assert_eq!(
            messages[1].dedup_key.as_deref(),
            Some("grok:session-1:signals")
        );
        assert_eq!(
            messages
                .iter()
                .map(|message| message.tokens.input)
                .sum::<i64>(),
            3396968
        );
    }

    #[test]
    fn signals_reconciliation_anchors_timestamp_to_last_update_not_file_mtime() {
        // The signals.json is written "now" (mtime far in the future relative to
        // the update timestamps). The reconciliation delta must be dated by the
        // last recorded update (1700000001000), NOT the signals.json mtime, so a
        // live session's extra does not migrate to a new day on every rescan.
        let (_temp, path) = write_fixture(
            r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"user_message_chunk","_meta":{"modelId":"grok-build"}},"_meta":{"agentTimestampMs":1700000000000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":171056,"agentTimestampMs":1700000001000}}}"#,
            None,
            Some(
                r#"{"primaryModelId":"grok-build","totalTokensBeforeCompaction":3224659,"contextTokensUsed":172309}"#,
            ),
        );

        let messages = parse_grok_updates_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[1].dedup_key.as_deref(),
            Some("grok:session-1:signals")
        );
        assert_eq!(messages[1].timestamp, 1700000001000);
    }

    #[test]
    fn skips_signals_reconciliation_when_updates_already_cover_signals() {
        let (_temp, path) = write_fixture(
            r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"user_message_chunk"},"_meta":{"agentTimestampMs":1700000000000}}}
{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"agent_message_chunk"},"_meta":{"totalTokens":500,"agentTimestampMs":1700000001000}}}"#,
            None,
            Some(r#"{"primaryModelId":"grok-build","contextTokensUsed":400}"#),
        );

        let messages = parse_grok_updates_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 500);
    }

    #[test]
    fn uses_signals_model_when_updates_model_is_missing() {
        let (_temp, path) = write_fixture(
            r#"{"method":"session/update","params":{"sessionId":"session-1","update":{"sessionUpdate":"available_commands_update"},"_meta":{"totalTokens":50,"agentTimestampMs":1700000000000}}}"#,
            None,
            Some(r#"{"primaryModelId":"grok-composer-2.5-fast","contextTokensUsed":250}"#),
        );

        let messages = parse_grok_updates_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 50);
        assert_eq!(messages[1].tokens.input, 200);
        assert_eq!(messages[1].model_id, "grok-composer-2.5-fast");
    }
}
