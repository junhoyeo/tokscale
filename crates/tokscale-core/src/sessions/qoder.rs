//! Qoder & Qoder CN (Alibaba) session parser
//!
//! Parses JSONL session transcripts from:
//! - `~/.qoder/projects/<project_slug>/<sessionId>.jsonl` (or `$QODER_CONFIG_DIR/projects/...`)
//! - `~/.qoder-cn/projects/<project_slug>/<sessionId>.jsonl` (or `$QODERCN_CONFIG_DIR/projects/...`)
//! - Nested subagent transcripts under `<sessionId>/subagents/agent-<agentId>.jsonl`
//! - Structured model completion logs (`type: "model.response.completed"`) when configured
//!
//! Qoder and Qoder CN share the same agent transcript engine:
//! - When using custom/BYOK models (`modelSource === "custom"`) or when
//!   `QODER_EXPOSE_TOKEN_USAGE=1` (`QODERCN_EXPOSE_TOKEN_USAGE=1`) is enabled,
//!   `message.usage` contains authoritative token counts (`input_tokens`,
//!   `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`,
//!   and `cache_creation.ephemeral_1h_input_tokens`).
//! - When using Qoder's managed models without token exposure enabled, Qoder
//!   scrubs `message.usage` (either omitting `usage` or zeroing out the token
//!   fields while retaining `credits` / `context_usage_ratio`). In that case,
//!   this parser falls back to content-based token estimation (~4 chars/token)
//!   across the assembled assistant turn and accumulated conversation context.

use super::utils::{
    estimate_tokens, extract_cache_write_1h, extract_i64, file_modified_timestamp_ms,
    for_each_json_line, parse_timestamp_value, session_id_from_path, workspace_key_from_path,
};
use super::{
    normalize_agent_name, normalize_workspace_key, workspace_label_from_key, SharedStr,
    UnifiedMessage,
};
use crate::{pricing, provider_identity, CostSource, TokenBreakdown};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

pub const QODER_PARSER_BASE_VERSION: u32 = 1;

const CLIENT_ID: &str = "qoder";
const DEFAULT_PROVIDER: &str = "qoder";
const DEFAULT_MODEL: &str = "auto";

#[derive(Debug, Deserialize)]
struct AgentMetaFile {
    #[serde(rename = "agentType", alias = "agent_type")]
    agent_type: Option<String>,
}

/// True for non-transcript files under the project tree that share the `.jsonl`
/// extension (`subagents/**/journal.jsonl` and `*.checkpoints.jsonl`).
fn should_skip_qoder_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return true;
    };
    if file_name == "journal.jsonl"
        && path
            .ancestors()
            .any(|a| a.file_name().and_then(|n| n.to_str()) == Some("subagents"))
    {
        return true;
    }
    file_name.ends_with(".checkpoints.jsonl")
}

fn is_synthetic_placeholder_model(model: &str) -> bool {
    let trimmed = model.trim();
    trimmed.is_empty() || trimmed.eq_ignore_ascii_case("<synthetic>")
}

fn canonicalize_qoder_model_and_provider(
    raw_model: &str,
    explicit_provider: Option<&str>,
) -> (String, String) {
    let trimmed_model = raw_model.trim();
    // BYOK models in Qoder can be stored as `<provider>/<type>/<modelId>` or
    // `<provider>/<modelId>`. Extract the provider prefix before stripping it.
    let prefix_provider = if trimmed_model.contains('/') {
        let first_segment = trimmed_model.split('/').next().unwrap_or("");
        provider_identity::canonical_provider(first_segment)
    } else {
        None
    };

    let bare_model = if prefix_provider.is_some() {
        trimmed_model
            .rsplit('/')
            .next()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(trimmed_model)
    } else {
        trimmed_model
    };

    let canonical_model = pricing::aliases::resolve_alias(bare_model)
        .unwrap_or(bare_model)
        .to_string();

    let explicit_canonical = explicit_provider
        .map(str::trim)
        .filter(|p| {
            !p.is_empty() && !p.eq_ignore_ascii_case("custom") && !p.eq_ignore_ascii_case("qoder")
        })
        .and_then(|p| provider_identity::canonical_provider(p).or_else(|| Some(p.to_lowercase())));

    let provider_id = explicit_canonical
        .or(prefix_provider)
        .or_else(|| {
            provider_identity::inferred_provider_from_model_delimited(&canonical_model)
                .map(str::to_string)
        })
        .or_else(|| {
            provider_identity::inferred_provider_from_model_delimited(trimmed_model)
                .map(str::to_string)
        })
        .unwrap_or_else(|| DEFAULT_PROVIDER.to_string());

    (canonical_model, provider_id)
}

