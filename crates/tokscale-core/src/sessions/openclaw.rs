//! OpenClaw session parser
//!
//! Parses OpenClaw transcript JSONL files from agent directories.
//! Supports legacy sessions.json index parsing for compatibility.

use super::utils::{for_each_json_line, lossy_lines, read_file_or_none, CamelUsage};
use super::UnifiedMessage;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

// Archived transcripts are immutable zstd files. Bound expansion before parsing
// so a corrupt archive cannot allocate its advertised decoded size.
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

fn read_archive(path: &Path, max_bytes: u64) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut decoder = zstd::stream::read::Decoder::new(file)?;
    decoder.window_log_max(26)?;
    let mut bytes = Vec::new();
    decoder.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(std::io::Error::other(
            "decoded transcript exceeds archive limit",
        ));
    }
    Ok(bytes)
}

fn for_each_transcript_line(path: &Path, sink: &mut dyn FnMut(usize, &str)) {
    if path.extension().is_none_or(|extension| extension != "zst") {
        for_each_json_line(path, sink);
        return;
    }

    match read_archive(path, MAX_ARCHIVE_BYTES) {
        Ok(bytes) => {
            for (index, line) in lossy_lines(bytes.as_slice()).enumerate() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    sink(index, trimmed);
                }
            }
        }
        Err(error) => tracing::warn!(path = %path.display(), %error, "Skipping OpenClaw archive"),
    }
}

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
    #[serde(rename = "customType")]
    custom_type: Option<String>,
    data: Option<OpenClawModelData>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
    provider: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenClawMessage {
    role: Option<String>,
    usage: Option<CamelUsage>,
    timestamp: Option<i64>,
    provider: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenClawModelData {
    provider: Option<String>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
}

pub fn parse_openclaw_index(index_path: &Path) -> Vec<UnifiedMessage> {
    let Some(data) = read_file_or_none(index_path) else {
        return Vec::new();
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
        .file_name()
        .and_then(|n| {
            n.to_string_lossy()
                .split_once(".jsonl")
                .map(|(id, _)| id.to_string())
        })
        .filter(|id| !id.is_empty())
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
    // Get file modification time as fallback for missing timestamps
    let file_mtime_ms = std::fs::metadata(session_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let mut messages = Vec::with_capacity(64);
    let mut current_model: Option<String> = None;
    let mut current_provider: Option<String> = None;
    let mut buffer = Vec::with_capacity(4096);

    for_each_transcript_line(session_path, &mut |_index, trimmed| {
        buffer.clear();
        buffer.extend_from_slice(trimmed.as_bytes());
        let entry: OpenClawEntry = match simd_json::from_slice(&mut buffer) {
            Ok(e) => e,
            Err(_) => return,
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
            "custom" => {
                if entry.custom_type.as_deref() != Some("model-snapshot") {
                    return;
                }

                if let Some(data) = entry.data {
                    if let Some(model) = data.model_id {
                        current_model = Some(model);
                    }
                    if let Some(provider) = data.provider {
                        current_provider = Some(provider);
                    }
                }
            }
            "message" => {
                if let Some(msg) = entry.message {
                    if msg.role.as_deref() != Some("assistant") {
                        return;
                    }

                    let usage = match msg.usage {
                        Some(u) => u,
                        None => return,
                    };

                    let model = msg
                        .model
                        .clone()
                        .filter(|m| !m.is_empty())
                        .or_else(|| current_model.clone().filter(|m| !m.is_empty()));
                    let provider = msg
                        .provider
                        .clone()
                        .filter(|p| !p.is_empty())
                        .or_else(|| current_provider.clone().filter(|p| !p.is_empty()))
                        .unwrap_or_else(|| "unknown".to_string());

                    let model = match model {
                        Some(model) => model,
                        None => return,
                    };

                    current_model = Some(model.clone());
                    current_provider = Some(provider.clone());
                    let timestamp = msg.timestamp.unwrap_or(file_mtime_ms);
                    let cost = usage.cost.as_ref().and_then(|c| c.total).unwrap_or(0.0);

                    messages.push(UnifiedMessage::new(
                        "openclaw",
                        model,
                        provider,
                        session_id.to_string(),
                        timestamp,
                        usage.to_breakdown(),
                        cost.max(0.0),
                    ));
                }
            }
            _ => {}
        }
    });

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
    fn compressed_archives_preserve_transcript_usage_and_session_identity() {
        let dir = TempDir::new().unwrap();
        let content = concat!(
            "\u{feff}{\"type\":\"model_change\",\"provider\":\"anthropic\",\"modelId\":\"claude-sonnet-4-6\"}\r\n",
            "\ninvalid json\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"usage\":{\"input\":100,\"output\":50,\"cacheRead\":200,\"cacheWrite\":10,\"cost\":{\"total\":0.05}},\"timestamp\":1788566869012}}\n",
        );
        let plain = create_test_session(&dir, "session.jsonl", content);
        let expected = parse_openclaw_transcript(Path::new(&plain));
        assert_eq!(expected.len(), 1);

        for filename in [
            "session.jsonl.zst",
            "session.jsonl.deleted.2026-09-05T00-00-00.000Z.nonce.zst",
            "session.jsonl.reset.2026-09-05T00-00-00.000Z.nonce.zst",
        ] {
            let path = dir.path().join(filename);
            std::fs::write(&path, zstd::encode_all(content.as_bytes(), 0).unwrap()).unwrap();
            assert_eq!(parse_openclaw_transcript(&path), expected, "{filename}");
        }
    }

    #[test]
    fn compressed_archive_enforces_decoded_limit() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.jsonl.zst");
        std::fs::write(&path, zstd::encode_all(&b"12345"[..], 0).unwrap()).unwrap();
        assert_eq!(read_archive(&path, 5).unwrap(), b"12345");
        assert!(read_archive(&path, 4).is_err());
    }

    #[test]
    fn invalid_or_truncated_compressed_archive_does_not_emit_partial_usage() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.jsonl.deleted.timestamp.zst");
        let content = br#"{"type":"message","message":{"role":"assistant","model":"example","usage":{"input":100},"timestamp":1700000000000}}"#;
        let mut bytes = zstd::encode_all(&content[..], 0).unwrap();
        bytes.pop();
        std::fs::write(&path, bytes).unwrap();
        assert!(parse_openclaw_transcript(&path).is_empty());
        std::fs::write(&path, b"not a zstd stream").unwrap();
        assert!(parse_openclaw_transcript(&path).is_empty());
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

    #[test]
    fn test_parse_openclaw_transcript_derives_session_id_from_archived_filename() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"model_change","provider":"openai-codex","modelId":"gpt-5.2"}
{"type":"message","id":"msg1","message":{"role":"assistant","content":[],"usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0},"timestamp":1700000000000}}"#;

        let session_path =
            create_test_session(&dir, "my-session-123.jsonl.deleted.1700000000000", content);
        let messages = parse_openclaw_transcript(Path::new(&session_path));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "my-session-123");
        assert_eq!(messages[0].model_id, "gpt-5.2");
        assert_eq!(messages[0].provider_id, "openai-codex");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 5);
    }

    #[test]
    fn test_parse_openclaw_transcript_derives_session_id_from_reset_filename() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"model_change","provider":"anthropic","modelId":"claude-opus-4-6"}
{"type":"message","id":"msg1","message":{"role":"assistant","content":[],"usage":{"input":10,"output":5,"cacheRead":1,"cacheWrite":2},"timestamp":1700000000000}}"#;

        let session_path = create_test_session(
            &dir,
            "my-session-123.jsonl.reset.2026-03-20T06-34-44.520Z",
            content,
        );
        let messages = parse_openclaw_transcript(Path::new(&session_path));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "my-session-123");
        assert_eq!(messages[0].model_id, "claude-opus-4-6");
        assert_eq!(messages[0].provider_id, "anthropic");
    }

    #[test]
    fn test_parse_openclaw_session_model_snapshot_updates_current_model() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"custom","customType":"model-snapshot","data":{"provider":"anthropic","modelId":"claude-opus-4-6"}}
{"type":"message","id":"msg1","message":{"role":"assistant","content":[],"usage":{"input":100,"output":50,"cacheRead":25,"cacheWrite":10},"timestamp":1700000000000}}"#;

        let session_path = create_test_session(&dir, "session.jsonl", content);
        let messages = parse_openclaw_session(Path::new(&session_path), "test-session");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-opus-4-6");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.cache_read, 25);
        assert_eq!(messages[0].tokens.cache_write, 10);
    }

    #[test]
    fn test_parse_openclaw_session_embedded_model_provider_without_model_change() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"message","id":"msg1","message":{"role":"assistant","provider":"anthropic","model":"claude-sonnet-4-6","content":[],"usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":5},"timestamp":1700000000000}}"#;

        let session_path = create_test_session(&dir, "session.jsonl", content);
        let messages = parse_openclaw_session(Path::new(&session_path), "test-session");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-4-6");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.cache_write, 5);
    }

    #[test]
    fn test_parse_openclaw_session_preserves_unknown_provider_fallback() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"model_change","modelId":"claude-sonnet-4-6"}
{"type":"message","id":"msg1","message":{"role":"assistant","content":[],"usage":{"input":10,"output":5},"timestamp":1700000000000}}"#;

        let session_path = create_test_session(&dir, "session.jsonl", content);
        let messages = parse_openclaw_session(Path::new(&session_path), "test-session");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-4-6");
        assert_eq!(messages[0].provider_id, "unknown");
    }

    #[test]
    fn test_parse_openclaw_session_empty_embedded_values_fall_back_to_current_model_state() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"model_change","provider":"anthropic","modelId":"claude-opus-4-6"}
{"type":"message","id":"msg1","message":{"role":"assistant","provider":"","model":"","content":[],"usage":{"input":10,"output":5},"timestamp":1700000000000}}"#;

        let session_path = create_test_session(&dir, "session.jsonl", content);
        let messages = parse_openclaw_session(Path::new(&session_path), "test-session");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-opus-4-6");
        assert_eq!(messages[0].provider_id, "anthropic");
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
