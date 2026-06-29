//! ZCode (z.ai) session parser
//!
//! Parses JSONL transcripts from `~/.zcode/projects/<slug>/<session>.jsonl`.
//!
//! ZCode is Z.ai's Agentic Development Environment (ADE), an Electron-based
//! desktop IDE deeply adapted for the GLM-5.2 model family. Session
//! transcripts follow a JSONL format similar to Claude Code, with each line
//! containing role/content metadata. Token usage may be embedded per-message
//! from the Z.ai API response.
//!
//! When token usage is present in the transcript (fields like `usage`,
//! `token_usage`, or `input_tokens`/`output_tokens`), those authoritative
//! counts are used. When absent, tokens are estimated at ~4 chars/token,
//! consistent with tokscale's other estimated sources (see CommandCode, Kiro).

use super::utils::{file_modified_timestamp_ms, open_readonly_sqlite};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde::Deserialize;
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;

const CLIENT_ID: &str = "zcode";
const PROVIDER_ID: &str = "zhipu";
const UNKNOWN_MODEL: &str = "glm-5.2";

/// A single JSONL line in a ZCode session transcript.
#[derive(Debug, Deserialize)]
struct ZcodeEntry {
    role: Option<String>,
    content: Option<serde_json::Value>,
    #[serde(default)]
    usage: Option<ZcodeUsage>,
    #[serde(default)]
    token_usage: Option<ZcodeUsage>,
    model: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

/// Token usage block — field names follow the Z.ai / GLM API convention.
#[derive(Debug, Deserialize)]
struct ZcodeUsage {
    #[serde(alias = "input_tokens", alias = "prompt_tokens")]
    input: Option<i64>,
    #[serde(alias = "output_tokens", alias = "completion_tokens")]
    output: Option<i64>,
    #[serde(alias = "input_cache_read", alias = "cache_read_tokens")]
    cache_read: Option<i64>,
    #[serde(alias = "input_cache_creation", alias = "cache_write_tokens")]
    cache_write: Option<i64>,
    #[serde(default)]
    reasoning: Option<i64>,
}

impl ZcodeUsage {
    fn to_breakdown(&self) -> Option<TokenBreakdown> {
        let input = self.input.unwrap_or(0).max(0);
        let output = self.output.unwrap_or(0).max(0);
        let cache_read = self.cache_read.unwrap_or(0).max(0);
        let cache_write = self.cache_write.unwrap_or(0).max(0);
        let reasoning = self.reasoning.unwrap_or(0).max(0);

        if input + output + cache_read + cache_write + reasoning == 0 {
            return None;
        }

        Some(TokenBreakdown {
            input,
            output,
            cache_read,
            cache_write,
            reasoning,
        })
    }
}

pub fn parse_zcode_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };

    let fallback_timestamp = file_modified_timestamp_ms(path);
    let session_id_from_path = session_id_from_path(path);
    let workspace_key = workspace_key_from_path(path);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

    let mut messages = Vec::new();
    let mut session_id: Option<String> = None;
    let mut model_id: Option<String> = None;
    // Running char count for token estimation fallback.
    let mut context_chars: usize = 0;
    let mut pending_turn_start = false;
    let mut assistant_index = 0usize;

    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let entry = match serde_json::from_str::<ZcodeEntry>(trimmed) {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        if session_id.is_none() {
            if let Some(id) = entry.session_id.as_deref().filter(|id| !id.is_empty()) {
                session_id = Some(id.to_string());
            }
        }

        // Track the most-recently-seen model so per-entry pricing reflects the
        // model in effect at that point in the transcript. When the user
        // switches models mid-session, later messages must not be priced under
        // the first model.
        if let Some(m) = entry.model.as_deref().filter(|m| !m.is_empty()) {
            model_id = Some(canonicalize_model(m));
        }

        let resolved_model = model_id.as_deref().unwrap_or(UNKNOWN_MODEL).to_string();
        let chars = entry.content.as_ref().map(content_chars).unwrap_or(0);

        // Prefer authoritative token usage from the API. Choose the first block
        // that actually yields a breakdown, so an empty `usage` does not shadow
        // a populated `token_usage`.
        let breakdown_from_usage = entry
            .usage
            .as_ref()
            .and_then(|u| u.to_breakdown())
            .or_else(|| entry.token_usage.as_ref().and_then(|u| u.to_breakdown()));

        match entry.role.as_deref() {
            Some("assistant") => {
                let breakdown = if let Some(u) = breakdown_from_usage {
                    u
                } else {
                    // Estimate from content.
                    let input = estimate_tokens(context_chars);
                    let output = estimate_tokens(chars);
                    if input + output == 0 {
                        // Do not consume pending_turn_start here: no message is
                        // emitted, so the next real assistant message in this
                        // turn must keep its is_turn_start marker.
                        context_chars += chars;
                        continue;
                    }
                    TokenBreakdown {
                        input,
                        output,
                        cache_read: 0,
                        cache_write: 0,
                        reasoning: 0,
                    }
                };

                context_chars += chars;
                let resolved_session = session_id
                    .clone()
                    .unwrap_or_else(|| session_id_from_path.clone());
                let timestamp = entry
                    .timestamp
                    .as_deref()
                    .and_then(parse_rfc3339_ms)
                    .unwrap_or(fallback_timestamp);

                let mut message = UnifiedMessage::new_with_dedup(
                    CLIENT_ID,
                    resolved_model,
                    PROVIDER_ID,
                    resolved_session.clone(),
                    timestamp,
                    breakdown,
                    0.0,
                    Some(format!("{}:{}", resolved_session, assistant_index)),
                );
                message.message_count = 1;
                message.is_turn_start = pending_turn_start;
                message.set_workspace(workspace_key.clone(), workspace_label.clone());
                messages.push(message);

                assistant_index += 1;
                pending_turn_start = false;
            }
            Some("user") => {
                pending_turn_start = true;
                context_chars += chars;
            }
            _ => {
                context_chars += chars;
            }
        }
    }

    messages
}