fn extract_usage_breakdown(usage: &Value) -> Option<(TokenBreakdown, bool)> {
    let obj = usage.as_object()?;

    // Check Gemini/Qwen-style usageMetadata fields first if present
    if obj.contains_key("promptTokenCount") || obj.contains_key("candidatesTokenCount") {
        let prompt = extract_i64(obj.get("promptTokenCount")).unwrap_or(0).max(0);
        let candidates = extract_i64(obj.get("candidatesTokenCount"))
            .unwrap_or(0)
            .max(0);
        let thoughts = extract_i64(obj.get("thoughtsTokenCount"))
            .unwrap_or(0)
            .max(0);
        let cache_read = extract_i64(obj.get("cachedContentTokenCount"))
            .unwrap_or(0)
            .max(0);
        let total = extract_i64(obj.get("totalTokenCount"));

        if prompt + candidates + thoughts + cache_read == 0 {
            return None;
        }

        let input = if cache_read <= prompt {
            prompt - cache_read
        } else {
            prompt
        };
        let thoughts_in_candidates = thoughts > 0
            && match total {
                Some(0) => candidates > 0,
                Some(t) => t > 0 && t == prompt.saturating_add(candidates),
                None => false,
            };
        let (output, reasoning) = if thoughts_in_candidates {
            let r = thoughts.min(candidates);
            (candidates - r, r)
        } else {
            (candidates, thoughts)
        };

        return Some((
            TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write: 0,
                cache_write_1h: 0,
                reasoning,
            },
            cache_read > 0,
        ));
    }

    let input = extract_i64(
        obj.get("input_tokens")
            .or_else(|| obj.get("inputTokens"))
            .or_else(|| obj.get("prompt_tokens"))
            .or_else(|| obj.get("promptTokens")),
    )
    .unwrap_or(0)
    .max(0);

    let output = extract_i64(
        obj.get("output_tokens")
            .or_else(|| obj.get("outputTokens"))
            .or_else(|| obj.get("completion_tokens"))
            .or_else(|| obj.get("completionTokens")),
    )
    .unwrap_or(0)
    .max(0);

    let cache_read = extract_i64(
        obj.get("cache_read_input_tokens")
            .or_else(|| obj.get("cacheReadInputTokens"))
            .or_else(|| obj.get("cache_read_tokens"))
            .or_else(|| obj.get("cacheReadTokens")),
    )
    .unwrap_or(0)
    .max(0);

    let cache_write = extract_i64(
        obj.get("cache_creation_input_tokens")
            .or_else(|| obj.get("cacheCreationInputTokens"))
            .or_else(|| obj.get("cache_write_tokens"))
            .or_else(|| obj.get("cacheWriteTokens")),
    )
    .unwrap_or(0)
    .max(0);

    let cache_write_1h = extract_cache_write_1h(usage).max(0);

    let reasoning = extract_i64(
        obj.get("thinking_tokens")
            .or_else(|| obj.get("thinkingTokens"))
            .or_else(|| obj.get("reasoning_tokens"))
            .or_else(|| obj.get("reasoningTokens")),
    )
    .unwrap_or(0)
    .max(0);

    if input + output + cache_read + cache_write + cache_write_1h + reasoning == 0 {
        return None;
    }

    let reports_cache_split = obj.contains_key("cache_read_input_tokens")
        || obj.contains_key("cacheReadInputTokens")
        || obj.contains_key("cache_creation_input_tokens")
        || obj.contains_key("cacheCreationInputTokens")
        || cache_read > 0
        || cache_write > 0;

    Some((
        TokenBreakdown {
            input,
            output,
            cache_read,
            cache_write: cache_write.max(cache_write_1h),
            cache_write_1h,
            reasoning,
        },
        reports_cache_split,
    ))
}

/// Count characters in a message content payload (`text`, `tool_use`, `tool_result`,
/// and `thinking` blocks), returning `(regular_chars, reasoning_chars)`.
fn count_content_chars(content: Option<&Value>) -> (usize, usize) {
    let Some(content) = content else {
        return (0, 0);
    };
    if let Some(s) = content.as_str() {
        return (s.chars().count(), 0);
    }
    let Some(blocks) = content.as_array() else {
        return (0, 0);
    };

    let mut regular = 0usize;
    let mut reasoning = 0usize;
    for block in blocks {
        let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
        match block_type {
            "thinking" => {
                if let Some(thinking) = block
                    .get("thinking")
                    .or_else(|| block.get("text"))
                    .and_then(Value::as_str)
                {
                    reasoning += thinking.chars().count();
                }
            }
            "tool_use" => {
                if let Some(name) = block.get("name").and_then(Value::as_str) {
                    regular += name.chars().count();
                }
                if let Some(input) = block.get("input") {
                    if let Some(s) = input.as_str() {
                        regular += s.chars().count();
                    } else if let Ok(serialized) = serde_json::to_string(input) {
                        regular += serialized.chars().count();
                    }
                }
            }
            "tool_result" => {
                let (sub_reg, sub_reas) = count_content_chars(block.get("content"));
                regular += sub_reg;
                reasoning += sub_reas;
            }
            _ => {
                if let Some(text) = block
                    .get("text")
                    .or_else(|| block.get("content"))
                    .and_then(Value::as_str)
                {
                    regular += text.chars().count();
                }
            }
        }
    }
    (regular, reasoning)
}

fn resolve_subagent_name(
    path: &Path,
    is_sidechain: bool,
    agent_id: Option<&str>,
) -> Option<String> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let in_subagents_dir = path
        .ancestors()
        .any(|a| a.file_name().and_then(|n| n.to_str()) == Some("subagents"));
    let is_agent_file = stem.starts_with("agent-") || in_subagents_dir;

    if !is_sidechain && !is_agent_file && agent_id.is_none() {
        return None;
    }

    if !stem.is_empty() {
        let meta_path = path.with_file_name(format!("{stem}.meta.json"));
        if let Ok(text) = std::fs::read_to_string(&meta_path) {
            if let Ok(meta) = serde_json::from_str::<AgentMetaFile>(&text) {
                if let Some(agent_type) = meta.agent_type.as_deref() {
                    if !agent_type.trim().is_empty() {
                        return Some(normalize_agent_name(agent_type));
                    }
                }
            }
        }
    }

    if let Some(id) = agent_id.map(str::trim).filter(|id| !id.is_empty()) {
        return Some(normalize_agent_name(id));
    }

    if let Some(stripped) = stem.strip_prefix("agent-") {
        if !stripped.trim().is_empty() {
            return Some(normalize_agent_name(stripped));
        }
    }

    Some(normalize_agent_name("qoder-subagent"))
}

struct AssembledAssistantTurn {
    raw_model: String,
    canonical_model: String,
    provider_id: String,
    session_id: String,
    parent_session_id: Option<String>,
    timestamp: i64,
    duration_ms: Option<i64>,
    is_turn_start: bool,
    dedup_key: String,
    workspace_key: Option<String>,
    workspace_label: Option<String>,
    agent: Option<String>,
    authoritative_tokens: Option<TokenBreakdown>,
    reports_cache_split: bool,
    input_context_chars: usize,
    output_chars: usize,
    reasoning_chars: usize,
    prompt_token_hint: Option<i64>,
}

