//! Jcode session parser
//!
//! Parses compact JSON session snapshots from `~/.jcode/sessions/session_*.json`.
//! Jcode stores authoritative assistant token usage on messages under
//! `token_usage`; user/tool messages without usage are skipped.

use super::utils::{file_modified_timestamp_ms, parse_timestamp_str};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct JcodeSession {
    id: Option<String>,
    provider_key: Option<String>,
    model: Option<String>,
    working_dir: Option<String>,
    #[serde(default)]
    messages: Vec<JcodeMessage>,
}

#[derive(Debug, Deserialize)]
struct JcodeJournalEntry {
    meta: Option<JcodeJournalMeta>,
    #[serde(default)]
    append_messages: Vec<JcodeMessage>,
}

#[derive(Debug, Deserialize)]
struct JcodeJournalMeta {
    provider_key: Option<String>,
    model: Option<String>,
    working_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JcodeMessage {
    id: Option<String>,
    role: Option<String>,
    timestamp: Option<String>,
    token_usage: Option<JcodeTokenUsage>,
    tool_duration_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct JcodeTokenUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    reasoning_output_tokens: Option<i64>,
}

fn provider_id(provider_key: Option<&str>) -> String {
    let provider = provider_key.unwrap_or("jcode").trim();
    let provider = if provider.is_empty() {
        "jcode".to_string()
    } else {
        provider.to_string()
    };
    provider_identity::canonical_provider(&provider).unwrap_or(provider)
}

fn model_id(model: Option<&str>) -> String {
    let model = model.unwrap_or("unknown").trim();
    if model.is_empty() {
        "unknown".to_string()
    } else {
        model.to_string()
    }
}

fn tokens_from_usage(usage: &JcodeTokenUsage) -> TokenBreakdown {
    TokenBreakdown {
        input: usage.input_tokens.unwrap_or(0).max(0),
        output: usage.output_tokens.unwrap_or(0).max(0),
        cache_read: usage.cache_read_input_tokens.unwrap_or(0).max(0),
        cache_write: usage.cache_creation_input_tokens.unwrap_or(0).max(0),
        reasoning: usage.reasoning_output_tokens.unwrap_or(0).max(0),
    }
}

#[derive(Debug, Clone)]
struct JcodeSessionContext {
    session_id: String,
    model: String,
    provider: String,
    workspace_key: Option<String>,
    workspace_label: Option<String>,
    pending_turn_start: bool,
}

impl JcodeSessionContext {
    fn new(session_id: String, session: &JcodeSession) -> Self {
        let workspace_key = session
            .working_dir
            .as_deref()
            .and_then(normalize_workspace_key);
        let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
        Self {
            session_id,
            model: model_id(session.model.as_deref()),
            provider: provider_id(session.provider_key.as_deref()),
            workspace_key,
            workspace_label,
            pending_turn_start: false,
        }
    }

    fn apply_meta(&mut self, meta: JcodeJournalMeta) {
        if let Some(model) = meta.model.as_deref() {
            self.model = model_id(Some(model));
        }
        if let Some(provider_key) = meta.provider_key.as_deref() {
            self.provider = provider_id(Some(provider_key));
        }
        if let Some(working_dir) = meta.working_dir.as_deref() {
            self.workspace_key = normalize_workspace_key(working_dir);
            self.workspace_label = self
                .workspace_key
                .as_deref()
                .and_then(workspace_label_from_key);
        }
    }
}

/// Resolve the append-only journal sidecar path for a Jcode session snapshot.
///
/// Jcode persists recent changes in `session_*.journal.jsonl` until the next
/// checkpoint rewrites the snapshot. This is the single source of truth for the
/// snapshot→journal mapping; `message_cache.rs` reuses it so the parser and the
/// cache-fingerprint logic can never disagree about which sidecar to read.
pub(crate) fn jcode_journal_path(path: &Path) -> std::path::PathBuf {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        let mut os = std::ffi::OsString::from(path.as_os_str());
        os.push(".journal.jsonl");
        return std::path::PathBuf::from(os);
    };
    let journal_name = file_name
        .strip_suffix(".json")
        .map(|stem| format!("{stem}.journal.jsonl"))
        .unwrap_or_else(|| format!("{file_name}.journal.jsonl"));
    path.with_file_name(journal_name)
}

