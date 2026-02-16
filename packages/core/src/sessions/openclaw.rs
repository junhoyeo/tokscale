//! OpenClaw session parser
//!
//! Parses OpenClaw transcript JSONL files from agent directories.
//! Supports legacy sessions.json index parsing for compatibility.

use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct SessionIndex {
    #[serde(flatten)]
    sessions: HashMap<String, SessionEntry>,
}

#[derive(Debug, Deserialize)]
struct SessionEntry {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "sessionFile")]
    session_file: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenClawEntry {
    #[serde(rename = "type")]
    entry_type: String,
    message: Option<OpenClawMessage>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
    provider: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenClawMessage {
    role: Option<String>,
    usage: Option<OpenClawUsage>,
    timestamp: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct OpenClawUsage {
    input: Option<i64>,
    output: Option<i64>,
    #[serde(rename = "cacheRead")]
    cache_read: Option<i64>,
    #[serde(rename = "cacheWrite")]
    cache_write: Option<i64>,
    #[serde(rename = "totalTokens")]
    total_tokens: Option<i64>,
    cost: Option<OpenClawCost>,
}

#[derive(Debug, Deserialize)]
struct OpenClawCost {
    total: Option<f64>,
}

pub fn parse_openclaw_index(index_path: &Path) -> Vec<UnifiedMessage> {
    let data = match std::fs::read(index_path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut bytes = data;
    let index: SessionIndex = match simd_json::from_slice(&mut bytes) {
        Ok(i) => i,
        Err(_) => return Vec::new(),
    };

    let mut all_messages = Vec::new();
    let index_dir = index_path.parent().unwrap_or_else(|| Path::new("."));

    for (_key, entry) in index.sessions {
        let session_path = resolve_session_path(index_dir, &entry);
        if session_path.exists() {
            let messages = parse_openclaw_session(&session_path, &entry.session_id);
            all_messages.extend(messages);
        }
    }

    all_messages
}

pub fn parse_openclaw_transcript(transcript_path: &Path) -> Vec<UnifiedMessage> {
    let session_id = match transcript_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .filter(|stem| !stem.is_empty())
    {
        Some(id) => id,
        None => return Vec::new(),
    };

    parse_openclaw_session(transcript_path, &session_id)
}

fn resolve_session_path(index_dir: &Path, entry: &SessionEntry) -> PathBuf {
    match entry
        .session_file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(session_file) => {
            let path = Path::new(session_file);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                index_dir.join(path)
            }
        }
        None => index_dir.join(format!("{}.jsonl", entry.session_id)),
    }
}

fn parse_openclaw_session(session_path: &Path, session_id: &str) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(session_path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut current_model: Option<String> = None;
    let mut current_provider: Option<String> = None;

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
        let entry: OpenClawEntry = match simd_json::from_slice(&mut bytes) {
            Ok(e) => e,
            Err(_) => continue,
        };

        match entry.entry_type.as_str() {
            "model_change" => {
                if let Some(model) = entry.model_id {
                    current_model = Some(model);
                }
                if let Some(provider) = entry.provider {
                    current_provider = Some(provider);
                }
            }
            "message" => {
                if let Some(msg) = entry.message {
                    if msg.role.as_deref() != Some("assistant") {
                        continue;
                    }

                    let usage = match msg.usage {
                        Some(u) => u,
                        None => continue,
                    };

                    let model = match &current_model {
                        Some(m) => m.clone(),
                        None => continue,
                    };

                    let provider = current_provider
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    let timestamp = msg.timestamp.unwrap_or(0);
                    let cost = usage.cost.and_then(|c| c.total).unwrap_or(0.0);

                    messages.push(UnifiedMessage::new(
                        "openclaw",
                        model,
                        provider,
                        session_id.to_string(),
                        timestamp,
                        TokenBreakdown {
                            input: usage.input.unwrap_or(0).max(0),
                            output: usage.output.unwrap_or(0).max(0),
                            cache_read: usage.cache_read.unwrap_or(0).max(0),
                            cache_write: usage.cache_write.unwrap_or(0).max(0),
                            reasoning: 0,
                        },
                        cost.max(0.0),
                    ));
                }
            }
            _ => {}
        }
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_session(dir: &TempDir, filename: &str, content: &str) -> String {
        let path = dir.path().join(filename);
        let mut file = File::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        path.to_string_lossy().to_string()
    }

    #[test]
    fn test_parse_openclaw_session_with_model_change() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"model_change","id":"abc","provider":"openai-codex","modelId":"gpt-5.2"}
{"type":"message","id":"msg1","message":{"role":"assistant","content":[],"usage":{"input":100,"output":50,"cacheRead":200,"totalTokens":350,"cost":{"total":0.05}},"timestamp":1700000000000}}"#;

        let session_path = create_test_session(&dir, "session.jsonl", content);
        let messages = parse_openclaw_session(Path::new(&session_path), "test-session");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.2");
        assert_eq!(messages[0].provider_id, "openai-codex");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.cache_read, 200);
        assert_eq!(messages[0].cost, 0.05);
    }

    #[test]
    fn test_parse_openclaw_session_user_messages_ignored() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"model_change","provider":"anthropic","modelId":"claude-3.5-sonnet"}
{"type":"message","id":"msg1","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}
{"type":"message","id":"msg2","message":{"role":"assistant","content":[],"usage":{"input":50,"output":25},"timestamp":1700000000000}}"#;

        let session_path = create_test_session(&dir, "session.jsonl", content);
        let messages = parse_openclaw_session(Path::new(&session_path), "test-session");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 50);
    }

    #[test]
    fn test_parse_openclaw_session_no_model_change() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"message","id":"msg1","message":{"role":"assistant","content":[],"usage":{"input":100,"output":50},"timestamp":1700000000000}}"#;

        let session_path = create_test_session(&dir, "session.jsonl", content);
        let messages = parse_openclaw_session(Path::new(&session_path), "test-session");

        assert_eq!(messages.len(), 0);
    }

    #[test]
    fn test_parse_openclaw_transcript_derives_session_id_from_filename() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"model_change","provider":"openai-codex","modelId":"gpt-5.2"}
{"type":"message","id":"msg1","message":{"role":"assistant","content":[],"usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0},"timestamp":1700000000000}}"#;

        let session_path = create_test_session(&dir, "my-session-123.jsonl", content);
        let messages = parse_openclaw_transcript(Path::new(&session_path));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "my-session-123");
        assert_eq!(messages[0].model_id, "gpt-5.2");
        assert_eq!(messages[0].provider_id, "openai-codex");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 5);
    }

    fn create_test_index(dir: &TempDir, content: &str) -> PathBuf {
        let index_path = dir.path().join("sessions.json");
        let mut file = File::create(&index_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        index_path
    }

    #[test]
    fn test_parse_openclaw_index_absolute_session_file() {
        let dir = TempDir::new().unwrap();

        let session_content = r#"{"type":"model_change","provider":"anthropic","modelId":"claude-3.5-sonnet"}
{"type":"message","id":"msg1","message":{"role":"assistant","content":[],"usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0},"timestamp":1700000000000}}"#;
        let session_path = create_test_session(&dir, "session-abc.jsonl", session_content);

        let index_content = format!(
            r#"{{
            "agent:main:main": {{
                "sessionId": "abc-123",
                "sessionFile": "{}"
            }}
        }}"#,
            session_path.replace('\\', "\\\\")
        );
        let index_path = create_test_index(&dir, &index_content);

        let messages = parse_openclaw_index(&index_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-3.5-sonnet");
        assert_eq!(messages[0].session_id, "abc-123");
    }

    #[test]
    fn test_parse_openclaw_index_relative_session_file() {
        let dir = TempDir::new().unwrap();

        let session_content = r#"{"type":"model_change","provider":"anthropic","modelId":"claude-3.5-sonnet"}
{"type":"message","id":"msg1","message":{"role":"assistant","content":[],"usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0},"timestamp":1700000000000}}"#;
        create_test_session(&dir, "session-relative.jsonl", session_content);

        let index_content = r#"{
            "agent:main:main": {
                "sessionId": "relative-123",
                "sessionFile": "session-relative.jsonl"
            }
        }"#;
        let index_path = create_test_index(&dir, index_content);

        let messages = parse_openclaw_index(&index_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-3.5-sonnet");
        assert_eq!(messages[0].session_id, "relative-123");
    }

    #[test]
    fn test_parse_openclaw_index_missing_session_file_fallback() {
        let dir = TempDir::new().unwrap();

        let session_content = r#"{"type":"model_change","provider":"anthropic","modelId":"claude-3.5-sonnet"}
{"type":"message","id":"msg1","message":{"role":"assistant","content":[],"usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0},"timestamp":1700000000000}}"#;
        create_test_session(&dir, "fallback-123.jsonl", session_content);

        let index_content = r#"{
            "agent:main:main": {
                "sessionId": "fallback-123"
            }
        }"#;
        let index_path = create_test_index(&dir, index_content);

        let messages = parse_openclaw_index(&index_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-3.5-sonnet");
        assert_eq!(messages[0].session_id, "fallback-123");
    }
}
