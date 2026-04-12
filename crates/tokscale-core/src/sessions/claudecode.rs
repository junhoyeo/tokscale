//! Claude Code session parser
//!
//! Parses JSONL files from ~/.claude/projects/

use super::utils::{
    extract_i64, extract_string, file_modified_timestamp_ms, parse_timestamp_value,
};
use super::{
    normalize_agent_name, normalize_workspace_key, workspace_label_from_key, UnifiedMessage,
};
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

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
}

/// Meta sidecar written next to nested-layout sidechain transcripts.
/// e.g. `agent-abc123.meta.json` alongside `agent-abc123.jsonl`
#[derive(Debug, Deserialize)]
struct AgentMetaFile {
    #[serde(rename = "agentType")]
    agent_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClaudeMessage {
    pub model: Option<String>,
    pub usage: Option<ClaudeUsage>,
    /// Message ID for deduplication (used with requestId)
    pub id: Option<String>,
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
/// Tier 3: Fall back to a generic "claude-code-subagent" label.
///
/// (Tier 2 — parent-session tool_use inference — is deferred to a follow-up.)
fn resolve_subagent_name(path: &Path) -> String {
    // Tier 1: sibling meta.json (e.g. agent-abc123.meta.json next to agent-abc123.jsonl)
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        let meta_path = path.with_file_name(format!("{}.meta.json", stem));
        if let Ok(text) = std::fs::read_to_string(&meta_path) {
            if let Ok(meta) = serde_json::from_str::<AgentMetaFile>(&text) {
                if let Some(ref agent_type) = meta.agent_type {
                    if !agent_type.is_empty() {
                        return normalize_agent_name(agent_type);
                    }
                }
            }
        }
    }

    // Tier 3: generic fallback (still visible in the Agents tab)
    normalize_agent_name("claude-code-subagent")
}

/// Parse a Claude Code JSONL file
pub fn parse_claude_file(path: &Path) -> Vec<UnifiedMessage> {
    let (workspace_key, workspace_label) = claude_workspace_from_path(path);
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
                    sidechain_agent = Some(resolve_subagent_name(path));
                }
            }

            if entry.entry_type == "user" {
                // Distinguish real human input from tool results / system messages.
                // Tool results have content as a JSON array (e.g. [{"type":"tool_result",...}]).
                // System messages have XML-tagged content (e.g. <local-command-stdout>).
                // Only plain text without XML tags counts as a genuine user turn.
                if is_human_turn(trimmed) {
                    pending_turn_start = true;
                }
                continue;
            }

            // Only process assistant messages with usage data
            if entry.entry_type == "assistant" {
                let message = match entry.message {
                    Some(m) => m,
                    None => continue,
                };

                let usage = match message.usage {
                    Some(u) => u,
                    None => continue,
                };

                // Build dedup key for global deduplication (messageId:requestId composite).
                // For streaming responses, merge using per-field max to capture the most
                // complete token counts across all duplicate entries.
                let pending_hash = match (&message.id, &entry.request_id) {
                    (Some(msg_id), Some(req_id)) => {
                        let hash = format!("{}:{}", msg_id, req_id);
                        if let Some(&existing_idx) = processed_hashes.get(&hash) {
                            // Per-field max merge: each token field is updated independently
                            let t = &mut messages[existing_idx].tokens;
                            t.input = t.input.max(usage.input_tokens.unwrap_or(0).max(0));
                            t.output = t.output.max(usage.output_tokens.unwrap_or(0).max(0));
                            t.cache_read = t
                                .cache_read
                                .max(usage.cache_read_input_tokens.unwrap_or(0).max(0));
                            t.cache_write = t
                                .cache_write
                                .max(usage.cache_creation_input_tokens.unwrap_or(0).max(0));
                            continue;
                        }
                        Some(hash)
                    }
                    _ => None,
                };

                let model = match message.model {
                    Some(m) => m,
                    None => continue,
                };

                let timestamp = entry
                    .timestamp
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
                    .map(|dt| dt.timestamp_millis())
                    .unwrap_or(fallback_timestamp);

                // Insert dedup index only after all checks pass, right before push
                let dedup_key = pending_hash.inspect(|hash| {
                    processed_hashes.insert(hash.clone(), messages.len());
                });

                let mut unified = UnifiedMessage::new_with_dedup(
                    "claude",
                    model,
                    "anthropic",
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
                unified.agent = sidechain_agent.clone();
                unified.set_workspace(workspace_key.clone(), workspace_label.clone());
                // Mark the first assistant response after a user message as a turn start
                if pending_turn_start {
                    unified.is_turn_start = true;
                    pending_turn_start = false;
                }
                messages.push(unified);
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
        ) {
            let mut message = message;
            message.set_workspace(workspace_key.clone(), workspace_label.clone());
            messages.push(message);
        }
    }

    if let Some(message) =
        finalize_headless_state(&mut headless_state, &session_id, fallback_timestamp)
    {
        let mut message = message;
        message.set_workspace(workspace_key, workspace_label);
        messages.push(message);
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

    (None, None)
}