pub fn parse_zcode_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite(db_path) else {
        return Vec::new();
    };

    let fallback_timestamp = file_modified_timestamp_ms(db_path);
    let modern_query = r#"
        SELECT
            mu.id,
            NULLIF(mu.session_id, ''),
            NULLIF(mu.turn_id, ''),
            NULLIF(mu.model_id, ''),
            mu.started_at,
            mu.completed_at,
            mu.duration_ms,
            mu.input_tokens,
            mu.output_tokens,
            mu.reasoning_tokens,
            mu.cache_read_input_tokens,
            mu.cache_creation_input_tokens,
            NULLIF(mu.agent, ''),
            NULLIF(mu.mode, ''),
            NULLIF(s.directory, ''),
            NULLIF(s.path, '')
        FROM model_usage mu
        LEFT JOIN session s ON s.id = mu.session_id
        WHERE COALESCE(mu.input_tokens, 0)
            + COALESCE(mu.output_tokens, 0)
            + COALESCE(mu.reasoning_tokens, 0)
            + COALESCE(mu.cache_read_input_tokens, 0)
            + COALESCE(mu.cache_creation_input_tokens, 0) > 0
        ORDER BY COALESCE(mu.completed_at, mu.started_at, 0), mu.id
    "#;
    let legacy_query = r#"
        SELECT
            mu.id,
            NULLIF(mu.session_id, ''),
            NULLIF(mu.turn_id, ''),
            NULLIF(mu.model_id, ''),
            mu.started_at,
            mu.completed_at,
            mu.duration_ms,
            mu.input_tokens,
            mu.output_tokens,
            mu.reasoning_tokens,
            mu.cache_read_input_tokens,
            mu.cache_creation_input_tokens,
            NULLIF(mu.agent, ''),
            NULLIF(mu.mode, ''),
            NULL,
            NULL
        FROM model_usage mu
        WHERE COALESCE(mu.input_tokens, 0)
            + COALESCE(mu.output_tokens, 0)
            + COALESCE(mu.reasoning_tokens, 0)
            + COALESCE(mu.cache_read_input_tokens, 0)
            + COALESCE(mu.cache_creation_input_tokens, 0) > 0
        ORDER BY COALESCE(mu.completed_at, mu.started_at, 0), mu.id
    "#;

    let mut stmt = match conn
        .prepare(modern_query)
        .or_else(|_| conn.prepare(legacy_query))
    {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };

    let rows = match stmt.query_map([], |row| {
        Ok(ZcodeUsageRow {
            id: row.get(0)?,
            session_id: row.get(1)?,
            turn_id: row.get(2)?,
            model_id: row.get(3)?,
            started_at: row.get(4)?,
            completed_at: row.get(5)?,
            duration_ms: row.get(6)?,
            input_tokens: row.get(7)?,
            output_tokens: row.get(8)?,
            reasoning_tokens: row.get(9)?,
            cache_read_input_tokens: row.get(10)?,
            cache_creation_input_tokens: row.get(11)?,
            agent: row.get(12)?,
            mode: row.get(13)?,
            session_directory: row.get(14)?,
            session_path: row.get(15)?,
        })
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };

    let mut messages = Vec::new();
    let mut seen_turns: HashSet<String> = HashSet::new();

    for row_result in rows {
        let row = match row_result {
            Ok(row) => row,
            Err(_) => continue,
        };

        let session_id = row.session_id.unwrap_or_else(|| "unknown".to_string());
        let model_id = row
            .model_id
            .as_deref()
            .map(canonicalize_model)
            .unwrap_or_else(|| UNKNOWN_MODEL.to_string());
        let timestamp = row
            .completed_at
            .or(row.started_at)
            .unwrap_or(fallback_timestamp);
        let tokens = TokenBreakdown {
            input: row.input_tokens.unwrap_or(0).max(0),
            output: row.output_tokens.unwrap_or(0).max(0),
            cache_read: row.cache_read_input_tokens.unwrap_or(0).max(0),
            cache_write: row.cache_creation_input_tokens.unwrap_or(0).max(0),
            reasoning: row.reasoning_tokens.unwrap_or(0).max(0),
        };

        if tokens.total() == 0 {
            continue;
        }

        let agent = row
            .agent
            .as_deref()
            .or(row.mode.as_deref())
            .map(str::to_string);
        let mut message = UnifiedMessage::new_with_agent(
            CLIENT_ID,
            model_id,
            PROVIDER_ID,
            session_id,
            timestamp,
            tokens,
            0.0,
            agent,
        );
        message.dedup_key = Some(format!("zcode-sqlite:{}", row.id));
        message.duration_ms = row.duration_ms.filter(|duration| *duration > 0);
        if let Some(turn_id) = row.turn_id.as_deref().filter(|id| !id.is_empty()) {
            message.is_turn_start = seen_turns.insert(turn_id.to_string());
        }

        let workspace_root = row.session_directory.or(row.session_path);
        let workspace_key = workspace_root.as_deref().and_then(normalize_workspace_key);
        let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
        message.set_workspace(workspace_key, workspace_label);

        messages.push(message);
    }

    messages
}

