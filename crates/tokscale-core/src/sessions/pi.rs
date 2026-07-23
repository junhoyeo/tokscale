//! Pi (badlogic/pi-mono) session parser
//!
//! Parses JSONL files from `~/.pi/agent/sessions/<encoded-cwd>/*.jsonl` (and,
//! via the `pi` client's OMP scan root, `~/.omp/agent/sessions/...`). Current
//! OMP builds write a `title` metadata record before the `session` header in
//! newly-created session files; see [`PRE_SESSION_METADATA_TYPES`].

use super::utils::file_modified_timestamp_ms;
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Pi session header (first line of JSONL)
#[derive(Debug, Deserialize)]
pub struct PiSessionHeader {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub id: String,
    #[allow(dead_code)]
    pub timestamp: Option<String>,
    #[allow(dead_code)]
    pub cwd: Option<String>,
}

/// Loose type-only probe for a JSONL line, used to identify pre-session
/// metadata records without requiring their full schema.
#[derive(Debug, Deserialize)]
struct PiEntryTypeProbe {
    #[serde(rename = "type")]
    entry_type: String,
}

/// Record types OMP may write before the `session` header (e.g. an
/// auto-generated-title record). The parser skips these while looking for
/// `session` rather than discarding the whole file. Any other unrecognized
/// type before `session` is still treated as a malformed file.
const PRE_SESSION_METADATA_TYPES: &[&str] = &["title"];

/// Pi session entry (subsequent lines of JSONL)
#[derive(Debug, Deserialize)]
pub struct PiSessionEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    #[allow(dead_code)]
    pub id: Option<String>,
    #[serde(rename = "parentId")]
    #[allow(dead_code)]
    pub parent_id: Option<String>,
    pub timestamp: Option<String>,
    pub message: Option<PiMessage>,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PiMessage {
    pub role: Option<String>,
    pub usage: Option<PiUsage>,
    pub model: Option<String>,
    pub provider: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiUsage {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    #[allow(dead_code)]
    pub total_tokens: Option<i64>,
}

fn is_generated_id(value: &str) -> bool {
    (value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || (value.len() == 36
            && value.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            }))
}

fn strip_generated_id(value: &str) -> Option<&str> {
    for id_len in [36, 8] {
        if value.len() <= id_len || value.as_bytes()[value.len() - id_len - 1] != b'-' {
            continue;
        }
        let id = &value[value.len() - id_len..];
        if is_generated_id(id) {
            return Some(&value[..value.len() - id_len - 1]);
        }
    }
    None
}

fn pi_subagent_name(session_name: &str) -> Option<String> {
    let name = session_name.strip_prefix("subagent-")?;
    let without_id = strip_generated_id(name).or_else(|| {
        let (without_index, index) = name.rsplit_once('-')?;
        if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        strip_generated_id(without_index)
    })?;

    (!without_id.is_empty()).then(|| without_id.to_string())
}

