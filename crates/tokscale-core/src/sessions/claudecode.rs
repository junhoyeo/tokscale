//! Claude Code session parser
//!
//! Parses JSONL files from ~/.claude/projects/

use super::utils::{
    extract_i64, extract_string, file_modified_timestamp_ms, parse_timestamp_value,
    read_file_or_none,
};
use super::{
    normalize_agent_name, normalize_workspace_key, workspace_label_from_key, UnifiedMessage,
};
use crate::{pricing, provider_identity, TokenBreakdown};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

type ParentSubagentTypeCache = HashMap<PathBuf, HashMap<String, String>>;

/// Claude Code entry structure (from JSONL files)
#[derive(Debug, Deserialize)]
pub struct ClaudeEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub timestamp: Option<String>,
    pub message: Option<ClaudeMessage>,
    /// Request ID for deduplication (used with message.id)
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    /// True for subagent (sidechain) transcript lines
    #[serde(rename = "isSidechain", default)]
    pub is_sidechain: bool,
    /// Stable subagent identifier within its parent session
    #[serde(rename = "agentId")]
    pub agent_id: Option<String>,
    /// Parent session UUID (present on every sidechain line)
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    /// Optional billing or routing provider emitted by wrappers around Claude Code.
    #[serde(rename = "providerId", alias = "provider_id", alias = "provider")]
    pub provider_id: Option<String>,
}

/// Meta sidecar written next to nested-layout sidechain transcripts.
/// e.g. `agent-abc123.meta.json` alongside `agent-abc123.jsonl`
#[derive(Debug, Deserialize)]
struct AgentMetaFile {
    #[serde(rename = "agentType")]
    agent_type: Option<String>,
}

#[derive(Debug, Clone)]
struct CcMirrorVariantMetadata {
    name: String,
    provider_id: Option<String>,
}

impl CcMirrorVariantMetadata {
    fn client_id(&self) -> String {
        format!("cc-mirror/{}", sanitize_cc_mirror_segment(&self.name))
    }
}

#[derive(Debug, Deserialize)]
pub struct ClaudeMessage {
    pub model: Option<String>,
    pub usage: Option<ClaudeUsage>,
    /// Message ID for deduplication (used with requestId)
    pub id: Option<String>,
    /// Optional billing or routing provider emitted by wrappers around Claude Code.
    #[serde(rename = "providerId", alias = "provider_id", alias = "provider")]
    pub provider_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClaudeUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_creation_input_tokens: Option<i64>,
}

/// Resolve the subagent display name for a sidechain transcript file.
///
/// Tier 1: Read the sibling `.meta.json` sidecar for the `agentType` field.
/// Tier 2: Scan the parent session JSONL for the tool_use that spawned this agent.
/// Tier 3: Fall back to a generic "claude-code-subagent" label.
fn resolve_subagent_name(
    path: &Path,
    parent_session_id: Option<&str>,
    entry_agent_id: Option<&str>,
    parent_cache: &mut ParentSubagentTypeCache,
) -> String {
    let stem = match path.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => return normalize_agent_name("claude-code-subagent"),
    };

    // Tier 1: sibling meta.json (e.g. agent-abc123.meta.json next to agent-abc123.jsonl)
    let meta_path = path.with_file_name(format!("{}.meta.json", stem));
    if let Ok(text) = std::fs::read_to_string(&meta_path) {
        if let Ok(meta) = serde_json::from_str::<AgentMetaFile>(&text) {
            if let Some(ref agent_type) = meta.agent_type {
                if !agent_type.trim().is_empty() {
                    return normalize_agent_name(agent_type);
                }
            }
        }
    }

    // Tier 2: parent session tool_use inference
    let lookup_agent_id = entry_agent_id
        .filter(|agent_id| !agent_id.trim().is_empty())
        .map(|agent_id| agent_id.to_string())
        .or_else(|| sidechain_agent_id_from_stem(stem));
    if let (Some(parent_id), Some(agent_id)) = (parent_session_id, lookup_agent_id.as_deref()) {
        if let Some(parent_path) = find_parent_session_path(path, parent_id) {
            if let Some(subagent_type) =
                lookup_subagent_type_in_parent(&parent_path, agent_id, parent_cache)
            {
                return normalize_agent_name(&subagent_type);
            }
        }
    }

    // Tier 3: generic fallback (still visible in the Agents tab)
    normalize_agent_name("claude-code-subagent")
}

/// True for nested-layout workflow orchestration journals
/// (`.../<session>/subagents/**/journal.jsonl`).
///
/// Claude Code writes a `journal.jsonl` alongside `agent-*.jsonl` transcripts to
/// record subagent workflow orchestration (spawn/verdict/result events). It shares
/// the `.jsonl` extension and lives under the recursively-scanned project dir, so
/// the dir-walk discovers it — but it is metadata, NOT a message transcript, and
/// must never be ingested as usage. Its lines carry `type: "started"`/`"result"`
/// (not `user`/`assistant`) so they currently parse to zero usage, but we drop it
/// explicitly so a future journal schema can't silently leak token-like fields.
fn is_workflow_journal(path: &Path) -> bool {
    if path.file_name().and_then(|n| n.to_str()) != Some("journal.jsonl") {
        return false;
    }
    path.ancestors()
        .any(|ancestor| ancestor.file_name().and_then(|n| n.to_str()) == Some("subagents"))
}