fn parse_jcode_messages(
    messages: Vec<JcodeMessage>,
    context: &mut JcodeSessionContext,
    fallback_timestamp: i64,
    fallback_id_scope: &str,
) -> Vec<UnifiedMessage> {
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(ordinal, message)| {
            match message.role.as_deref() {
                Some("user") => context.pending_turn_start = true,
                Some("assistant") => {}
                _ => {}
            }
            let usage = message.token_usage?;
            let tokens = tokens_from_usage(&usage);
            if tokens.total() <= 0 {
                return None;
            }
            let timestamp = message
                .timestamp
                .as_deref()
                .and_then(parse_timestamp_str)
                .unwrap_or(fallback_timestamp);
            let message_id = message
                .id
                // Real Jcode messages include stable IDs; this fallback keeps
                // malformed/custom files parseable without colliding across
                // snapshot and journal batches.
                .unwrap_or_else(|| format!("{fallback_id_scope}:{ordinal}"));
            let dedup_key = format!("jcode:{}:{message_id}", context.session_id);
            let mut unified = UnifiedMessage::new_with_dedup(
                "jcode",
                context.model.clone(),
                context.provider.clone(),
                context.session_id.clone(),
                timestamp,
                tokens,
                0.0,
                Some(dedup_key),
            );
            unified.duration_ms = message.tool_duration_ms.filter(|duration| *duration > 0);
            if message.role.as_deref() == Some("assistant") && context.pending_turn_start {
                unified.is_turn_start = true;
                context.pending_turn_start = false;
            }
            unified.set_workspace(
                context.workspace_key.clone(),
                context.workspace_label.clone(),
            );
            Some(unified)
        })
        .collect()
}