/// Parse a Pi JSONL session file
pub fn parse_pi_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let fallback_timestamp = file_modified_timestamp_ms(path);

    let reader = BufReader::new(file);
    let mut messages: Vec<UnifiedMessage> = Vec::with_capacity(64);
    let mut buffer = Vec::with_capacity(4096);

    let mut session_id: Option<String> = None;
    let mut workspace_key: Option<String> = None;
    let mut workspace_label: Option<String> = None;
    let mut agent: Option<String> = None;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if session_id.is_none() {
            buffer.clear();
            buffer.extend_from_slice(trimmed.as_bytes());
            let entry_type = match simd_json::from_slice::<PiEntryTypeProbe>(&mut buffer) {
                Ok(probe) => probe.entry_type,
                Err(_) => return Vec::new(),
            };

            if entry_type != "session" {
                if PRE_SESSION_METADATA_TYPES.contains(&entry_type.as_str()) {
                    continue;
                }
                return Vec::new();
            }

            buffer.clear();
            buffer.extend_from_slice(trimmed.as_bytes());
            let header = match simd_json::from_slice::<PiSessionHeader>(&mut buffer) {
                Ok(h) => h,
                Err(_) => return Vec::new(),
            };

            session_id = Some(header.id);
            workspace_key = header.cwd.as_deref().and_then(normalize_workspace_key);
            workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
            continue;
        }

        buffer.clear();
        buffer.extend_from_slice(trimmed.as_bytes());
        let entry = match simd_json::from_slice::<PiSessionEntry>(&mut buffer) {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry.entry_type == "session_info" {
            agent = entry.name.as_deref().and_then(pi_subagent_name);
            continue;
        }

        if entry.entry_type != "message" {
            continue;
        }

        let message = match entry.message {
            Some(m) => m,
            None => continue,
        };

        if message.role.as_deref() != Some("assistant") {
            continue;
        }

        let usage = match message.usage {
            Some(u) => u,
            None => continue,
        };

        let model = match message.model {
            Some(m) => m,
            None => continue,
        };

        // A missing/blank provider field is recoverable: infer it from the
        // model name (e.g. a Pi "gpt-5" message with no provider maps to
        // "openai"), falling back to "pi" only when inference can't
        // identify the model, rather than dropping a message that carries
        // valid tokens.
        let provider = match message.provider {
            Some(p) if !p.is_empty() => p,
            _ => inferred_provider_from_model(&model)
                .unwrap_or("pi")
                .to_string(),
        };

        let timestamp = entry
            .timestamp
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
            .map(|dt| dt.timestamp_millis())
            .unwrap_or(fallback_timestamp);

        let mut unified = UnifiedMessage::new_with_agent(
            "pi",
            model,
            provider,
            session_id.clone().unwrap_or_else(|| "unknown".to_string()),
            timestamp,
            TokenBreakdown {
                input: usage.input.unwrap_or(0).max(0),
                output: usage.output.unwrap_or(0).max(0),
                cache_read: usage.cache_read.unwrap_or(0).max(0),
                cache_write: usage.cache_write.unwrap_or(0).max(0),
                reasoning: 0,
            },
            0.0,
            agent.clone(),
        );
        unified.set_workspace(workspace_key.clone(), workspace_label.clone());
        messages.push(unified);
    }

    messages
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
    fn test_parse_pi_jsonl_valid_assistant_message() {
        // given
        let content = r#"{"type":"session","id":"pi_ses_001","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","id":"msg_001","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"claude-3-5-sonnet","provider":"anthropic","usage":{"input":100,"output":50,"cacheRead":10,"cacheWrite":5,"totalTokens":165}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "pi");
        assert_eq!(messages[0].session_id, "pi_ses_001");
        assert_eq!(messages[0].model_id, "claude-3-5-sonnet");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.cache_read, 10);
        assert_eq!(messages[0].tokens.cache_write, 5);
        assert_eq!(messages[0].workspace_key, Some("/tmp".to_string()));
        assert_eq!(messages[0].workspace_label, Some("tmp".to_string()));
    }

    #[test]
    fn test_parse_pi_infers_provider_from_model_when_absent() {
        // given: no "provider" key at all — a missing provider must be
        // inferred from the model name (gpt-5 -> openai), not hardcoded
        // to "pi".
        let content = r#"{"type":"session","id":"pi_ses_005","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","id":"msg_001","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"gpt-5","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5");
        assert_eq!(messages[0].provider_id, "openai");
    }

    #[test]
    fn test_parse_pi_infers_provider_from_model_when_blank() {
        // given: "provider" present but blank — same inference path as
        // fully absent.
        let content = r#"{"type":"session","id":"pi_ses_006","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","id":"msg_001","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"gpt-5","provider":"","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openai");
    }

    #[test]
    fn test_parse_pi_falls_back_to_pi_when_provider_unrecoverable() {
        // given: no provider and a model name inference can't identify —
        // falls back to "pi" rather than dropping the message.
        let content = r#"{"type":"session","id":"pi_ses_007","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","id":"msg_001","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"totally-unrecognized-model-xyz","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "pi");
    }

    #[test]
    fn test_parse_pi_subagent_session_name_as_agent() {
        let content = r#"{"type":"session","id":"pi_subagent_001","timestamp":"2026-07-10T00:00:00.000Z","cwd":"/tmp"}
{"type":"session_info","id":"info_001","parentId":null,"timestamp":"2026-07-10T00:00:00.100Z","name":"subagent-go-reviewer-e2e7405c-cb84-4f0a-a6da-9d987494d130-1"}
{"type":"message","id":"msg_001","parentId":"info_001","timestamp":"2026-07-10T00:00:01.000Z","message":{"role":"assistant","model":"gpt-5","provider":"openai","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        let messages = parse_pi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent.as_deref(), Some("go-reviewer"));
        assert_eq!(
            pi_subagent_name("subagent-context-builder-208242ce-1").as_deref(),
            Some("context-builder")
        );
        assert_eq!(pi_subagent_name("Refactor auth module"), None);
    }

    #[test]
    fn test_parse_pi_skips_non_assistant_messages() {
        // given
        let content = r#"{"type":"session","id":"pi_ses_002","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"user","model":"claude-3-5-sonnet","provider":"anthropic","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_pi_skips_missing_usage() {
        // given
        let content = r#"{"type":"session","id":"pi_ses_003","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"claude-3-5-sonnet","provider":"anthropic"}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_pi_skips_malformed_json_lines() {
        // given
        let content = r#"{"type":"session","id":"pi_ses_004","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
not valid json
{"type":"message","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"gpt-4o-mini","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o-mini");
        assert_eq!(messages[0].provider_id, "openai");
    }

    #[test]
    fn test_parse_pi_skips_leading_title_record() {
        // given: current OMP builds write a `title` metadata record before
        // `session` (tokscale#802) — the parser must skip it, not discard
        // the whole file.
        let content = r#"{"type":"title","v":1,"title":"Comment on GitHub issue","source":"auto","updatedAt":"2026-07-02T18:08:49.723Z"}
{"type":"session","id":"pi_ses_005","timestamp":"2026-07-02T18:07:14.690Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-02T18:08:53.229Z","message":{"role":"assistant","model":"claude-sonnet-5","provider":"anthropic","usage":{"input":2,"output":180,"cacheRead":0,"cacheWrite":70844,"totalTokens":71026}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "pi_ses_005");
        assert_eq!(messages[0].model_id, "claude-sonnet-5");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 2);
        assert_eq!(messages[0].tokens.output, 180);
        assert_eq!(messages[0].tokens.cache_write, 70844);
    }

    #[test]
    fn test_parse_pi_skips_multiple_leading_title_records() {
        // given: defensive against more than one pre-session metadata line
        // in a row (e.g. a title record rewritten by a later auto-rename).
        let content = r#"{"type":"title","v":1,"title":"first"}
{"type":"title","v":1,"title":"renamed"}
{"type":"session","id":"pi_ses_006","timestamp":"2026-07-02T18:07:14.690Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-02T18:08:53.229Z","message":{"role":"assistant","model":"gpt-4o-mini","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "pi_ses_006");
    }

    #[test]
    fn test_parse_pi_rejects_unknown_leading_record_type() {
        // given: an unrecognized type before `session` is still treated as
        // a malformed file rather than silently scanned through.
        let content = r#"{"type":"totally_unknown_thing","foo":"bar"}
{"type":"session","id":"pi_ses_007","timestamp":"2026-07-02T18:07:14.690Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-02T18:08:53.229Z","message":{"role":"assistant","model":"gpt-4o-mini","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert!(messages.is_empty());
    }
}