/// Locate the parent main-session JSONL for a sidechain transcript.
///
/// Nested layout: `.../projects/<key>/<session>/subagents/agent-X.jsonl`
///   → parent at `.../projects/<key>/<session>.jsonl`
/// Deep nested layout (workflows): `.../projects/<key>/<session>/subagents/workflows/<wf>/agent-X.jsonl`
///   → parent at `.../projects/<key>/<session>.jsonl`
/// Flat layout: `.../projects/<key>/agent-X.jsonl`
///   → parent at `.../projects/<key>/<session-id>.jsonl`
fn find_parent_session_path(sidechain_path: &Path, parent_session_id: &str) -> Option<PathBuf> {
    let parent_filename = format!("{}.jsonl", parent_session_id);

    // Nested layout: locate the `subagents` directory anywhere in the ancestry.
    // The session dir is its parent and the project dir its grandparent, so the
    // parent session file sits at `<project>/<session>.jsonl`. Anchoring on the
    // `subagents` marker (rather than a fixed depth) handles both the shallow
    // `subagents/agent-X.jsonl` and the deeper `subagents/workflows/<wf>/agent-X.jsonl`.
    for ancestor in sidechain_path.ancestors() {
        if ancestor.file_name().and_then(|n| n.to_str()) == Some("subagents") {
            if let Some(project_dir) = ancestor.parent().and_then(|d| d.parent()) {
                let candidate = project_dir.join(&parent_filename);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
            break;
        }
    }

    // Flat layout: parent dir is 1 level up
    if let Some(project_dir) = sidechain_path.parent() {
        let candidate = project_dir.join(&parent_filename);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

/// Scan a parent session JSONL to recover `subagent_type` for a given `agent_id`.
///
/// The parent session contains:
/// - Assistant messages with `tool_use` blocks (`name: "Agent"`, `input.subagent_type`)
/// - User messages with `tool_result` blocks whose text contains `agentId: <hex>`
///
/// We join on `tool_use_id` to map `agentId → subagent_type`.
fn lookup_subagent_type_in_parent(
    parent_path: &Path,
    target_agent_id: &str,
    parent_cache: &mut ParentSubagentTypeCache,
) -> Option<String> {
    if !parent_cache.contains_key(parent_path) {
        parent_cache.insert(
            parent_path.to_path_buf(),
            build_parent_subagent_type_lookup(parent_path)?,
        );
    }

    parent_cache
        .get(parent_path)
        .and_then(|lookup| lookup.get(target_agent_id).cloned())
}

fn build_parent_subagent_type_lookup(parent_path: &Path) -> Option<HashMap<String, String>> {
    let file = std::fs::File::open(parent_path).ok()?;
    let reader = BufReader::new(file);

    // tool_use.id → subagent_type
    let mut tool_use_types: HashMap<String, String> = HashMap::new();
    // tool_use_id → agentId (from tool_result text)
    let mut agent_id_links: HashMap<String, String> = HashMap::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Quick pre-filter: skip lines that can't contain what we need
        let has_subagent_type = trimmed.contains("subagent_type");
        let has_agent_id_text = trimmed.contains("agentId:");
        if !has_subagent_type && !has_agent_id_text {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let content = match value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        {
            Some(arr) => arr,
            None => continue,
        };

        for block in content {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match block_type {
                "tool_use" if has_subagent_type => {
                    if let (Some(id), Some(subagent_type)) = (
                        block.get("id").and_then(|i| i.as_str()),
                        block
                            .get("input")
                            .and_then(|inp| inp.get("subagent_type"))
                            .and_then(|s| s.as_str()),
                    ) {
                        tool_use_types.insert(id.to_string(), subagent_type.to_string());
                    }
                }
                "tool_result" if has_agent_id_text => {
                    let tool_use_id = match block.get("tool_use_id").and_then(|i| i.as_str()) {
                        Some(id) => id.to_string(),
                        None => continue,
                    };
                    // Walk content blocks looking for "agentId: <hex>" in text
                    let result_content = match block.get("content").and_then(|c| c.as_array()) {
                        Some(arr) => arr,
                        None => continue,
                    };
                    for cb in result_content {
                        if let Some(text) = cb.get("text").and_then(|t| t.as_str()) {
                            if let Some(aid) = extract_agent_id_from_text(text) {
                                agent_id_links.insert(tool_use_id.clone(), aid);
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let mut subagent_types = HashMap::new();
    for (tool_use_id, agent_id) in &agent_id_links {
        if let Some(subagent_type) = tool_use_types.get(tool_use_id) {
            subagent_types.insert(agent_id.clone(), subagent_type.clone());
        }
    }

    Some(subagent_types)
}

fn sidechain_agent_id_from_stem(stem: &str) -> Option<String> {
    let agent_stem = stem.strip_prefix("agent-")?;
    if !agent_stem.contains('-') {
        return Some(agent_stem.to_string());
    }

    let trailing_segment = agent_stem.rsplit('-').next()?;
    if trailing_segment.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(trailing_segment.to_string())
    } else {
        Some(agent_stem.to_string())
    }
}

/// Extract the `agentId` hex string from a tool_result text block.
/// Matches the pattern `agentId: <alphanumeric>` written by Claude Code's Agent tool.
fn extract_agent_id_from_text(text: &str) -> Option<String> {
    let marker = "agentId: ";
    let pos = text.find(marker)?;
    let start = pos + marker.len();
    let rest = &text[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    if end > 0 {
        Some(rest[..end].to_string())
    } else {
        None
    }
}

/// Parse a Claude Code JSONL file
pub fn parse_claude_file(path: &Path) -> Vec<UnifiedMessage> {
    let home_dir = dirs::home_dir();
    parse_claude_file_with_home(path, home_dir.as_deref())
}

pub fn parse_claude_file_with_home(path: &Path, home_dir: Option<&Path>) -> Vec<UnifiedMessage> {
    let mut parent_cache = ParentSubagentTypeCache::new();
    parse_claude_file_with_cache_and_home(path, &mut parent_cache, home_dir)
}

pub fn parse_claude_file_with_cache(
    path: &Path,
    parent_cache: &mut ParentSubagentTypeCache,
) -> Vec<UnifiedMessage> {
    let home_dir = dirs::home_dir();
    parse_claude_file_with_cache_and_home(path, parent_cache, home_dir.as_deref())
}

pub fn parse_claude_file_with_cache_and_home(
    path: &Path,
    parent_cache: &mut ParentSubagentTypeCache,
    home_dir: Option<&Path>,
) -> Vec<UnifiedMessage> {
    // Workflow orchestration journals are metadata, not transcripts — never ingest.
    if is_workflow_journal(path) {
        return Vec::new();
    }

    let (workspace_key, workspace_label) = claude_workspace_from_path(path);
    let cc_mirror_metadata = cc_mirror_variant_metadata_from_path(path, home_dir);
    let client_id = cc_mirror_metadata
        .as_ref()
        .map(CcMirrorVariantMetadata::client_id)
        .unwrap_or_else(|| "claude".to_string());
    let metadata_provider_hint = cc_mirror_metadata
        .as_ref()
        .and_then(|metadata| metadata.provider_id.as_deref());
    let mut session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let fallback_timestamp = file_modified_timestamp_ms(path);

    if path.extension().and_then(|s| s.to_str()) == Some("json") {
        let json_messages = parse_claude_headless_json(
            path,
            &session_id,
            fallback_timestamp,
            workspace_key.clone(),
            workspace_label.clone(),
            &client_id,
            metadata_provider_hint,
        );
        if !json_messages.is_empty() {
            return json_messages;
        }
    }

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let reader = BufReader::new(file);
    let mut messages: Vec<UnifiedMessage> = Vec::with_capacity(64);
    let mut provider_confidences: Vec<u8> = Vec::with_capacity(64);
    // Maps dedup_key to the index in `messages` of the first occurrence.
    // CC's streaming API writes the same messageId:requestId multiple times as the
    // response streams in; later entries often carry more complete token counts.
    // We merge duplicates using per-field max to always keep the highest value seen
    // for each token type, ensuring we capture the most complete record.
    let mut processed_hashes: HashMap<String, usize> = HashMap::new();
    let mut headless_state = ClaudeHeadlessState::default();
    let mut buffer = Vec::with_capacity(4096);
    // Tracks whether the previous entry was a user message,
    // so the next assistant message can be marked as a turn start.
    let mut pending_turn_start = false;
    let mut pending_request_start_timestamp_ms: Option<i64> = None;
    let mut last_model: Option<String> = None;
    let mut last_provider_hint: Option<String> = None;
    // Sidechain detection state (resolved lazily on first parseable entry)
    let mut sidechain_agent: Option<String> = None;
    let mut sidechain_detected = false;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut handled = false;
        buffer.clear();
        buffer.extend_from_slice(trimmed.as_bytes());
        if let Ok(entry) = simd_json::from_slice::<ClaudeEntry>(&mut buffer) {
            // Detect sidechain on the first parseable entry (any type).
            // All lines in a subagent file carry isSidechain: true.
            if !sidechain_detected {
                sidechain_detected = true;
                if entry.is_sidechain {
                    // Use parent session ID to fix inflated session counts
                    if let Some(ref parent_id) = entry.session_id {
                        session_id = parent_id.clone();
                    }
                    sidechain_agent = Some(resolve_subagent_name(
                        path,
                        entry.session_id.as_deref(),
                        entry.agent_id.as_deref(),
                        parent_cache,
                    ));
                }
            }

            if entry.entry_type == "user" || entry.entry_type == "tool_result" {
                let tool_result_message = extract_claude_tool_result_message(
                    trimmed,
                    ClaudeToolResultContext {
                        entry: &entry,
                        last_model: last_model.as_deref(),
                        last_provider_hint: last_provider_hint.as_deref(),
                        client_id: &client_id,
                        default_provider_hint: metadata_provider_hint,
                        session_id: &session_id,
                        fallback_timestamp,
                        workspace_key: workspace_key.clone(),
                        workspace_label: workspace_label.clone(),
                        sidechain_agent: sidechain_agent.clone(),
                    },
                );

                if let Some(timestamp_ms) = parse_claude_entry_timestamp(entry.timestamp.as_deref())
                {
                    pending_request_start_timestamp_ms = Some(timestamp_ms);
                }

                if entry.entry_type == "user" && is_human_turn(trimmed) {
                    pending_turn_start = true;
                }

                if let Some(tool_message) = tool_result_message {
                    if let Some(ref dedup_key) = tool_message.dedup_key {
                        if let Some(&existing_idx) = processed_hashes.get(dedup_key) {
                            merge_claude_tool_result_duplicate(
                                &mut messages[existing_idx],
                                tool_message.tokens.input,
                                tool_message.timestamp,
                            );
                            continue;
                        }
                        processed_hashes.insert(dedup_key.clone(), messages.len());
                    }
                    let provider_confidence =
                        stored_claude_provider_confidence(&tool_message.provider_id);
                    messages.push(tool_message);
                    provider_confidences.push(provider_confidence);
                }
                continue;
            }

            // Only process assistant messages with usage data
            if entry.entry_type == "assistant" {
                let message = match entry.message {
                    Some(m) => m,
                    None => continue,
                };

                if let Some(model) = message.model.as_deref() {
                    last_model = Some(model.to_string());
                    last_provider_hint = message
                        .provider_id
                        .as_deref()
                        .or(entry.provider_id.as_deref())
                        .map(str::to_string);
                }

                let usage = match message.usage {
                    Some(u) => u,
                    None => continue,
                };

                let duplicate_provider_choice = claude_provider_choice_from_parts(
                    message.model.as_deref(),
                    message
                        .provider_id
                        .as_deref()
                        .or(entry.provider_id.as_deref())
                        .or(metadata_provider_hint),
                );

                // Build dedup key for global deduplication (messageId:requestId composite).
                // For streaming responses, merge using per-field max to capture the most
                // complete token counts across all duplicate entries.
                let pending_hash = match (&message.id, &entry.request_id) {
                    (Some(msg_id), Some(req_id)) => {
                        let hash = format!("{}:{}", msg_id, req_id);
                        if let Some(&existing_idx) = processed_hashes.get(&hash) {
                            merge_claude_duplicate(
                                &mut messages[existing_idx],
                                &usage,
                                parse_claude_entry_timestamp(entry.timestamp.as_deref()),
                                pending_request_start_timestamp_ms,
                            );
                            if let Some(choice) = duplicate_provider_choice {
                                update_claude_provider_id(
                                    &mut messages[existing_idx].provider_id,
                                    &mut provider_confidences[existing_idx],
                                    choice,
                                );
                            }
                            continue;
                        }
                        Some(hash)
                    }
                    (Some(msg_id), None) => {
                        let hash = format!("message:{}", msg_id);
                        if let Some(&existing_idx) = processed_hashes.get(&hash) {
                            merge_claude_duplicate(
                                &mut messages[existing_idx],
                                &usage,
                                parse_claude_entry_timestamp(entry.timestamp.as_deref()),
                                pending_request_start_timestamp_ms,
                            );
                            if let Some(choice) = duplicate_provider_choice {
                                update_claude_provider_id(
                                    &mut messages[existing_idx].provider_id,
                                    &mut provider_confidences[existing_idx],
                                    choice,
                                );
                            }
                            continue;
                        }
                        Some(hash)
                    }
                    _ => None,
                };

                let raw_model = match message.model {
                    Some(m) => m,
                    None => continue,
                };
                let provider_choice = claude_provider_choice(
                    &raw_model,
                    message
                        .provider_id
                        .as_deref()
                        .or(entry.provider_id.as_deref())
                        .or(metadata_provider_hint),
                );
                let provider_confidence = provider_choice.confidence;
                let model = canonicalize_claude_model(&raw_model);

                let parsed_timestamp = parse_claude_entry_timestamp(entry.timestamp.as_deref());
                let timestamp = parsed_timestamp.unwrap_or(fallback_timestamp);
                let duration_ms =
                    duration_between_ms(pending_request_start_timestamp_ms, parsed_timestamp);

                // Insert dedup index only after all checks pass, right before push
                let dedup_key = pending_hash.inspect(|hash| {
                    processed_hashes.insert(hash.clone(), messages.len());
                });

                let mut unified = UnifiedMessage::new_with_dedup(
                    client_id.clone(),
                    model,
                    provider_choice.id,
                    session_id.clone(),
                    timestamp,
                    TokenBreakdown {
                        input: usage.input_tokens.unwrap_or(0).max(0),
                        output: usage.output_tokens.unwrap_or(0).max(0),
                        cache_read: usage.cache_read_input_tokens.unwrap_or(0).max(0),
                        cache_write: usage.cache_creation_input_tokens.unwrap_or(0).max(0),
                        reasoning: 0,
                    },
                    0.0,
                    dedup_key,
                );
                unified.duration_ms = duration_ms;
                unified.agent = sidechain_agent.clone();
                unified.set_workspace(workspace_key.clone(), workspace_label.clone());
                // Mark the first assistant response after a user message as a turn start
                if pending_turn_start {
                    unified.is_turn_start = true;
                    pending_turn_start = false;
                }
                messages.push(unified);
                provider_confidences.push(provider_confidence);
                // Consume the pending request-start timestamp so a back-to-back
                // assistant message with no intervening user entry doesn't reuse
                // it and report an inflated duration. Streaming duplicates of
                // this same message have already been captured in the dedup map
                // above, so they merge via merge_claude_duplicate without needing
                // the global pending value again.
                pending_request_start_timestamp_ms = None;
                handled = true;
            }
        }

        if handled {
            continue;
        }

        if let Some(message) = process_claude_headless_line(
            trimmed,
            &session_id,
            &mut headless_state,
            fallback_timestamp,
            &client_id,
            metadata_provider_hint,
        ) {
            let mut message = message;
            message.set_workspace(workspace_key.clone(), workspace_label.clone());
            let provider_confidence = stored_claude_provider_confidence(&message.provider_id);
            messages.push(message);
            provider_confidences.push(provider_confidence);
        }
    }

    if let Some(message) = finalize_headless_state(
        &mut headless_state,
        &session_id,
        fallback_timestamp,
        &client_id,
        metadata_provider_hint,
    ) {
        let mut message = message;
        message.set_workspace(workspace_key, workspace_label);
        let provider_confidence = stored_claude_provider_confidence(&message.provider_id);
        messages.push(message);
        provider_confidences.push(provider_confidence);
    }

    messages
}

fn claude_workspace_from_path(path: &Path) -> (Option<String>, Option<String>) {
    let components: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect();

    for window in components.windows(3) {
        if window[0] == ".claude" && window[1] == "projects" {
            let key = normalize_workspace_key(&window[2]);
            let label = key.as_deref().and_then(workspace_label_from_key);
            return (key, label);
        }
    }

    for window in components.windows(5) {
        if window[0] == ".cc-mirror" && window[2] == "config" && window[3] == "projects" {
            let key = normalize_workspace_key(&window[4]);
            let label = key.as_deref().and_then(workspace_label_from_key);
            return (key, label);
        }
    }

    for window in components.windows(2).rev() {
        if window[0] == "projects" {
            let key = normalize_workspace_key(&window[1]);
            let label = key.as_deref().and_then(workspace_label_from_key);
            return (key, label);
        }
    }

    (None, None)
}

fn sanitize_cc_mirror_segment(raw: &str) -> String {
    let mut segment: String = raw
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    while segment.contains("--") {
        segment = segment.replace("--", "-");
    }
    let mut segment = segment
        .trim_matches(|ch| matches!(ch, '-' | '_' | '.'))
        .to_string();
    if segment.len() > 96 {
        segment.truncate(96);
        segment = segment
            .trim_matches(|ch| matches!(ch, '-' | '_' | '.'))
            .to_string();
    }
    if segment.is_empty() {
        "variant".to_string()
    } else {
        segment
    }
}

fn cc_mirror_provider_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("mirror") {
        return Some("anthropic".to_string());
    }
    provider_identity::canonical_provider(trimmed)
}

fn cc_mirror_variant_metadata_from_path(
    path: &Path,
    home_dir: Option<&Path>,
) -> Option<CcMirrorVariantMetadata> {
    let variant_dir = crate::cc_mirror::variant_dir_from_session_path(path, home_dir)?;
    let variant_name = variant_dir.file_name()?.to_string_lossy().to_string();
    let variant_path = crate::cc_mirror::variant_file_path(&variant_dir);
    let metadata = crate::cc_mirror::read_variant_file(&variant_path);

    let name = metadata
        .as_ref()
        .and_then(|metadata| metadata.name.as_deref())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&variant_name)
        .to_string();
    let provider_id = metadata
        .as_ref()
        .and_then(|metadata| {
            metadata
                .provider_id
                .as_deref()
                .or(metadata.provider.as_deref())
        })
        .and_then(cc_mirror_provider_id);

    Some(CcMirrorVariantMetadata { name, provider_id })
}

fn parse_claude_entry_timestamp(timestamp: Option<&str>) -> Option<i64> {
    timestamp
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.timestamp_millis())
}

fn duration_between_ms(start_ms: Option<i64>, end_ms: Option<i64>) -> Option<i64> {
    let duration = end_ms?.saturating_sub(start_ms?);
    (duration > 0).then_some(duration)
}

fn merge_claude_duplicate(
    existing: &mut UnifiedMessage,
    usage: &ClaudeUsage,
    parsed_timestamp: Option<i64>,
    request_start_timestamp_ms: Option<i64>,
) {
    // Per-field max merge: each token field is updated independently.
    let t = &mut existing.tokens;
    t.input = t.input.max(usage.input_tokens.unwrap_or(0).max(0));
    t.output = t.output.max(usage.output_tokens.unwrap_or(0).max(0));
    t.cache_read = t
        .cache_read
        .max(usage.cache_read_input_tokens.unwrap_or(0).max(0));
    t.cache_write = t
        .cache_write
        .max(usage.cache_creation_input_tokens.unwrap_or(0).max(0));

    if let Some(timestamp_ms) = parsed_timestamp {
        if timestamp_ms >= existing.timestamp {
            // Recover the original request-start timestamp from the existing
            // message's recorded duration. The parent loop clears
            // `pending_request_start_timestamp_ms` after the first chunk of a
            // message commits (so a NEW message with no preceding user doesn't
            // inflate by reusing a stale start), which would otherwise blank
            // out streaming duplicates' duration. Recovering from
            // `existing.timestamp - existing.duration_ms` keeps the duration
            // honest for late chunks of the same logical message.
            let recovered_start = existing
                .duration_ms
                .map(|d| existing.timestamp - d)
                .or(request_start_timestamp_ms);
            existing.set_timestamp(timestamp_ms);
            if let Some(new_duration) = duration_between_ms(recovered_start, Some(timestamp_ms)) {
                existing.duration_ms = Some(new_duration);
            }
        }
    }
}

fn merge_claude_tool_result_duplicate(
    existing: &mut UnifiedMessage,
    input_tokens: i64,
    timestamp_ms: i64,
) {
    existing.tokens.input = existing.tokens.input.max(input_tokens.max(0));
    if timestamp_ms >= existing.timestamp {
        existing.set_timestamp(timestamp_ms);
    }
}

struct ClaudeToolResultUsage {
    input_tokens: i64,
    dedup_key: Option<String>,
}

struct ClaudeToolResultContext<'a> {
    entry: &'a ClaudeEntry,
    last_model: Option<&'a str>,
    last_provider_hint: Option<&'a str>,
    client_id: &'a str,
    default_provider_hint: Option<&'a str>,
    session_id: &'a str,
    fallback_timestamp: i64,
    workspace_key: Option<String>,
    workspace_label: Option<String>,
    sidechain_agent: Option<String>,
}

fn extract_claude_tool_result_message(
    line: &str,
    context: ClaudeToolResultContext<'_>,
) -> Option<UnifiedMessage> {
    let value: Value = serde_json::from_str(line).ok()?;
    let usage = extract_claude_tool_result_usage(&value)?;

    let raw_model = extract_claude_model(&value)
        .or_else(|| {
            context
                .entry
                .message
                .as_ref()
                .and_then(|message| message.model.clone())
        })
        .or_else(|| context.last_model.map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    let provider_hint = extract_claude_provider(&value)
        .or_else(|| {
            context
                .entry
                .message
                .as_ref()
                .and_then(|message| message.provider_id.clone())
        })
        .or_else(|| context.entry.provider_id.clone())
        .or_else(|| context.last_provider_hint.map(str::to_string))
        .or_else(|| context.default_provider_hint.map(str::to_string));

    let provider_choice = claude_provider_choice(&raw_model, provider_hint.as_deref());
    let model = canonicalize_claude_model(&raw_model);
    let timestamp = parse_claude_entry_timestamp(context.entry.timestamp.as_deref())
        .or_else(|| extract_claude_timestamp(&value))
        .unwrap_or(context.fallback_timestamp);

    let mut message = UnifiedMessage::new_with_dedup(
        context.client_id,
        model,
        provider_choice.id,
        context.session_id.to_string(),
        timestamp,
        TokenBreakdown {
            input: usage.input_tokens,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
        },
        0.0,
        usage.dedup_key.map(|key| {
            format!(
                "{}:tool_result:{}:{key}",
                context.client_id, context.session_id
            )
        }),
    );
    message.message_count = 0;
    message.agent = context.sidechain_agent;
    message.set_workspace(context.workspace_key, context.workspace_label);
    Some(message)
}

fn extract_claude_tool_result_usage(value: &Value) -> Option<ClaudeToolResultUsage> {
    let mut total_tokens = 0;
    let mut first_dedup_id: Option<String> = None;
    let mut seen_ids = HashSet::new();

    for tool_result in claude_tool_result_values(value) {
        let tool_result_id = extract_tool_result_id(tool_result);
        if let Some(id) = tool_result_id.as_ref() {
            if !seen_ids.insert(id.clone()) {
                continue;
            }
        }
        if first_dedup_id.is_none() {
            first_dedup_id = tool_result_id;
        }
        total_tokens += extract_tool_result_input_tokens(tool_result).unwrap_or(0);
    }

    if total_tokens <= 0 {
        return None;
    }

    Some(ClaudeToolResultUsage {
        input_tokens: total_tokens,
        dedup_key: first_dedup_id.map(|id| format!("tool_result:{id}")),
    })
}

fn claude_tool_result_values(value: &Value) -> Vec<&Value> {
    let mut results = Vec::new();

    if value
        .get("type")
        .and_then(|kind| kind.as_str())
        .is_some_and(|kind| kind == "tool_result")
    {
        results.push(value);
    }

    if let Some(tool_result) = value.get("tool_result") {
        results.push(tool_result);
    }

    if let Some(message_tool_result) = value
        .get("message")
        .and_then(|message| message.get("tool_result"))
    {
        results.push(message_tool_result);
    }

    if let Some(content) = value
        .get("message")
        .and_then(|message| message.get("content"))
        .or_else(|| value.get("content"))
    {
        collect_tool_result_blocks(content, &mut results);
    }

    results
}

fn collect_tool_result_blocks<'a>(value: &'a Value, results: &mut Vec<&'a Value>) {
    if let Some(blocks) = value.as_array() {
        for block in blocks {
            if block
                .get("type")
                .and_then(|kind| kind.as_str())
                .is_some_and(|kind| kind == "tool_result")
            {
                results.push(block);
            }
        }
    }
}

fn extract_tool_result_id(tool_result: &Value) -> Option<String> {
    extract_string(tool_result.get("tool_use_id"))
        .or_else(|| extract_string(tool_result.get("id")))
        .or_else(|| extract_string(tool_result.get("tool_result_id")))
}

fn extract_tool_result_input_tokens(tool_result: &Value) -> Option<i64> {
    explicit_tool_result_input_tokens(tool_result).or_else(|| {
        let chars = tool_result_output_char_count(tool_result);
        (chars > 0).then(|| estimate_tokens_from_chars(chars))
    })
}

fn explicit_tool_result_input_tokens(tool_result: &Value) -> Option<i64> {
    for candidate in [
        tool_result.get("input_tokens"),
        tool_result.get("token_count"),
        tool_result.get("tokens"),
        tool_result
            .get("usage")
            .and_then(|usage| usage.get("input_tokens")),
        tool_result
            .get("tool_output")
            .and_then(|tool_output| tool_output.get("input_tokens")),
        tool_result
            .get("tool_output")
            .and_then(|tool_output| tool_output.get("token_count")),
        tool_result
            .get("tool_output")
            .and_then(|tool_output| tool_output.get("tokens")),
        tool_result
            .get("tool_output")
            .and_then(|tool_output| tool_output.get("usage"))
            .and_then(|usage| usage.get("input_tokens")),
    ] {
        if let Some(tokens) = extract_i64(candidate) {
            return Some(tokens.max(0));
        }
    }
    None
}

fn tool_result_output_char_count(tool_result: &Value) -> usize {
    let mut chars = 0;

    if let Some(output) = tool_result
        .get("tool_output")
        .and_then(|tool_output| tool_output.get("output"))
        .and_then(|output| output.as_str())
    {
        chars += output.chars().count();
    }

    match tool_result.get("content") {
        Some(content) if content.is_string() => {
            chars += content
                .as_str()
                .map(str::chars)
                .map(Iterator::count)
                .unwrap_or(0);
        }
        Some(content) => {
            chars += tool_result_content_output_chars(content);
        }
        None => {}
    }

    chars
}

fn tool_result_content_output_chars(content: &Value) -> usize {
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .map(|block| {
                    block
                        .get("tool_output")
                        .and_then(|tool_output| tool_output.get("output"))
                        .and_then(|output| output.as_str())
                        .or_else(|| block.get("text").and_then(|text| text.as_str()))
                        .map(str::chars)
                        .map(Iterator::count)
                        .unwrap_or(0)
                })
                .sum()
        })
        .unwrap_or(0)
}

fn estimate_tokens_from_chars(chars: usize) -> i64 {
    // Claude Code tool outputs may not include token metadata. Match the
    // existing Kiro fallback of one token per four characters, rounded up.
    chars.div_ceil(4) as i64
}

fn canonicalize_claude_model(model: &str) -> String {
    pricing::aliases::resolve_alias(model)
        .unwrap_or(model)
        .to_string()
}

#[derive(Default)]
struct ClaudeHeadlessState {
    model: Option<String>,
    provider_id: Option<String>,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    timestamp_ms: Option<i64>,
}

fn parse_claude_headless_json(
    path: &Path,
    session_id: &str,
    fallback_timestamp: i64,
    workspace_key: Option<String>,
    workspace_label: Option<String>,
    client_id: &str,
    default_provider_hint: Option<&str>,
) -> Vec<UnifiedMessage> {
    let Some(data) = read_file_or_none(path) else {
        return Vec::new();
    };

    let mut bytes = data;
    let value: Value = match simd_json::from_slice(&mut bytes) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut messages = Vec::with_capacity(1);
    if let Some(message) = extract_claude_headless_message(
        &value,
        session_id,
        fallback_timestamp,
        client_id,
        default_provider_hint,
    ) {
        let mut message = message;
        message.set_workspace(workspace_key, workspace_label);
        messages.push(message);
    }

    messages
}

fn process_claude_headless_line(
    line: &str,
    session_id: &str,
    state: &mut ClaudeHeadlessState,
    fallback_timestamp: i64,
    client_id: &str,
    default_provider_hint: Option<&str>,
) -> Option<UnifiedMessage> {
    let mut bytes = line.as_bytes().to_vec();
    let value: Value = simd_json::from_slice(&mut bytes).ok()?;

    let event_type = value.get("type").and_then(|val| val.as_str()).unwrap_or("");
    let mut completed_message: Option<UnifiedMessage> = None;

    match event_type {
        "message_start" => {
            completed_message = finalize_headless_state(
                state,
                session_id,
                fallback_timestamp,
                client_id,
                default_provider_hint,
            );

            state.model = extract_claude_model(&value);
            state.provider_id = extract_claude_provider(&value);
            state.timestamp_ms = extract_claude_timestamp(&value).or(state.timestamp_ms);
            if let Some(usage) = value
                .get("message")
                .and_then(|msg| msg.get("usage"))
                .or_else(|| value.get("usage"))
            {
                update_claude_usage(state, usage);
            }
        }
        "message_delta" => {
            if let Some(usage) = value
                .get("usage")
                .or_else(|| value.get("delta").and_then(|delta| delta.get("usage")))
            {
                update_claude_usage(state, usage);
            }
        }
        "message_stop" => {
            completed_message = finalize_headless_state(
                state,
                session_id,
                fallback_timestamp,
                client_id,
                default_provider_hint,
            );
        }
        _ => {
            if let Some(message) = extract_claude_headless_message(
                &value,
                session_id,
                fallback_timestamp,
                client_id,
                default_provider_hint,
            ) {
                completed_message = Some(message);
            }
        }
    }

    completed_message
}

fn extract_claude_headless_message(
    value: &Value,
    session_id: &str,
    fallback_timestamp: i64,
    client_id: &str,
    default_provider_hint: Option<&str>,
) -> Option<UnifiedMessage> {
    let usage = value
        .get("usage")
        .or_else(|| value.get("message").and_then(|msg| msg.get("usage")))?;
    let raw_model = extract_claude_model(value)?;
    let provider_hint = extract_claude_provider(value);
    let provider_id = claude_provider_id(
        &raw_model,
        provider_hint.as_deref().or(default_provider_hint),
    );
    let model = canonicalize_claude_model(&raw_model);
    let timestamp = extract_claude_timestamp(value).unwrap_or(fallback_timestamp);

    Some(UnifiedMessage::new(
        client_id,
        model,
        provider_id,
        session_id.to_string(),
        timestamp,
        TokenBreakdown {
            input: extract_i64(usage.get("input_tokens")).unwrap_or(0).max(0),
            output: extract_i64(usage.get("output_tokens")).unwrap_or(0).max(0),
            cache_read: extract_i64(usage.get("cache_read_input_tokens"))
                .unwrap_or(0)
                .max(0),
            cache_write: extract_i64(usage.get("cache_creation_input_tokens"))
                .unwrap_or(0)
                .max(0),
            reasoning: 0,
        },
        0.0,
    ))
}

/// Internal Claude Code system/tool tags that should NOT be counted as human turns.
/// User prompts containing arbitrary HTML/XML (e.g. `<div>hello</div>`) are still
/// counted, only this narrow allowlist is excluded.
const CLAUDECODE_INTERNAL_USER_TAGS: &[&str] = &[
    "<local-command-stdout>",
    "<local-command-stderr>",
    "<command-name>",
    "<command-message>",
    "<system-reminder>",
    "<bash-input>",
    "<bash-stdout>",
    "<bash-stderr>",
];

/// Returns true if a `type: "user"` JSONL entry is genuine human input (not tool results or system messages).
fn is_human_turn(raw_line: &str) -> bool {
    if let Some(pos) = raw_line.find("\"content\":") {
        let after = &raw_line[pos + 10..];
        let after_trimmed = after.trim_start();
        if after_trimmed.starts_with('[') {
            return false;
        }
        if let Some(content_start) = after_trimmed.strip_prefix('"') {
            // Only filter out content that begins with a known internal tag.
            // Anything else (including `<div>`, `<table>`, etc. in genuine prompts)
            // is treated as a real human turn.
            for tag in CLAUDECODE_INTERNAL_USER_TAGS {
                if content_start.starts_with(tag) {
                    return false;
                }
            }
            return true;
        }
    }
    false
}

fn extract_claude_model(value: &Value) -> Option<String> {
    extract_string(value.get("model")).or_else(|| {
        value
            .get("message")
            .and_then(|msg| extract_string(msg.get("model")))
    })
}

fn extract_claude_provider(value: &Value) -> Option<String> {
    extract_string(value.get("providerId"))
        .or_else(|| extract_string(value.get("provider_id")))
        .or_else(|| extract_string(value.get("provider")))
        .or_else(|| {
            value.get("message").and_then(|msg| {
                extract_string(msg.get("providerId"))
                    .or_else(|| extract_string(msg.get("provider_id")))
                    .or_else(|| extract_string(msg.get("provider")))
            })
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaudeProviderChoice {
    id: String,
    confidence: u8,
}

impl ClaudeProviderChoice {
    fn new(id: impl Into<String>, confidence: u8) -> Self {
        Self {
            id: id.into(),
            confidence,
        }
    }
}

const CLAUDE_PROVIDER_DEFAULT_CONFIDENCE: u8 = 1;
const CLAUDE_PROVIDER_INFERRED_CONFIDENCE: u8 = 2;
const CLAUDE_PROVIDER_EXPLICIT_CONFIDENCE: u8 = 3;

fn claude_provider_id(model: &str, provider_hint: Option<&str>) -> String {
    claude_provider_choice(model, provider_hint).id
}

fn claude_provider_choice_from_parts(
    model: Option<&str>,
    provider_hint: Option<&str>,
) -> Option<ClaudeProviderChoice> {
    match model {
        Some(model) => Some(claude_provider_choice(model, provider_hint)),
        None => claude_provider_choice_from_hint(None, provider_hint),
    }
}

fn claude_provider_choice(model: &str, provider_hint: Option<&str>) -> ClaudeProviderChoice {
    if let Some(choice) = claude_provider_choice_from_hint(Some(model), provider_hint) {
        return choice;
    }

    let inferred = provider_identity::inferred_provider_from_model(model);

    if let Some(provider) = provider_from_model_prefix(model) {
        return ClaudeProviderChoice::new(provider, CLAUDE_PROVIDER_EXPLICIT_CONFIDENCE);
    }

    if let Some(provider) = inferred {
        return ClaudeProviderChoice::new(provider, CLAUDE_PROVIDER_INFERRED_CONFIDENCE);
    }

    ClaudeProviderChoice::new("unknown", 0)
}

fn claude_provider_choice_from_hint(
    model: Option<&str>,
    provider_hint: Option<&str>,
) -> Option<ClaudeProviderChoice> {
    let hint = provider_hint.and_then(provider_identity::canonical_provider)?;

    if hint == "anthropic" {
        if let Some(inferred_provider) =
            model.and_then(provider_identity::inferred_provider_from_model)
        {
            if inferred_provider != "anthropic" {
                return Some(ClaudeProviderChoice::new(
                    inferred_provider,
                    CLAUDE_PROVIDER_INFERRED_CONFIDENCE,
                ));
            }
        }
        return Some(ClaudeProviderChoice::new(
            hint,
            CLAUDE_PROVIDER_DEFAULT_CONFIDENCE,
        ));
    }

    Some(ClaudeProviderChoice::new(
        hint,
        CLAUDE_PROVIDER_EXPLICIT_CONFIDENCE,
    ))
}

fn update_claude_provider_id(
    existing: &mut String,
    existing_confidence: &mut u8,
    candidate: ClaudeProviderChoice,
) {
    if candidate.confidence > *existing_confidence {
        *existing_confidence = candidate.confidence;
        *existing = candidate.id;
    }
}

fn stored_claude_provider_confidence(provider_id: &str) -> u8 {
    match provider_identity::canonical_provider(provider_id) {
        None => 0,
        Some(provider) if provider == "anthropic" => CLAUDE_PROVIDER_DEFAULT_CONFIDENCE,
        Some(_) => CLAUDE_PROVIDER_INFERRED_CONFIDENCE,
    }
}

fn provider_from_model_prefix(model: &str) -> Option<String> {
    if model.trim().contains('/') {
        provider_identity::canonical_provider(model)
    } else {
        None
    }
}

fn extract_claude_timestamp(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("message").and_then(|msg| msg.get("created_at")))
        .and_then(parse_timestamp_value)
}

fn update_claude_usage(state: &mut ClaudeHeadlessState, usage: &Value) {
    if let Some(input) = extract_i64(usage.get("input_tokens")) {
        state.input = state.input.max(input);
    }
    if let Some(output) = extract_i64(usage.get("output_tokens")) {
        state.output = state.output.max(output);
    }
    if let Some(cache_read) = extract_i64(usage.get("cache_read_input_tokens")) {
        state.cache_read = state.cache_read.max(cache_read);
    }
    if let Some(cache_write) = extract_i64(usage.get("cache_creation_input_tokens")) {
        state.cache_write = state.cache_write.max(cache_write);
    }
}

fn finalize_headless_state(
    state: &mut ClaudeHeadlessState,
    session_id: &str,
    fallback_timestamp: i64,
    client_id: &str,
    default_provider_hint: Option<&str>,
) -> Option<UnifiedMessage> {
    let raw_model = state.model.clone()?;
    let provider_id = claude_provider_id(
        &raw_model,
        state.provider_id.as_deref().or(default_provider_hint),
    );
    let model = canonicalize_claude_model(&raw_model);
    let timestamp = state.timestamp_ms.unwrap_or(fallback_timestamp);
    if state.input == 0 && state.output == 0 && state.cache_read == 0 && state.cache_write == 0 {
        *state = ClaudeHeadlessState::default();
        return None;
    }

    let message = UnifiedMessage::new(
        client_id,
        model,
        provider_id,
        session_id.to_string(),
        timestamp,
        TokenBreakdown {
            input: state.input.max(0),
            output: state.output.max(0),
            cache_read: state.cache_read.max(0),
            cache_write: state.cache_write.max(0),
            reasoning: 0,
        },
        0.0,
    );

    *state = ClaudeHeadlessState::default();
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn is_human_turn_counts_html_user_prompt() {
        let line = r#"{"type":"user","message":{"content":"<div>hello</div>"}}"#;
        assert!(is_human_turn(line));
    }

    #[test]
    fn is_human_turn_skips_internal_tool_tags() {
        for tag in CLAUDECODE_INTERNAL_USER_TAGS {
            let line =
                format!(r#"{{"type":"user","message":{{"content":"{tag}some output</...>"}}}}"#);
            assert!(
                !is_human_turn(&line),
                "expected tag {tag} to be filtered as non-human"
            );
        }
    }

    #[test]
    fn is_human_turn_skips_array_content() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result"}]}}"#;
        assert!(!is_human_turn(line));
    }

    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    fn create_project_file(
        content: &str,
        project: &str,
        filename: &str,
    ) -> (TempDir, std::path::PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir
            .path()
            .join(".claude")
            .join("projects")
            .join(project)
            .join(filename);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        (temp_dir, path)
    }

    fn create_cc_mirror_project_file(
        content: &str,
        variant: &str,
        provider: &str,
        project: &str,
        filename: &str,
    ) -> (TempDir, std::path::PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let variant_dir = temp_dir.path().join(".cc-mirror").join(variant);
        let config_dir = variant_dir.join("config");
        let path = config_dir.join("projects").join(project).join(filename);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            variant_dir.join("variant.json"),
            format!(
                r#"{{"name":"{variant}","provider":"{provider}","configDir":"{}"}}"#,
                config_dir.display()
            ),
        )
        .unwrap();
        std::fs::write(&path, content).unwrap();
        (temp_dir, path)
    }

    fn create_transcript_file(content: &str, filename: &str) -> (TempDir, std::path::PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir
            .path()
            .join(".claude")
            .join("transcripts")
            .join(filename);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        (temp_dir, path)
    }

    #[test]
    fn test_deduplication_skips_duplicate_entries() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:02.000Z","requestId":"req_002","message":{"id":"msg_002","model":"claude-3-5-sonnet","usage":{"input_tokens":200,"output_tokens":100}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(
            messages.len(),
            2,
            "Should deduplicate to 2 messages (first duplicate skipped)"
        );
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[1].tokens.input, 200);
    }

    #[test]
    fn test_parse_cc_mirror_claude_variant_attributes_client_provider_and_workspace() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":10,"cache_creation_input_tokens":5}}}"#;

        let (_temp_dir, path) = create_cc_mirror_project_file(
            content,
            "zai-worker",
            "zai",
            "-Users-example-work",
            "session.jsonl",
        );

        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "cc-mirror/zai-worker");
        assert_eq!(messages[0].provider_id, "zai");
        assert_eq!(messages[0].model_id, "claude-3-5-sonnet");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.cache_read, 10);
        assert_eq!(messages[0].tokens.cache_write, 5);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("-Users-example-work")
        );
        assert_eq!(
            messages[0].workspace_label.as_deref(),
            Some("-Users-example-work")
        );
    }

    #[test]
    fn test_cc_mirror_variant_client_segment_is_submit_safe() {
        assert_eq!(sanitize_cc_mirror_segment(" zaicc "), "zaicc");
        assert_eq!(sanitize_cc_mirror_segment("../Zai CC!"), "zai-cc");
        assert_eq!(sanitize_cc_mirror_segment("..."), "variant");
        assert_eq!(sanitize_cc_mirror_segment(&"a".repeat(120)).len(), 96);
    }

    #[test]
    fn test_deduplication_keeps_max_output_for_streaming_duplicates() {
        // CC streaming writes the same messageId:requestId multiple times.
        // The first entry has a partial output_tokens count; the last has the
        // final (largest) count. We must keep the entry with the highest
        // output_tokens, not the first-seen entry.
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":10,"output_tokens":31}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:00.100Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":10,"output_tokens":31}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:00.200Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":10,"output_tokens":300}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(
            messages.len(),
            1,
            "Streaming duplicates should collapse to one entry"
        );
        assert_eq!(
            messages[0].tokens.output, 300,
            "Should keep the max output_tokens"
        );
        assert_eq!(messages[0].tokens.input, 10);
    }

    #[test]
    fn test_deduplication_per_field_max_not_just_output() {
        // Later entry has same output but higher input - should still update input
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":10,"output_tokens":100,"cache_read_input_tokens":5}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:00.100Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":50,"output_tokens":100,"cache_read_input_tokens":20}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.output, 100);
        assert_eq!(
            messages[0].tokens.input, 50,
            "Should keep max input even if output unchanged"
        );
        assert_eq!(
            messages[0].tokens.cache_read, 20,
            "Should keep max cache_read even if output unchanged"
        );
    }

    #[test]
    fn test_deduplication_higher_first_lower_later() {
        // First entry has higher output than later - should keep first's higher values
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":500}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:00.100Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":10,"output_tokens":100}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].tokens.output, 500,
            "Should keep max output (first entry)"
        );
        assert_eq!(
            messages[0].tokens.input, 100,
            "Should keep max input (first entry)"
        );
    }

    #[test]
    fn test_deduplication_promotes_provider_hint_from_later_duplicate() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:00.100Z","requestId":"req_001","message":{"id":"msg_001","provider":"openrouter/anthropic","model":"claude-3-5-sonnet","usage":{"input_tokens":120,"output_tokens":75}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openrouter");
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].tokens.output, 75);
    }

    #[test]
    fn test_deduplication_promotes_provider_hint_without_later_model() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","provider":"openrouter/anthropic","timestamp":"2024-12-01T10:00:00.100Z","requestId":"req_001","message":{"id":"msg_001","usage":{"input_tokens":120,"output_tokens":75}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openrouter");
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].tokens.output, 75);
    }

    #[test]
    fn test_deduplication_preserves_explicit_provider_against_later_inference() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","provider":"openrouter/anthropic","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:00.100Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":120,"output_tokens":75}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openrouter");
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].tokens.output, 75);
    }

    #[test]
    fn test_deduplication_skips_model_none_without_stale_index() {
        // First entry has id+requestId+usage but model=null → skipped, no push.
        // Second entry is a valid duplicate. Must not panic on stale index.
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","usage":{"input_tokens":10,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:00.100Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":10,"output_tokens":100}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(
            messages.len(),
            1,
            "Only the entry with model should be kept"
        );
        assert_eq!(messages[0].tokens.output, 100);
    }

    #[test]
    fn test_deduplication_allows_same_message_different_request() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_002","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":150,"output_tokens":75}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(
            messages.len(),
            2,
            "Different requestId should not be deduplicated"
        );
    }

    #[test]
    fn test_deduplication_uses_message_id_without_request_id_and_keeps_final_duration() {
        let content = r#"{"type":"user","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"Hello"}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","message":{"id":"msg_stream","model":"claude-3-5-sonnet","usage":{"input_tokens":10,"output_tokens":25}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:03.500Z","message":{"id":"msg_stream","model":"claude-3-5-sonnet","usage":{"input_tokens":10,"output_tokens":250}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.output, 250);
        assert_eq!(messages[0].timestamp, 1_733_047_203_500);
        assert_eq!(messages[0].duration_ms, Some(3500));
        assert_eq!(messages[0].dedup_key.as_deref(), Some("message:msg_stream"));
    }

    #[test]
    fn test_pending_request_start_is_cleared_between_assistant_messages() {
        // Regression: previously, the user-entry timestamp was set into
        // `pending_request_start_timestamp_ms` and never cleared after the
        // first assistant message consumed it. A subsequent assistant message
        // with no intervening user entry would then reuse the stale start
        // timestamp and report a wildly inflated duration.
        let content = r#"{"type":"user","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"Hello"}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:01:30.000Z","requestId":"req_002","message":{"id":"msg_002","model":"claude-3-5-sonnet","usage":{"input_tokens":200,"output_tokens":80}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].duration_ms,
            Some(1_000),
            "first assistant should report duration vs the user entry (1s)"
        );
        assert_eq!(
            messages[1].duration_ms, None,
            "second assistant has no preceding user entry; duration must NOT \
             reuse the stale pending_request_start_timestamp_ms"
        );
    }

    #[test]
    fn test_entries_without_dedup_fields_still_processed() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","message":{"model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","message":{"model":"claude-3-5-sonnet","usage":{"input_tokens":200,"output_tokens":100}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(
            messages.len(),
            2,
            "Entries without messageId/requestId should still be processed"
        );
    }

    #[test]
    fn test_user_messages_ignored() {
        let content = r#"{"type":"user","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"Hello"}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1, "User messages should be ignored");
        assert_eq!(messages[0].tokens.input, 100);
    }

    #[test]
    fn test_turn_start_detection() {
        // Simulate: user asks → assistant responds → tool_result (as user) → assistant responds
        //         → real user asks again → assistant responds
        // Expected: 2 turns (tool_result should NOT count as a turn)
        let content = r#"{"type":"user","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"Hello"}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"user","timestamp":"2024-12-01T10:00:02.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"tu_001","content":"file contents here"}]}}
{"type":"assistant","timestamp":"2024-12-01T10:00:03.000Z","requestId":"req_002","message":{"id":"msg_002","model":"claude-3-5-sonnet","usage":{"input_tokens":200,"output_tokens":80}}}
{"type":"user","timestamp":"2024-12-01T10:00:04.000Z","message":{"content":"Thanks, now do X"}}
{"type":"assistant","timestamp":"2024-12-01T10:00:05.000Z","requestId":"req_003","message":{"id":"msg_003","model":"claude-3-5-sonnet","usage":{"input_tokens":300,"output_tokens":120}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(
            messages.len(),
            4,
            "Should include 3 assistant messages plus 1 tool-result input message"
        );
        let assistant_messages: Vec<_> = messages
            .iter()
            .filter(|message| message.tokens.output > 0)
            .collect();
        assert_eq!(
            assistant_messages.len(),
            3,
            "Should have 3 assistant usage messages"
        );

        // First assistant after first human user → turn start
        assert!(
            assistant_messages[0].is_turn_start,
            "First response should be turn start"
        );
        // Assistant after tool_result → NOT a new turn
        assert!(
            !assistant_messages[1].is_turn_start,
            "Response after tool_result should NOT be turn start"
        );
        // First assistant after second human user → turn start
        assert!(
            assistant_messages[2].is_turn_start,
            "Response after real user input should be turn start"
        );

        let turn_count: usize = messages.iter().filter(|m| m.is_turn_start).count();
        assert_eq!(turn_count, 2, "Should detect 2 turns");
    }

    #[test]
    fn test_turn_start_ignores_system_messages() {
        // XML-tagged content like <local-command-stdout> should not count as turns
        let content = r#"{"type":"user","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"Do something"}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"user","timestamp":"2024-12-01T10:00:02.000Z","message":{"content":"<local-command-stdout>ok</local-command-stdout>"}}
{"type":"assistant","timestamp":"2024-12-01T10:00:03.000Z","requestId":"req_002","message":{"id":"msg_002","model":"claude-3-5-sonnet","usage":{"input_tokens":200,"output_tokens":80}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 2);
        assert!(
            messages[0].is_turn_start,
            "First response after human input is a turn"
        );
        assert!(
            !messages[1].is_turn_start,
            "Response after local-command should NOT be a turn"
        );

        let turn_count: usize = messages.iter().filter(|m| m.is_turn_start).count();
        assert_eq!(turn_count, 1);
    }

    #[test]
    fn test_turn_start_without_user_message() {
        // No user message → no turn starts (e.g. headless or partial log)
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","message":{"model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","message":{"model":"claude-3-5-sonnet","usage":{"input_tokens":200,"output_tokens":100}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 2);
        assert!(!messages[0].is_turn_start);
        assert!(!messages[1].is_turn_start);
    }

    #[test]
    fn test_token_breakdown_parsing() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":200,"cache_creation_input_tokens":100}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[0].tokens.output, 500);
        assert_eq!(messages[0].tokens.cache_read, 200);
        assert_eq!(messages[0].tokens.cache_write, 100);
        assert_eq!(messages[0].tokens.reasoning, 0);
    }

    #[test]
    fn test_opus_4_7_usage_is_parsed_when_usage_metadata_exists() {
        let content = r#"{"type":"assistant","timestamp":"2026-04-16T10:00:00.000Z","requestId":"req_opus47","message":{"id":"msg_opus47","model":"claude-opus-4-7","usage":{"input_tokens":321,"output_tokens":654,"cache_read_input_tokens":987,"cache_creation_input_tokens":111}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-opus-4-7");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 321);
        assert_eq!(messages[0].tokens.output, 654);
        assert_eq!(messages[0].tokens.cache_read, 987);
        assert_eq!(messages[0].tokens.cache_write, 111);
    }

    #[test]
    fn test_tool_result_output_counts_as_input() {
        let content = r#"{"type":"user","timestamp":"2026-05-27T10:00:00.000Z","message":{"model":"anthropic/claude-4-6-sonnet","content":[{"type":"tool_result","tool_use_id":"toolu_input","tool_output":{"output":"abcdefghijklmnop"}}]}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-4-6");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 4);
        assert_eq!(messages[0].tokens.output, 0);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.cache_write, 0);
        let expected_dedup_key = format!(
            "claude:tool_result:{}:tool_result:toolu_input",
            messages[0].session_id
        );
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some(expected_dedup_key.as_str())
        );
        assert_eq!(messages[0].message_count, 0);
    }

    #[test]
    fn test_cc_mirror_tool_result_keeps_variant_client_and_provider() {
        let content = r#"{"type":"user","timestamp":"2026-05-27T10:00:00.000Z","message":{"model":"sonnet","content":[{"type":"tool_result","tool_use_id":"toolu_cc_mirror","tool_output":{"input_tokens":7,"output":"tool output"}}]}}"#;

        let (_temp_dir, path) = create_cc_mirror_project_file(
            content,
            "zai-worker",
            "zai",
            "project-one",
            "session.jsonl",
        );
        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "cc-mirror/zai-worker");
        assert_eq!(messages[0].provider_id, "zai");
        assert_eq!(messages[0].model_id, "sonnet");
        assert_eq!(messages[0].tokens.input, 7);
        assert_eq!(messages[0].message_count, 0);
    }

    #[test]
    fn test_tool_result_duplicate_uses_max_input_tokens() {
        let content = r#"{"type":"tool_result","timestamp":"2026-05-27T10:00:00.000Z","model":"anthropic/claude-4-6-sonnet","tool_result":{"tool_use_id":"toolu_stream","tool_output":{"output":"abcdefghijklmnop"}}}
{"type":"tool_result","timestamp":"2026-05-27T10:00:00.100Z","model":"anthropic/claude-4-6-sonnet","tool_result":{"tool_use_id":"toolu_stream","tool_output":{"output":"abcdefghijklmnopqrstuvwxyzabcd"}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-4-6");
        assert_eq!(messages[0].tokens.input, 8);
        assert_eq!(messages[0].timestamp, 1_779_876_000_100);
    }

    #[test]
    fn test_tool_result_repeated_in_same_record_is_not_counted_twice() {
        let content = r#"{"type":"tool_result","timestamp":"2026-05-27T10:00:00.000Z","model":"anthropic/claude-4-6-sonnet","tool_result":{"tool_use_id":"toolu_same","tool_output":{"output":"abcdefghijklmnop"}},"message":{"content":[{"type":"tool_result","tool_use_id":"toolu_same","tool_output":{"output":"abcdefghijklmnop"}}]}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 4);
    }

    #[test]
    fn test_tool_result_prefers_input_token_metadata_over_char_estimate() {
        let content = r#"{"type":"user","timestamp":"2026-05-27T10:00:00.000Z","message":{"model":"claude-sonnet-4-6","content":[{"type":"tool_result","tool_use_id":"toolu_metadata","tool_output":{"output":"abcdefghijklmnopqrstuvwxyzabcd","input_tokens":3}}]}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 3);
    }

    #[test]
    fn test_assistant_usage_with_tool_use_is_not_estimated_from_prompt_text() {
        let content = r#"{"type":"assistant","timestamp":"2026-05-27T10:00:00.000Z","message":{"id":"msg_tool_use","model":"claude-sonnet-4-6","content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"/tmp/large.txt"}}],"usage":{"input_tokens":100,"output_tokens":50}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
    }

    #[test]
    fn test_anthropic_prefixed_claude_model_is_canonicalized() {
        let content = r#"{"type":"assistant","timestamp":"2026-05-27T10:00:00.000Z","message":{"model":"anthropic/claude-4-6-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-4-6");
        assert_eq!(messages[0].provider_id, "anthropic");
    }

    #[test]
    fn test_multi_provider_models_infer_provider_from_model() {
        let content = r#"{"type":"assistant","timestamp":"2026-02-18T10:00:00.000Z","message":{"model":"claude-opus-4-6","usage":{"input_tokens":100,"output_tokens":10}}}
{"type":"assistant","timestamp":"2026-02-18T10:00:01.000Z","message":{"model":"gpt-5.3-codex","usage":{"input_tokens":200,"output_tokens":20}}}
{"type":"assistant","timestamp":"2026-02-18T10:00:02.000Z","message":{"model":"gemini-3-flash-preview","usage":{"input_tokens":300,"output_tokens":30}}}
{"type":"assistant","timestamp":"2026-02-18T10:00:03.000Z","message":{"model":"MiniMax-M2.1","usage":{"input_tokens":400,"output_tokens":40}}}
{"type":"assistant","timestamp":"2026-02-18T10:00:04.000Z","message":{"model":"<synthetic>","usage":{"input_tokens":500,"output_tokens":50}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[1].provider_id, "openai");
        assert_eq!(messages[2].provider_id, "google");
        assert_eq!(messages[3].provider_id, "minimax");
        assert_eq!(messages[4].provider_id, "unknown");
    }

    #[test]
    fn test_multi_provider_models_prefer_specific_model_over_default_anthropic_hint() {
        let content = r#"{"type":"assistant","provider":"anthropic","timestamp":"2026-02-18T10:00:00.000Z","message":{"model":"gpt-5.3-codex","usage":{"input_tokens":200,"output_tokens":20}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.3-codex");
        assert_eq!(messages[0].provider_id, "openai");
    }

    #[test]
    fn test_multi_provider_models_preserve_reseller_provider_hint() {
        let content = r#"{"type":"assistant","timestamp":"2026-02-18T10:00:00.000Z","message":{"provider":"openrouter/anthropic","model":"claude-opus-4-6","usage":{"input_tokens":100,"output_tokens":10}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-opus-4-6");
        assert_eq!(messages[0].provider_id, "openrouter");
    }

    #[test]
    fn test_headless_json_output() {
        let content = r#"{"type":"message","message":{"model":"claude-3-5-sonnet","usage":{"input_tokens":120,"output_tokens":60,"cache_read_input_tokens":10}}}"#;
        let file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        std::fs::write(file.path(), content).unwrap();

        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-3-5-sonnet");
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].tokens.output, 60);
        assert_eq!(messages[0].tokens.cache_read, 10);
    }

    #[test]
    fn test_headless_json_output_infers_subprovider() {
        let content = r#"{"type":"message","message":{"model":"gpt-5.3-codex","usage":{"input_tokens":120,"output_tokens":60,"cache_read_input_tokens":10}}}"#;
        let file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        std::fs::write(file.path(), content).unwrap();

        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.3-codex");
        assert_eq!(messages[0].provider_id, "openai");
    }

    #[test]
    fn test_headless_json_output_keeps_workspace_metadata() {
        let content = r#"{"type":"message","message":{"model":"claude-3-5-sonnet","usage":{"input_tokens":120,"output_tokens":60,"cache_read_input_tokens":10}}}"#;
        let (_dir, path) = create_project_file(content, "myproject", "session.json");

        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key.as_deref(), Some("myproject"));
        assert_eq!(messages[0].workspace_label.as_deref(), Some("myproject"));
    }

    #[test]
    fn test_headless_stream_output() {
        let content = r#"{"type":"message_start","timestamp":"2025-01-01T00:00:00Z","message":{"id":"msg_1","model":"claude-3-5-sonnet","usage":{"input_tokens":200,"cache_read_input_tokens":20,"cache_creation_input_tokens":5}}}
{"type":"message_delta","usage":{"output_tokens":80}}
{"type":"message_stop"}"#;
        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-3-5-sonnet");
        assert_eq!(messages[0].tokens.input, 200);
        assert_eq!(messages[0].tokens.output, 80);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.cache_write, 5);
    }

    #[test]
    fn test_headless_stream_output_infers_subprovider() {
        let content = r#"{"type":"message_start","timestamp":"2026-02-18T10:00:00Z","message":{"id":"msg_1","model":"gemini-3-pro-preview","usage":{"input_tokens":200,"cache_read_input_tokens":20}}}
{"type":"message_delta","usage":{"output_tokens":80}}
{"type":"message_stop"}"#;
        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-3-pro-preview");
        assert_eq!(messages[0].provider_id, "google");
        assert_eq!(messages[0].tokens.input, 200);
        assert_eq!(messages[0].tokens.output, 80);
    }

    #[test]
    fn test_workspace_metadata_from_claude_project_path() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","message":{"model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        let (_dir, path) = create_project_file(content, "myproject", "session.jsonl");

        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key, Some("myproject".to_string()));
        assert_eq!(messages[0].workspace_label, Some("myproject".to_string()));
    }

    #[test]
    fn test_wrapper_transcript_with_usage_is_parsed() {
        let content = r#"{"type":"user","timestamp":"2026-04-01T10:00:00.000Z","message":{"content":"Wrapped prompt"}}
{"type":"assistant","timestamp":"2026-04-01T10:00:01.000Z","requestId":"req_wrapper","message":{"id":"msg_wrapper","model":"claude-sonnet-4","usage":{"input_tokens":123,"output_tokens":45,"cache_read_input_tokens":67,"cache_creation_input_tokens":8}}}"#;
        let (_dir, path) = create_transcript_file(content, "ses_123456789012345678901234567.jsonl");

        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "ses_123456789012345678901234567");
        assert_eq!(messages[0].model_id, "claude-sonnet-4");
        assert_eq!(messages[0].tokens.input, 123);
        assert_eq!(messages[0].tokens.output, 45);
        assert_eq!(messages[0].tokens.cache_read, 67);
        assert_eq!(messages[0].tokens.cache_write, 8);
        assert_eq!(messages[0].workspace_key, None);
        assert_eq!(messages[0].workspace_label, None);
    }

    #[test]
    fn test_wrapper_transcript_without_usage_is_skipped() {
        let content = r#"{"type":"user","timestamp":"2026-04-01T10:00:00.000Z","message":{"content":"Wrapped prompt"}}
{"type":"tool_use","timestamp":"2026-04-01T10:00:01.000Z","message":{"content":"Run tool"}}
{"type":"tool_result","timestamp":"2026-04-01T10:00:02.000Z","message":{"content":"Tool result"}}"#;
        let (_dir, path) = create_transcript_file(content, "ses_765432109876543210987654321.jsonl");

        let messages = parse_claude_file(&path);

        assert!(
            messages.is_empty(),
            "wrapper transcripts without usage metadata must not be estimated"
        );
    }

    // --- Sidechain / Agent tracking tests ---

    /// Helper: create a sidechain JSONL file and optional meta sidecar in a nested layout.
    fn create_sidechain_files(
        project: &str,
        parent_session: &str,
        agent_file_stem: &str,
        jsonl_content: &str,
        meta_content: Option<&str>,
    ) -> (TempDir, std::path::PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let subagents_dir = temp_dir
            .path()
            .join(".claude")
            .join("projects")
            .join(project)
            .join(parent_session)
            .join("subagents");
        std::fs::create_dir_all(&subagents_dir).unwrap();

        let jsonl_path = subagents_dir.join(format!("{}.jsonl", agent_file_stem));
        std::fs::write(&jsonl_path, jsonl_content).unwrap();

        if let Some(meta) = meta_content {
            let meta_path = subagents_dir.join(format!("{}.meta.json", agent_file_stem));
            std::fs::write(&meta_path, meta).unwrap();
        }

        (temp_dir, jsonl_path)
    }

    #[test]
    fn test_sidechain_nested_with_meta_sidecar() {
        let jsonl = r#"{"type":"user","isSidechain":true,"sessionId":"parent-uuid-001","agentId":"abc123","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"Find files"}}
{"type":"assistant","isSidechain":true,"sessionId":"parent-uuid-001","agentId":"abc123","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_s01","message":{"id":"msg_s01","model":"claude-3-5-sonnet","usage":{"input_tokens":200,"output_tokens":80,"cache_read_input_tokens":50}}}"#;
        let meta = r#"{"agentType":"explore","description":"Find session creation UI"}"#;

        let (_dir, path) = create_sidechain_files(
            "myproject",
            "parent-uuid-001",
            "agent-abc123",
            jsonl,
            Some(meta),
        );
        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].agent,
            Some("Explore".to_string()),
            "Should resolve agent name from meta sidecar and normalize"
        );
        assert_eq!(
            messages[0].session_id, "parent-uuid-001",
            "Should use parent session ID from transcript, not filename"
        );
        assert_eq!(messages[0].tokens.input, 200);
        assert_eq!(messages[0].tokens.output, 80);
        assert_eq!(messages[0].tokens.cache_read, 50);
    }

    #[test]
    fn test_sidechain_nested_without_meta_falls_back() {
        let jsonl = r#"{"type":"user","isSidechain":true,"sessionId":"parent-uuid-002","agentId":"def456","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"Do something"}}
{"type":"assistant","isSidechain":true,"sessionId":"parent-uuid-002","agentId":"def456","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_s02","message":{"id":"msg_s02","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":40}}}"#;

        let (_dir, path) =
            create_sidechain_files("myproject", "parent-uuid-002", "agent-def456", jsonl, None);
        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].agent,
            Some("Claude Code Subagent".to_string()),
            "Without meta sidecar, should fall back to generic label"
        );
        assert_eq!(messages[0].session_id, "parent-uuid-002");
    }

    /// Helper: create a deep nested-layout workflow transcript
    /// `.../projects/<project>/<parent_session>/subagents/workflows/<wf>/<agent_stem>.jsonl`.
    fn create_workflow_files(
        project: &str,
        parent_session: &str,
        workflow: &str,
        file_stem: &str,
        jsonl_content: &str,
        meta_content: Option<&str>,
    ) -> (TempDir, std::path::PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let workflow_dir = temp_dir
            .path()
            .join(".claude")
            .join("projects")
            .join(project)
            .join(parent_session)
            .join("subagents")
            .join("workflows")
            .join(workflow);
        std::fs::create_dir_all(&workflow_dir).unwrap();

        let jsonl_path = workflow_dir.join(format!("{}.jsonl", file_stem));
        std::fs::write(&jsonl_path, jsonl_content).unwrap();

        if let Some(meta) = meta_content {
            let meta_path = workflow_dir.join(format!("{}.meta.json", file_stem));
            std::fs::write(&meta_path, meta).unwrap();
        }

        (temp_dir, jsonl_path)
    }

    #[test]
    fn test_workflow_agent_transcript_counts_tokens() {
        // #815: agent-*.jsonl nested under subagents/workflows/<wf>/ is a real
        // transcript and its usage must be counted, keyed to the parent session.
        let jsonl = r#"{"type":"user","isSidechain":true,"sessionId":"wf-parent-001","agentId":"wfa1","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"do work"}}
{"type":"assistant","isSidechain":true,"sessionId":"wf-parent-001","agentId":"wfa1","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_wf1","message":{"id":"msg_wf1","model":"claude-3-5-sonnet","usage":{"input_tokens":500,"output_tokens":200,"cache_read_input_tokens":100,"cache_creation_input_tokens":40}}}"#;
        let meta = r#"{"agentType":"workflow-subagent","spawnDepth":1}"#;

        let (_dir, path) = create_workflow_files(
            "myproject",
            "wf-parent-001",
            "wf_de048031",
            "agent-wfa1",
            jsonl,
            Some(meta),
        );
        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].session_id, "wf-parent-001",
            "deep nested transcript must key to the parent session, not the file/workflow"
        );
        assert_eq!(messages[0].tokens.input, 500);
        assert_eq!(messages[0].tokens.output, 200);
        assert_eq!(messages[0].tokens.cache_read, 100);
        assert_eq!(messages[0].tokens.cache_write, 40);
        assert_eq!(
            messages[0].agent,
            Some("Workflow Subagent".to_string()),
            "Tier 1 meta sidecar next to the deep nested transcript should resolve the name"
        );
    }

    #[test]
    fn test_workflow_journal_not_ingested() {
        // #815 CRITICAL: journal.jsonl is workflow orchestration metadata, not a
        // transcript. Even if it grows lines that superficially resemble usage, the
        // parser must drop the whole file.
        let journal = r#"{"type":"started","key":"v2:abc","agentId":"wfa1"}
{"type":"result","key":"v2:abc","agentId":"wfa1","result":{"verdict":"needs_fixes","summary":"Input tokens are estimated"}}
{"type":"assistant","isSidechain":true,"sessionId":"wf-parent-002","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_j","message":{"id":"msg_j","model":"claude-3-5-sonnet","usage":{"input_tokens":9999,"output_tokens":9999}}}"#;

        let (_dir, path) = create_workflow_files(
            "myproject",
            "wf-parent-002",
            "wf_journal",
            "journal",
            journal,
            None,
        );
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "journal.jsonl");
        let messages = parse_claude_file(&path);

        assert!(
            messages.is_empty(),
            "journal.jsonl must never be ingested, even with usage-shaped lines; got {:?}",
            messages
        );
    }

    #[test]
    fn test_tier2_deep_nested_workflow_recovers_agent() {
        // Deep nested layout without a meta sidecar: agent name must be recovered
        // from the parent session tool_use, proving find_parent_session_path walks
        // up past the extra workflows/<wf>/ levels.
        let temp_dir = tempfile::tempdir().unwrap();
        let project_dir = temp_dir
            .path()
            .join(".claude")
            .join("projects")
            .join("myproject");
        std::fs::create_dir_all(&project_dir).unwrap();

        let parent_session_id = "deep-parent-uuid";
        let parent_content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","message":{"id":"msg_dp","model":"claude-3-5-sonnet","role":"assistant","content":[{"type":"tool_use","id":"toolu_deep","name":"Agent","input":{"subagent_type":"code-reviewer","prompt":"review"}}],"usage":{"input_tokens":80,"output_tokens":40}}}
{"type":"user","timestamp":"2024-12-01T10:00:01.000Z","message":{"role":"user","content":[{"tool_use_id":"toolu_deep","type":"tool_result","content":[{"type":"text","text":"agentId: deepagent1 (use SendMessage)"}]}]}}"#;
        std::fs::write(
            project_dir.join(format!("{}.jsonl", parent_session_id)),
            parent_content,
        )
        .unwrap();

        let workflow_dir = project_dir
            .join(parent_session_id)
            .join("subagents")
            .join("workflows")
            .join("wf_deep");
        std::fs::create_dir_all(&workflow_dir).unwrap();
        let sidechain_content = r#"{"type":"user","isSidechain":true,"sessionId":"deep-parent-uuid","agentId":"deepagent1","timestamp":"2024-12-01T10:00:00.500Z","message":{"content":"task"}}
{"type":"assistant","isSidechain":true,"sessionId":"deep-parent-uuid","agentId":"deepagent1","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_deep","message":{"id":"msg_deep","model":"claude-3-5-sonnet","usage":{"input_tokens":300,"output_tokens":120}}}"#;
        let sidechain_path = workflow_dir.join("agent-deepagent1.jsonl");
        std::fs::write(&sidechain_path, sidechain_content).unwrap();

        let messages = parse_claude_file(&sidechain_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].agent,
            Some("Code Reviewer".to_string()),
            "Tier 2 must resolve the agent name across the deep workflows/<wf>/ nesting"
        );
        assert_eq!(messages[0].session_id, parent_session_id);
        assert_eq!(messages[0].tokens.input, 300);
    }

    #[test]
    fn test_sidechain_flat_legacy_layout() {
        // Flat layout: agent file lives directly under the project dir, no meta sidecar
        let jsonl = r#"{"type":"user","isSidechain":true,"sessionId":"legacy-session-001","agentId":"ac0c74c","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"Warmup"}}
{"type":"assistant","isSidechain":true,"sessionId":"legacy-session-001","agentId":"ac0c74c","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_l01","message":{"id":"msg_l01","model":"claude-3-5-sonnet","usage":{"input_tokens":150,"output_tokens":60}}}"#;

        let (_dir, path) = create_project_file(jsonl, "myproject", "agent-ac0c74c.jsonl");
        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].agent,
            Some("Claude Code Subagent".to_string()),
            "Legacy flat layout has no meta → Tier 3 fallback"
        );
        assert_eq!(
            messages[0].session_id, "legacy-session-001",
            "Should use parent session ID from transcript body"
        );
    }

    #[test]
    fn test_sidechain_session_id_correction() {
        // Multiple sidechain files from the same parent should share the parent's session_id
        let make_jsonl = |agent_id: &str, req: &str, msg: &str| {
            format!(
                r#"{{"type":"user","isSidechain":true,"sessionId":"shared-parent-uuid","agentId":"{agent_id}","timestamp":"2024-12-01T10:00:00.000Z","message":{{"content":"task"}}}}
{{"type":"assistant","isSidechain":true,"sessionId":"shared-parent-uuid","agentId":"{agent_id}","timestamp":"2024-12-01T10:00:01.000Z","requestId":"{req}","message":{{"id":"{msg}","model":"claude-3-5-sonnet","usage":{{"input_tokens":100,"output_tokens":50}}}}}}"#
            )
        };

        let (_dir1, path1) = create_sidechain_files(
            "myproject",
            "shared-parent-uuid",
            "agent-aaa",
            &make_jsonl("aaa", "req_a", "msg_a"),
            Some(r#"{"agentType":"explore"}"#),
        );
        let (_dir2, path2) = create_sidechain_files(
            "myproject",
            "shared-parent-uuid",
            "agent-bbb",
            &make_jsonl("bbb", "req_b", "msg_b"),
            Some(r#"{"agentType":"executor"}"#),
        );
        let (_dir3, path3) = create_sidechain_files(
            "myproject",
            "shared-parent-uuid",
            "agent-ccc",
            &make_jsonl("ccc", "req_c", "msg_c"),
            None,
        );

        let msgs1 = parse_claude_file(&path1);
        let msgs2 = parse_claude_file(&path2);
        let msgs3 = parse_claude_file(&path3);

        // All three should share the parent session ID
        assert_eq!(msgs1[0].session_id, "shared-parent-uuid");
        assert_eq!(msgs2[0].session_id, "shared-parent-uuid");
        assert_eq!(msgs3[0].session_id, "shared-parent-uuid");

        // Agent names should differ
        assert_eq!(msgs1[0].agent, Some("Explore".to_string()));
        assert_eq!(msgs2[0].agent, Some("Executor".to_string()));
        assert_eq!(msgs3[0].agent, Some("Claude Code Subagent".to_string()));
    }

    #[test]
    fn test_sidechain_token_totals_preserved() {
        // Verify that sidechain parsing doesn't change token accounting
        let sidechain_jsonl = r#"{"type":"user","isSidechain":true,"sessionId":"parent-001","agentId":"xyz","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"task"}}
{"type":"assistant","isSidechain":true,"sessionId":"parent-001","agentId":"xyz","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_t1","message":{"id":"msg_t1","model":"claude-3-5-sonnet","usage":{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":200,"cache_creation_input_tokens":100}}}
{"type":"assistant","isSidechain":true,"sessionId":"parent-001","agentId":"xyz","timestamp":"2024-12-01T10:00:02.000Z","requestId":"req_t2","message":{"id":"msg_t2","model":"claude-3-5-sonnet","usage":{"input_tokens":800,"output_tokens":300,"cache_read_input_tokens":150,"cache_creation_input_tokens":50}}}"#;

        let (_dir, path) = create_sidechain_files(
            "myproject",
            "parent-001",
            "agent-xyz",
            sidechain_jsonl,
            Some(r#"{"agentType":"code-reviewer"}"#),
        );
        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 2);

        let total_input: i64 = messages.iter().map(|m| m.tokens.input).sum();
        let total_output: i64 = messages.iter().map(|m| m.tokens.output).sum();
        let total_cache_read: i64 = messages.iter().map(|m| m.tokens.cache_read).sum();
        let total_cache_write: i64 = messages.iter().map(|m| m.tokens.cache_write).sum();

        assert_eq!(total_input, 1800, "input: 1000 + 800");
        assert_eq!(total_output, 800, "output: 500 + 300");
        assert_eq!(total_cache_read, 350, "cache_read: 200 + 150");
        assert_eq!(total_cache_write, 150, "cache_write: 100 + 50");

        // Both messages should have the same agent
        assert_eq!(messages[0].agent, Some("Code Reviewer".to_string()));
        assert_eq!(messages[1].agent, Some("Code Reviewer".to_string()));
    }

    #[test]
    fn test_main_session_no_agent_regression() {
        // Non-sidechain (main session) files must produce agent: None
        let content = r#"{"type":"user","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"Hello"}}
{"type":"assistant","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_m01","message":{"id":"msg_m01","model":"claude-3-5-sonnet","usage":{"input_tokens":500,"output_tokens":200}}}
{"type":"assistant","timestamp":"2024-12-01T10:00:02.000Z","requestId":"req_m02","message":{"id":"msg_m02","model":"claude-3-5-sonnet","usage":{"input_tokens":600,"output_tokens":250}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].agent, None,
            "Main session messages must not have an agent"
        );
        assert_eq!(messages[1].agent, None);
    }

    #[test]
    fn test_main_session_with_is_sidechain_false() {
        // Explicit isSidechain: false should be treated as main session
        let content = r#"{"type":"assistant","isSidechain":false,"timestamp":"2024-12-01T10:00:00.000Z","requestId":"req_001","message":{"id":"msg_001","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;

        let file = create_test_file(content);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].agent, None,
            "isSidechain=false should not set agent"
        );
    }

    #[test]
    fn test_sidechain_dedup_preserves_agent() {
        // Streaming duplicates within a sidechain file should still carry the agent
        let jsonl = r#"{"type":"user","isSidechain":true,"sessionId":"parent-dedup","agentId":"dd1","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"task"}}
{"type":"assistant","isSidechain":true,"sessionId":"parent-dedup","agentId":"dd1","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_d1","message":{"id":"msg_d1","model":"claude-3-5-sonnet","usage":{"input_tokens":10,"output_tokens":30}}}
{"type":"assistant","isSidechain":true,"sessionId":"parent-dedup","agentId":"dd1","timestamp":"2024-12-01T10:00:01.100Z","requestId":"req_d1","message":{"id":"msg_d1","model":"claude-3-5-sonnet","usage":{"input_tokens":10,"output_tokens":300}}}"#;

        let (_dir, path) = create_sidechain_files(
            "myproject",
            "parent-dedup",
            "agent-dd1",
            jsonl,
            Some(r#"{"agentType":"architect"}"#),
        );
        let messages = parse_claude_file(&path);

        assert_eq!(
            messages.len(),
            1,
            "Streaming duplicates should collapse to one"
        );
        assert_eq!(
            messages[0].tokens.output, 300,
            "Should keep max output_tokens"
        );
        assert_eq!(
            messages[0].agent,
            Some("Architect".to_string()),
            "Deduped message should retain agent"
        );
        assert_eq!(messages[0].session_id, "parent-dedup");
    }

    #[test]
    fn test_sidechain_meta_with_omc_prefix_agent() {
        // Meta file might contain oh-my-claudecode: prefixed agent types
        let jsonl = r#"{"type":"user","isSidechain":true,"sessionId":"parent-omc","agentId":"omc1","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"task"}}
{"type":"assistant","isSidechain":true,"sessionId":"parent-omc","agentId":"omc1","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_omc","message":{"id":"msg_omc","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;

        let (_dir, path) = create_sidechain_files(
            "myproject",
            "parent-omc",
            "agent-omc1",
            jsonl,
            Some(r#"{"agentType":"oh-my-claudecode:code-reviewer"}"#),
        );
        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].agent,
            Some("Code Reviewer".to_string()),
            "Should strip oh-my-claudecode: prefix and normalize"
        );
    }

    #[test]
    fn test_sidechain_without_session_id_uses_filename() {
        // Edge case: sidechain entry without sessionId should fall back to filename stem
        let jsonl = r#"{"type":"user","isSidechain":true,"agentId":"noid","timestamp":"2024-12-01T10:00:00.000Z","message":{"content":"task"}}
{"type":"assistant","isSidechain":true,"agentId":"noid","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_no","message":{"id":"msg_no","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;

        let file = create_test_file(jsonl);
        let messages = parse_claude_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].agent,
            Some("Claude Code Subagent".to_string()),
            "Still detected as sidechain"
        );
        // session_id should be the file stem (fallback)
        let expected_stem = file
            .path()
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(messages[0].session_id, expected_stem);
    }

    // --- Tier 2: parent session tool_use inference tests ---

    #[test]
    fn test_tier2_recovers_agent_from_parent_tool_use() {
        // Nested layout: sidechain without meta, but parent session has matching tool_use
        let temp_dir = tempfile::tempdir().unwrap();
        let project_dir = temp_dir
            .path()
            .join(".claude")
            .join("projects")
            .join("myproject");
        std::fs::create_dir_all(&project_dir).unwrap();

        // Create parent session file with tool_use (Agent) and tool_result (agentId)
        let parent_session_id = "parent-tier2-uuid";
        let parent_content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","message":{"id":"msg_p1","model":"claude-3-5-sonnet","role":"assistant","content":[{"type":"tool_use","id":"toolu_abc","name":"Agent","input":{"subagent_type":"document-specialist","prompt":"Research something"}}],"usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"user","timestamp":"2024-12-01T10:00:01.000Z","message":{"role":"user","content":[{"tool_use_id":"toolu_abc","type":"tool_result","content":[{"type":"text","text":"Found the docs"},{"type":"text","text":"agentId: t2agent1 (use SendMessage with to: 't2agent1' to continue this agent)\n<usage>total_tokens: 5000</usage>"}]}]}}"#;
        let parent_path = project_dir.join(format!("{}.jsonl", parent_session_id));
        std::fs::write(&parent_path, parent_content).unwrap();

        // Create sidechain file (nested layout, no meta sidecar)
        let subagents_dir = project_dir.join(parent_session_id).join("subagents");
        std::fs::create_dir_all(&subagents_dir).unwrap();
        let sidechain_content = r#"{"type":"user","isSidechain":true,"sessionId":"parent-tier2-uuid","agentId":"t2agent1","timestamp":"2024-12-01T10:00:00.500Z","message":{"content":"Research something"}}
{"type":"assistant","isSidechain":true,"sessionId":"parent-tier2-uuid","agentId":"t2agent1","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_t2","message":{"id":"msg_t2","model":"claude-3-5-sonnet","usage":{"input_tokens":300,"output_tokens":120}}}"#;
        let sidechain_path = subagents_dir.join("agent-t2agent1.jsonl");
        std::fs::write(&sidechain_path, sidechain_content).unwrap();

        let messages = parse_claude_file(&sidechain_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].agent,
            Some("Document Specialist".to_string()),
            "Tier 2 should recover agent name from parent tool_use"
        );
        assert_eq!(messages[0].session_id, parent_session_id);
    }

    #[test]
    fn test_tier2_flat_layout_recovers_agent() {
        // Flat layout: sidechain file in same dir as parent, no meta sidecar
        let temp_dir = tempfile::tempdir().unwrap();
        let project_dir = temp_dir
            .path()
            .join(".claude")
            .join("projects")
            .join("myproject");
        std::fs::create_dir_all(&project_dir).unwrap();

        let parent_session_id = "flat-parent-uuid";
        let parent_content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","message":{"id":"msg_fp","model":"claude-3-5-sonnet","role":"assistant","content":[{"type":"tool_use","id":"toolu_flat","name":"Agent","input":{"subagent_type":"explore","prompt":"Find files"}}],"usage":{"input_tokens":50,"output_tokens":30}}}
{"type":"user","timestamp":"2024-12-01T10:00:01.000Z","message":{"role":"user","content":[{"tool_use_id":"toolu_flat","type":"tool_result","content":[{"type":"text","text":"agentId: flatagent1 (use SendMessage)"}]}]}}"#;
        std::fs::write(
            project_dir.join(format!("{}.jsonl", parent_session_id)),
            parent_content,
        )
        .unwrap();

        let sidechain_content = r#"{"type":"user","isSidechain":true,"sessionId":"flat-parent-uuid","agentId":"flatagent1","timestamp":"2024-12-01T10:00:00.500Z","message":{"content":"task"}}
{"type":"assistant","isSidechain":true,"sessionId":"flat-parent-uuid","agentId":"flatagent1","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_flat","message":{"id":"msg_flat","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        std::fs::write(
            project_dir.join("agent-flatagent1.jsonl"),
            sidechain_content,
        )
        .unwrap();

        let messages = parse_claude_file(&project_dir.join("agent-flatagent1.jsonl"));

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].agent,
            Some("Explore".to_string()),
            "Tier 2 should work for flat layout too"
        );
    }

    #[test]
    fn test_tier1_takes_precedence_over_tier2() {
        // When meta sidecar exists, Tier 1 wins even if parent has a different subagent_type
        let temp_dir = tempfile::tempdir().unwrap();
        let project_dir = temp_dir
            .path()
            .join(".claude")
            .join("projects")
            .join("myproject");
        std::fs::create_dir_all(&project_dir).unwrap();

        let parent_session_id = "precedence-parent";
        let parent_content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","message":{"id":"msg_prec","model":"claude-3-5-sonnet","role":"assistant","content":[{"type":"tool_use","id":"toolu_prec","name":"Agent","input":{"subagent_type":"wrong-type","prompt":"task"}}],"usage":{"input_tokens":50,"output_tokens":30}}}
{"type":"user","timestamp":"2024-12-01T10:00:01.000Z","message":{"role":"user","content":[{"tool_use_id":"toolu_prec","type":"tool_result","content":[{"type":"text","text":"agentId: precagent1 done"}]}]}}"#;
        std::fs::write(
            project_dir.join(format!("{}.jsonl", parent_session_id)),
            parent_content,
        )
        .unwrap();

        let subagents_dir = project_dir.join(parent_session_id).join("subagents");
        std::fs::create_dir_all(&subagents_dir).unwrap();

        let sidechain_content = r#"{"type":"user","isSidechain":true,"sessionId":"precedence-parent","agentId":"precagent1","timestamp":"2024-12-01T10:00:00.500Z","message":{"content":"task"}}
{"type":"assistant","isSidechain":true,"sessionId":"precedence-parent","agentId":"precagent1","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_prec","message":{"id":"msg_prec2","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        std::fs::write(
            subagents_dir.join("agent-precagent1.jsonl"),
            sidechain_content,
        )
        .unwrap();
        std::fs::write(
            subagents_dir.join("agent-precagent1.meta.json"),
            r#"{"agentType":"code-reviewer"}"#,
        )
        .unwrap();

        let messages = parse_claude_file(&subagents_dir.join("agent-precagent1.jsonl"));

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].agent,
            Some("Code Reviewer".to_string()),
            "Tier 1 (meta sidecar) should take precedence over Tier 2 (parent lookup)"
        );
    }

    #[test]
    fn test_extract_agent_id_from_text() {
        assert_eq!(
            extract_agent_id_from_text(
                "agentId: a8f80f8f33163def2 (use SendMessage with to: 'a8f80f8f33163def2')"
            ),
            Some("a8f80f8f33163def2".to_string())
        );
        assert_eq!(
            extract_agent_id_from_text("agentId: abc123\n<usage>total_tokens: 5000</usage>"),
            Some("abc123".to_string())
        );
        assert_eq!(extract_agent_id_from_text("no agent id here"), None);
        assert_eq!(
            extract_agent_id_from_text("agentId: "),
            None,
            "Empty agent id should return None"
        );
    }

    #[test]
    fn test_sidechain_agent_id_from_stem_extracts_aside_question_suffix() {
        assert_eq!(
            sidechain_agent_id_from_stem("agent-aside_question-0320a3d71bc1d01e"),
            Some("0320a3d71bc1d01e".to_string())
        );
        assert_eq!(
            sidechain_agent_id_from_stem("agent-flatagent1"),
            Some("flatagent1".to_string())
        );
    }

    #[test]
    fn test_tier2_uses_entry_agent_id_when_filename_prefix_differs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let project_dir = temp_dir
            .path()
            .join(".claude")
            .join("projects")
            .join("myproject");
        std::fs::create_dir_all(&project_dir).unwrap();

        let parent_session_id = "aside-parent-uuid";
        let parent_content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","message":{"id":"msg_aside_parent","model":"claude-3-5-sonnet","role":"assistant","content":[{"type":"tool_use","id":"toolu_aside","name":"Agent","input":{"subagent_type":"writer","prompt":"Summarize findings"}}],"usage":{"input_tokens":50,"output_tokens":30}}}
{"type":"user","timestamp":"2024-12-01T10:00:01.000Z","message":{"role":"user","content":[{"tool_use_id":"toolu_aside","type":"tool_result","content":[{"type":"text","text":"agentId: 0320a3d71bc1d01e (use SendMessage)"}]}]}}"#;
        std::fs::write(
            project_dir.join(format!("{}.jsonl", parent_session_id)),
            parent_content,
        )
        .unwrap();

        let subagents_dir = project_dir.join(parent_session_id).join("subagents");
        std::fs::create_dir_all(&subagents_dir).unwrap();
        let sidechain_content = r#"{"type":"user","isSidechain":true,"sessionId":"aside-parent-uuid","agentId":"0320a3d71bc1d01e","timestamp":"2024-12-01T10:00:00.500Z","message":{"content":"task"}}
{"type":"assistant","isSidechain":true,"sessionId":"aside-parent-uuid","agentId":"0320a3d71bc1d01e","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_aside","message":{"id":"msg_aside","model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        let sidechain_path = subagents_dir.join("agent-aside_question-0320a3d71bc1d01e.jsonl");
        std::fs::write(&sidechain_path, sidechain_content).unwrap();

        let messages = parse_claude_file(&sidechain_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent, Some("Writer".to_string()));
    }

    #[test]
    fn test_parent_subagent_lookup_cache_reuses_parsed_parent_results() {
        let temp_dir = tempfile::tempdir().unwrap();
        let parent_path = temp_dir.path().join("parent.jsonl");
        let initial_parent = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_a","name":"Agent","input":{"subagent_type":"explore"}},{"type":"tool_use","id":"toolu_b","name":"Agent","input":{"subagent_type":"executor"}}]}}
{"type":"user","message":{"content":[{"tool_use_id":"toolu_a","type":"tool_result","content":[{"type":"text","text":"agentId: cacheA"}]},{"tool_use_id":"toolu_b","type":"tool_result","content":[{"type":"text","text":"agentId: cacheB"}]}]}}"#;
        std::fs::write(&parent_path, initial_parent).unwrap();

        let mut parent_cache = ParentSubagentTypeCache::new();
        assert_eq!(
            lookup_subagent_type_in_parent(&parent_path, "cacheA", &mut parent_cache),
            Some("explore".to_string())
        );

        std::fs::write(
            &parent_path,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_b","name":"Agent","input":{"subagent_type":"writer"}}]}}"#,
        )
        .unwrap();

        assert_eq!(
            lookup_subagent_type_in_parent(&parent_path, "cacheB", &mut parent_cache),
            Some("executor".to_string())
        );
    }

    #[test]
    fn test_tier2_multiple_agents_in_same_parent() {
        // Parent spawns multiple agents; each sidechain should get the correct type
        let temp_dir = tempfile::tempdir().unwrap();
        let project_dir = temp_dir
            .path()
            .join(".claude")
            .join("projects")
            .join("myproject");
        std::fs::create_dir_all(&project_dir).unwrap();

        let parent_session_id = "multi-agent-parent";
        let parent_content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","message":{"id":"msg_ma1","model":"claude-3-5-sonnet","role":"assistant","content":[{"type":"tool_use","id":"toolu_m1","name":"Agent","input":{"subagent_type":"explore","prompt":"find files"}}],"usage":{"input_tokens":50,"output_tokens":30}}}
{"type":"user","timestamp":"2024-12-01T10:00:01.000Z","message":{"role":"user","content":[{"tool_use_id":"toolu_m1","type":"tool_result","content":[{"type":"text","text":"agentId: multiA1 done"}]}]}}
{"type":"assistant","timestamp":"2024-12-01T10:00:02.000Z","message":{"id":"msg_ma2","model":"claude-3-5-sonnet","role":"assistant","content":[{"type":"tool_use","id":"toolu_m2","name":"Agent","input":{"subagent_type":"executor","prompt":"implement feature"}}],"usage":{"input_tokens":60,"output_tokens":40}}}
{"type":"user","timestamp":"2024-12-01T10:00:03.000Z","message":{"role":"user","content":[{"tool_use_id":"toolu_m2","type":"tool_result","content":[{"type":"text","text":"agentId: multiB2 done"}]}]}}"#;
        std::fs::write(
            project_dir.join(format!("{}.jsonl", parent_session_id)),
            parent_content,
        )
        .unwrap();

        let subagents_dir = project_dir.join(parent_session_id).join("subagents");
        std::fs::create_dir_all(&subagents_dir).unwrap();

        let make_sidechain = |agent_id: &str| {
            format!(
                r#"{{"type":"user","isSidechain":true,"sessionId":"{parent_session_id}","agentId":"{agent_id}","timestamp":"2024-12-01T10:00:00.500Z","message":{{"content":"task"}}}}
{{"type":"assistant","isSidechain":true,"sessionId":"{parent_session_id}","agentId":"{agent_id}","timestamp":"2024-12-01T10:00:01.000Z","requestId":"req_{agent_id}","message":{{"id":"msg_{agent_id}","model":"claude-3-5-sonnet","usage":{{"input_tokens":100,"output_tokens":50}}}}}}"#
            )
        };

        std::fs::write(
            subagents_dir.join("agent-multiA1.jsonl"),
            make_sidechain("multiA1"),
        )
        .unwrap();
        std::fs::write(
            subagents_dir.join("agent-multiB2.jsonl"),
            make_sidechain("multiB2"),
        )
        .unwrap();

        let msgs_a = parse_claude_file(&subagents_dir.join("agent-multiA1.jsonl"));
        let msgs_b = parse_claude_file(&subagents_dir.join("agent-multiB2.jsonl"));

        assert_eq!(
            msgs_a[0].agent,
            Some("Explore".to_string()),
            "First agent should be explore"
        );
        assert_eq!(
            msgs_b[0].agent,
            Some("Executor".to_string()),
            "Second agent should be executor"
        );
    }
}
