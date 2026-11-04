//! BB (`get-bb/bb`) session parser.
//!
//! Parses local session data from the BB SQLite database (`~/.bb/bb.db`, or
//! `$BB_DATA_DIR/bb.db`, or development instances at `~/.bb-dev/<instance>/bb.db`).
//!
//! ### Database Layout
//! BB is an agentic IDE and multi-agent orchestrator. It manages sessions as
//! **threads** backed by a local SQLite database (managed via Drizzle ORM in
//! `packages/db/src/schema.ts`):
//!
//! - `threads`: Primary metadata for each conversation thread (`id`, `project_id`,
//!   `provider_id`, `model_override`, `title`, `parent_thread_id`, `created_at`, `updated_at`).
//! - `projects`: Project workspaces (`id`, `name`, `git_remote_url`).
//! - `project_execution_defaults`: Default provider configuration per project
//!   (`project_id`, `provider_id`, `model`, `service_tier`).
//! - `events`: Timeline event stream for each thread (`id`, `thread_id`, `turn_id`,
//!   `sequence`, `type`, `data`, `created_at`).
//!
//! ### Token Breakdown Extraction
//! When BB executes agent turns (via Claude Code, Codex, Pi, or ACP bridge), it
//! records `thread/tokenUsage/updated` events containing `tokenUsage.last` (the current
//! generation/turn step breakdown) and `tokenUsage.total` (cumulative total).
//!
//! Fields in `tokenUsage.last`:
//! - `totalTokens`: Reported total token count for this request.
//! - `inputTokens`: Raw input/prompt tokens.
//! - `cachedInputTokens`: Total cached prompt tokens.
//! - `cacheReadInputTokens`: Cached prompt read tokens.
//! - `cacheWriteInputTokens`: Cache creation tokens.
//! - `outputTokens`: Generated output tokens.
//! - `reasoningOutputTokens`: Thinking/reasoning tokens.
//! - `promptCacheTtlMs`: Cache TTL (e.g. 3,600,000 for 1-hour cache write tier).
//!
//! Both Anthropic conventions (disjoint uncached input and cache buckets) and
//! OpenAI/Codex conventions (cached tokens reported inside `inputTokens`, reasoning
//! reported inside `outputTokens`) are detected and normalized to additive
//! [`TokenBreakdown`] buckets without double-counting.

use super::utils::{
    file_modified_timestamp_ms, open_readonly_sqlite, parse_json_line, sqlite_for_each_row_on,
};
use super::{SharedStr, UnifiedMessage};
use crate::{provider_identity, CostSource, TokenBreakdown};
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;
use tracing::{debug, warn};

