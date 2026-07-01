use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::Path;

use super::sessions::utils::open_readonly_sqlite;

const MAX_CONTENT_CHARS: usize = 1000;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionContent {
    pub session_id: String,
    pub first_user_message: Option<String>,
    pub client: String,
}

pub fn extract_opencode_content(db_path: &Path, session_id: &str) -> Option<SessionContent> {
    let conn = open_readonly_sqlite(db_path)?;

    let sql = r#"
        SELECT json_extract(data, '$.parts') as parts
        FROM message
        WHERE session_id = ?1
        AND json_extract(data, '$.role') = 'user'
        ORDER BY CAST(json_extract(data, '$.time.created') AS REAL) ASC
        LIMIT 1
    "#;

    let first_user: Option<String> = conn
        .query_row(sql, [session_id], |row| row.get::<_, Option<String>>(0))
        .ok()
        .flatten();

    let text = first_user.and_then(|parts_json| {
        let parts: Vec<Value> = serde_json::from_str(&parts_json).ok()?;
        let mut combined = String::new();
        for part in &parts {
            if let Some(t) = part.get("content").and_then(|c| c.as_str()) {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(t);
            } else if let Some(t) = part.as_str() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(t);
            }
        }
        if combined.is_empty() {
            None
        } else {
            Some(truncate(&combined, MAX_CONTENT_CHARS))
        }
    });

    let text = text.or_else(|| {
        let sql_fallback = r#"
            SELECT json_extract(data, '$.content') as content
            FROM message
            WHERE session_id = ?1
            AND json_extract(data, '$.role') = 'user'
            ORDER BY CAST(json_extract(data, '$.time.created') AS REAL) ASC
            LIMIT 1
        "#;
        conn.query_row(sql_fallback, [session_id], |row| {
            row.get::<_, Option<String>>(0)
        })
        .ok()
        .flatten()
        .map(|s| truncate(&s, MAX_CONTENT_CHARS))
    });

    Some(SessionContent {
        session_id: session_id.to_string(),
        first_user_message: text,
        client: "opencode".to_string(),
    })
}

pub fn extract_claudecode_content(jsonl_path: &Path, session_id: &str) -> Option<SessionContent> {
    let file = std::fs::File::open(jsonl_path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if !trimmed.contains("\"human\"") && !trimmed.contains("\"user\"") {
            continue;
        }

        let value: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let entry_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if entry_type != "human" && entry_type != "user" {
            continue;
        }

        let text = extract_text_from_claude_message(&value);
        if let Some(t) = text {
            if !t.trim().is_empty() {
                return Some(SessionContent {
                    session_id: session_id.to_string(),
                    first_user_message: Some(truncate(&t, MAX_CONTENT_CHARS)),
                    client: "claude".to_string(),
                });
            }
        }
    }

    Some(SessionContent {
        session_id: session_id.to_string(),
        first_user_message: None,
        client: "claude".to_string(),
    })
}

/// System-injected `user_message` bodies the Codex harness writes alongside real
/// human turns. They open with one of these tags (after trimming) and must not be
/// reported as the user's first prompt. Mirrors the detection in
/// `sessions::codex` — matching specific tags (not any leading `<`) avoids
/// dropping legitimate prompts that happen to start with markup.
const CODEX_SYSTEM_INJECTED_PREFIXES: [&str; 3] = [
    "<environment_context>",
    "<system-reminder>",
    "<user_instructions>",
];

