//! OpenCodeReview session parser
//!
//! OpenCodeReview stores sessions as JSONL files under
//! `~/.opencodereview/sessions/<encoded-repo-path>/<session-id>.jsonl`.

use super::utils::{file_modified_timestamp_ms, parse_timestamp_value};
use super::UnifiedMessage;
use crate::{pricing, provider_identity, TokenBreakdown};
use serde_json::Value;
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub fn parse_opencodereview_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let session_id = session_id_from_path(path);
    let fallback_timestamp = file_modified_timestamp_ms(path);

    let mut workspace: Option<String> = None;
    let mut messages = Vec::new();
    let mut seen = HashSet::new();

    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { continue };

        if !line.contains("llm_response") && !line.contains("session_start") {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        let record_type = value.get("type").and_then(Value::as_str).unwrap_or("");

        if record_type == "session_start" {
            if workspace.is_none() {
                workspace = value
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            continue;
        }

        if record_type != "llm_response" {
            continue;
        }

        let usage = match value.get("usage") {
            Some(u) => u,
            None => continue,
        };

        let tokens = tokens_from_usage(usage);
        if tokens.total() == 0 {
            continue;
        }

        let timestamp = value
            .get("timestamp")
            .and_then(parse_timestamp_value)
            .unwrap_or(fallback_timestamp);

        let model_raw = value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let model_id = pricing::aliases::resolve_alias(model_raw)
            .unwrap_or(model_raw)
            .to_string();

        let provider_id = provider_identity::inferred_provider_from_model(&model_id)
            .map(str::to_string)
            .unwrap_or_else(|| "opencodereview".to_string());

        let duration_ms = value
            .get("duration_ms")
            .and_then(Value::as_i64)
            .filter(|d| *d > 0);

        let dedup_key = format!(
            "opencodereview:{session_id}:{timestamp}:{model_id}:{}:{}:{}:{}",
            tokens.input, tokens.output, tokens.cache_read, tokens.cache_write,
        );
        if !seen.insert(dedup_key.clone()) {
            continue;
        }

        let mut msg = UnifiedMessage::new(
            "opencodereview",
            model_id,
            provider_id,
            &session_id,
            timestamp,
            tokens,
            0.0,
        );
        msg.dedup_key = Some(dedup_key);
        msg.duration_ms = duration_ms;

        if let Some(ws) = &workspace {
            if let Some(key) = super::normalize_workspace_key(ws) {
                msg.workspace_label = super::workspace_label_from_key(&key);
                msg.workspace_key = Some(key);
            }
        }

        messages.push(msg);
    }

    messages
}

fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn tokens_from_usage(usage: &Value) -> TokenBreakdown {
    TokenBreakdown {
        input: number_field(usage, "prompt_tokens"),
        output: number_field(usage, "completion_tokens"),
        cache_read: number_field(usage, "cache_read_tokens"),
        cache_write: number_field(usage, "cache_write_tokens"),
        reasoning: 0,
    }
}

fn number_field(value: &Value, field: &str) -> i64 {
    value
        .get(field)
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)))
        .unwrap_or(0)
        .max(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn parse_events(content: &str) -> Vec<UnifiedMessage> {
        let dir = TempDir::new().unwrap();
        let repo_dir = dir.path().join("test-repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        let path = repo_dir.join("test-session-123.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        parse_opencodereview_file(&path)
    }

    fn session_start(cwd: &str) -> String {
        format!(
            r#"{{"type":"session_start","sessionId":"test-session-123","timestamp":"2026-01-15T10:00:00Z","cwd":"{cwd}","model":"claude-sonnet-4-20250514"}}"#
        )
    }

    fn llm_response(
        timestamp: &str,
        model: &str,
        prompt: i64,
        completion: i64,
        cache_read: i64,
        cache_write: i64,
    ) -> String {
        format!(
            r#"{{"type":"llm_response","sessionId":"test-session-123","timestamp":"{timestamp}","model":"{model}","duration_ms":1500,"usage":{{"prompt_tokens":{prompt},"completion_tokens":{completion},"cache_read_tokens":{cache_read},"cache_write_tokens":{cache_write}}}}}"#
        )
    }

    #[test]
    fn parses_single_llm_response() {
        let content = format!(
            "{}\n{}\n",
            session_start("/home/user/project"),
            llm_response("2026-01-15T10:00:05Z", "claude-sonnet-4-20250514", 1000, 200, 500, 100),
        );
        let msgs = parse_events(&content);

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].client, "opencodereview");
        assert_eq!(msgs[0].tokens.input, 1000);
        assert_eq!(msgs[0].tokens.output, 200);
        assert_eq!(msgs[0].tokens.cache_read, 500);
        assert_eq!(msgs[0].tokens.cache_write, 100);
        assert_eq!(msgs[0].tokens.reasoning, 0);
        assert_eq!(msgs[0].duration_ms, Some(1500));
        assert_eq!(msgs[0].session_id, "test-session-123");
        assert!(msgs[0].workspace_key.is_some());
    }

    #[test]
    fn parses_multiple_responses() {
        let content = format!(
            "{}\n{}\n{}\n",
            session_start("/home/user/project"),
            llm_response("2026-01-15T10:00:05Z", "claude-sonnet-4-20250514", 1000, 200, 0, 0),
            llm_response("2026-01-15T10:01:00Z", "gpt-4o", 500, 100, 0, 0),
        );
        let msgs = parse_events(&content);
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn deduplicates_identical_records() {
        let resp = llm_response("2026-01-15T10:00:05Z", "claude-sonnet-4-20250514", 1000, 200, 0, 0);
        let content = format!(
            "{}\n{}\n{}\n",
            session_start("/home/user/project"),
            resp,
            resp,
        );
        let msgs = parse_events(&content);
        assert_eq!(msgs.len(), 1, "duplicate records should be collapsed");
    }

    #[test]
    fn skips_zero_token_records() {
        let content = format!(
            "{}\n{}\n",
            session_start("/home/user/project"),
            llm_response("2026-01-15T10:00:05Z", "claude-sonnet-4-20250514", 0, 0, 0, 0),
        );
        let msgs = parse_events(&content);
        assert_eq!(msgs.len(), 0, "zero-token records should be skipped");
    }

    #[test]
    fn works_without_session_start() {
        let content = format!(
            "{}\n",
            llm_response("2026-01-15T10:00:05Z", "claude-sonnet-4-20250514", 1000, 200, 0, 0),
        );
        let msgs = parse_events(&content);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].workspace_key.is_none());
    }

    #[test]
    fn session_id_derived_from_filename() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("my-unique-session.jsonl");
        let content = llm_response("2026-01-15T10:00:05Z", "gpt-4o", 100, 50, 0, 0);
        std::fs::write(&path, format!("{content}\n")).unwrap();

        let msgs = parse_opencodereview_file(&path);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].session_id, "my-unique-session");
    }
}
