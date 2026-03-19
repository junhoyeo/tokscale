//! Qwen CLI session parser
//!
//! Parses JSONL files from ~/.qwen/projects/{projectPath}/chats/*.jsonl
//! Token data comes from assistant messages with usageMetadata field.

use super::utils::{file_modified_timestamp_ms, parse_timestamp_str};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Qwen CLI JSONL line structure
#[derive(Debug, Deserialize)]
struct QwenLine {
    #[serde(rename = "type")]
    msg_type: Option<String>,
    model: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,

    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<UsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct UsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<i64>,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<i64>,
    #[serde(rename = "thoughtsTokenCount")]
    thoughts_token_count: Option<i64>,
    #[serde(rename = "cachedContentTokenCount")]
    cached_content_token_count: Option<i64>,
}

/// Default model name when not specified
const DEFAULT_MODEL: &str = "unknown";
const DEFAULT_PROVIDER: &str = "qwen";

/// Extract session ID with fallback logic:
/// 1. Use JSON session_id if present and non-empty
/// 2. Otherwise derive from path including project name to avoid collisions
///
/// Path format: ~/.qwen/projects/{project}/chats/{filename}.jsonl
pub fn extract_session_id_with_fallback(path: &Path, json_session_id: Option<&str>) -> String {
    // Priority 1: Use JSON sessionId if present and non-empty
    if let Some(id) = json_session_id {
        if !id.is_empty() {
            return id.to_string();
        }
    }

    // Priority 2: Derive from path with project context
    // Extract project name from path structure: .../projects/{project}/chats/{file}.jsonl
    let filename = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    // Try to extract project name from the path
    let project_name = path
        .parent() // .../chats
        .and_then(|p| p.parent()) // .../projects/{project}
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    // Combine project and filename for unique session ID
    format!("{}-{}", project_name, filename)
}

/// Parse a Qwen CLI JSONL file
pub fn parse_qwen_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let file_mtime = file_modified_timestamp_ms(path);
    let (workspace_key, workspace_label) = qwen_workspace_from_path(path);

    let reader = BufReader::new(file);
    let mut messages: Vec<UnifiedMessage> = Vec::new();

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
        let qwen_line = match simd_json::from_slice::<QwenLine>(&mut bytes) {
            Ok(q) => q,
            Err(_) => continue,
        };

        // Only process assistant type messages with usageMetadata
        if qwen_line.msg_type.as_deref() != Some("assistant") {
            continue;
        }

        let usage = match qwen_line.usage_metadata {
            Some(u) => u,
            None => continue,
        };

        // Parse timestamp, fallback to file mtime
        let timestamp_ms = qwen_line
            .timestamp
            .and_then(|ts| parse_timestamp_str(&ts))
            .unwrap_or(file_mtime);

        // Extract token counts with defaults
        let input = usage.prompt_token_count.unwrap_or(0).max(0);
        let output = usage.candidates_token_count.unwrap_or(0).max(0);
        let reasoning = usage.thoughts_token_count.unwrap_or(0).max(0);
        let cache_read = usage.cached_content_token_count.unwrap_or(0).max(0);
        let cache_write = 0; // Qwen CLI doesn't report cache write tokens

        // Skip entries with zero tokens
        if input + output + cache_read + reasoning == 0 {
            continue;
        }

        // Use model from line or fallback to "unknown"
        let model = qwen_line.model.unwrap_or_else(|| DEFAULT_MODEL.to_string());

        // Resolve session ID: prefer JSON sessionId, fallback to path-derived
        let line_session_id =
            extract_session_id_with_fallback(path, qwen_line.session_id.as_deref());

        let mut unified = UnifiedMessage::new(
            "qwen",
            model,
            DEFAULT_PROVIDER,
            line_session_id,
            timestamp_ms,
            TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                reasoning,
            },
            0.0, // Cost calculated later by pricing resolver
        );
        unified.set_workspace(workspace_key.clone(), workspace_label.clone());
        messages.push(unified);
    }

    messages
}

