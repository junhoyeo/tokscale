//! Kimi CLI / Kimi Code session parser
//!
//! Parses wire.jsonl from both `kimi-cli` and `kimi-code`.
//!
//! ~/.kimi/sessions/[GROUP_ID]/[SESSION_UUID]/wire.jsonl
//!   Token data comes from StatusUpdate messages.
//!
//! ~/.kimi-code/sessions/[WORKSPACE]/[SESSION]/agents/[AGENT]/wire.jsonl
//!   Token data comes from usage.record lines.

use super::utils::{file_modified_timestamp_ms, for_each_json_line};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Top-level wire.jsonl line: either metadata or a timestamped message
#[derive(Debug, Deserialize)]
struct WireLine {
    timestamp: Option<f64>,
    message: Option<WireMessage>,
    #[serde(rename = "type")]
    line_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireMessage {
    #[serde(rename = "type")]
    msg_type: String,
    payload: Option<StatusPayload>,
}

#[derive(Debug, Deserialize)]
struct StatusPayload {
    token_usage: Option<TokenUsage>,
    #[allow(dead_code)]
    message_id: Option<String>,
}

/// Token usage counts shared by both wire formats.
///
/// Legacy kimi-cli StatusUpdate payloads use snake_case field names;
/// kimi-code usage.record lines use the camelCase aliases.
#[derive(Debug, Deserialize)]
struct TokenUsage {
    #[serde(alias = "inputOther")]
    input_other: Option<i64>,
    output: Option<i64>,
    #[serde(alias = "inputCacheRead")]
    input_cache_read: Option<i64>,
    #[serde(alias = "inputCacheCreation")]
    input_cache_creation: Option<i64>,
}

impl TokenUsage {
    /// Clamp negative counts to zero and build a breakdown.
    /// Returns `None` when every count is zero so callers can skip the entry.
    fn to_breakdown(&self) -> Option<TokenBreakdown> {
        let input = self.input_other.unwrap_or(0).max(0);
        let output = self.output.unwrap_or(0).max(0);
        let cache_read = self.input_cache_read.unwrap_or(0).max(0);
        let cache_write = self.input_cache_creation.unwrap_or(0).max(0);

        if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 {
            return None;
        }

        Some(TokenBreakdown {
            input,
            output,
            cache_read,
            cache_write,
            // Kimi wire protocols do not expose reasoning tokens; all reasoning included in output
            reasoning: 0,
        })
    }
}

/// Default model name when config.json is not available
const DEFAULT_MODEL: &str = "kimi-for-coding";
const DEFAULT_PROVIDER: &str = "moonshot";

/// Locate the legacy Kimi CLI config consumed by `parse_kimi_file`. Kimi Code
/// embeds model information in each wire record and does not use this file.
pub(crate) fn kimi_config_path(wire_path: &Path) -> Option<PathBuf> {
    let sessions_dir = wire_path.parent()?.parent()?.parent()?;
    Some(sessions_dir.parent()?.join("config.json"))
}

/// Read model name from ~/.kimi/config.json if available
fn read_model_from_config(wire_path: &Path) -> String {
    if let Some(config_path) = kimi_config_path(wire_path) {
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            if let Ok(bytes) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(model) = bytes.get("model").and_then(|v| v.as_str()) {
                    if !model.is_empty() {
                        return model.to_string();
                    }
                }
            }
        }
    }
    DEFAULT_MODEL.to_string()
}