const BB_SQLITE_BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// Metadata for a BB thread, resolved from `threads`, `projects`, and
/// `project_execution_defaults`.
#[derive(Debug, Clone)]
pub struct BbThreadMeta {
    pub id: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub provider_id: String,
    pub model_override: Option<String>,
    pub default_model: Option<String>,
    pub service_tier: Option<String>,
    pub title: Option<String>,
    pub parent_thread_id: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct BbEventData {
    #[serde(rename = "tokenUsage")]
    token_usage: Option<BbTokenUsageContainer>,
    #[serde(rename = "fallbackModel")]
    fallback_model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BbTokenUsageContainer {
    last: Option<BbTokenUsageBreakdown>,
    total: Option<BbTokenUsageBreakdown>,
    #[serde(rename = "promptCacheTtlMs")]
    prompt_cache_ttl_ms: Option<i64>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct BbTokenUsageBreakdown {
    #[serde(rename = "totalTokens")]
    total_tokens: Option<i64>,
    #[serde(rename = "inputTokens")]
    input_tokens: Option<i64>,
    #[serde(rename = "cachedInputTokens")]
    cached_input_tokens: Option<i64>,
    #[serde(rename = "cacheReadInputTokens")]
    cache_read_input_tokens: Option<i64>,
    #[serde(rename = "cacheWriteInputTokens")]
    cache_write_input_tokens: Option<i64>,
    #[serde(rename = "outputTokens")]
    output_tokens: Option<i64>,
    #[serde(rename = "reasoningOutputTokens")]
    reasoning_output_tokens: Option<i64>,
}

impl BbTokenUsageBreakdown {
    fn is_zero(&self) -> bool {
        self.total_tokens.unwrap_or(0) == 0
            && self.input_tokens.unwrap_or(0) == 0
            && self.output_tokens.unwrap_or(0) == 0
            && self.cached_input_tokens.unwrap_or(0) == 0
            && self.cache_read_input_tokens.unwrap_or(0) == 0
            && self.cache_write_input_tokens.unwrap_or(0) == 0
            && self.reasoning_output_tokens.unwrap_or(0) == 0
    }

    fn to_breakdown(&self, prompt_cache_ttl_ms: Option<i64>) -> TokenBreakdown {
        let cache_read = self
            .cache_read_input_tokens
            .or(self.cached_input_tokens)
            .unwrap_or(0)
            .max(0);
        let cache_write = self.cache_write_input_tokens.unwrap_or(0).max(0);
        let raw_input = self.input_tokens.unwrap_or(0).max(0);
        let raw_output = self.output_tokens.unwrap_or(0).max(0);
        let reasoning = self.reasoning_output_tokens.unwrap_or(0).max(0);

        // In Codex/OpenAI convention, `raw_input` contains `cache_read`.
        // If `raw_input >= cache_read + cache_write` and `total_tokens == raw_input + raw_output`,
        // then the input figure was cumulative and includes cache.
        let uncached_input = if raw_input >= cache_read + cache_write
            && self
                .total_tokens
                .is_some_and(|total| total == raw_input + raw_output)
        {
            raw_input - cache_read - cache_write
        } else {
            raw_input
        };

        // In Codex/OpenAI convention, `reasoning_output_tokens` is inside `output_tokens`.
        let output = if raw_output >= reasoning
            && self.total_tokens.is_some_and(|total| {
                total == uncached_input + cache_read + cache_write + raw_output
            }) {
            raw_output - reasoning
        } else {
            raw_output
        };

        let cache_write_1h = if prompt_cache_ttl_ms.is_some_and(|ttl| ttl >= 3_600_000) {
            cache_write
        } else {
            0
        };

        TokenBreakdown {
            input: uncached_input,
            output,
            cache_read,
            cache_write,
            cache_write_1h,
            reasoning,
        }
    }
}

/// Result of scanning a BB SQLite database.
pub struct BbSqliteScan {
    pub messages: Vec<UnifiedMessage>,
    pub complete: bool,
}

/// Check if the required tables exist in the BB database.
fn has_required_tables(conn: &Connection) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('threads', 'events')",
    )?;
    let count: i64 = stmt.query_row([], |row| row.get(0))?;
    Ok(count >= 2)
}

/// Load thread metadata indexed by thread ID.
fn load_thread_metadata(conn: &Connection) -> rusqlite::Result<HashMap<String, BbThreadMeta>> {
    let mut query = String::from(
        "SELECT t.id, t.project_id, p.name, t.provider_id, t.model_override, \
         t.title, t.parent_thread_id, t.created_at \
         FROM threads t \
         LEFT JOIN projects p ON p.id = t.project_id",
    );

    // Check if project_execution_defaults table exists
    let has_defaults: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'project_execution_defaults'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);

    let mut stmt = if has_defaults {
        query = String::from(
            "SELECT t.id, t.project_id, p.name, t.provider_id, t.model_override, \
             t.title, t.parent_thread_id, t.created_at, d.model, d.service_tier \
             FROM threads t \
             LEFT JOIN projects p ON p.id = t.project_id \
             LEFT JOIN project_execution_defaults d ON d.project_id = t.project_id AND d.provider_id = t.provider_id",
        );
        conn.prepare(&query)?
    } else {
        conn.prepare(&query)?
    };

    let mut map = HashMap::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let project_id: Option<String> = row.get(1)?;
        let project_name: Option<String> = row.get(2)?;
        let provider_id: String = row.get(3)?;
        let model_override: Option<String> = row.get(4)?;
        let title: Option<String> = row.get(5)?;
        let parent_thread_id: Option<String> = row.get(6)?;
        let created_at_ms: i64 = row.get(7)?;