struct ZcodeUsageRow {
    id: String,
    session_id: Option<String>,
    turn_id: Option<String>,
    model_id: Option<String>,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    duration_ms: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    agent: Option<String>,
    mode: Option<String>,
    session_directory: Option<String>,
    session_path: Option<String>,
}

/// Canonicalize ZCode model ids. ZCode reports GLM model names in various
/// forms (e.g. "glm-5.2", "GLM-5.2", "glm-5-turbo"); normalize to lowercase
/// canonical form for pricing lookup.
fn canonicalize_model(model: &str) -> String {
    model.to_lowercase()
}

/// Char count of a message's `content` for token estimation.
fn content_chars(content: &serde_json::Value) -> usize {
    match content {
        serde_json::Value::Null => 0,
        serde_json::Value::String(s) if s.is_empty() => 0,
        serde_json::Value::Array(items) if items.is_empty() => 0,
        serde_json::Value::Object(map) if map.is_empty() => 0,
        _ => serde_json::to_string(content)
            .map(|serialized| serialized.chars().count())
            .unwrap_or(0),
    }
}

fn estimate_tokens(chars: usize) -> i64 {
    chars.div_ceil(4) as i64
}

fn parse_rfc3339_ms(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn workspace_key_from_path(path: &Path) -> Option<String> {
    path.parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .and_then(normalize_workspace_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use serde_json::json;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_session(dir: &TempDir, slug: &str, session: &str, jsonl: &str) -> std::path::PathBuf {
        let project_dir = dir.path().join("projects").join(slug);
        std::fs::create_dir_all(&project_dir).unwrap();
        let path = project_dir.join(format!("{session}.jsonl"));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(jsonl.as_bytes()).unwrap();
        path
    }

    fn create_zcode_sqlite_db(dir: &TempDir) -> std::path::PathBuf {
        let db_path = dir.path().join("db.sqlite");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE model_usage (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                turn_id TEXT,
                model_id TEXT,
                started_at INTEGER,
                completed_at INTEGER,
                duration_ms INTEGER,
                input_tokens INTEGER,
                output_tokens INTEGER,
                reasoning_tokens INTEGER,
                cache_read_input_tokens INTEGER,
                cache_creation_input_tokens INTEGER,
                agent TEXT,
                mode TEXT
            );
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                directory TEXT,
                path TEXT
            );
            "#,
        )
        .unwrap();
        db_path
    }

    #[test]
    fn test_parse_with_authoritative_usage() {
        let dir = TempDir::new().unwrap();
        let jsonl = format!(
            "{}\n{}",
            json!({
                "role": "user",
                "sessionId": "s1",
                "timestamp": "2026-06-20T10:00:00Z",
                "content": "hello"
            }),
            json!({
                "role": "assistant",
                "sessionId": "s1",
                "timestamp": "2026-06-20T10:00:05Z",
                "model": "glm-5.2",
                "content": "Hi there!",
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 50,
                    "input_cache_read": 20
                }
            }),
        );
        let path = write_session(&dir, "proj", "s1", &jsonl);
        let messages = parse_zcode_file(&path);

        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.client, "zcode");
        assert_eq!(msg.provider_id, "zhipu");
        assert_eq!(msg.model_id, "glm-5.2");
        assert_eq!(msg.session_id, "s1");
        assert_eq!(msg.tokens.input, 100);
        assert_eq!(msg.tokens.output, 50);
        assert_eq!(msg.tokens.cache_read, 20);
        assert!(msg.is_turn_start);
    }

    #[test]
    fn test_parse_with_estimated_tokens() {
        let dir = TempDir::new().unwrap();
        let user_content = json!([{"type": "text", "text": "12345678"}]);
        let asst_content = json!([{"type": "text", "text": "abcd"}]);
        let jsonl = format!(
            "{}\n{}",
            json!({"role": "user", "sessionId": "s2", "content": user_content}),
            json!({"role": "assistant", "sessionId": "s2", "content": asst_content}),
        );
        let path = write_session(&dir, "repo", "s2", &jsonl);
        let messages = parse_zcode_file(&path);

        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.model_id, "glm-5.2"); // default
        assert!(msg.tokens.input > 0);
        assert!(msg.tokens.output > 0);
        assert_eq!(msg.tokens.cache_read, 0);
    }

    #[test]
    fn test_canonicalize_model() {
        assert_eq!(canonicalize_model("GLM-5.2"), "glm-5.2");
        assert_eq!(canonicalize_model("GLM-5-Turbo"), "glm-5-turbo");
        assert_eq!(canonicalize_model("glm-5.2"), "glm-5.2");
    }

    #[test]
    fn test_content_chars_treats_empty_string_as_empty() {
        // Empty string content must count as 0 chars, consistent with null,
        // empty array, and empty object — otherwise serializing `""` yields 2
        // chars and produces a spurious estimated token.
        assert_eq!(content_chars(&json!("")), 0);
        assert_eq!(content_chars(&serde_json::Value::Null), 0);
        assert_eq!(content_chars(&json!([])), 0);
        assert_eq!(content_chars(&json!({})), 0);
        assert!(content_chars(&json!("abcd")) > 0);
    }

    #[test]
    fn test_empty_string_assistant_content_emits_no_message() {
        // An assistant entry with empty-string content and no token usage has
        // nothing to estimate, so it must take the zero-token continue path
        // instead of emitting a fake 1-token message.
        let dir = TempDir::new().unwrap();
        let jsonl = format!(
            "{}\n{}",
            json!({"role": "user", "sessionId": "s", "content": ""}),
            json!({"role": "assistant", "sessionId": "s", "content": ""}),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);
        let messages = parse_zcode_file(&path);

        assert!(messages.is_empty());
    }

    #[test]
    fn test_usage_with_alternative_field_names() {
        let dir = TempDir::new().unwrap();
        let jsonl = format!(
            "{}\n{}",
            json!({"role": "user", "sessionId": "s3", "content": "hi"}),
            json!({
                "role": "assistant",
                "sessionId": "s3",
                "content": "bye",
                "token_usage": {
                    "prompt_tokens": 200,
                    "completion_tokens": 100
                }
            }),
        );
        let path = write_session(&dir, "p", "s3", &jsonl);
        let messages = parse_zcode_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 200);
        assert_eq!(messages[0].tokens.output, 100);
    }

    #[test]
    fn test_cumulative_context_estimation() {
        let dir = TempDir::new().unwrap();
        let jsonl = concat!(
            r#"{"role":"user","sessionId":"s","content":[{"type":"text","text":"aaaa"}]}"#,
            "\n",
            r#"{"role":"assistant","sessionId":"s","content":[{"type":"text","text":"bbbb"}]}"#,
            "\n",
            r#"{"role":"user","sessionId":"s","content":[{"type":"text","text":"cccc"}]}"#,
            "\n",
            r#"{"role":"assistant","sessionId":"s","content":[{"type":"text","text":"dddd"}]}"#,
        );
        let path = write_session(&dir, "proj", "s", jsonl);
        let messages = parse_zcode_file(&path);

        assert_eq!(messages.len(), 2);
        assert!(messages[1].tokens.input > messages[0].tokens.input);
    }

    #[test]
    fn test_model_switch_mid_session() {
        let dir = TempDir::new().unwrap();
        let jsonl = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            json!({"role": "user", "sessionId": "s", "content": "hi"}),
            json!({
                "role": "assistant",
                "sessionId": "s",
                "model": "GLM-5.2",
                "content": "first",
                "usage": {"input_tokens": 10, "output_tokens": 5}
            }),
            json!({"role": "user", "sessionId": "s", "content": "switch"}),
            json!({
                "role": "assistant",
                "sessionId": "s",
                "model": "glm-5-turbo",
                "content": "second",
                "usage": {"input_tokens": 10, "output_tokens": 5}
            }),
            json!({"role": "user", "sessionId": "s", "content": "again"}),
            json!({
                "role": "assistant",
                "sessionId": "s",
                "content": "third",
                "usage": {"input_tokens": 10, "output_tokens": 5}
            }),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);
        let messages = parse_zcode_file(&path);

        assert_eq!(messages.len(), 3);
        // Each assistant message reflects the model in effect at that point.
        assert_eq!(messages[0].model_id, "glm-5.2");
        assert_eq!(messages[1].model_id, "glm-5-turbo");
        assert_ne!(messages[0].model_id, messages[1].model_id);
        // An entry with no `model` field inherits the most-recently-seen model.
        assert_eq!(messages[2].model_id, "glm-5-turbo");
    }

    #[test]
    fn test_empty_usage_falls_back_to_token_usage() {
        let dir = TempDir::new().unwrap();
        let jsonl = format!(
            "{}\n{}",
            json!({"role": "user", "sessionId": "s", "content": "hi"}),
            json!({
                "role": "assistant",
                "sessionId": "s",
                "content": "bye",
                "usage": {},
                "token_usage": {
                    "input_tokens": 321,
                    "output_tokens": 123,
                    "input_cache_read": 7
                }
            }),
        );
        let path = write_session(&dir, "p", "s", &jsonl);
        let messages = parse_zcode_file(&path);

        assert_eq!(messages.len(), 1);
        // Authoritative token_usage counts are used, NOT estimated.
        assert_eq!(messages[0].tokens.input, 321);
        assert_eq!(messages[0].tokens.output, 123);
        assert_eq!(messages[0].tokens.cache_read, 7);
    }

    #[test]
    fn test_parse_zcode_sqlite_model_usage() {
        let dir = TempDir::new().unwrap();
        let db_path = create_zcode_sqlite_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO session (id, directory, path) VALUES (?1, ?2, ?3)",
            params!["sess_1", "/Users/alice/work/demo", "/Users/alice/work/demo"],
        )
        .unwrap();
        conn.execute(
            r#"
            INSERT INTO model_usage (
                id, session_id, turn_id, model_id, started_at, completed_at,
                duration_ms, input_tokens, output_tokens, reasoning_tokens,
                cache_read_input_tokens, cache_creation_input_tokens, agent, mode
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
            params![
                "usage_1",
                "sess_1",
                "turn_1",
                "GLM-5.2",
                1_782_718_000_000_i64,
                1_782_718_001_000_i64,
                1000_i64,
                100_i64,
                20_i64,
                5_i64,
                7_i64,
                3_i64,
                "zcode-agent",
                "yolo",
            ],
        )
        .unwrap();

        let messages = parse_zcode_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.client, "zcode");
        assert_eq!(msg.provider_id, "zhipu");
        assert_eq!(msg.model_id, "glm-5.2");
        assert_eq!(msg.session_id, "sess_1");
        assert_eq!(msg.timestamp, 1_782_718_001_000_i64);
        assert_eq!(msg.duration_ms, Some(1000));
        assert_eq!(msg.tokens.input, 100);
        assert_eq!(msg.tokens.output, 20);
        assert_eq!(msg.tokens.reasoning, 5);
        assert_eq!(msg.tokens.cache_read, 7);
        assert_eq!(msg.tokens.cache_write, 3);
        assert_eq!(msg.agent.as_deref(), Some("zcode-agent"));
        assert_eq!(msg.workspace_key.as_deref(), Some("/Users/alice/work/demo"));
        assert_eq!(msg.workspace_label.as_deref(), Some("demo"));
        assert!(msg.is_turn_start);
        assert_eq!(msg.dedup_key.as_deref(), Some("zcode-sqlite:usage_1"));
    }

    #[test]
    fn test_parse_zcode_sqlite_marks_only_first_request_per_turn() {
        let dir = TempDir::new().unwrap();
        let db_path = create_zcode_sqlite_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        for (id, completed_at) in [("usage_1", 1_000_i64), ("usage_2", 2_000_i64)] {
            conn.execute(
                r#"
                INSERT INTO model_usage (
                    id, session_id, turn_id, model_id, completed_at,
                    input_tokens, output_tokens
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    id,
                    "sess_1",
                    "turn_1",
                    "glm-5.2",
                    completed_at,
                    10_i64,
                    1_i64
                ],
            )
            .unwrap();
        }

        let messages = parse_zcode_sqlite(&db_path);

        assert_eq!(messages.len(), 2);
        assert!(messages[0].is_turn_start);
        assert!(!messages[1].is_turn_start);
    }
}