/// Extract session ID from the wire.jsonl path
/// Path format: ~/.kimi/sessions/GROUP_ID/SESSION_UUID/wire.jsonl
fn extract_session_id(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Check whether a wire.jsonl path belongs to kimi-code.
///
/// kimi-code writes `<root>/sessions/WORKSPACE/SESSION/agents/AGENT/wire.jsonl`
/// while legacy kimi-cli writes `<root>/sessions/GROUP/UUID/wire.jsonl`, so the
/// grandparent directory component (`agents`) distinguishes the formats. The
/// layout under the root is created by kimi-code itself, so this holds for the
/// default `~/.kimi-code` root and custom `KIMI_CODE_HOME` roots alike.
pub fn is_kimi_code_path(path: &Path) -> bool {
    path.parent()
        .and_then(|agent_dir| agent_dir.parent())
        .and_then(|agents_dir| agents_dir.file_name())
        .is_some_and(|name| name == "agents")
}

/// Extract session ID from a kimi-code wire.jsonl path.
/// Path format: ~/.kimi-code/sessions/WORKSPACE/SESSION_UUID/agents/AGENT/wire.jsonl
fn extract_session_id_from_kimi_code_path(path: &Path) -> String {
    // Walk up: wire.jsonl -> AGENT -> agents -> SESSION_UUID -> ...
    path.parent() // AGENT
        .and_then(|p| p.parent()) // agents
        .and_then(|p| p.parent()) // SESSION_UUID
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// The workspace slug and the kimi-code root for a kimi-code wire path.
/// Path format: <root>/sessions/<SLUG>/<SESSION>/agents/<AGENT>/wire.jsonl
fn kimi_code_slug_and_root(path: &Path) -> Option<(&str, &Path)> {
    let agent_dir = path.parent()?; // AGENT
    let agents_dir = agent_dir.parent()?; // agents
    let session_dir = agents_dir.parent()?; // SESSION
    let slug_dir = session_dir.parent()?; // SLUG
    let sessions_dir = slug_dir.parent()?; // sessions
    let root = sessions_dir.parent()?; // kimi-code root
    Some((slug_dir.file_name()?.to_str()?, root))
}

/// A project root recovered from a kimi-code index, plus the label to display.
#[derive(Clone)]
struct KimiCodeWorkspace {
    key: String,
    label: Option<String>,
}

impl KimiCodeWorkspace {
    /// `name` is the display name kimi-code records in workspaces.json; without
    /// it the label is the project root's basename.
    fn from_root(root: &str, name: Option<&str>) -> Option<Self> {
        let key = normalize_workspace_key(root)?;
        let label = name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .or_else(|| workspace_label_from_key(&key));
        Some(Self { key, label })
    }
}

/// The slug -> workspace map from `<root>/workspaces.json`
/// (`{"version":1,"workspaces":{"<SLUG>":{"root":"/abs/path","name":"..."}}}`).
fn read_workspaces_index(root: &Path) -> HashMap<String, KimiCodeWorkspace> {
    let mut by_slug = HashMap::new();
    let Ok(contents) = std::fs::read_to_string(root.join("workspaces.json")) else {
        return by_slug;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return by_slug;
    };
    let Some(workspaces) = value.get("workspaces").and_then(|v| v.as_object()) else {
        return by_slug;
    };
    for (slug, entry) in workspaces {
        let workspace = entry.get("root").and_then(|v| v.as_str()).and_then(|root| {
            KimiCodeWorkspace::from_root(root, entry.get("name").and_then(|v| v.as_str()))
        });
        if let Some(workspace) = workspace {
            by_slug.insert(slug.clone(), workspace);
        }
    }
    by_slug
}

/// The session -> workspace fallback map from `<root>/session_index.jsonl`
/// (one JSON object per line with `sessionId`, `sessionDir`, `workDir`), for
/// sessions whose slug is absent from workspaces.json. Keyed by both the
/// session id and the session directory's basename, which the wire path's
/// SESSION component can match as either.
fn read_session_index(root: &Path) -> HashMap<String, KimiCodeWorkspace> {
    let mut by_session = HashMap::new();
    let Ok(contents) = std::fs::read_to_string(root.join("session_index.jsonl")) else {
        return by_session;
    };
    for line in contents.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let workspace = value
            .get("workDir")
            .and_then(|v| v.as_str())
            .and_then(|work_dir| KimiCodeWorkspace::from_root(work_dir, None));
        let Some(workspace) = workspace else {
            continue;
        };
        if let Some(session_id) = value.get("sessionId").and_then(|v| v.as_str()) {
            by_session.insert(session_id.to_string(), workspace.clone());
        }
        if let Some(dir_name) = value
            .get("sessionDir")
            .and_then(|v| v.as_str())
            .and_then(|dir| Path::new(dir).file_name())
            .and_then(|name| name.to_str())
        {
            by_session.insert(dir_name.to_string(), workspace);
        }
    }
    by_session
}

/// Attach workspace identity to kimi-code messages after the cache read.
///
/// Runs after the source-message cache, not inside [`parse_kimi_code_file`],
/// because `workspaces.json` is shared by every session under one kimi-code
/// root: folding it into each wire file's cache fingerprint would let one new
/// workspace invalidate every session's entry. Resolving here keeps a
/// workspaces-only edit free — nothing is invalidated, each root's indexes are
/// read once per scan rather than once per wire file, and a cache-served
/// message still gets the current workspace.
///
/// Legacy kimi-cli paths carry no workspace data and are left untouched.
pub fn apply_code_workspaces(sources: &mut [(PathBuf, Vec<UnifiedMessage>)]) {
    struct RootIndexes {
        by_slug: HashMap<String, KimiCodeWorkspace>,
        by_session: HashMap<String, KimiCodeWorkspace>,
    }

    let mut indexes: HashMap<PathBuf, RootIndexes> = HashMap::new();
    for (path, messages) in sources.iter() {
        if messages.is_empty() || !is_kimi_code_path(path) {
            continue;
        }
        let Some((_, root)) = kimi_code_slug_and_root(path) else {
            continue;
        };
        if indexes.contains_key(root) {
            continue;
        }
        indexes.insert(
            root.to_path_buf(),
            RootIndexes {
                by_slug: read_workspaces_index(root),
                by_session: read_session_index(root),
            },
        );
    }

    if indexes.is_empty() {
        return;
    }

    for (path, messages) in sources.iter_mut() {
        if messages.is_empty() || !is_kimi_code_path(path) {
            continue;
        }
        let Some((slug, root)) = kimi_code_slug_and_root(path) else {
            continue;
        };
        let Some(index) = indexes.get(root) else {
            continue;
        };
        let session_id = extract_session_id_from_kimi_code_path(path);
        let workspace = index
            .by_slug
            .get(slug)
            .or_else(|| index.by_session.get(&session_id));
        let Some(workspace) = workspace else {
            continue;
        };
        for message in messages.iter_mut() {
            if message.client != "kimi" {
                continue;
            }
            message.set_workspace(Some(workspace.key.clone()), workspace.label.clone());
        }
    }
}

/// Strip the "kimi-code/" prefix from model IDs emitted by kimi-code.
fn normalize_kimi_code_model(model: &str) -> String {
    model
        .strip_prefix("kimi-code/")
        .unwrap_or(model)
        .to_string()
}

/// Normalize a Kimi Code model, excluding symbolic config references such as
/// `__kimi_env_model__` that do not identify the model sent to the provider.
fn concrete_kimi_code_model(model: &str) -> Option<String> {
    let normalized = normalize_kimi_code_model(model.trim());
    let normalized = normalized.trim();
    let symbolic =
        normalized.len() >= 4 && normalized.starts_with("__") && normalized.ends_with("__");
    (!normalized.is_empty() && !symbolic).then(|| normalized.to_string())
}

/// Kimi Code wire.jsonl line structure.
#[derive(Debug, Deserialize)]
struct KimiCodeWireLine {
    #[serde(rename = "type")]
    line_type: String,
    model: Option<String>,
    usage: Option<TokenUsage>,
    #[serde(rename = "usageScope")]
    usage_scope: Option<String>,
    time: Option<i64>,
}

/// Parse a Kimi Code wire.jsonl file.
pub fn parse_kimi_code_file(path: &Path) -> Vec<UnifiedMessage> {
    let session_id = extract_session_id_from_kimi_code_path(path);
    let fallback_timestamp = file_modified_timestamp_ms(path);

    let mut messages: Vec<UnifiedMessage> = Vec::new();
    let mut latest_request_model: Option<String> = None;

    for_each_json_line(path, &mut |_index, trimmed| {
        let mut bytes = trimmed.as_bytes().to_vec();
        let wire_line = match simd_json::from_slice::<KimiCodeWireLine>(&mut bytes) {
            Ok(wl) => wl,
            Err(_) => return,
        };

        // usage.record can contain only a symbolic config reference, while the
        // preceding llm.request records the concrete model sent to the provider.
        if wire_line.line_type == "llm.request" {
            if let Some(model) = wire_line
                .model
                .as_deref()
                .and_then(concrete_kimi_code_model)
            {
                latest_request_model = Some(model);
            }
            return;
        }

        // Only process usage.record lines.
        // step.end also carries usage, but it duplicates the same usage.record
        // that was emitted in the same turn, so we ignore it to avoid double counting.
        if wire_line.line_type != "usage.record" {
            return;
        }

        // Only count turn-scoped usage. kimi-code tags every usage.record with
        // usageScope: "turn" for per-step LLM calls made inside a user turn and
        // "session" for non-turn bookkeeping (e.g. context compaction), and its
        // own tooling treats a missing usageScope as session-scoped, so require
        // an explicit "turn" to avoid counting aggregate records.
        if wire_line.usage_scope.as_deref() != Some("turn") {
            return;
        }

        // Skip entries with zero tokens
        let Some(tokens) = wire_line.usage.as_ref().and_then(TokenUsage::to_breakdown) else {
            return;
        };

        let model = wire_line
            .model
            .as_deref()
            .and_then(concrete_kimi_code_model)
            .or_else(|| latest_request_model.clone())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        // `time` is Unix milliseconds, so only positivity is checked here —
        // deliberately not routed through `parse_timestamp_value`, whose
        // seconds-vs-milliseconds heuristic would rescale anything below 1e12.
        // This field is never seconds, so rescaling would invent a plausible
        // instant for a value that is simply corrupt; the mtime fallback says
        // "unknown" instead.
        let timestamp_ms = wire_line
            .time
            .filter(|ms| *ms > 0)
            .unwrap_or(fallback_timestamp);

        messages.push(UnifiedMessage::new(
            "kimi",
            model,
            DEFAULT_PROVIDER,
            session_id.clone(),
            timestamp_ms,
            tokens,
            0.0,
        ));
    });

    messages
}

/// Parse a Kimi CLI wire.jsonl file
pub fn parse_kimi_file(path: &Path) -> Vec<UnifiedMessage> {
    let model = read_model_from_config(path);
    let session_id = extract_session_id(path);
    let fallback_timestamp = file_modified_timestamp_ms(path);

    let mut messages: Vec<UnifiedMessage> = Vec::new();
    let mut timestamp_sources: Vec<TimestampSource> = Vec::new();
    let mut keyed_indices: HashMap<String, usize> = HashMap::new();

    for_each_json_line(path, &mut |_index, trimmed| {
        let mut bytes = trimmed.as_bytes().to_vec();
        let wire_line = match simd_json::from_slice::<WireLine>(&mut bytes) {
            Ok(wl) => wl,
            Err(_) => return,
        };

        // Skip metadata lines (first line: {"type": "metadata", ...})
        if wire_line.line_type.as_deref() == Some("metadata") {
            return;
        }

        let message = match wire_line.message {
            Some(m) => m,
            None => return,
        };

        // Only process StatusUpdate messages
        if message.msg_type != "StatusUpdate" {
            return;
        }

        let payload = match message.payload {
            Some(p) => p,
            None => return,
        };

        let token_usage = match payload.token_usage {
            Some(u) => u,
            None => return,
        };

        // Convert Unix seconds (float) to milliseconds, falling back to file
        // mtime when the wire value is missing or does not convert to a
        // positive instant. A corrupt `{"timestamp": -1.5}` would otherwise
        // anchor the message in a pre-epoch daily bucket; the float->int cast
        // also collapses NaN to 0, so the same check catches that.
        let (timestamp_ms, timestamp_source) = match wire_line
            .timestamp
            .map(|ts| (ts * 1000.0) as i64)
            .filter(|ms| *ms > 0)
        {
            Some(ms) => (ms, TimestampSource::Wire),
            None => (fallback_timestamp, TimestampSource::FileMtime),
        };

        // Skip entries with zero tokens
        let Some(tokens) = token_usage.to_breakdown() else {
            return;
        };

        let dedup_key = payload.message_id;

        let message = UnifiedMessage::new_with_dedup(
            "kimi",
            model.clone(),
            DEFAULT_PROVIDER,
            session_id.clone(),
            timestamp_ms,
            tokens,
            0.0,
            dedup_key,
        );
        push_or_replace_status_update(
            &mut messages,
            &mut timestamp_sources,
            &mut keyed_indices,
            message,
            timestamp_source,
        );
    });

    messages
}

fn exact_token_total(tokens: &TokenBreakdown) -> i128 {
    i128::from(tokens.input)
        + i128::from(tokens.output)
        + i128::from(tokens.cache_read)
        + i128::from(tokens.cache_write)
        + i128::from(tokens.reasoning)
}

/// Where a StatusUpdate's anchor came from. The mtime fallback is a guess for a
/// line whose own timestamp was unusable, so it ranks below a real wire value
/// when duplicates are compared.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TimestampSource {
    Wire,
    FileMtime,
}