        let (default_model, service_tier) = if has_defaults {
            (
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
            )
        } else {
            (None, None)
        };

        map.insert(
            id.clone(),
            BbThreadMeta {
                id,
                project_id,
                project_name,
                provider_id,
                model_override,
                default_model,
                service_tier,
                title,
                parent_thread_id,
                created_at_ms,
            },
        );
    }

    Ok(map)
}

/// Canonicalize provider ID and model ID for a BB thread.
fn resolve_provider_and_model(
    thread: &BbThreadMeta,
    active_fallback: Option<&str>,
) -> (String, String) {
    let model_id = active_fallback
        .map(str::to_string)
        .or_else(|| thread.model_override.clone())
        .or_else(|| thread.default_model.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let raw_provider = thread.provider_id.trim();
    let canonical = match raw_provider {
        "claude-code" => "anthropic".to_string(),
        "codex" => "openai".to_string(),
        other => provider_identity::canonical_provider(other)
            .map(|s| s.to_string())
            .or_else(|| {
                provider_identity::inferred_provider_from_model_delimited(&model_id)
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| other.to_string()),
    };

    (canonical, model_id)
}

/// Scan a BB SQLite database and extract all token usage events into [`UnifiedMessage`].
pub fn scan_bb_sqlite(db_path: &Path) -> BbSqliteScan {
    let conn = match open_readonly_sqlite(db_path) {
        Ok(conn) => conn,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open BB database"
            );
            return BbSqliteScan {
                messages: Vec::new(),
                complete: false,
            };
        }
    };

    if let Err(err) = conn.busy_timeout(BB_SQLITE_BUSY_TIMEOUT) {
        debug!(
            db_path = %db_path.display(),
            error = %err,
            "Failed to set BB database busy timeout"
        );
    }

    match has_required_tables(&conn) {
        Ok(true) => {}
        Ok(false) => {
            debug!(
                db_path = %db_path.display(),
                "BB database missing threads or events table; skipping"
            );
            return BbSqliteScan {
                messages: Vec::new(),
                complete: true,
            };
        }
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to probe BB database tables"
            );
            return BbSqliteScan {
                messages: Vec::new(),
                complete: false,
            };
        }
    }

    let thread_metas = match load_thread_metadata(&conn) {
        Ok(metas) => metas,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to load BB thread metadata"
            );
            return BbSqliteScan {
                messages: Vec::new(),
                complete: false,
            };
        }
    };

    let db_mtime_ms = file_modified_timestamp_ms(db_path);
    let mut messages: Vec<UnifiedMessage> = Vec::new();
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let mut malformed_rows = 0;
    let mut current_thread_id: Option<String> = None;
    let mut active_fallback_model: Option<String> = None;
    let mut turn_first_message_seen: HashMap<String, bool> = HashMap::new();
    let mut internal_seen: HashSet<String> = HashSet::new();
    let mut thread_totals: HashMap<String, TokenBreakdown> = HashMap::new();

    let query = "SELECT id, thread_id, turn_id, sequence, type, data, created_at \
                 FROM events \
                 WHERE type IN ('thread/tokenUsage/updated', 'provider/modelFallback') \
                 ORDER BY thread_id, sequence ASC";

    let scan = sqlite_for_each_row_on(&conn, db_path, query, Some("BB event"), &mut |row| {
        let _event_id: String = row.get(0)?;
        let thread_id: String = row.get(1)?;
        let turn_id: Option<String> = row.get(2)?;
        let sequence: i64 = row.get(3)?;
        let event_type: String = row.get(4)?;
        let data_json: String = row.get(5)?;
        let created_at_ms: Option<i64> = row.get(6)?;

        if current_thread_id.as_deref() != Some(thread_id.as_str()) {
            current_thread_id = Some(thread_id.clone());
            active_fallback_model = None;
        }

        let Some(event_data) = parse_json_line::<BbEventData>(&data_json, &mut buffer) else {
            malformed_rows += 1;
            return Ok(());
        };

        if event_type == "provider/modelFallback" {
            if let Some(fallback) = event_data.fallback_model {
                active_fallback_model = Some(fallback);
            }
            return Ok(());
        }

        let Some(token_container) = event_data.token_usage else {
            return Ok(());
        };

        let tokens = if let Some(last) = token_container.last.as_ref().filter(|u| !u.is_zero()) {
            let breakdown = last.to_breakdown(token_container.prompt_cache_ttl_ms);
            if let Some(total) = &token_container.total {
                thread_totals.insert(
                    thread_id.clone(),
                    total.to_breakdown(token_container.prompt_cache_ttl_ms),
                );
            }
            breakdown
        } else if let Some(total) = token_container.total.as_ref().filter(|u| !u.is_zero()) {
            let current_total = total.to_breakdown(token_container.prompt_cache_ttl_ms);
            let prev_total = thread_totals.entry(thread_id.clone()).or_default();
            let delta = TokenBreakdown {
                input: current_total.input.saturating_sub(prev_total.input),
                output: current_total.output.saturating_sub(prev_total.output),
                cache_read: current_total
                    .cache_read
                    .saturating_sub(prev_total.cache_read),
                cache_write: current_total
                    .cache_write
                    .saturating_sub(prev_total.cache_write),
                cache_write_1h: current_total
                    .cache_write_1h
                    .saturating_sub(prev_total.cache_write_1h),
                reasoning: current_total.reasoning.saturating_sub(prev_total.reasoning),
            };
            *prev_total = current_total;
            delta
        } else {
            return Ok(());
        };

        if tokens.total() == 0 {
            return Ok(());
        }

        let timestamp = created_at_ms.filter(|ts| *ts > 0).unwrap_or(db_mtime_ms);

        let (
            provider_id,
            model_id,
            workspace_key,
            workspace_label,
            service_tier,
            session_title,
            parent_session_id,
        ) = if let Some(thread) = thread_metas.get(&thread_id) {
            let (prov, model) =
                resolve_provider_and_model(thread, active_fallback_model.as_deref());
            (
                prov,
                model,
                thread.project_id.clone(),
                thread.project_name.clone(),
                thread.service_tier.clone(),
                thread.title.clone(),
                thread.parent_thread_id.clone(),
            )
        } else {
            let model = active_fallback_model
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let prov = provider_identity::inferred_provider_from_model_delimited(&model)
                .unwrap_or("unknown")
                .to_string();
            (prov, model, None, None, None, None, None)
        };

        let is_turn_start = if let Some(turn) = &turn_id {
            let seen = turn_first_message_seen.entry(turn.clone()).or_insert(false);
            if !*seen {
                *seen = true;
                true
            } else {
                false
            }
        } else {
            false
        };

        // Deduplication key: forked threads replicate past turns with new event IDs
        // but matching `turn_id` and sequence within turn.
        let dedup_key = if let Some(turn) = &turn_id {
            format!("bb:{turn}:{sequence}:{}:{}", tokens.input, tokens.output)
        } else {
            format!(
                "bb:{timestamp}:{sequence}:{}:{}",
                tokens.input, tokens.output
            )
        };

        let mut msg = UnifiedMessage::new_with_dedup(
            "bb",
            model_id,
            provider_id,
            thread_id,
            timestamp,
            tokens,
            0.0,
            Some(dedup_key),
        );
        msg.workspace_key = workspace_key.map(SharedStr::from);
        msg.workspace_label = workspace_label.map(SharedStr::from);
        msg.service_tier = service_tier.map(SharedStr::from);
        msg.session_title = session_title.map(SharedStr::from);
        msg.parent_session_id = parent_session_id.map(SharedStr::from);
        msg.is_turn_start = is_turn_start;
        msg.cost_source = CostSource::Estimated;
        msg.agent = Some("BB".into());

        if msg
            .dedup_key
            .as_ref()
            .is_none_or(|key| internal_seen.insert(key.clone()))
        {
            messages.push(msg);
        }

        Ok(())
    });

    if malformed_rows > 0 {
        debug!(
            db_path = %db_path.display(),
            malformed_rows,
            "Skipped malformed BB event rows"
        );
    }

    BbSqliteScan {
        messages,
        complete: scan.completed() && malformed_rows == 0,
    }
}