pub fn parse_jcode_file(path: &Path) -> Vec<UnifiedMessage> {
    let mut data = match std::fs::read(path) {
        Ok(data) => data,
        Err(_) => return Vec::new(),
    };
    let session: JcodeSession = match simd_json::from_slice(&mut data) {
        Ok(session) => session,
        Err(_) => return Vec::new(),
    };

    let session_id = session.id.clone().unwrap_or_else(|| {
        path.file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_string()
    });
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let mut context = JcodeSessionContext::new(session_id, &session);
    let mut parsed = parse_jcode_messages(
        session.messages,
        &mut context,
        fallback_timestamp,
        "snapshot",
    );

    // Track where each dedup_key landed in `parsed`. The journal is written
    // after the snapshot, so a journal entry that repeats a snapshotted
    // message_id carries the *authoritative* (updated) token_usage. The
    // downstream dedup (`should_keep_deduped_message`) keeps the FIRST
    // occurrence per dedup_key, so emitting the snapshot then appending the
    // journal would silently drop the journal's correction. Instead, replace
    // the snapshot entry in place when the journal repeats its id — journal
    // wins, and the message_id still collapses to exactly one entry.
    // Downstream dedup is first-wins, so when the snapshot itself replays a
    // duplicate dedup_key we must map the key to its FIRST index — that's the
    // entry that survives dedup. Mapping to the last index would let a journal
    // update overwrite a row that is later discarded, preserving the stale
    // first snapshot row. `collect()` keeps the last insertion, so build the
    // map explicitly with `or_insert` to keep the first.
    let mut index_by_dedup_key: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (idx, message) in parsed.iter().enumerate() {
        if let Some(key) = message.dedup_key.clone() {
            index_by_dedup_key.entry(key).or_insert(idx);
        }
    }

    let journal_path = jcode_journal_path(path);
    if let Ok(file) = std::fs::File::open(&journal_path) {
        use std::io::{BufRead, BufReader};
        let journal_fallback_timestamp = file_modified_timestamp_ms(&journal_path);
        for (line_index, line) in BufReader::new(file).lines().enumerate() {
            let Ok(line) = line else {
                continue;
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(entry) = serde_json::from_str::<JcodeJournalEntry>(trimmed) else {
                continue;
            };
            if let Some(meta) = entry.meta {
                context.apply_meta(meta);
            }
            let journal_messages = parse_jcode_messages(
                entry.append_messages,
                &mut context,
                journal_fallback_timestamp,
                &format!("journal:{line_index}"),
            );
            for mut message in journal_messages {
                match message
                    .dedup_key
                    .as_ref()
                    .and_then(|key| index_by_dedup_key.get(key).copied())
                {
                    Some(existing_index) => {
                        // Preserve the snapshot's turn-start flag: turn structure
                        // is derived from snapshot ordering, while the journal only
                        // carries the corrected token_usage for this message_id.
                        message.is_turn_start = parsed[existing_index].is_turn_start;
                        parsed[existing_index] = message;
                    }
                    None => {
                        if let Some(key) = message.dedup_key.clone() {
                            index_by_dedup_key.insert(key, parsed.len());
                        }
                        parsed.push(message);
                    }
                }
            }
        }
    }

    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jcode_token_usage_messages() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            r#"{
  "id":"session_test",
  "provider_key":"cliproxyapi",
  "model":"claude-sonnet-4",
  "working_dir":"/Users/alice/project",
  "messages":[
    {"id":"user_1","role":"user","timestamp":"2026-06-16T12:00:00Z","content":[]},
    {"id":"assistant_1","role":"assistant","timestamp":"2026-06-16T12:00:01Z","token_usage":{"input_tokens":1200,"output_tokens":300,"cache_read_input_tokens":800,"cache_creation_input_tokens":50,"reasoning_output_tokens":25},"tool_duration_ms":1234}
  ]
}"#,
        )
        .unwrap();

        let messages = parse_jcode_file(file.path());
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "jcode");
        assert_eq!(message.session_id, "session_test");
        assert_eq!(message.model_id, "claude-sonnet-4");
        assert_eq!(message.provider_id, "cliproxyapi");
        assert_eq!(message.tokens.input, 1200);
        assert_eq!(message.tokens.cache_read, 800);
        assert_eq!(message.tokens.cache_write, 50);
        assert_eq!(message.tokens.output, 300);
        assert_eq!(message.tokens.reasoning, 25);
        assert_eq!(message.duration_ms, Some(1234));
        assert!(message.is_turn_start);
        assert_eq!(message.workspace_label.as_deref(), Some("project"));
    }

    #[test]
    fn marks_only_first_assistant_after_user_as_turn_start() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            r#"{
  "id":"session_turns",
  "messages":[
    {"id":"user_1","role":"user","timestamp":"2026-06-16T12:00:00Z"},
    {"id":"assistant_1","role":"assistant","timestamp":"2026-06-16T12:00:01Z","token_usage":{"input_tokens":100,"output_tokens":10}},
    {"id":"assistant_2","role":"assistant","timestamp":"2026-06-16T12:00:02Z","token_usage":{"input_tokens":50,"output_tokens":5}},
    {"id":"user_2","role":"user","timestamp":"2026-06-16T12:00:03Z"},
    {"id":"assistant_3","role":"assistant","timestamp":"2026-06-16T12:00:04Z","token_usage":{"input_tokens":25,"output_tokens":2}}
  ]
}"#,
        )
        .unwrap();

        let messages = parse_jcode_file(file.path());
        assert_eq!(messages.len(), 3);
        assert!(messages[0].is_turn_start);
        assert!(!messages[1].is_turn_start);
        assert!(messages[2].is_turn_start);
    }

    #[test]
    fn parses_jcode_journal_append_messages() {
        let dir = tempfile::TempDir::new().unwrap();
        let snapshot = dir.path().join("session_test.json");
        std::fs::write(
            &snapshot,
            r#"{
  "id":"session_test",
  "provider_key":"cliproxyapi",
  "model":"snapshot-model",
  "working_dir":"/Users/alice/project",
  "messages":[
    {"id":"assistant_snapshot","role":"assistant","timestamp":"2026-06-16T12:00:01Z","token_usage":{"input_tokens":100,"output_tokens":10}}
  ]
}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("session_test.journal.jsonl"),
            r#"{"meta":{"provider_key":"openai","model":"journal-model","working_dir":"/Users/alice/journal-project"},"append_messages":[{"id":"assistant_journal","role":"assistant","timestamp":"2026-06-16T12:00:02Z","token_usage":{"input_tokens":200,"output_tokens":20,"cache_read_input_tokens":50}}]}
"#,
        )
        .unwrap();

        let messages = parse_jcode_file(&snapshot);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].model_id, "snapshot-model");
        assert_eq!(messages[0].provider_id, "cliproxyapi");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[1].model_id, "journal-model");
        assert_eq!(messages[1].provider_id, "openai");
        assert_eq!(messages[1].tokens.input, 200);
        assert_eq!(messages[1].tokens.cache_read, 50);
        assert_eq!(
            messages[1].workspace_label.as_deref(),
            Some("journal-project")
        );
    }

    #[test]
    fn uses_journal_mtime_for_journal_messages_without_timestamps() {
        let dir = tempfile::TempDir::new().unwrap();
        let snapshot = dir.path().join("session_test.json");
        let journal = dir.path().join("session_test.journal.jsonl");
        std::fs::write(
            &snapshot,
            r#"{
  "id":"session_test",
  "model":"snapshot-model",
  "messages":[
    {"id":"assistant_snapshot","role":"assistant","token_usage":{"input_tokens":100,"output_tokens":10}}
  ]
}"#,
        )
        .unwrap();
        std::fs::write(
            &journal,
            r#"{"append_messages":[{"id":"assistant_journal","role":"assistant","token_usage":{"input_tokens":200,"output_tokens":20}}]}
"#,
        )
        .unwrap();

        let snapshot_time =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let journal_time =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_086_400);
        let snapshot_file = std::fs::OpenOptions::new()
            .write(true)
            .open(&snapshot)
            .unwrap();
        let Ok(()) = snapshot_file.set_modified(snapshot_time) else {
            return;
        };
        drop(snapshot_file);
        let journal_file = std::fs::OpenOptions::new()
            .write(true)
            .open(&journal)
            .unwrap();
        let Ok(()) = journal_file.set_modified(journal_time) else {
            return;
        };
        drop(journal_file);

        let snapshot_fallback = file_modified_timestamp_ms(&snapshot);
        let journal_fallback = file_modified_timestamp_ms(&journal);
        assert_ne!(snapshot_fallback, journal_fallback);

        let messages = parse_jcode_file(&snapshot);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].timestamp, snapshot_fallback);
        assert_eq!(messages[1].timestamp, journal_fallback);
    }

    #[test]
    fn journal_update_for_snapshotted_id_wins_and_collapses_to_one_entry() {
        // The snapshot persists an in-flight assistant message with partial
        // token_usage; the next checkpoint hasn't rewritten the snapshot yet, so
        // the journal carries the SAME message_id with the final (larger)
        // token_usage. The journal value must win and the message_id must
        // collapse to exactly one entry (no double-counting).
        let dir = tempfile::TempDir::new().unwrap();
        let snapshot = dir.path().join("session_test.json");
        std::fs::write(
            &snapshot,
            r#"{
  "id":"session_test",
  "model":"snapshot-model",
  "messages":[
    {"id":"assistant_live","role":"assistant","timestamp":"2026-06-16T12:00:01Z","token_usage":{"input_tokens":100,"output_tokens":10}}
  ]
}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("session_test.journal.jsonl"),
            r#"{"append_messages":[{"id":"assistant_live","role":"assistant","timestamp":"2026-06-16T12:00:05Z","token_usage":{"input_tokens":900,"output_tokens":300,"cache_read_input_tokens":40}}]}
"#,
        )
        .unwrap();

        let messages = parse_jcode_file(&snapshot);
        // Exactly one entry for the repeated id (no double-counting).
        assert_eq!(messages.len(), 1);
        // Journal value wins over the stale snapshot value.
        assert_eq!(messages[0].tokens.input, 900);
        assert_eq!(messages[0].tokens.output, 300);
        assert_eq!(messages[0].tokens.cache_read, 40);
    }

    #[test]
    fn journal_update_replaces_value_after_downstream_dedup() {
        // Mirror the lib.rs dedup contract: should_keep_deduped_message keeps the
        // FIRST occurrence per dedup_key. The in-parser merge must therefore have
        // already replaced the snapshot value in place, so the surviving entry
        // carries the journal's token_usage even after downstream dedup.
        use std::collections::HashSet;

        let dir = tempfile::TempDir::new().unwrap();
        let snapshot = dir.path().join("session_test.json");
        std::fs::write(
            &snapshot,
            r#"{
  "id":"session_test",
  "model":"snapshot-model",
  "messages":[
    {"id":"assistant_live","role":"assistant","timestamp":"2026-06-16T12:00:01Z","token_usage":{"input_tokens":100,"output_tokens":10}}
  ]
}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("session_test.journal.jsonl"),
            r#"{"append_messages":[{"id":"assistant_live","role":"assistant","timestamp":"2026-06-16T12:00:05Z","token_usage":{"input_tokens":900,"output_tokens":300}}]}
"#,
        )
        .unwrap();

        let messages = parse_jcode_file(&snapshot);
        let mut seen: HashSet<String> = HashSet::new();
        let deduped: Vec<_> = messages
            .into_iter()
            .filter(|message| {
                message
                    .dedup_key
                    .as_ref()
                    .is_none_or(|key| seen.insert(key.clone()))
            })
            .collect();
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].tokens.input, 900);
        assert_eq!(deduped[0].tokens.output, 300);
    }

    #[test]
    fn parses_timezone_less_timestamps_instead_of_falling_back_to_mtime() {
        // Jcode (and proxy variants) sometimes emit naive ISO-8601 datetimes
        // with no `Z`/offset. These must parse as UTC, not collapse to the
        // file mtime (which would scatter the message into the wrong bucket).
        let dir = tempfile::TempDir::new().unwrap();
        let snapshot = dir.path().join("session_test.json");
        std::fs::write(
            &snapshot,
            r#"{
  "id":"session_test",
  "model":"snapshot-model",
  "messages":[
    {"id":"assistant_naive","role":"assistant","timestamp":"2026-06-16T12:00:00","token_usage":{"input_tokens":100,"output_tokens":10}}
  ]
}"#,
        )
        .unwrap();

        // Force a clearly-different mtime so a fallback would be detectable.
        let mtime =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&snapshot) {
            let _ = file.set_modified(mtime);
        }
        let fallback = file_modified_timestamp_ms(&snapshot);

        let messages = parse_jcode_file(&snapshot);
        assert_eq!(messages.len(), 1);
        // "2026-06-16T12:00:00" UTC == 1781611200000 ms.
        assert_eq!(messages[0].timestamp, 1_781_611_200_000);
        assert_ne!(messages[0].timestamp, fallback);
    }

    #[test]
    fn skips_unreadable_journal_lines_and_continues_parsing() {
        let dir = tempfile::TempDir::new().unwrap();
        let snapshot = dir.path().join("session_test.json");
        std::fs::write(
            &snapshot,
            r#"{
  "id":"session_test",
  "model":"snapshot-model",
  "messages":[]
}"#,
        )
        .unwrap();

        let mut journal = Vec::new();
        journal.extend_from_slice(
            br#"{"append_messages":[{"id":"assistant_before","role":"assistant","timestamp":"2026-06-16T12:00:01Z","token_usage":{"input_tokens":100,"output_tokens":10}}]}
"#,
        );
        journal.extend_from_slice(b"\xff\n");
        journal.extend_from_slice(
            br#"{"append_messages":[{"id":"assistant_after","role":"assistant","timestamp":"2026-06-16T12:00:02Z","token_usage":{"input_tokens":200,"output_tokens":20}}]}
"#,
        );
        std::fs::write(dir.path().join("session_test.journal.jsonl"), journal).unwrap();

        let messages = parse_jcode_file(&snapshot);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[1].tokens.input, 200);
    }
}