fn codex_message_is_human_turn(message: &str) -> bool {
    let trimmed = message.trim_start();
    !CODEX_SYSTEM_INJECTED_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

pub fn extract_codex_content(jsonl_path: &Path, session_id: &str) -> Option<SessionContent> {
    let file = std::fs::File::open(jsonl_path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let value: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let entry_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let payload = value.get("payload");
        let payload_type = payload
            .and_then(|p| p.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("");

        // Current on-disk format: an `event_msg` entry whose payload is a
        // `user_message`, with the human text in `payload.message`.
        let is_user_message = (entry_type == "event_msg" && payload_type == "user_message")
            // Tolerate older/top-level shapes that may still appear in the wild.
            || entry_type == "user_message"
            || entry_type == "input";

        if !is_user_message {
            continue;
        }

        let text = payload
            .and_then(|p| p.get("message"))
            .or_else(|| value.get("content"))
            .or_else(|| value.get("text"))
            .or_else(|| payload.and_then(|p| p.get("content")))
            .and_then(|v| v.as_str());

        if let Some(t) = text {
            // Skip harness-injected context blocks; only real human turns count.
            if !t.trim().is_empty() && codex_message_is_human_turn(t) {
                return Some(SessionContent {
                    session_id: session_id.to_string(),
                    first_user_message: Some(truncate(t, MAX_CONTENT_CHARS)),
                    client: "codex".to_string(),
                });
            }
        }
    }

    Some(SessionContent {
        session_id: session_id.to_string(),
        first_user_message: None,
        client: "codex".to_string(),
    })
}

/// Pull the first non-empty user message out of a parsed Gemini message object.
/// Gemini chat recordings store user turns as `{"type":"user","content":"..."}`;
/// some variants use `text` instead of `content` or `role` instead of `type`.
fn gemini_user_text(msg: &Value) -> Option<String> {
    let role = msg
        .get("type")
        .or_else(|| msg.get("role"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    if role != "user" && role != "human" {
        return None;
    }
    let text = msg
        .get("content")
        .or_else(|| msg.get("text"))
        .and_then(|v| v.as_str())?;
    if text.trim().is_empty() {
        None
    } else {
        Some(truncate(text, MAX_CONTENT_CHARS))
    }
}

pub fn extract_gemini_content(json_path: &Path, session_id: &str) -> Option<SessionContent> {
    let none = || SessionContent {
        session_id: session_id.to_string(),
        first_user_message: None,
        client: "gemini".to_string(),
    };
    let found = |text: String| SessionContent {
        session_id: session_id.to_string(),
        first_user_message: Some(text),
        client: "gemini".to_string(),
    };

    let content = std::fs::read_to_string(json_path).ok()?;

    // Chat-recording format: a single JSON document with a `messages` array.
    if let Ok(value) = serde_json::from_str::<Value>(&content) {
        if let Some(messages) = value.get("messages").and_then(|m| m.as_array()) {
            for msg in messages {
                if let Some(text) = gemini_user_text(msg) {
                    return Some(found(text));
                }
            }
            return Some(none());
        }
    }

    // Headless / line-delimited JSONL: scan each line for a user turn. Telemetry
    // (`init`/`result`/token-count) lines carry no user text and are skipped.
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some(text) = gemini_user_text(&value) {
            return Some(found(text));
        }
        // Some recordings nest the turns under a `messages` array per line.
        if let Some(messages) = value.get("messages").and_then(|m| m.as_array()) {
            for msg in messages {
                if let Some(text) = gemini_user_text(msg) {
                    return Some(found(text));
                }
            }
        }
    }

    Some(none())
}

pub fn metadata_only_content(session_id: &str, client: &str) -> SessionContent {
    SessionContent {
        session_id: session_id.to_string(),
        first_user_message: None,
        client: client.to_string(),
    }
}

/// Dispatch to the correct per-client extractor for `client`, reading the
/// session's actual file(s) from `candidate_paths`, and return the first result
/// that yields a real `first_user_message`.
///
/// Falls back to [`metadata_only_content`] — never an error or panic — when:
/// - the client has no dedicated extractor,
/// - `candidate_paths` is empty,
/// - every candidate file is missing/unreadable/unparseable, or
/// - no candidate produced a non-empty first user message.
///
/// For file-keyed clients (claude, codex, gemini) each candidate is the
/// session's own transcript file. For opencode the session lives as rows inside
/// a shared SQLite database, so `candidate_paths` should carry the opencode
/// database(s) and the extractor selects the session by `session_id` internally.
pub fn extract_session_content(
    client: &str,
    session_id: &str,
    candidate_paths: &[std::path::PathBuf],
) -> SessionContent {
    let extractor: fn(&Path, &str) -> Option<SessionContent> = match client {
        "opencode" => extract_opencode_content,
        "claude" => extract_claudecode_content,
        "codex" => extract_codex_content,
        "gemini" => extract_gemini_content,
        // Unknown/unsupported client: no dedicated extractor.
        _ => return metadata_only_content(session_id, client),
    };

    for path in candidate_paths {
        if let Some(content) = extractor(path, session_id) {
            // A real extractor can still return `Some` with `first_user_message:
            // None` (or an empty/whitespace-only string — e.g. the file parsed
            // but held no usable user message); keep scanning the remaining
            // candidates for one that yields real text.
            if content
                .first_user_message
                .as_deref()
                .is_some_and(|m| !m.trim().is_empty())
            {
                return content;
            }
        }
    }

    metadata_only_content(session_id, client)
}

fn extract_text_from_claude_message(value: &Value) -> Option<String> {
    if let Some(content) = value.get("message").and_then(|m| m.get("content")) {
        if let Some(arr) = content.as_array() {
            let mut combined = String::new();
            for block in arr {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str(t);
                }
            }
            if !combined.is_empty() {
                return Some(combined);
            }
        }
        if let Some(s) = content.as_str() {
            return Some(s.to_string());
        }
    }

    if let Some(content) = value.get("content") {
        if let Some(s) = content.as_str() {
            return Some(s.to_string());
        }
        if let Some(arr) = content.as_array() {
            let mut combined = String::new();
            for block in arr {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str(t);
                }
            }
            if !combined.is_empty() {
                return Some(combined);
            }
        }
    }

    None
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let boundary = s
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}...", &s[..boundary])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn extract_session_content_dispatches_to_real_claude_extractor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sess.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Add a CLI flag"}]}}
"#,
        )
        .unwrap();

        let content = extract_session_content("claude", "sess", &[path]);
        assert_eq!(
            content.first_user_message.as_deref(),
            Some("Add a CLI flag")
        );
        assert_eq!(content.client, "claude");
    }

    #[test]
    fn extract_session_content_unknown_client_is_metadata_only() {
        let content = extract_session_content("totally-unknown", "sess", &[PathBuf::from("/nope")]);
        assert!(content.first_user_message.is_none());
        assert_eq!(content.client, "totally-unknown");
    }

    #[test]
    fn extract_session_content_no_candidates_is_metadata_only() {
        let content = extract_session_content("claude", "sess", &[]);
        assert!(content.first_user_message.is_none());
        assert_eq!(content.client, "claude");
    }

    #[test]
    fn extract_session_content_unreadable_file_does_not_panic() {
        // Missing/unparseable candidate must degrade gracefully, never panic.
        let content = extract_session_content(
            "codex",
            "sess",
            &[PathBuf::from("/definitely/missing.jsonl")],
        );
        assert!(content.first_user_message.is_none());
        assert_eq!(content.client, "codex");
    }

    #[test]
    fn extract_codex_content_parses_current_event_msg_format() {
        // Current on-disk shape: event_msg / payload.type == user_message, with
        // the human text in payload.message.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"event_msg","payload":{"type":"environment_context"}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"<environment_context>cwd=/tmp</environment_context>"}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"Refactor the parser"}}"#,
                "\n",
            ),
        )
        .unwrap();

        let content = extract_codex_content(&path, "sess").unwrap();
        // The system-injected user_message must be skipped; the real prompt wins.
        assert_eq!(
            content.first_user_message.as_deref(),
            Some("Refactor the parser")
        );
        assert_eq!(content.client, "codex");
    }

    #[test]
    fn extract_codex_content_skips_only_injected_returns_none() {
        // A transcript with only harness-injected context yields no human turn.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"<system-reminder>be concise</system-reminder>"}}"#,
                "\n",
            ),
        )
        .unwrap();

        let content = extract_codex_content(&path, "sess").unwrap();
        assert!(content.first_user_message.is_none());
    }

    #[test]
    fn extract_gemini_content_parses_chat_recording_format() {
        // Chat recording: single JSON doc with a messages array; user turns use
        // {"type":"user","content":"..."}.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-2026.json");
        std::fs::write(
            &path,
            r#"{"sessionId":"b8d9ab56","messages":[{"type":"user","content":"Review the patch"},{"type":"gemini","content":"sure"}]}"#,
        )
        .unwrap();

        let content = extract_gemini_content(&path, "b8d9ab56").unwrap();
        assert_eq!(
            content.first_user_message.as_deref(),
            Some("Review the patch")
        );
        assert_eq!(content.client, "gemini");
    }

    #[test]
    fn extract_gemini_content_empty_user_text_yields_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-2026.json");
        std::fs::write(
            &path,
            r#"{"sessionId":"x","messages":[{"type":"user","content":"   "}]}"#,
        )
        .unwrap();

        let content = extract_gemini_content(&path, "x").unwrap();
        assert!(content.first_user_message.is_none());
    }

    #[test]
    fn extract_session_content_empty_message_keeps_scanning_to_real_text() {
        // First candidate parses but its only user message is whitespace; the
        // dispatcher must not accept it as success and must fall through to the
        // second candidate that holds real text.
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.jsonl");
        std::fs::write(
            &empty,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"   "}]}}
"#,
        )
        .unwrap();
        let real = dir.path().join("real.jsonl");
        std::fs::write(
            &real,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Real prompt"}]}}
"#,
        )
        .unwrap();

        let content = extract_session_content("claude", "sess", &[empty, real]);
        assert_eq!(content.first_user_message.as_deref(), Some("Real prompt"));
    }
}
