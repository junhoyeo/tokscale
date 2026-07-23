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

use super::utils::{back_anchor_timestamp, file_modified_timestamp_ms, open_readonly_sqlite};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde::Deserialize;
use std::collections::HashMap;
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
    #[serde(alias = "input_tokens", alias = "prompt_tokens", alias = "inputTokens")]
    input: Option<i64>,
    #[serde(
        alias = "output_tokens",
        alias = "completion_tokens",
        alias = "outputTokens"
    )]
    output: Option<i64>,
    #[serde(
        alias = "input_cache_read",
        alias = "cache_read_tokens",
        alias = "cacheReadTokens"
    )]
    cache_read: Option<i64>,
    #[serde(
        alias = "input_cache_creation",
        alias = "cache_write_tokens",
        alias = "cacheCreationTokens"
    )]
    cache_write: Option<i64>,
    #[serde(default, alias = "reasoningTokens")]
    reasoning: Option<i64>,
    #[serde(default, alias = "totalTokens")]
    total: Option<i64>,
}

impl ZcodeUsage {
    fn to_breakdown(&self) -> Option<TokenBreakdown> {
        let raw_input = self.input.unwrap_or(0).max(0);
        let raw_output = self.output.unwrap_or(0).max(0);
        let raw_cache_read = self.cache_read.unwrap_or(0).max(0);
        let raw_cache_write = self.cache_write.unwrap_or(0).max(0);
        let raw_reasoning = self.reasoning.unwrap_or(0).max(0);

        if raw_input + raw_output + raw_cache_read + raw_cache_write + raw_reasoning == 0 {
            return None;
        }

        let (net_input, net_output) = normalize_zcode_input_and_output(
            raw_input,
            raw_output,
            raw_cache_read,
            raw_cache_write,
            raw_reasoning,
            self.total,
        );

        Some(TokenBreakdown {
            input: net_input,
            output: net_output,
            cache_read: raw_cache_read,
            cache_write: raw_cache_write,
            reasoning: raw_reasoning,
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

/// Subtract `overlap` out of `value`, clamping both operands to non-negative
/// and never going below zero. Mirrors `gemini.rs`'s `subtract_cached_overlap`
/// but takes the pre-summed overlap directly, since ZCode's `input_tokens`
/// absorbs two separate buckets (cache read + cache write) rather than one.
fn subtract_overlap(value: i64, overlap: i64) -> i64 {
    let value = value.max(0);
    let overlap = overlap.max(0);
    value.saturating_sub(overlap.min(value))
}

/// ZCode's `model_usage` rows report `input_tokens` and `output_tokens` as
/// cache/reasoning-inclusive: `input_tokens` already contains
/// `cache_read_input_tokens` + `cache_creation_input_tokens`, and
/// `output_tokens` already contains `reasoning_tokens`. Tokscale's
/// `TokenBreakdown` instead expects five non-overlapping buckets, so passing
/// the raw columns straight through double-counts cache and reasoning in
/// `TokenBreakdown::total()`.
///
/// When a reported `total` is available we use it to detect which shape
/// we're looking at, mirroring `gemini.rs`'s
/// `normalize_gemini_session_input_and_cache`: if the reported total matches
/// the cache/reasoning-inclusive sum (`input + output`) rather than the fully
/// additive sum (`input + output + cache_read + cache_write + reasoning`),
/// the row is inclusive and needs the overlap subtracted.
///
/// When `total` is absent, the shape can't be detected here, so the raw
/// input/output are returned unchanged; callers that have separate evidence
/// about their data source's shape (e.g. `parse_zcode_sqlite`'s legacy-schema
/// fallback) apply their own subtraction. Returns `(net_input, net_output)`.
fn normalize_zcode_input_and_output(
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
    total: Option<i64>,
) -> (i64, i64) {
    let input = input.max(0);
    let output = output.max(0);
    let cache_overlap = cache_read.max(0).saturating_add(cache_write.max(0));
    let reasoning = reasoning.max(0);

    let Some(total) = total.map(|value| value.max(0)) else {
        return (input, output);
    };

    let inclusive_total = input.saturating_add(output);
    let exclusive_total = inclusive_total
        .saturating_add(cache_overlap)
        .saturating_add(reasoning);

    if (cache_overlap > 0 || reasoning > 0) && total == inclusive_total && total != exclusive_total
    {
        return (
            subtract_overlap(input, cache_overlap),
            subtract_overlap(output, reasoning),
        );
    }

    (input, output)
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
            mu.computed_total_tokens,
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
            NULL,
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

    // Probe the `computed_total_tokens` column directly instead of inferring
    // legacy schema from the modern query failing to prepare: the modern query
    // also LEFT JOINs the `session` table, so it can fail for reasons
    // unrelated to the column's existence (e.g. a missing or renamed session
    // table). Conflating those would send modern-schema rows with NULL totals
    // through the unconditional subtraction below (potential undercount)
    // instead of the safe pass-through.
    let is_legacy_schema = conn
        .prepare("SELECT computed_total_tokens FROM model_usage LIMIT 1")
        .is_err();

    let mut stmt = match conn.prepare(modern_query) {
        Ok(stmt) => stmt,
        Err(_) => match conn.prepare(legacy_query) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        },
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
            computed_total_tokens: row.get(12)?,
            agent: row.get(13)?,
            mode: row.get(14)?,
            session_directory: row.get(15)?,
            session_path: row.get(16)?,
        })
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };

    let mut messages = Vec::new();
    // Parallel to `messages`: each row's turn_id (if any), so is_turn_start
    // can be assigned in a second pass once every row's start-anchored
    // timestamp is known (see below).
    let mut turn_ids: Vec<Option<String>> = Vec::new();

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
        let timestamp = resolve_zcode_timestamp(
            row.started_at,
            row.completed_at,
            row.duration_ms,
            fallback_timestamp,
        );

        let raw_input = row.input_tokens.unwrap_or(0);
        let raw_output = row.output_tokens.unwrap_or(0);
        let raw_cache_read = row.cache_read_input_tokens.unwrap_or(0);
        let raw_cache_write = row.cache_creation_input_tokens.unwrap_or(0);
        let raw_reasoning = row.reasoning_tokens.unwrap_or(0);

        let (net_input, net_output) = match row.computed_total_tokens {
            Some(total) => normalize_zcode_input_and_output(
                raw_input,
                raw_output,
                raw_cache_read,
                raw_cache_write,
                raw_reasoning,
                Some(total),
            ),
            // When `computed_total_tokens` is NULL, distinguish two cases:
            // 1. Legacy schema (column doesn't exist): unconditionally subtract,
            //    since every sampled row in a real ZCode database is confirmed
            //    cache/reasoning-inclusive.
            // 2. Modern schema but this row's value is NULL: can't detect shape,
            //    so pass through unchanged (the normalize function's default when
            //    total is None). Subtracting unconditionally here would undercount
            //    rows that are already cache-exclusive.
            None if is_legacy_schema => (
                subtract_overlap(
                    raw_input,
                    raw_cache_read.max(0).saturating_add(raw_cache_write.max(0)),
                ),
                subtract_overlap(raw_output, raw_reasoning),
            ),
            None => normalize_zcode_input_and_output(
                raw_input,
                raw_output,
                raw_cache_read,
                raw_cache_write,
                raw_reasoning,
                None,
            ),
        };

        let tokens = TokenBreakdown {
            input: net_input,
            output: net_output,
            cache_read: raw_cache_read.max(0),
            cache_write: raw_cache_write.max(0),
            reasoning: raw_reasoning.max(0),
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

        let workspace_root = row.session_directory.or(row.session_path);
        let workspace_key = workspace_root.as_deref().and_then(normalize_workspace_key);
        let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
        message.set_workspace(workspace_key, workspace_label);

        turn_ids.push(
            row.turn_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .map(str::to_string),
        );
        messages.push(message);
    }

    // Assign is_turn_start to the earliest-STARTED request per turn, not the
    // first one encountered in query order (which is ordered by
    // completed_at). Timestamps are now start-anchored (see above), so a
    // later-started-but-earlier-completed request could otherwise win the
    // flag and land the turn in the wrong hour/day bucket downstream (see
    // lib.rs's hourly turn_count aggregation).
    let mut earliest_index_per_turn: HashMap<&str, usize> = HashMap::new();
    for (index, turn_id) in turn_ids.iter().enumerate() {
        let Some(turn_id) = turn_id.as_deref() else {
            continue;
        };
        earliest_index_per_turn
            .entry(turn_id)
            .and_modify(|current| {
                if messages[index].timestamp < messages[*current].timestamp {
                    *current = index;
                }
            })
            .or_insert(index);
    }
    for index in earliest_index_per_turn.into_values() {
        messages[index].is_turn_start = true;
    }

    messages
}

/// Resolve the anchor timestamp for a `model_usage` row.
///
/// Prefers `started_at` when it's a positive epoch, since it anchors the
/// message at the call's actual start, matching `duration_ms`'s own
/// start-to-end span. When `started_at` is missing or non-positive, falls
/// back to `completed_at`, back-calculating the start anchor from
/// `completed_at - duration_ms` when a positive `duration_ms` is available
/// — anchoring at `completed_at` directly would make sessionize()'s
/// `[timestamp, timestamp + duration_ms]` span project forward past the
/// actual completion into phantom idle time (see #890). The back-calculation
/// is guarded against a non-positive result (which sessionize() silently
/// drops) by falling back to the unadjusted `completed_at`.
fn resolve_zcode_timestamp(
    started_at: Option<i64>,
    completed_at: Option<i64>,
    duration_ms: Option<i64>,
    fallback_timestamp: i64,
) -> i64 {
    if let Some(started) = started_at.filter(|value| *value > 0) {
        return started;
    }
    match completed_at {
        Some(completed) => match duration_ms.filter(|duration| *duration > 0) {
            Some(duration) => back_anchor_timestamp(completed, duration),
            None => completed,
        },
        None => fallback_timestamp,
    }
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
    computed_total_tokens: Option<i64>,
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
                computed_total_tokens INTEGER,
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
                cache_read_input_tokens, cache_creation_input_tokens, computed_total_tokens, agent, mode
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
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
                120_i64,
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
        // Timestamp anchors to `started_at` (the call's start), not
        // `completed_at` (the call's end). See #890 (follow-up).
        assert_eq!(msg.timestamp, 1_782_718_000_000_i64);
        assert_eq!(msg.duration_ms, Some(1000));
        assert_eq!(msg.tokens.input, 90);
        assert_eq!(msg.tokens.output, 15);
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

    #[test]
    fn test_model_usage_timestamp_is_start_anchored() {
        // Regression (follow-up to #890): `model_usage` records both
        // `started_at` and `completed_at` for a call, plus an explicit
        // `duration_ms`. Anchoring the message timestamp at `completed_at`
        // would make sessionize()'s `[timestamp, timestamp + duration_ms]`
        // span project forward past the actual completion into phantom idle
        // time. The parser must prefer `started_at`.
        let dir = TempDir::new().unwrap();
        let db_path = create_zcode_sqlite_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            r#"
            INSERT INTO model_usage (
                id, session_id, turn_id, model_id, started_at, completed_at,
                duration_ms, input_tokens, output_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                "usage_1",
                "sess_1",
                "turn_1",
                "glm-5.2",
                1_782_718_000_000_i64,
                1_782_718_005_000_i64,
                5000_i64,
                10_i64,
                1_i64,
            ],
        )
        .unwrap();

        let messages = parse_zcode_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].timestamp, 1_782_718_000_000_i64,
            "timestamp must anchor at started_at, not completed_at"
        );
        assert_eq!(
            messages[0].duration_ms,
            Some(5000),
            "duration_ms must still span from start to completion"
        );
    }

    #[test]
    fn test_model_usage_missing_started_at_back_calculates_from_completed_at() {
        // Second-round review fix: when `started_at` is NULL but
        // `completed_at` and a positive `duration_ms` are present, the row
        // must not stay end-anchored at `completed_at` (a phantom forward
        // projection past the call's actual completion). Back-calculate the
        // start anchor from `completed_at - duration_ms` instead.
        let dir = TempDir::new().unwrap();
        let db_path = create_zcode_sqlite_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            r#"
            INSERT INTO model_usage (
                id, session_id, turn_id, model_id, completed_at,
                duration_ms, input_tokens, output_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                "usage_1",
                "sess_1",
                "turn_1",
                "glm-5.2",
                1_782_718_005_000_i64,
                5000_i64,
                10_i64,
                1_i64,
            ],
        )
        .unwrap();

        let messages = parse_zcode_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].timestamp, 1_782_718_000_000_i64,
            "timestamp must be back-calculated from completed_at - duration_ms when started_at is missing"
        );
        assert_eq!(messages[0].duration_ms, Some(5000));
    }

    #[test]
    fn test_parse_zcode_sqlite_cache_inclusive_normalization() {
        let dir = TempDir::new().unwrap();
        let db_path = create_zcode_sqlite_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            r#"
            INSERT INTO model_usage (
                id, session_id, model_id, completed_at,
                input_tokens, output_tokens, reasoning_tokens,
                cache_read_input_tokens, cache_creation_input_tokens, computed_total_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                "usage_cache_incl",
                "sess_cache",
                "glm-5.2",
                1_000_i64,
                100_i64,
                50_i64,
                10_i64,
                80_i64,
                5_i64,
                150_i64,
            ],
        )
        .unwrap();

        let messages = parse_zcode_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.tokens.input, 15);
        assert_eq!(msg.tokens.output, 40);
        assert_eq!(msg.tokens.cache_read, 80);
        assert_eq!(msg.tokens.cache_write, 5);
        assert_eq!(msg.tokens.reasoning, 10);
        assert_eq!(msg.tokens.total(), 150);
    }

    #[test]
    fn test_parse_zcode_sqlite_legacy_schema_subtracts_unconditionally() {
        // True legacy schema: no `computed_total_tokens` column (and no
        // `session` table), so the column probe and the modern query both
        // fail and the legacy fallback runs with is_legacy_schema=true.
        // Every row must then take the unconditional-subtraction branch.
        let dir = TempDir::new().unwrap();
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
            "#,
        )
        .unwrap();
        conn.execute(
            r#"
            INSERT INTO model_usage (
                id, session_id, model_id, completed_at,
                input_tokens, output_tokens, reasoning_tokens,
                cache_read_input_tokens, cache_creation_input_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                "usage_legacy",
                "sess_legacy",
                "glm-5.2",
                1_000_i64,
                100_i64,
                50_i64,
                10_i64,
                80_i64,
                5_i64,
            ],
        )
        .unwrap();

        let messages = parse_zcode_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.tokens.input, 15);
        assert_eq!(msg.tokens.output, 40);
        assert_eq!(msg.tokens.cache_read, 80);
        assert_eq!(msg.tokens.cache_write, 5);
        assert_eq!(msg.tokens.reasoning, 10);
        assert_eq!(msg.tokens.total(), 150);
    }

    #[test]
    fn test_parse_zcode_sqlite_modern_schema_null_total_passes_through() {
        // Modern schema (computed_total_tokens column exists) but this row's
        // value is NULL: the shape can't be detected, so input/output must
        // pass through unchanged rather than being unconditionally subtracted.
        let dir = TempDir::new().unwrap();
        let db_path = create_zcode_sqlite_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            r#"
            INSERT INTO model_usage (
                id, session_id, model_id, completed_at,
                input_tokens, output_tokens, reasoning_tokens,
                cache_read_input_tokens, cache_creation_input_tokens, computed_total_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)
            "#,
            params![
                "usage_null_total",
                "sess_null",
                "glm-5.2",
                1_000_i64,
                100_i64,
                50_i64,
                10_i64,
                80_i64,
                5_i64,
            ],
        )
        .unwrap();

        let messages = parse_zcode_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.tokens.input, 100);
        assert_eq!(msg.tokens.output, 50);
        assert_eq!(msg.tokens.cache_read, 80);
        assert_eq!(msg.tokens.cache_write, 5);
        assert_eq!(msg.tokens.reasoning, 10);
    }

    #[test]
    fn test_parse_zcode_sqlite_cache_exclusive_preserved() {
        let dir = TempDir::new().unwrap();
        let db_path = create_zcode_sqlite_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            r#"
            INSERT INTO model_usage (
                id, session_id, model_id, completed_at,
                input_tokens, output_tokens, reasoning_tokens,
                cache_read_input_tokens, cache_creation_input_tokens, computed_total_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                "usage_cache_excl",
                "sess_excl",
                "claude-sonnet-5",
                1_000_i64,
                20_i64,
                30_i64,
                5_i64,
                80_i64,
                10_i64,
                145_i64,
            ],
        )
        .unwrap();

        let messages = parse_zcode_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.tokens.input, 20);
        assert_eq!(msg.tokens.output, 30);
        assert_eq!(msg.tokens.cache_read, 80);
        assert_eq!(msg.tokens.cache_write, 10);
        assert_eq!(msg.tokens.reasoning, 5);
        assert_eq!(msg.tokens.total(), 145);
    }
}