impl AssembledAssistantTurn {
    fn merge_authoritative(&mut self, incoming: TokenBreakdown, incoming_split: bool) {
        if let Some(existing) = &mut self.authoritative_tokens {
            match (incoming_split, self.reports_cache_split) {
                (true, false) => {
                    existing.input = incoming.input;
                }
                (false, true) => {}
                (true, true) | (false, false) => {
                    existing.input = existing.input.max(incoming.input);
                }
            }
            existing.output = existing.output.max(incoming.output);
            existing.cache_read = existing.cache_read.max(incoming.cache_read);
            existing.cache_write = existing.cache_write.max(incoming.cache_write);
            existing.cache_write_1h = existing.cache_write_1h.max(incoming.cache_write_1h);
            existing.reasoning = existing.reasoning.max(incoming.reasoning);
            self.reports_cache_split |= incoming_split;
        } else {
            self.authoritative_tokens = Some(incoming);
            self.reports_cache_split = incoming_split;
        }
    }

    fn into_unified_message(self) -> Option<UnifiedMessage> {
        let mut tokens = if let Some(mut auth) = self.authoritative_tokens {
            auth.cache_write_1h = auth.cache_write_1h.min(auth.cache_write);
            auth
        } else {
            let estimated_input = self
                .prompt_token_hint
                .filter(|&n| n > 0)
                .unwrap_or_else(|| estimate_tokens(self.input_context_chars));
            let estimated_output = estimate_tokens(self.output_chars);
            let estimated_reasoning = estimate_tokens(self.reasoning_chars);
            TokenBreakdown {
                input: estimated_input,
                output: estimated_output,
                cache_read: 0,
                cache_write: 0,
                cache_write_1h: 0,
                reasoning: estimated_reasoning,
            }
        };

        if tokens.total() == 0 {
            return None;
        }
        tokens.cache_write_1h = tokens.cache_write_1h.min(tokens.cache_write);

        let mut msg = UnifiedMessage::new_with_dedup(
            CLIENT_ID,
            self.canonical_model,
            self.provider_id,
            self.session_id,
            self.timestamp,
            tokens,
            0.0,
            Some(self.dedup_key),
        );
        msg.preserve_pricing_model(&self.raw_model);
        msg.cost_source = CostSource::Estimated;
        msg.is_turn_start = self.is_turn_start;
        msg.duration_ms = self.duration_ms;
        msg.agent = self.agent.map(SharedStr::from);
        msg.parent_session_id = self.parent_session_id.map(SharedStr::from);
        msg.set_workspace(self.workspace_key, self.workspace_label);
        Some(msg)
    }
}

fn workspace_key_from_qoder_path(path: &Path) -> Option<String> {
    let parent = path.parent()?;

    // 1. Extract project directory when path is under `projects/<project>/...`
    let components: Vec<&str> = parent
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    for window in components.windows(2).rev() {
        if window[0] == "projects" && !window[1].is_empty() && window[1] != "subagents" {
            if let Some(key) = normalize_workspace_key(window[1]) {
                return Some(key);
            }
        }
    }

    // 2. If path is a subagent transcript under `<project>/<session>/subagents/`
    // (outside of a `projects/` directory), walk up past `subagents` and the
    // session directory to the project directory.
    let mut current = Some(parent);
    while let Some(dir) = current {
        if dir.file_name().and_then(|n| n.to_str()) == Some("subagents") {
            if let Some(project_dir) = dir.parent().and_then(|p| p.parent()) {
                if let Some(name) = project_dir.file_name().and_then(|n| n.to_str()) {
                    if name != "projects" {
                        if let Some(key) = normalize_workspace_key(name) {
                            return Some(key);
                        }
                    }
                }
            }
            break;
        }
        current = dir.parent();
    }

    // 3. Fallback to standard parent directory workspace key, filtering out "subagents"
    workspace_key_from_path(path).filter(|k| k != "subagents")
}