fn should_replace_status_update(
    existing: (&UnifiedMessage, TimestampSource),
    candidate: (&UnifiedMessage, TimestampSource),
) -> bool {
    let (existing, existing_source) = existing;
    let (candidate, candidate_source) = candidate;
    let existing_total = exact_token_total(&existing.tokens);
    let candidate_total = exact_token_total(&candidate.tokens);

    if candidate_total != existing_total {
        return candidate_total > existing_total;
    }

    // Totals tie, so the anchor decides. Compare provenance before the value:
    // mtime is >= every real timestamp in a file still being written, so a
    // corrupt duplicate that fell back to it would otherwise outrank the good
    // line it duplicates and move the message off its true day.
    if existing_source != candidate_source {
        return candidate_source == TimestampSource::Wire;
    }

    candidate.timestamp >= existing.timestamp
}

fn push_or_replace_status_update(
    messages: &mut Vec<UnifiedMessage>,
    timestamp_sources: &mut Vec<TimestampSource>,
    keyed_indices: &mut HashMap<String, usize>,
    message: UnifiedMessage,
    timestamp_source: TimestampSource,
) {
    let dedup_key = message
        .dedup_key
        .as_ref()
        .filter(|key| !key.is_empty())
        .cloned();

    let Some(dedup_key) = dedup_key else {
        messages.push(message);
        timestamp_sources.push(timestamp_source);
        return;
    };

    if let Some(index) = keyed_indices.get(&dedup_key).copied() {
        if should_replace_status_update(
            (&messages[index], timestamp_sources[index]),
            (&message, timestamp_source),
        ) {
            messages[index] = message;
            timestamp_sources[index] = timestamp_source;
        }
        return;
    }

    let index = messages.len();
    messages.push(message);
    timestamp_sources.push(timestamp_source);
    keyed_indices.insert(dedup_key, index);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_parse_kimi_valid_status_update() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983426.420942, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 1562, "output": 2463, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "chatcmpl-xxx"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "kimi");
        assert_eq!(messages[0].model_id, "kimi-for-coding");
        assert_eq!(messages[0].provider_id, "moonshot");
        assert_eq!(messages[0].tokens.input, 1562);
        assert_eq!(messages[0].tokens.output, 2463);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.cache_write, 0);
        // Timestamp: 1770983426.420942 * 1000 = 1770983426420
        assert_eq!(messages[0].timestamp, 1770983426420);
    }

    #[test]
    fn test_parse_kimi_multi_turn() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983400.0, "message": {"type": "TurnBegin", "payload": {"user_input": "hello"}}}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 100, "output": 200, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-1"}}}
{"timestamp": 1770983420.0, "message": {"type": "TurnBegin", "payload": {"user_input": "world"}}}
{"timestamp": 1770983430.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 300, "output": 400, "input_cache_read": 50, "input_cache_creation": 0}, "message_id": "msg-2"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 200);
        assert_eq!(messages[1].tokens.input, 300);
        assert_eq!(messages[1].tokens.output, 400);
        assert_eq!(messages[1].tokens.cache_read, 50);
    }

    #[test]
    fn test_parse_kimi_skip_non_status_update() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983400.0, "message": {"type": "TurnBegin", "payload": {"user_input": "hello"}}}
{"timestamp": 1770983410.0, "message": {"type": "ContentPart", "payload": {"type": "text", "text": "response"}}}
{"timestamp": 1770983420.0, "message": {"type": "ToolCall", "payload": {"type": "function", "id": "tool_1", "function": {"name": "ReadFile", "arguments": "{}"}}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_kimi_empty_file() {
        let file = create_test_file("");

        let messages = parse_kimi_file(file.path());

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_kimi_tool_call_multi_step() {
        // Simulates a tool-call scenario with multiple StatusUpdate messages in one turn
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983400.0, "message": {"type": "TurnBegin", "payload": {"user_input": "read file"}}}
{"timestamp": 1770983405.0, "message": {"type": "StepBegin", "payload": {"n": 1}}}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 500, "output": 100, "input_cache_read": 200, "input_cache_creation": 0}, "message_id": "msg-step1"}}}
{"timestamp": 1770983415.0, "message": {"type": "ToolCall", "payload": {"type": "function", "id": "tool_1", "function": {"name": "ReadFile", "arguments": "{}"}}}}
{"timestamp": 1770983420.0, "message": {"type": "StepBegin", "payload": {"n": 2}}}
{"timestamp": 1770983425.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 800, "output": 300, "input_cache_read": 400, "input_cache_creation": 100}, "message_id": "msg-step2"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 2);
        // Step 1
        assert_eq!(messages[0].tokens.input, 500);
        assert_eq!(messages[0].tokens.output, 100);
        assert_eq!(messages[0].tokens.cache_read, 200);
        assert_eq!(messages[0].tokens.cache_write, 0);
        // Step 2
        assert_eq!(messages[1].tokens.input, 800);
        assert_eq!(messages[1].tokens.output, 300);
        assert_eq!(messages[1].tokens.cache_read, 400);
        assert_eq!(messages[1].tokens.cache_write, 100);
    }

    #[test]
    fn test_parse_kimi_with_cache_tokens() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1771123711.615454, "message": {"type": "StatusUpdate", "payload": {"context_usage": 0.024, "token_usage": {"input_other": 1508, "output": 205, "input_cache_read": 4864, "input_cache_creation": 0}, "message_id": "chatcmpl-2tNw2mhUNfdPMP0Jyie7gDhD"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 1508);
        assert_eq!(messages[0].tokens.output, 205);
        assert_eq!(messages[0].tokens.cache_read, 4864);
        assert_eq!(messages[0].tokens.cache_write, 0);
    }

    #[test]
    fn test_parse_kimi_deduplicates_repeated_status_updates_by_message_id() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 100, "output": 10, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-progressive"}}}
{"timestamp": 1770983420.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 120, "output": 30, "input_cache_read": 5, "input_cache_creation": 0}, "message_id": "msg-progressive"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("msg-progressive"));
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[0].timestamp, 1770983420000);
    }

    #[test]
    fn test_parse_kimi_keeps_larger_extreme_status_update() {
        // Both saturating totals equal i64::MAX, but the first exact total is larger.
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 9223372036854775807, "output": 9223372036854775807, "input_cache_read": 2, "input_cache_creation": 0}, "message_id": "msg-extreme"}}}
{"timestamp": 1770983420.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 9223372036854775807, "output": 0, "input_cache_read": 1, "input_cache_creation": 0}, "message_id": "msg-extreme"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("msg-extreme"));
        assert_eq!(messages[0].tokens.input, i64::MAX);
        assert_eq!(messages[0].tokens.output, i64::MAX);
        assert_eq!(messages[0].tokens.cache_read, 2);
        assert_eq!(messages[0].tokens.cache_write, 0);
        assert_eq!(messages[0].timestamp, 1770983410000);
    }

    #[test]
    fn test_parse_kimi_keeps_distinct_and_missing_message_ids_separate() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 10, "output": 1, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-1"}}}
{"timestamp": 1770983420.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 20, "output": 2, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-2"}}}
{"timestamp": 1770983430.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 30, "output": 3, "input_cache_read": 0, "input_cache_creation": 0}}}}
{"timestamp": 1770983440.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 40, "output": 4, "input_cache_read": 0, "input_cache_creation": 0}}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("msg-1"));
        assert_eq!(messages[1].dedup_key.as_deref(), Some("msg-2"));
        assert!(messages[2].dedup_key.is_none());
        assert!(messages[3].dedup_key.is_none());
    }

    #[test]
    fn test_parse_kimi_skips_zero_token_entries() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 0, "output": 0, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-empty"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_kimi_keeps_extreme_buckets_and_skips_only_all_zero() {
        // MAX + MAX + 2 panics in debug and wraps to zero in release.
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 9223372036854775807, "output": 9223372036854775807, "input_cache_read": 2, "input_cache_creation": 0}, "message_id": "msg-extreme"}}}
{"timestamp": 1770983420.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 0, "output": 0, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-zero"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, i64::MAX);
        assert_eq!(messages[0].tokens.output, i64::MAX);
        assert_eq!(messages[0].tokens.cache_read, 2);
        assert_eq!(messages[0].tokens.cache_write, 0);
    }

    #[test]
    fn test_parse_kimi_non_positive_timestamps_fall_back_to_mtime() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": -1.5, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 10, "output": 1, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-negative"}}}
{"timestamp": 0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 20, "output": 2, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-zero"}}}
{"timestamp": 1770983426.420942, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 30, "output": 3, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-valid"}}}"#;
        let file = create_test_file(content);
        let mtime = file_modified_timestamp_ms(file.path());

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 3);
        // -1.5s would otherwise become -1500ms and bucket into 1969-12-31 (UTC;
        // the exact pre-epoch day depends on the local zone, the mis-dating
        // does not).
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].timestamp, mtime);
        assert_eq!(messages[1].tokens.input, 20);
        assert_eq!(messages[1].timestamp, mtime);
        assert_eq!(messages[2].tokens.input, 30);
        assert_eq!(messages[2].timestamp, 1770983426420);
    }

    #[test]
    fn test_parse_kimi_mtime_fallback_does_not_outrank_a_real_timestamp() {
        // Same message_id, same totals, second line's timestamp unusable. The
        // fallback lands on mtime, which is newer than every real timestamp in
        // a live session file, so an untied comparison would let the corrupt
        // line's anchor replace the good one.
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": 1770983426.420942, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 100, "output": 10, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-dup"}}}
{"timestamp": -1, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 100, "output": 10, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-dup"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].timestamp, 1770983426420);
    }

    #[test]
    fn test_parse_kimi_real_timestamp_still_wins_a_tie_over_mtime_fallback() {
        // Mirror of the above with the corrupt line first, so the good anchor
        // arrives as the candidate rather than the incumbent.
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
{"timestamp": -1, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 100, "output": 10, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-dup"}}}
{"timestamp": 1770983426.420942, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 100, "output": 10, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-dup"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].timestamp, 1770983426420);
    }

    #[test]
    fn test_parse_kimi_malformed_lines() {
        let content = r#"{"type": "metadata", "protocol_version": "1.3"}
not valid json at all
{"timestamp": 1770983410.0, "message": {"type": "StatusUpdate", "payload": {"token_usage": {"input_other": 100, "output": 200, "input_cache_read": 0, "input_cache_creation": 0}, "message_id": "msg-1"}}}"#;
        let file = create_test_file(content);

        let messages = parse_kimi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 100);
    }

    // -------------------------------------------------------------------------
    // Kimi Code tests
    // -------------------------------------------------------------------------

    fn create_kimi_code_test_file(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        // Build a fake kimi-code path so extract_session_id_from_kimi_code_path works:
        //   .../.kimi-code/sessions/ws/session-uuid/agents/main/wire.jsonl
        let fake_path = dir
            .path()
            .join(".kimi-code")
            .join("sessions")
            .join("test-ws")
            .join("sess-abc-123")
            .join("agents")
            .join("main")
            .join("wire.jsonl");
        std::fs::create_dir_all(fake_path.parent().unwrap()).unwrap();
        std::fs::write(&fake_path, content).unwrap();
        (dir, fake_path)
    }

    #[test]
    fn test_parse_kimi_code_valid_usage_record() {
        let content = r#"{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":5102,"output":172,"inputCacheRead":13312,"inputCacheCreation":0},"usageScope":"turn","time":1780319377014}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "kimi");
        assert_eq!(messages[0].model_id, "kimi-for-coding");
        assert_eq!(messages[0].provider_id, "moonshot");
        assert_eq!(messages[0].session_id, "sess-abc-123");
        assert_eq!(messages[0].tokens.input, 5102);
        assert_eq!(messages[0].tokens.output, 172);
        assert_eq!(messages[0].tokens.cache_read, 13312);
        assert_eq!(messages[0].tokens.cache_write, 0);
        assert_eq!(messages[0].timestamp, 1780319377014);
    }

    #[test]
    fn test_parse_kimi_code_keeps_latest_concrete_model_across_invalid_requests() {
        let content = r#"{"type":"llm.request","model":"k3","time":1780319377000}
{"type":"llm.request","time":1780319377001}
{"type":"llm.request","model":" ","time":1780319377002}
{"type":"llm.request","model":"__runtime_model__","time":1780319377003}
{"type":"llm.request","model":"kimi-code/   ","time":1780319377004}
{"type":"llm.request","model":"kimi-code/ __runtime_model__ ","time":1780319377005}
{"type":"usage.record","model":"kimi-code/__kimi_env_model__","usage":{"inputOther":100,"output":50,"inputCacheRead":25,"inputCacheCreation":0},"usageScope":"turn","time":1780319377010}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "k3");
    }

    #[test]
    fn test_parse_kimi_code_prefers_concrete_usage_model_and_tracks_requests() {
        let content = r#"{"type":"llm.request","model":"k3","time":1780319377000}
{"type":"usage.record","model":"__runtime_model__","usage":{"inputOther":100,"output":50,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377010}
{"type":"llm.request","model":"kimi-code/k3-256k","time":1780319377020}
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":200,"output":75,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377030}
{"type":"usage.record","model":"__another_model_alias__","usage":{"inputOther":300,"output":100,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377040}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].model_id, "k3");
        assert_eq!(messages[1].model_id, "kimi-for-coding");
        assert_eq!(messages[2].model_id, "k3-256k");
    }

    #[test]
    fn test_parse_kimi_code_invalid_usage_without_request_uses_default_model() {
        let content = r#"{"type":"usage.record","model":"__kimi_env_model__","usage":{"inputOther":100,"output":50,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377010}
{"type":"usage.record","model":"kimi-code/__runtime_model__","usage":{"inputOther":100,"output":50,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377020}
{"type":"usage.record","model":"kimi-code/ __runtime_model__ ","usage":{"inputOther":100,"output":50,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377030}
{"type":"usage.record","model":"kimi-code/   ","usage":{"inputOther":100,"output":50,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377040}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);

        assert_eq!(messages.len(), 4);
        assert!(messages
            .iter()
            .all(|message| message.model_id == DEFAULT_MODEL));
    }

    #[test]
    fn test_parse_kimi_code_skip_non_usage_record() {
        let content = r#"{"type":"context.append_loop_event","event":{"type":"tool.call","name":"Read"},"time":1780319377000}
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":100,"output":50,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319378000}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].timestamp, 1780319378000);
    }

    #[test]
    fn test_parse_kimi_code_non_positive_time_falls_back_to_mtime() {
        let content = r#"{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":10,"output":1,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":-1500}
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":20,"output":2,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":0}
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":30,"output":3,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377014}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);
        let mtime = file_modified_timestamp_ms(&fake_path);

        let messages = parse_kimi_code_file(&fake_path);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].timestamp, mtime);
        assert_eq!(messages[1].tokens.input, 20);
        assert_eq!(messages[1].timestamp, mtime);
        assert_eq!(messages[2].tokens.input, 30);
        assert_eq!(messages[2].timestamp, 1780319377014);
    }

    #[test]
    fn test_normalize_kimi_code_model() {
        assert_eq!(
            normalize_kimi_code_model("kimi-code/kimi-for-coding"),
            "kimi-for-coding"
        );
        // No prefix: returned unchanged
        assert_eq!(
            normalize_kimi_code_model("kimi-for-coding"),
            "kimi-for-coding"
        );
        assert_eq!(normalize_kimi_code_model(""), "");
    }

    #[test]
    fn test_parse_kimi_code_session_id_extraction() {
        assert_eq!(
            extract_session_id_from_kimi_code_path(std::path::Path::new(
                "/home/user/.kimi-code/sessions/workspace/session-uuid/agents/main/wire.jsonl"
            )),
            "session-uuid"
        );
        assert_eq!(
            extract_session_id_from_kimi_code_path(std::path::Path::new(
                "C:/Users/Alice/.kimi-code/sessions/workspace/sess-123/agents/coder/wire.jsonl"
            )),
            "sess-123"
        );
        assert_eq!(
            extract_session_id_from_kimi_code_path(std::path::Path::new("wire.jsonl")),
            "unknown"
        );
    }

    #[test]
    fn test_parse_kimi_code_only_counts_turn_scoped_usage() {
        // "session"-scoped records are non-turn bookkeeping (e.g. compaction)
        // and records without usageScope are treated as session-scoped by
        // kimi-code itself; neither should be counted.
        let content = r#"{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":999,"output":999,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"session","time":1780319377000}
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":888,"output":888,"inputCacheRead":0,"inputCacheCreation":0},"time":1780319377005}
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":100,"output":50,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377010}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].timestamp, 1780319377010);
    }

    #[test]
    fn test_parse_kimi_code_zero_tokens_skipped() {
        let content = r#"{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":0,"output":0,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377014}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_kimi_code_keeps_extreme_buckets_and_skips_only_all_zero() {
        // MAX + MAX + 2 panics in debug and wraps to zero in release.
        let content = r#"{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":9223372036854775807,"output":9223372036854775807,"inputCacheRead":2,"inputCacheCreation":0},"usageScope":"turn","time":1780319377014}
{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":0,"output":0,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377015}"#;
        let (_dir, fake_path) = create_kimi_code_test_file(content);

        let messages = parse_kimi_code_file(&fake_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, i64::MAX);
        assert_eq!(messages[0].tokens.output, i64::MAX);
        assert_eq!(messages[0].tokens.cache_read, 2);
        assert_eq!(messages[0].tokens.cache_write, 0);
    }

    #[test]
    fn test_is_kimi_code_path() {
        assert!(is_kimi_code_path(std::path::Path::new(
            "/home/user/.kimi-code/sessions/workspace/sess/agents/main/wire.jsonl"
        )));
        // Custom KIMI_CODE_HOME root: kimi-code still creates the
        // agents/<AGENT>/wire.jsonl layout underneath it.
        assert!(is_kimi_code_path(std::path::Path::new(
            "/data/kimi/sessions/ws/sess/agents/main/wire.jsonl"
        )));
        assert!(!is_kimi_code_path(std::path::Path::new(
            "/home/user/.kimi/sessions/group/uuid/wire.jsonl"
        )));
        assert!(!is_kimi_code_path(std::path::Path::new("wire.jsonl")));
    }

    // -------------------------------------------------------------------------
    // Kimi Code workspace resolution tests
    // -------------------------------------------------------------------------

    const WORKSPACE_WIRE_LINE: &str = r#"{"type":"usage.record","model":"kimi-code/kimi-for-coding","usage":{"inputOther":100,"output":50,"inputCacheRead":0,"inputCacheCreation":0},"usageScope":"turn","time":1780319377014}"#;

    /// Build <tmp>/.kimi-code/sessions/<SLUG>/<SESSION>/agents/main/wire.jsonl
    /// and return the tempdir guard, the wire path, and the kimi-code root.
    fn create_kimi_code_layout(
        slug: &str,
        session: &str,
    ) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".kimi-code");
        let wire_path = root
            .join("sessions")
            .join(slug)
            .join(session)
            .join("agents")
            .join("main")
            .join("wire.jsonl");
        std::fs::create_dir_all(wire_path.parent().unwrap()).unwrap();
        std::fs::write(&wire_path, WORKSPACE_WIRE_LINE).unwrap();
        (dir, wire_path, root)
    }

    #[test]
    fn test_apply_code_workspaces_resolves_slug_via_workspaces_json() {
        let (_dir, wire_path, root) =
            create_kimi_code_layout("wd_odyssey_4c02790435a7", "sess-abc-123");
        std::fs::write(
            root.join("workspaces.json"),
            r#"{"version":1,"workspaces":{"wd_odyssey_4c02790435a7":{"root":"/home/user/Projects/golang/Odyssey","name":"Odyssey"}}}"#,
        )
        .unwrap();

        let mut sources = vec![(wire_path.clone(), parse_kimi_code_file(&wire_path))];
        apply_code_workspaces(&mut sources);

        let messages = &sources[0].1;
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/home/user/Projects/golang/Odyssey")
        );
        // The sidecar's `name` is preferred over the root's basename.
        assert_eq!(messages[0].workspace_label.as_deref(), Some("Odyssey"));
    }

    #[test]
    fn test_apply_code_workspaces_falls_back_to_session_index() {
        let (_dir, wire_path, root) = create_kimi_code_layout("wd_gone_0000", "sess-fallback-1");
        // No workspaces.json entry for this slug; the session index still
        // knows where the session ran.
        std::fs::write(
            root.join("workspaces.json"),
            r#"{"version":1,"workspaces":{}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("session_index.jsonl"),
            concat!(
                r#"{"sessionId":"sess-fallback-1","sessionDir":"/x/sessions/wd_gone_0000/sess-fallback-1","workDir":"/home/user/work/api-server"}"#,
                "\n",
                "not json at all\n"
            ),
        )
        .unwrap();

        let mut sources = vec![(wire_path.clone(), parse_kimi_code_file(&wire_path))];
        apply_code_workspaces(&mut sources);

        let messages = &sources[0].1;
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/home/user/work/api-server")
        );
        // No recorded name: the label is the workDir's basename.
        assert_eq!(messages[0].workspace_label.as_deref(), Some("api-server"));
    }

    #[test]
    fn test_apply_code_workspaces_leaves_missing_slug_untouched() {
        let (_dir, wire_path, root) = create_kimi_code_layout("wd_unknown_9999", "sess-none-1");
        std::fs::write(
            root.join("workspaces.json"),
            r#"{"version":1,"workspaces":{}}"#,
        )
        .unwrap();

        let mut sources = vec![(wire_path.clone(), parse_kimi_code_file(&wire_path))];
        apply_code_workspaces(&mut sources);

        let messages = &sources[0].1;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key, None);
        assert_eq!(messages[0].workspace_label, None);
    }

    #[test]
    fn test_apply_code_workspaces_ignores_legacy_kimi_cli_paths() {
        let dir = tempfile::tempdir().unwrap();
        // Legacy layout: <root>/sessions/GROUP/UUID/wire.jsonl — no `agents`.
        let wire_path = dir
            .path()
            .join(".kimi")
            .join("sessions")
            .join("group-1")
            .join("uuid-1")
            .join("wire.jsonl");
        std::fs::create_dir_all(wire_path.parent().unwrap()).unwrap();
        std::fs::write(&wire_path, WORKSPACE_WIRE_LINE).unwrap();

        let mut sources = vec![(wire_path.clone(), parse_kimi_code_file(&wire_path))];
        apply_code_workspaces(&mut sources);

        let messages = &sources[0].1;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key, None);
    }
}