#[derive(Default)]
struct ClaudeHeadlessState {
    model: Option<String>,
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
) -> Vec<UnifiedMessage> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut bytes = data;
    let value: Value = match simd_json::from_slice(&mut bytes) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut messages = Vec::with_capacity(1);
    if let Some(message) = extract_claude_headless_message(&value, session_id, fallback_timestamp) {
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
) -> Option<UnifiedMessage> {
    let mut bytes = line.as_bytes().to_vec();
    let value: Value = simd_json::from_slice(&mut bytes).ok()?;

    let event_type = value.get("type").and_then(|val| val.as_str()).unwrap_or("");
    let mut completed_message: Option<UnifiedMessage> = None;

    match event_type {
        "message_start" => {
            completed_message = finalize_headless_state(state, session_id, fallback_timestamp);

            state.model = extract_claude_model(&value);
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
            completed_message = finalize_headless_state(state, session_id, fallback_timestamp);
        }
        _ => {
            if let Some(message) =
                extract_claude_headless_message(&value, session_id, fallback_timestamp)
            {
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
) -> Option<UnifiedMessage> {
    let usage = value
        .get("usage")
        .or_else(|| value.get("message").and_then(|msg| msg.get("usage")))?;
    let model = extract_claude_model(value)?;
    let timestamp = extract_claude_timestamp(value).unwrap_or(fallback_timestamp);

    Some(UnifiedMessage::new(
        "claude",
        model,
        "anthropic",
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

/// Returns true if a `type: "user"` JSONL entry is genuine human input (not tool results or system messages).
fn is_human_turn(raw_line: &str) -> bool {
    if let Some(pos) = raw_line.find("\"content\":") {
        let after = &raw_line[pos + 10..];
        let after_trimmed = after.trim_start();
        if after_trimmed.starts_with('[') {
            return false;
        }
        if let Some(content_start) = after_trimmed.strip_prefix('"') {
            if after_trimmed.len() > 1 && content_start.starts_with('<') {
                return false;
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
) -> Option<UnifiedMessage> {
    let model = state.model.clone()?;
    let timestamp = state.timestamp_ms.unwrap_or(fallback_timestamp);
    if state.input == 0 && state.output == 0 && state.cache_read == 0 && state.cache_write == 0 {
        *state = ClaudeHeadlessState::default();
        return None;
    }

    let message = UnifiedMessage::new(
        "claude",
        model,
        "anthropic",
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

        assert_eq!(messages.len(), 3, "Should have 3 assistant messages");

        // First assistant after first human user → turn start
        assert!(
            messages[0].is_turn_start,
            "First response should be turn start"
        );
        // Assistant after tool_result → NOT a new turn
        assert!(
            !messages[1].is_turn_start,
            "Response after tool_result should NOT be turn start"
        );
        // First assistant after second human user → turn start
        assert!(
            messages[2].is_turn_start,
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
    fn test_workspace_metadata_from_claude_project_path() {
        let content = r#"{"type":"assistant","timestamp":"2024-12-01T10:00:00.000Z","message":{"model":"claude-3-5-sonnet","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        let (_dir, path) = create_project_file(content, "myproject", "session.jsonl");

        let messages = parse_claude_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key, Some("myproject".to_string()));
        assert_eq!(messages[0].workspace_label, Some("myproject".to_string()));
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
}