/// Parse a Qoder or Qoder CN `.jsonl` transcript file into [`UnifiedMessage`] records.
pub fn parse_qoder_file(path: &Path) -> Vec<UnifiedMessage> {
    if should_skip_qoder_file(path) {
        return Vec::new();
    }

    let fallback_timestamp = file_modified_timestamp_ms(path);
    let fallback_session_id = session_id_from_path(path);
    let path_workspace_key = workspace_key_from_qoder_path(path);
    let path_workspace_label = path_workspace_key
        .as_deref()
        .and_then(workspace_label_from_key);

    let mut session_id: Option<String> = None;
    let mut workspace_key = path_workspace_key;
    let mut workspace_label = path_workspace_label;
    let mut session_runtime_model: Option<String> = None;
    let mut last_business_model: Option<String> = None;
    let mut active_model: Option<String> = None;
    let mut active_provider: Option<String> = None;
    let mut context_chars: usize = 0;
    let mut pending_turn_start = false;
    let mut pending_request_start_ms: Option<i64> = None;
    let mut pending_prompt_token_hint: Option<i64> = None;
    let mut sidechain_agent_cache: Option<String> = None;

    let mut turns: Vec<AssembledAssistantTurn> = Vec::new();
    let mut turn_index_by_key: HashMap<String, usize> = HashMap::new();
    let mut anonymous_turn_counter: usize = 0;

    for_each_json_line(path, &mut |_line_idx, trimmed| {
        let mut bytes = trimmed.as_bytes().to_vec();
        let Ok(value) = simd_json::from_slice::<Value>(&mut bytes) else {
            return;
        };

        let entry_type = value
            .get("type")
            .or_else(|| value.get("role"))
            .and_then(Value::as_str)
            .unwrap_or("");

        if session_id.is_none() {
            if let Some(sid) = value
                .get("sessionId")
                .or_else(|| value.get("session_id"))
                .or_else(|| value.get("ids").and_then(|ids| ids.get("session_id")))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                session_id = Some(sid.to_string());
            }
        }

        if let Some(cwd) = value
            .get("cwd")
            .or_else(|| value.get("relocatedCwd"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Some(key) = normalize_workspace_key(cwd) {
                workspace_label = workspace_label_from_key(&key);
                workspace_key = Some(key);
            }
        }

        match entry_type {
            "runtime-config" => {
                if let Some(m) = value
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|m| !is_synthetic_placeholder_model(m))
                {
                    if !m.eq_ignore_ascii_case("auto") {
                        session_runtime_model = Some(m.to_string());
                        active_model = Some(m.to_string());
                        last_business_model = Some(m.to_string());
                    }
                }
                if let Some(p) = value
                    .get("provider")
                    .or_else(|| value.get("providerId"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                {
                    active_provider = Some(p.to_string());
                }
            }
            "token-stats" => {
                if let Some(prompt_tokens) = extract_i64(value.get("promptTokenCount")) {
                    if prompt_tokens > 0 {
                        pending_prompt_token_hint = Some(prompt_tokens);
                    }
                }
            }
            "user" => {
                let message_val = value.get("message");
                if let Some(bmk) = value
                    .get("businessModelKey")
                    .or_else(|| message_val.and_then(|m| m.get("businessModelKey")))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("auto"))
                {
                    last_business_model = Some(bmk.to_string());
                    active_model = Some(bmk.to_string());
                }

                let content_val = message_val
                    .and_then(|m| m.get("content"))
                    .or_else(|| value.get("content"));
                let (reg_chars, reas_chars) = count_content_chars(content_val);
                context_chars = context_chars.saturating_add(reg_chars + reas_chars);

                let is_synthetic = value
                    .get("isSynthetic")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    || value
                        .get("isMeta")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                let is_tool_result = content_val.and_then(Value::as_array).is_some_and(|arr| {
                    arr.iter()
                        .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
                });

                if !is_synthetic && !is_tool_result {
                    pending_turn_start = true;
                }
                if let Some(ts) = value.get("timestamp").and_then(parse_timestamp_value) {
                    pending_request_start_ms = Some(ts);
                }
            }
            "model.response.completed" => {
                // Structured session segment log entry (`logs/sessions/<project>/<session>/segments/<runId>.jsonl`)
                let Some(data) = value.get("data") else {
                    return;
                };
                let Some((breakdown, reports_split)) = extract_usage_breakdown(data) else {
                    return;
                };
                let raw_model_candidate = data
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|m| !m.is_empty());

                let raw_model = match raw_model_candidate {
                    Some(m) if !m.eq_ignore_ascii_case("auto") => m,
                    _ => last_business_model
                        .as_deref()
                        .or(session_runtime_model.as_deref())
                        .or(active_model.as_deref())
                        .or(raw_model_candidate)
                        .unwrap_or(DEFAULT_MODEL),
                };

                if is_synthetic_placeholder_model(raw_model) {
                    return;
                }
                let explicit_provider = data
                    .get("provider")
                    .and_then(Value::as_str)
                    .or(active_provider.as_deref());
                let (canonical_model, provider_id) =
                    canonicalize_qoder_model_and_provider(raw_model, explicit_provider);

                let resolved_session = session_id
                    .clone()
                    .unwrap_or_else(|| fallback_session_id.clone());
                let req_id = value
                    .get("request_id")
                    .or_else(|| data.get("request_id"))
                    .or_else(|| value.get("ids").and_then(|ids| ids.get("request_id")))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty());

                let turn_key = if let Some(req) = req_id {
                    format!("req:{req}")
                } else {
                    let idx = anonymous_turn_counter;
                    anonymous_turn_counter += 1;
                    format!("log:{resolved_session}:{idx}")
                };

                let parsed_ts = value
                    .get("ts")
                    .or_else(|| value.get("timestamp"))
                    .and_then(parse_timestamp_value)
                    .unwrap_or(fallback_timestamp);

                if let Some(&existing_idx) = turn_index_by_key.get(&turn_key) {
                    turns[existing_idx].merge_authoritative(breakdown, reports_split);
                } else {
                    let idx = turns.len();
                    turn_index_by_key.insert(turn_key.clone(), idx);
                    turns.push(AssembledAssistantTurn {
                        raw_model: raw_model.to_string(),
                        canonical_model,
                        provider_id,
                        session_id: resolved_session,
                        parent_session_id: None,
                        timestamp: parsed_ts,
                        duration_ms: None,
                        is_turn_start: pending_turn_start,
                        dedup_key: format!("qoder:{turn_key}"),
                        workspace_key: workspace_key.clone(),
                        workspace_label: workspace_label.clone(),
                        agent: None,
                        authoritative_tokens: Some(breakdown),
                        reports_cache_split: reports_split,
                        input_context_chars: 0,
                        output_chars: 0,
                        reasoning_chars: 0,
                        prompt_token_hint: None,
                    });
                    pending_turn_start = false;
                }
            }
            "assistant" => {
                if value
                    .get("isApiErrorMessage")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    return;
                }

                let message_val = value.get("message");
                let raw_model_opt = message_val
                    .and_then(|m| m.get("model"))
                    .or_else(|| value.get("model"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());

                if let Some(m) = raw_model_opt {
                    if is_synthetic_placeholder_model(m) {
                        return;
                    }
                    if !m.eq_ignore_ascii_case("auto") {
                        active_model = Some(m.to_string());
                    }
                }

                let assistant_bmk = value
                    .get("businessModelKey")
                    .or_else(|| message_val.and_then(|m| m.get("businessModelKey")))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("auto"));

                if let Some(bmk) = assistant_bmk {
                    last_business_model = Some(bmk.to_string());
                }

                // In Qoder 1.1.82 - 1.1.85+, assistant turns carry generic model: "auto".
                // Resolve actual model using user.businessModelKey (preceding user turn)
                // or fallback to runtime-config.model (session-wide).
                let raw_model = match raw_model_opt {
                    Some(m) if !m.eq_ignore_ascii_case("auto") => m.to_string(),
                    _ => assistant_bmk
                        .or(last_business_model.as_deref())
                        .or(session_runtime_model.as_deref())
                        .or(active_model.as_deref())
                        .or(raw_model_opt)
                        .unwrap_or(DEFAULT_MODEL)
                        .to_string(),
                };

                if !raw_model.eq_ignore_ascii_case("auto") {
                    active_model = Some(raw_model.clone());
                }

                let explicit_provider = message_val
                    .and_then(|m| {
                        m.get("providerId")
                            .or_else(|| m.get("provider_id"))
                            .or_else(|| m.get("provider"))
                    })
                    .or_else(|| {
                        value
                            .get("providerId")
                            .or_else(|| value.get("provider_id"))
                            .or_else(|| value.get("provider"))
                    })
                    .and_then(Value::as_str)
                    .or(active_provider.as_deref());

                let (canonical_model, provider_id) =
                    canonicalize_qoder_model_and_provider(&raw_model, explicit_provider);

                let content_val = message_val
                    .and_then(|m| m.get("content"))
                    .or_else(|| value.get("content"));
                let (out_chars, reas_chars) = count_content_chars(content_val);

                let usage_val = message_val
                    .and_then(|m| m.get("usage"))
                    .or_else(|| value.get("usage"))
                    .or_else(|| value.get("usageMetadata"))
                    .or_else(|| value.get("token_usage"));
                let usage_breakdown = usage_val.and_then(extract_usage_breakdown);

                let msg_id = message_val
                    .and_then(|m| m.get("id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let req_id = value
                    .get("requestId")
                    .or_else(|| value.get("request_id"))
                    .or_else(|| usage_val.and_then(|u| u.get("request_id")))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let uuid = value
                    .get("uuid")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());

                let resolved_session = session_id
                    .clone()
                    .unwrap_or_else(|| fallback_session_id.clone());

                // Group multi-block streaming fragments of the same API call by
                // `message.id` (or `request_id` when `message.id` is absent).
                // When neither is present, fall back to `uuid` or positional index.
                let turn_key = if let Some(mid) = msg_id {
                    format!("msg:{mid}")
                } else if let Some(rid) = req_id {
                    format!("req:{rid}")
                } else if let Some(u) = uuid {
                    format!("uuid:{u}")
                } else {
                    let idx = anonymous_turn_counter;
                    anonymous_turn_counter += 1;
                    format!("session:{resolved_session}:{idx}")
                };

                let parsed_ts = value.get("timestamp").and_then(parse_timestamp_value);

                if let Some(&existing_idx) = turn_index_by_key.get(&turn_key) {
                    let turn = &mut turns[existing_idx];
                    if let Some((breakdown, reports_split)) = usage_breakdown {
                        turn.merge_authoritative(breakdown, reports_split);
                    }
                    turn.output_chars = turn.output_chars.saturating_add(out_chars);
                    turn.reasoning_chars = turn.reasoning_chars.saturating_add(reas_chars);
                    if let Some(ts) = parsed_ts {
                        if ts >= turn.timestamp {
                            let dur = ts.saturating_sub(turn.timestamp);
                            if dur > 0 {
                                turn.duration_ms = Some(turn.duration_ms.unwrap_or(0).max(dur));
                            }
                        }
                    }
                    context_chars = context_chars.saturating_add(out_chars + reas_chars);
                    return;
                }

                let is_sidechain = value
                    .get("isSidechain")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let entry_agent_id = value
                    .get("agentId")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                if sidechain_agent_cache.is_none() {
                    sidechain_agent_cache =
                        resolve_subagent_name(path, is_sidechain, entry_agent_id);
                }

                let (effective_session_id, parent_session_id) =
                    if is_sidechain || sidechain_agent_cache.is_some() {
                        let subagent_session = entry_agent_id
                            .map(|aid| format!("{resolved_session}:agent:{aid}"))
                            .unwrap_or_else(|| fallback_session_id.clone());
                        (subagent_session, Some(resolved_session))
                    } else {
                        (resolved_session, None)
                    };

                let timestamp = pending_request_start_ms
                    .unwrap_or_else(|| parsed_ts.unwrap_or(fallback_timestamp));
                let duration_ms = match (pending_request_start_ms, parsed_ts) {
                    (Some(start), Some(end)) if end > start => Some(end - start),
                    _ => None,
                };

                let input_snapshot = context_chars;
                context_chars = context_chars.saturating_add(out_chars + reas_chars);

                let (authoritative_tokens, reports_cache_split) = match usage_breakdown {
                    Some((b, s)) => (Some(b), s),
                    None => (None, false),
                };

                let idx = turns.len();
                turn_index_by_key.insert(turn_key.clone(), idx);
                turns.push(AssembledAssistantTurn {
                    raw_model,
                    canonical_model,
                    provider_id,
                    session_id: effective_session_id,
                    parent_session_id,
                    timestamp,
                    duration_ms,
                    is_turn_start: pending_turn_start,
                    dedup_key: format!("qoder:{turn_key}"),
                    workspace_key: workspace_key.clone(),
                    workspace_label: workspace_label.clone(),
                    agent: sidechain_agent_cache.clone(),
                    authoritative_tokens,
                    reports_cache_split,
                    input_context_chars: input_snapshot,
                    output_chars: out_chars,
                    reasoning_chars: reas_chars,
                    prompt_token_hint: pending_prompt_token_hint.take(),
                });

                pending_turn_start = false;
                pending_request_start_ms = None;
            }
            _ => {}
        }
    });

    turns
        .into_iter()
        .filter_map(AssembledAssistantTurn::into_unified_message)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn test_parse_qoder_authoritative_usage_and_fragments() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"{{"type":"user","uuid":"u1","sessionId":"sess-qoder-1","cwd":"/Users/alice/work/app","timestamp":"2026-11-07T10:00:00.000Z","message":{{"role":"user","content":"Implement the login handler"}}}}"#
        )
        .unwrap();
        // Multi-block streaming fragments sharing message.id = "msg_01"
        writeln!(
            file,
            r#"{{"type":"assistant","uuid":"a1-0","parentUuid":"u1","sessionId":"sess-qoder-1","cwd":"/Users/alice/work/app","timestamp":"2026-11-07T10:00:02.000Z","message":{{"id":"msg_01","type":"message","role":"assistant","model":"qwen3.8-max","stop_reason":null,"content":[{{"type":"thinking","thinking":"Let me plan the handler."}}],"usage":{{"input_tokens":1200,"output_tokens":40,"cache_read_input_tokens":800,"cache_creation_input_tokens":100,"cache_creation":{{"ephemeral_1h_input_tokens":50,"ephemeral_5m_input_tokens":50}},"request_id":"req_01"}}}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"assistant","uuid":"a1-1","parentUuid":"a1-0","sessionId":"sess-qoder-1","cwd":"/Users/alice/work/app","timestamp":"2026-11-07T10:00:05.000Z","message":{{"id":"msg_01","type":"message","role":"assistant","model":"qwen3.8-max","stop_reason":"end_turn","content":[{{"type":"text","text":"Here is the login handler implementation."}}],"usage":{{"input_tokens":1200,"output_tokens":350,"cache_read_input_tokens":800,"cache_creation_input_tokens":100,"cache_creation":{{"ephemeral_1h_input_tokens":50,"ephemeral_5m_input_tokens":50}},"thinking_tokens":60,"request_id":"req_01"}}}}}}"#
        )
        .unwrap();

        let messages = parse_qoder_file(file.path());
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.client.as_ref(), "qoder");
        assert_eq!(msg.model_id.as_ref(), "qwen3.8-max");
        assert_eq!(msg.provider_id.as_ref(), "qwen");
        assert_eq!(msg.session_id.as_ref(), "sess-qoder-1");
        assert_eq!(msg.tokens.input, 1200);
        assert_eq!(msg.tokens.output, 350);
        assert_eq!(msg.tokens.cache_read, 800);
        assert_eq!(msg.tokens.cache_write, 100);
        assert_eq!(msg.tokens.cache_write_1h, 50);
        assert_eq!(msg.tokens.reasoning, 60);
        assert!(msg.is_turn_start);
        assert_eq!(msg.duration_ms, Some(5000));
        assert_eq!(msg.workspace_label.as_deref(), Some("app"));
        assert_eq!(msg.dedup_key.as_deref(), Some("qoder:msg:msg_01"));
    }

    #[test]
    fn test_parse_qoder_scrubbed_zero_usage_falls_back_to_content_estimation() {
        let mut file = NamedTempFile::new().unwrap();
        let prompt_text = "a".repeat(400); // ~100 tokens
        let thinking_text = "b".repeat(200); // ~50 reasoning tokens
        let reply_text = "c".repeat(600); // ~150 output tokens
        writeln!(
            file,
            r#"{{"type":"runtime-config","sessionId":"sess-managed","model":"claude-sonnet-4-20250514","timestamp":1762509600000}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"user","uuid":"u1","sessionId":"sess-managed","timestamp":"2026-11-07T10:00:00.000Z","message":{{"role":"user","content":"{prompt_text}"}}}}"#
        )
        .unwrap();
        // Managed Qoder model with scrubbed zero usage (credits only) across two fragments
        writeln!(
            file,
            r#"{{"type":"assistant","uuid":"a1-0","sessionId":"sess-managed","timestamp":"2026-11-07T10:00:01.000Z","message":{{"id":"msg_scrubbed","role":"assistant","model":"claude-sonnet-4-20250514","stop_reason":null,"content":[{{"type":"thinking","thinking":"{thinking_text}"}}],"usage":{{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"credits":1.25,"billable":true,"context_usage_ratio":0.12}}}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"assistant","uuid":"a1-1","sessionId":"sess-managed","timestamp":"2026-11-07T10:00:03.000Z","message":{{"id":"msg_scrubbed","role":"assistant","model":"claude-sonnet-4-20250514","stop_reason":"end_turn","content":[{{"type":"text","text":"{reply_text}"}}],"usage":{{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"credits":1.25,"billable":true,"context_usage_ratio":0.12}}}}}}"#
        )
        .unwrap();

        let messages = parse_qoder_file(file.path());
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.provider_id.as_ref(), "anthropic");
        assert_eq!(msg.tokens.input, 100);
        assert_eq!(msg.tokens.output, 150);
        assert_eq!(msg.tokens.reasoning, 50);
        assert_eq!(msg.dedup_key.as_deref(), Some("qoder:msg:msg_scrubbed"));
    }

    #[test]
    fn test_parse_qoder_subagent_meta_and_skip_journal() {
        let dir = TempDir::new().unwrap();
        let subagents_dir = dir
            .path()
            .join(".qoder-cn/projects/my-repo/sess-main/subagents");
        std::fs::create_dir_all(&subagents_dir).unwrap();

        let journal_path = subagents_dir.join("journal.jsonl");
        std::fs::write(
            &journal_path,
            r#"{"type":"assistant","message":{"id":"j1","model":"qwen3-max","usage":{"input_tokens":999,"output_tokens":999}}}"#,
        )
        .unwrap();
        assert!(parse_qoder_file(&journal_path).is_empty());

        let meta_path = subagents_dir.join("agent-abc123.meta.json");
        std::fs::write(&meta_path, r#"{"agentType":"code-reviewer"}"#).unwrap();

        let agent_path = subagents_dir.join("agent-abc123.jsonl");
        std::fs::write(
            &agent_path,
            r#"{"type":"assistant","isSidechain":true,"sessionId":"sess-main","agentId":"abc123","timestamp":"2026-11-07T11:00:00.000Z","message":{"id":"msg_sub_1","model":"deepseek/chat/deepseek-v3","usage":{"input_tokens":500,"output_tokens":200}}}"#,
        )
        .unwrap();

        let messages = parse_qoder_file(&agent_path);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.agent.as_deref(), Some("Code Reviewer"));
        assert_eq!(msg.parent_session_id.as_deref(), Some("sess-main"));
        assert_eq!(msg.session_id.as_ref(), "sess-main:agent:abc123");
        assert_eq!(msg.provider_id.as_ref(), "deepseek");
        assert_eq!(msg.model_id.as_ref(), "deepseek-v3");
        assert_eq!(msg.workspace_key.as_deref(), Some("my-repo"));
        assert_eq!(msg.workspace_label.as_deref(), Some("my-repo"));
        assert_eq!(
            msg.pricing_model_id.as_deref(),
            Some("deepseek/chat/deepseek-v3")
        );
    }

    #[test]
    fn test_workspace_key_from_qoder_path_variations() {
        // Standard session under projects
        assert_eq!(
            workspace_key_from_qoder_path(Path::new(
                "/home/user/.qoder/projects/my-repo/sess-1.jsonl"
            )),
            Some("my-repo".to_string())
        );
        // Subagent under projects/<project>/<session>/subagents/
        assert_eq!(
            workspace_key_from_qoder_path(Path::new(
                "/home/user/.qoder/projects/my-repo/sess-main/subagents/agent-abc123.jsonl"
            )),
            Some("my-repo".to_string())
        );
        // Subagent under projects/<project>/sessions/<session>/subagents/ (intermediate dir)
        assert_eq!(
            workspace_key_from_qoder_path(Path::new(
                "/home/user/.qoder/projects/my-repo/sessions/sess-main/subagents/agent-abc123.jsonl"
            )),
            Some("my-repo".to_string())
        );
        // Subagent with deep workflow nesting under projects
        assert_eq!(
            workspace_key_from_qoder_path(Path::new(
                "/home/user/.qoder-cn/projects/my-repo/sess-main/subagents/workflows/wf-1/agent-abc123.jsonl"
            )),
            Some("my-repo".to_string())
        );
        // Direct orphan session file under projects/ (should not resolve to the filename)
        assert_eq!(
            workspace_key_from_qoder_path(Path::new("/home/user/.qoder/projects/orphan.jsonl")),
            Some("projects".to_string())
        );
        // Subagent outside of projects root: walks up past subagents and session dir to project
        assert_eq!(
            workspace_key_from_qoder_path(Path::new(
                "/var/workspaces/my-repo/sess-1/subagents/agent-abc123.jsonl"
            )),
            Some("my-repo".to_string())
        );
        // Deep workflow subagent outside of projects root
        assert_eq!(
            workspace_key_from_qoder_path(Path::new(
                "/var/workspaces/my-repo/sess-1/subagents/workflows/wf-1/agent-abc123.jsonl"
            )),
            Some("my-repo".to_string())
        );
        // Bare subagent without parent project: filtered out to avoid labeling workspace as "subagents"
        assert_eq!(
            workspace_key_from_qoder_path(Path::new("subagents/agent-abc123.jsonl")),
            None
        );
    }

    #[test]
    fn test_parse_qoder_model_source_does_not_override_provider() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"{{"type":"assistant","sessionId":"sess-1","modelSource":"managed","message":{{"id":"msg_1","model":"deepseek-v3","usage":{{"input_tokens":100,"output_tokens":50}}}}}}"#
        )
        .unwrap();

        let messages = parse_qoder_file(file.path());
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        // Without modelSource acting as explicit provider, "deepseek-v3" infers "deepseek"
        assert_eq!(msg.provider_id.as_ref(), "deepseek");
    }

    #[test]
    fn test_parse_qoder_auto_model_resolved_via_business_model_key_and_runtime_config() {
        let mut file = NamedTempFile::new().unwrap();
        // Session starting with runtime-config = gfmodel
        writeln!(
            file,
            r#"{{"type":"workspace-directories","sessionId":"sess-auto-1","directories":["/work/demo"]}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"runtime-config","sessionId":"sess-auto-1","model":"gfmodel","timestamp":1794041573406}}"#
        )
        .unwrap();
        // First user turn with businessModelKey = "gfmodel"
        writeln!(
            file,
            r#"{{"type":"user","uuid":"u1","sessionId":"sess-auto-1","timestamp":"2026-11-07T08:56:16.000Z","businessModelKey":"gfmodel","version":"1.1.84","message":{{"role":"user","content":"Run tests"}}}}"#
        )
        .unwrap();
        // Assistant multi-block fragments sharing message.id and carrying generic model "auto"
        writeln!(
            file,
            r#"{{"type":"assistant","uuid":"a1-0","sessionId":"sess-auto-1","timestamp":"2026-11-07T08:56:20.000Z","version":"1.1.84","message":{{"id":"msg_auto_1","role":"assistant","model":"auto","content":[{{"type":"thinking","thinking":"Planning test run"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"assistant","uuid":"a1-1","sessionId":"sess-auto-1","timestamp":"2026-11-07T08:56:22.000Z","version":"1.1.84","message":{{"id":"msg_auto_1","role":"assistant","model":"auto","content":[{{"type":"tool_use","id":"call_1","name":"Bash","input":{{"command":"cargo test"}}}}],"usage":{{"input_tokens":28000,"output_tokens":270,"cache_read_input_tokens":9000,"cache_creation_input_tokens":0}}}}}}"#
        )
        .unwrap();
        // Follow-up tool result turn without businessModelKey
        writeln!(
            file,
            r#"{{"type":"user","uuid":"u2","sessionId":"sess-auto-1","timestamp":"2026-11-07T08:56:25.000Z","version":"1.1.84","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"call_1","content":"test result ok"}}]}}}}"#
        )
        .unwrap();
        // Second assistant turn with model "auto", should still resolve to gfmodel
        writeln!(
            file,
            r#"{{"type":"assistant","uuid":"a2","sessionId":"sess-auto-1","timestamp":"2026-11-07T08:56:28.000Z","version":"1.1.84","message":{{"id":"msg_auto_2","role":"assistant","model":"auto","content":[{{"type":"text","text":"All tests passed!"}}],"usage":{{"input_tokens":32000,"output_tokens":150,"cache_read_input_tokens":12000,"cache_creation_input_tokens":0}}}}}}"#
        )
        .unwrap();

        let messages = parse_qoder_file(file.path());
        assert_eq!(messages.len(), 2);

        // Turn 1
        assert_eq!(messages[0].model_id.as_ref(), "glm-4-flash");
        assert_eq!(messages[0].provider_id.as_ref(), "zai");
        assert_eq!(messages[0].pricing_model_id.as_deref(), Some("gfmodel"));
        assert_eq!(messages[0].tokens.input, 28000);
        assert_eq!(messages[0].tokens.output, 270);
        assert_eq!(messages[0].tokens.cache_read, 9000);

        // Turn 2
        assert_eq!(messages[1].model_id.as_ref(), "glm-4-flash");
        assert_eq!(messages[1].provider_id.as_ref(), "zai");
        assert_eq!(messages[1].pricing_model_id.as_deref(), Some("gfmodel"));
        assert_eq!(messages[1].tokens.input, 32000);
        assert_eq!(messages[1].tokens.output, 150);
        assert_eq!(messages[1].tokens.cache_read, 12000);
    }

    #[test]
    fn test_parse_qoder_absent_usage_field_and_missing_version_lines() {
        let mut file = NamedTempFile::new().unwrap();
        let prompt_text = "p".repeat(400); // ~100 tokens
        let thinking_text = "t".repeat(200); // ~50 reasoning tokens
        let reply_text = "r".repeat(600); // ~150 output tokens

        // Lines without version field (workspace-directories, runtime-config, ai-title, last-prompt)
        writeln!(
            file,
            r#"{{"type":"workspace-directories","sessionId":"sess-v115","directories":["C:\\Users\\alice\\work"]}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"runtime-config","sessionId":"sess-v115","model":"qmodel_preview","timestamp":1784988512000}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"ai-title","sessionId":"sess-v115","aiTitle":"Refactor handler"}}"#
        )
        .unwrap();
        // User turn (version 1.1.5, usage absent)
        writeln!(
            file,
            r#"{{"type":"user","uuid":"u1","sessionId":"sess-v115","version":"1.1.5","timestamp":"2026-07-25T14:10:00.000Z","message":{{"role":"user","content":"{prompt_text}"}}}}"#
        )
        .unwrap();
        // Assistant multi-block fragments without any usage field at all
        writeln!(
            file,
            r#"{{"type":"assistant","uuid":"a1-0","sessionId":"sess-v115","version":"1.1.5","timestamp":"2026-07-25T14:10:02.000Z","message":{{"id":"msg_no_usage","type":"message","role":"assistant","model":"qmodel_preview","content":[{{"type":"thinking","thinking":"{thinking_text}"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"assistant","uuid":"a1-1","sessionId":"sess-v115","version":"1.1.5","timestamp":"2026-07-25T14:10:04.000Z","message":{{"id":"msg_no_usage","type":"message","role":"assistant","model":"qmodel_preview","content":[{{"type":"text","text":"{reply_text}"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"last-prompt","sessionId":"sess-v115","lastPrompt":"{prompt_text}"}}"#
        )
        .unwrap();

        let messages = parse_qoder_file(file.path());
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.model_id.as_ref(), "qwen3-coder");
        assert_eq!(msg.provider_id.as_ref(), "qwen");
        assert_eq!(msg.pricing_model_id.as_deref(), Some("qmodel_preview"));
        assert_eq!(msg.tokens.input, 100);
        assert_eq!(msg.tokens.output, 150);
        assert_eq!(msg.tokens.reasoning, 50);
        assert_eq!(msg.tokens.cache_read, 0);
        assert_eq!(msg.dedup_key.as_deref(), Some("qoder:msg:msg_no_usage"));
    }

    #[test]
    fn test_parse_qoder_dynamic_runtime_config_change_between_turns() {
        let mut file = NamedTempFile::new().unwrap();
        // Turn 1: runtime-config = gfmodel, user businessModelKey = gfmodel
        writeln!(
            file,
            r#"{{"type":"runtime-config","sessionId":"sess-dyn","model":"gfmodel","timestamp":1000}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"user","uuid":"u1","sessionId":"sess-dyn","businessModelKey":"gfmodel","timestamp":"2026-11-07T08:00:00.000Z","message":{{"role":"user","content":"Turn 1"}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"assistant","uuid":"a1","sessionId":"sess-dyn","timestamp":"2026-11-07T08:00:05.000Z","message":{{"id":"msg_1","role":"assistant","model":"auto","content":[{{"type":"text","text":"reply 1"}}],"usage":{{"input_tokens":100,"output_tokens":50}}}}}}"#
        )
        .unwrap();

        // User changes model to dfmodel in IDE dropdown (emits runtime-config)
        writeln!(
            file,
            r#"{{"type":"runtime-config","sessionId":"sess-dyn","model":"dfmodel","timestamp":2000}}"#
        )
        .unwrap();

        // Turn 2: user follow-up prompt without businessModelKey
        writeln!(
            file,
            r#"{{"type":"user","uuid":"u2","sessionId":"sess-dyn","timestamp":"2026-11-07T08:01:00.000Z","message":{{"role":"user","content":"Turn 2"}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"assistant","uuid":"a2","sessionId":"sess-dyn","timestamp":"2026-11-07T08:01:05.000Z","message":{{"id":"msg_2","role":"assistant","model":"auto","content":[{{"type":"text","text":"reply 2"}}],"usage":{{"input_tokens":200,"output_tokens":80}}}}}}"#
        )
        .unwrap();

        let msgs = parse_qoder_file(file.path());
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].model_id.as_ref(), "glm-4-flash");
        assert_eq!(msgs[0].pricing_model_id.as_deref(), Some("gfmodel"));
        assert_eq!(msgs[1].model_id.as_ref(), "deepseek-v3");
        assert_eq!(msgs[1].pricing_model_id.as_deref(), Some("dfmodel"));
    }
}