/// Convenience wrapper for [`scan_bb_sqlite`] returning only the parsed messages.
pub fn parse_bb_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    scan_bb_sqlite(db_path).messages
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    use rusqlite::Connection;
    use std::path::Path;

    pub fn create_test_bb_db(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY NOT NULL,
                project_id TEXT,
                provider_id TEXT NOT NULL,
                model_override TEXT,
                title TEXT,
                parent_thread_id TEXT,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE projects (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL
            );

            CREATE TABLE project_execution_defaults (
                project_id TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                service_tier TEXT NOT NULL
            );

            CREATE TABLE events (
                id TEXT PRIMARY KEY NOT NULL,
                thread_id TEXT NOT NULL,
                turn_id TEXT,
                sequence INTEGER NOT NULL,
                type TEXT NOT NULL,
                data TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            "#,
        )
        .unwrap();
        conn
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::create_test_bb_db;
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_bb_sqlite_claude_code() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bb.db");
        let conn = create_test_bb_db(&db_path);

        conn.execute(
            "INSERT INTO projects (id, name) VALUES (?1, ?2)",
            ("prj_1", "tokscale"),
        )
        .unwrap();

        conn.execute(
            "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "thr_1",
                "prj_1",
                "claude-code",
                Some("claude-3-7-sonnet"),
                Some("Add feature"),
                None::<String>,
                1_787_275_050_000_i64,
            ),
        )
        .unwrap();

        let event_data = r#"{
            "tokenUsage": {
                "last": {
                    "totalTokens": 89409,
                    "inputTokens": 6,
                    "cachedInputTokens": 89145,
                    "cacheReadInputTokens": 87563,
                    "cacheWriteInputTokens": 1582,
                    "outputTokens": 258,
                    "reasoningOutputTokens": 0
                },
                "promptCacheTtlMs": 3600000
            }
        }"#;

        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "evt_1",
                "thr_1",
                Some("turn_1"),
                1,
                "thread/tokenUsage/updated",
                event_data,
                1_787_275_053_000_i64,
            ),
        )
        .unwrap();

        let messages = parse_bb_sqlite(&db_path);
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(msg.client, "bb");
        assert_eq!(msg.provider_id, "anthropic");
        assert_eq!(msg.model_id, "claude-3-7-sonnet");
        assert_eq!(msg.session_id, "thr_1");
        assert_eq!(msg.workspace_key.as_deref(), Some("prj_1"));
        assert_eq!(msg.workspace_label.as_deref(), Some("tokscale"));
        assert_eq!(msg.session_title.as_deref(), Some("Add feature"));
        assert_eq!(msg.tokens.input, 6);
        assert_eq!(msg.tokens.output, 258);
        assert_eq!(msg.tokens.cache_read, 87563);
        assert_eq!(msg.tokens.cache_write, 1582);
        assert_eq!(msg.tokens.cache_write_1h, 1582);
        assert_eq!(msg.tokens.reasoning, 0);
        assert_eq!(msg.tokens.total(), 89409);
        assert!(msg.is_turn_start);
    }

    #[test]
    fn test_parse_bb_sqlite_codex_with_reasoning() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bb.db");
        let conn = create_test_bb_db(&db_path);

        conn.execute(
            "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "thr_2",
                None::<String>,
                "codex",
                Some("o3-mini"),
                None::<String>,
                None::<String>,
                1_787_275_050_000_i64,
            ),
        )
        .unwrap();

        let event_data = r#"{
            "tokenUsage": {
                "last": {
                    "totalTokens": 20000,
                    "inputTokens": 19000,
                    "cachedInputTokens": 18000,
                    "cacheReadInputTokens": 18000,
                    "cacheWriteInputTokens": 0,
                    "outputTokens": 1000,
                    "reasoningOutputTokens": 600
                }
            }
        }"#;

        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "evt_2",
                "thr_2",
                Some("turn_2"),
                1,
                "thread/tokenUsage/updated",
                event_data,
                1_787_275_053_000_i64,
            ),
        )
        .unwrap();

        let messages = parse_bb_sqlite(&db_path);
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(msg.client, "bb");
        assert_eq!(msg.provider_id, "openai");
        assert_eq!(msg.model_id, "o3-mini");
        // In Codex: raw_input = 19000, cached = 18000 -> uncached input = 1000
        assert_eq!(msg.tokens.input, 1000);
        assert_eq!(msg.tokens.cache_read, 18000);
        // raw_output = 1000, reasoning = 600 -> output = 400
        assert_eq!(msg.tokens.output, 400);
        assert_eq!(msg.tokens.reasoning, 600);
        assert_eq!(msg.tokens.total(), 20000);
    }

    #[test]
    fn test_parse_bb_sqlite_model_fallback() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bb.db");
        let conn = create_test_bb_db(&db_path);

        conn.execute(
            "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "thr_3",
                None::<String>,
                "claude-code",
                Some("claude-3-7-sonnet"),
                None::<String>,
                None::<String>,
                1_787_275_050_000_i64,
            ),
        )
        .unwrap();

        // 1. Model fallback event
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "evt_fallback",
                "thr_3",
                Some("turn_3"),
                1,
                "provider/modelFallback",
                r#"{"fallbackModel": "claude-3-5-haiku"}"#,
                1_787_275_051_000_i64,
            ),
        )
        .unwrap();

        // 2. Token usage event after fallback
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "evt_usage",
                "thr_3",
                Some("turn_3"),
                2,
                "thread/tokenUsage/updated",
                r#"{"tokenUsage": {"last": {"totalTokens": 150, "inputTokens": 100, "outputTokens": 50}}}"#,
                1_787_275_052_000_i64,
            ),
        )
        .unwrap();

        let messages = parse_bb_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-3-5-haiku");
    }

    #[test]
    fn test_parse_bb_sqlite_fork_dedup() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bb.db");
        let conn = create_test_bb_db(&db_path);

        // Thread 1: original thread with turn 1
        conn.execute(
            "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
             VALUES ('thr_parent', NULL, 'claude-code', 'claude-3-7-sonnet', 'Parent Thread', NULL, 1000)",
            [],
        )
        .unwrap();

        // Thread 2: forked from Thread 1
        conn.execute(
            "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
             VALUES ('thr_child', NULL, 'claude-code', 'claude-3-7-sonnet', 'Child Thread', 'thr_parent', 2000)",
            [],
        )
        .unwrap();

        // Turn 1 event in Thread 1
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('evt_t1_1', 'thr_parent', 'turn_1', 1, 'thread/tokenUsage/updated', \
             '{\"tokenUsage\": {\"last\": {\"totalTokens\": 500, \"inputTokens\": 400, \"outputTokens\": 100}}}', 1100)",
            [],
        )
        .unwrap();

        // Fork copies turn 1 into Thread 2 with matching turn_id and sequence
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('evt_t2_1', 'thr_child', 'turn_1', 1, 'thread/tokenUsage/updated', \
             '{\"tokenUsage\": {\"last\": {\"totalTokens\": 500, \"inputTokens\": 400, \"outputTokens\": 100}}}', 2100)",
            [],
        )
        .unwrap();

        // Turn 2 is a new turn only in Thread 2
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('evt_t2_2', 'thr_child', 'turn_2', 2, 'thread/tokenUsage/updated', \
             '{\"tokenUsage\": {\"last\": {\"totalTokens\": 600, \"inputTokens\": 450, \"outputTokens\": 150}}}', 2200)",
            [],
        )
        .unwrap();

        let messages = parse_bb_sqlite(&db_path);
        // Turn 1 was duplicated across threads, but should be deduplicated so only 2 messages exist
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 400);
        assert_eq!(messages[0].tokens.output, 100);
        assert_eq!(messages[1].tokens.input, 450);
        assert_eq!(messages[1].tokens.output, 150);
    }

    #[test]
    fn test_parse_bb_sqlite_cumulative_total_delta() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bb.db");
        let conn = create_test_bb_db(&db_path);

        conn.execute(
            "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
             VALUES ('thr_cumul', NULL, 'claude-code', 'claude-3-7-sonnet', 'Cumulative Thread', NULL, 1000)",
            [],
        )
        .unwrap();

        // Event 1: only has `total` (1000 input, 200 output)
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('evt_c1', 'thr_cumul', 'turn_c1', 1, 'thread/tokenUsage/updated', \
             '{\"tokenUsage\": {\"total\": {\"totalTokens\": 1200, \"inputTokens\": 1000, \"outputTokens\": 200}}}', 1100)",
            [],
        )
        .unwrap();

        // Event 2: cumulative total has grown to 2500 input, 500 output (delta: 1500 input, 300 output)
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('evt_c2', 'thr_cumul', 'turn_c2', 2, 'thread/tokenUsage/updated', \
             '{\"tokenUsage\": {\"total\": {\"totalTokens\": 3000, \"inputTokens\": 2500, \"outputTokens\": 500}}}', 1200)",
            [],
        )
        .unwrap();

        let messages = parse_bb_sqlite(&db_path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[0].tokens.output, 200);
        assert_eq!(messages[1].tokens.input, 1500);
        assert_eq!(messages[1].tokens.output, 300);
        let total_input: i64 = messages.iter().map(|m| m.tokens.input).sum();
        let total_output: i64 = messages.iter().map(|m| m.tokens.output).sum();
        assert_eq!(total_input, 2500);
        assert_eq!(total_output, 500);
    }

    #[test]
    fn test_parse_bb_sqlite_unmodeled_thread_preserves_unknown() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bb.db");
        let conn = create_test_bb_db(&db_path);

        conn.execute(
            "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
             VALUES ('thr_unmodeled', NULL, 'claude-code', NULL, 'Unmodeled', NULL, 1000)",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('evt_u1', 'thr_unmodeled', 'turn_u1', 1, 'thread/tokenUsage/updated', \
             '{\"tokenUsage\": {\"last\": {\"totalTokens\": 120, \"inputTokens\": 100, \"outputTokens\": 20}}}', 1100)",
            [],
        )
        .unwrap();

        let messages = parse_bb_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.provider_id, "anthropic");
        assert_eq!(msg.model_id, "unknown");
    }

    #[test]
    fn test_parse_bb_sqlite_delimited_provider_matching() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bb.db");
        let conn = create_test_bb_db(&db_path);

        // Substring "claude" in "myclaudex-1" should NOT match anthropic under delimited inference.
        conn.execute(
            "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
             VALUES ('thr_custom', NULL, 'custom-proxy', 'myclaudex-1', 'Custom', NULL, 1000)",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('evt_d1', 'thr_custom', 'turn_d1', 1, 'thread/tokenUsage/updated', \
             '{\"tokenUsage\": {\"last\": {\"totalTokens\": 120, \"inputTokens\": 100, \"outputTokens\": 20}}}', 1100)",
            [],
        )
        .unwrap();

        let messages = parse_bb_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.provider_id, "custom_proxy");
        assert_eq!(msg.model_id, "myclaudex-1");
    }

    #[test]
    fn test_scan_bb_sqlite_malformed_event_row_marks_incomplete() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bb.db");
        let conn = create_test_bb_db(&db_path);

        conn.execute(
            "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
             VALUES ('thr_malformed', NULL, 'claude-code', 'claude-3-7-sonnet', 'Malformed', NULL, 1000)",
            [],
        )
        .unwrap();

        // Valid event 1
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('evt_valid', 'thr_malformed', 'turn_1', 1, 'thread/tokenUsage/updated', \
             '{\"tokenUsage\": {\"last\": {\"totalTokens\": 120, \"inputTokens\": 100, \"outputTokens\": 20}}}', 1100)",
            [],
        )
        .unwrap();

        // Malformed event 2
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('evt_bad', 'thr_malformed', 'turn_2', 2, 'thread/tokenUsage/updated', \
             'NOT_JSON_DATA', 1200)",
            [],
        )
        .unwrap();

        let scan = scan_bb_sqlite(&db_path);
        assert_eq!(scan.messages.len(), 1);
        assert!(
            !scan.complete,
            "scan with malformed rows must not be marked complete"
        );
    }
}