fn qwen_workspace_from_path(path: &Path) -> (Option<String>, Option<String>) {
    let components: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect();

    for window in components.windows(4).rev() {
        if window[0] == "projects" && !window[1].is_empty() && window[2] == "chats" {
            let key = normalize_workspace_key(&window[1]);
            let label = key.as_deref().and_then(workspace_label_from_key);
            return (key, label);
        }
    }

    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;
    use tempfile::{NamedTempFile, TempDir};

    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    fn create_test_file_with_name(content: &str, filename: &str) -> (TempDir, std::path::PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir
            .path()
            .join(format!("test_project/chats/{}.jsonl", filename));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        (temp_dir, path)
    }

    fn create_project_test_file(
        content: &str,
        project: &str,
        filename: &str,
    ) -> (TempDir, std::path::PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir
            .path()
            .join(format!("projects/{project}/chats/{filename}.jsonl"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        (temp_dir, path)
    }

    #[test]
    fn test_parse_qwen_valid_assistant_message() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "d96bf338", "usageMetadata": {"promptTokenCount": 12414, "candidatesTokenCount": 76, "thoughtsTokenCount": 39, "cachedContentTokenCount": 0}}"#;
        let file = create_test_file(content);

        let messages = parse_qwen_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "qwen");
        assert_eq!(messages[0].model_id, "qwen3.5-plus");
        assert_eq!(messages[0].provider_id, "qwen");
        // Session ID comes from filename, not JSON content (temp file has random name)
        assert!(!messages[0].session_id.is_empty());
        assert_eq!(messages[0].tokens.input, 12414);
        assert_eq!(messages[0].tokens.output, 76);
        assert_eq!(messages[0].tokens.reasoning, 39);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.cache_write, 0);
        assert_eq!(messages[0].workspace_key, None);
        assert_eq!(messages[0].workspace_label, None);
    }

    #[test]
    fn test_parse_qwen_multi_turn() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "session1", "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 200, "thoughtsTokenCount": 10, "cachedContentTokenCount": 5}}
{"type": "assistant", "model": "qwen3-coder-plus", "timestamp": "2026-02-23T14:25:00.000Z", "sessionId": "session1", "usageMetadata": {"promptTokenCount": 300, "candidatesTokenCount": 400, "thoughtsTokenCount": 20, "cachedContentTokenCount": 10}}"#;
        let file = create_test_file(content);

        let messages = parse_qwen_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].model_id, "qwen3.5-plus");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 200);
        assert_eq!(messages[0].tokens.reasoning, 10);
        assert_eq!(messages[0].tokens.cache_read, 5);
        assert_eq!(messages[1].model_id, "qwen3-coder-plus");
        assert_eq!(messages[1].tokens.input, 300);
        assert_eq!(messages[1].tokens.output, 400);
        assert_eq!(messages[1].tokens.reasoning, 20);
        assert_eq!(messages[1].tokens.cache_read, 10);
    }

    #[test]
    fn test_workspace_metadata_from_qwen_project_path() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "d96bf338", "usageMetadata": {"promptTokenCount": 12414, "candidatesTokenCount": 76, "thoughtsTokenCount": 39, "cachedContentTokenCount": 0}}"#;
        let (_dir, path) = create_project_test_file(content, "test_project", "abc123");

        let messages = parse_qwen_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key, Some("test_project".to_string()));
        assert_eq!(
            messages[0].workspace_label,
            Some("test_project".to_string())
        );
    }

    #[test]
    fn test_workspace_metadata_ignores_unanchored_projects_segments() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "d96bf338", "usageMetadata": {"promptTokenCount": 12414, "candidatesTokenCount": 76, "thoughtsTokenCount": 39, "cachedContentTokenCount": 0}}"#;
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir
            .path()
            .join("projects/noise/not-chats/demo/.qwen/projects/real_project/chats/abc123.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();

        let messages = parse_qwen_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key.as_deref(), Some("real_project"));
        assert_eq!(messages[0].workspace_label.as_deref(), Some("real_project"));
    }

    #[test]
    fn test_parse_qwen_skip_non_assistant() {
        let content = r#"{"type": "user", "timestamp": "2026-02-23T14:24:50.000Z", "content": "Hello"}
{"type": "system", "timestamp": "2026-02-23T14:24:51.000Z", "subtype": "ui_telemetry"}
{"type": "tool_result", "timestamp": "2026-02-23T14:24:52.000Z", "result": "success"}"#;
        let file = create_test_file(content);

        let messages = parse_qwen_file(file.path());

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_qwen_empty_file() {
        let file = create_test_file("");

        let messages = parse_qwen_file(file.path());

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_qwen_malformed_lines() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "session1", "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 200, "thoughtsTokenCount": 10, "cachedContentTokenCount": 5}}
not valid json at all
{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:25:00.000Z", "sessionId": "session1", "usageMetadata": {"promptTokenCount": 300, "candidatesTokenCount": 400, "thoughtsTokenCount": 20, "cachedContentTokenCount": 10}}"#;
        let file = create_test_file(content);

        let messages = parse_qwen_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[1].tokens.input, 300);
    }

    #[test]
    fn test_parse_qwen_skips_zero_token_entries() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "session1", "usageMetadata": {"promptTokenCount": 0, "candidatesTokenCount": 0, "thoughtsTokenCount": 0, "cachedContentTokenCount": 0}}"#;
        let file = create_test_file(content);

        let messages = parse_qwen_file(file.path());

        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_qwen_with_cache_and_reasoning() {
        let content = r#"{"type": "assistant", "model": "qwen3-max-2026-01-23", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "session1", "usageMetadata": {"promptTokenCount": 1508, "candidatesTokenCount": 205, "thoughtsTokenCount": 50, "cachedContentTokenCount": 4864}}"#;
        let file = create_test_file(content);

        let messages = parse_qwen_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 1508);
        assert_eq!(messages[0].tokens.output, 205);
        assert_eq!(messages[0].tokens.reasoning, 50);
        assert_eq!(messages[0].tokens.cache_read, 4864);
        assert_eq!(messages[0].tokens.cache_write, 0);
    }

    #[test]
    fn test_parse_qwen_unknown_model_fallback() {
        let content = r#"{"type": "assistant", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "session1", "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 200, "thoughtsTokenCount": 10, "cachedContentTokenCount": 5}}"#;
        let file = create_test_file(content);

        let messages = parse_qwen_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "unknown");
        assert_eq!(messages[0].tokens.input, 100);
    }

    #[test]
    fn test_session_id_from_json_when_present() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "abc123def456", "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 200, "thoughtsTokenCount": 10, "cachedContentTokenCount": 5}}"#;
        let (_dir, path) = create_test_file_with_name(content, "json_present");

        let messages = parse_qwen_file(&path);

        assert_eq!(messages.len(), 1);
        // Should use the sessionId from JSON, not the filename
        assert_eq!(messages[0].session_id, "abc123def456");
    }

    #[test]
    fn test_session_id_fallback_when_empty_string() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "", "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 200, "thoughtsTokenCount": 10, "cachedContentTokenCount": 5}}"#;
        let (_dir, path) = create_test_file_with_name(content, "json_empty");

        let messages = parse_qwen_file(&path);

        assert_eq!(messages.len(), 1);
        // Should fallback to path-derived ID (not empty string)
        assert!(!messages[0].session_id.is_empty());
        assert_ne!(messages[0].session_id, "");
        // Verify it's not the JSON empty value
        assert_ne!(messages[0].session_id, "");
    }

    #[test]
    fn test_session_id_fallback_when_missing() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 200, "thoughtsTokenCount": 10, "cachedContentTokenCount": 5}}"#;
        let (_dir, path) = create_test_file_with_name(content, "json_missing");

        let messages = parse_qwen_file(&path);

        assert_eq!(messages.len(), 1);
        // Should fallback to path-derived ID
        assert!(!messages[0].session_id.is_empty());
    }

    #[test]
    fn test_session_id_fallback_when_null() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": null, "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 200, "thoughtsTokenCount": 10, "cachedContentTokenCount": 5}}"#;
        let (_dir, path) = create_test_file_with_name(content, "json_null");

        let messages = parse_qwen_file(&path);

        assert_eq!(messages.len(), 1);
        // Should fallback to path-derived ID
        assert!(!messages[0].session_id.is_empty());
        assert_ne!(messages[0].session_id, "null");
    }

    #[test]
    fn test_cross_project_session_id_uniqueness() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 200, "thoughtsTokenCount": 10, "cachedContentTokenCount": 5}}"#;

        // Create two files with same name in different projects
        let (_dir1, path1) = create_test_file_with_name(content, "session");

        // Manually create a second file in a different project
        let temp_dir = tempfile::tempdir().unwrap();
        let path2 = temp_dir.path().join("other_project/chats/session.jsonl");
        std::fs::create_dir_all(path2.parent().unwrap()).unwrap();
        let mut file2 = std::fs::File::create(&path2).unwrap();
        file2.write_all(content.as_bytes()).unwrap();

        let messages1 = parse_qwen_file(&path1);
        let messages2 = parse_qwen_file(&path2);

        assert_eq!(messages1.len(), 1);
        assert_eq!(messages2.len(), 1);

        // Session IDs should be different despite same filename
        assert_ne!(messages1[0].session_id, messages2[0].session_id);
    }

    #[test]
    fn test_extract_session_id_with_fallback_uses_json_value() {
        let path = Path::new("/home/user/.qwen/projects/myapp/chats/abc123.jsonl");
        let json_session_id = Some("json_session_456");

        let result = extract_session_id_with_fallback(path, json_session_id);

        assert_eq!(result, "json_session_456");
    }

    #[test]
    fn test_extract_session_id_with_fallback_empty_uses_path() {
        let path = Path::new("/home/user/.qwen/projects/myapp/chats/abc123.jsonl");
        let json_session_id = Some("");

        let result = extract_session_id_with_fallback(path, json_session_id);

        // Should use path-derived ID containing project and filename
        assert!(result.contains("myapp") || result.contains("abc123"));
    }

    #[test]
    fn test_extract_session_id_with_fallback_none_uses_path() {
        let path = Path::new("/home/user/.qwen/projects/myapp/chats/abc123.jsonl");
        let json_session_id: Option<&str> = None;

        let result = extract_session_id_with_fallback(path, json_session_id);

        // Should use path-derived ID containing project and filename
        assert!(result.contains("myapp") || result.contains("abc123"));
    }

    #[test]
    fn test_path_derived_session_id_includes_project() {
        let path = Path::new("/home/user/.qwen/projects/some-project/chats/chat-session.jsonl");
        let result = extract_session_id_with_fallback(path, None);

        // Should include both project name and filename stem
        assert!(
            result.contains("some-project"),
            "Session ID should contain project name"
        );
        assert!(
            result.contains("chat-session"),
            "Session ID should contain filename"
        );
    }

    #[test]
    fn test_multi_turn_same_session_id() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "shared_session", "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 200, "thoughtsTokenCount": 10, "cachedContentTokenCount": 5}}
{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:25:00.000Z", "sessionId": "shared_session", "usageMetadata": {"promptTokenCount": 300, "candidatesTokenCount": 400, "thoughtsTokenCount": 20, "cachedContentTokenCount": 10}}"#;
        let (_dir, path) = create_test_file_with_name(content, "multi");

        let messages = parse_qwen_file(&path);

        assert_eq!(messages.len(), 2);
        // Both messages should have the same session ID from JSON
        assert_eq!(messages[0].session_id, "shared_session");
        assert_eq!(messages[1].session_id, "shared_session");
    }

    #[test]
    fn test_mixed_session_id_in_file() {
        let content = r#"{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:24:56.857Z", "sessionId": "valid_id", "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 200, "thoughtsTokenCount": 10, "cachedContentTokenCount": 5}}
{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:25:00.000Z", "usageMetadata": {"promptTokenCount": 300, "candidatesTokenCount": 400, "thoughtsTokenCount": 20, "cachedContentTokenCount": 10}}
{"type": "assistant", "model": "qwen3.5-plus", "timestamp": "2026-02-23T14:26:00.000Z", "sessionId": "", "usageMetadata": {"promptTokenCount": 500, "candidatesTokenCount": 600, "thoughtsTokenCount": 30, "cachedContentTokenCount": 15}}"#;
        let (_dir, path) = create_test_file_with_name(content, "mixed");

        let messages = parse_qwen_file(&path);

        assert_eq!(messages.len(), 3);
        // First message uses JSON sessionId
        assert_eq!(messages[0].session_id, "valid_id");
        // Second message (no sessionId) uses fallback
        assert!(
            messages[1].session_id.contains("mixed")
                || messages[1].session_id.contains("test_project")
        );
        // Third message (empty sessionId) uses fallback
        assert!(
            messages[2].session_id.contains("mixed")
                || messages[2].session_id.contains("test_project")
        );
    }
}
