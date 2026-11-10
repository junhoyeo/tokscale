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
//!
//! ### Provider Session Identity
//! BB does not call a model itself. A `claude-code` thread runs the Claude
//! Agent SDK with `persistSession: true` and the user's normal Claude
//! environment, and a `codex` thread runs Codex app-server with
//! `ephemeral: false` and the user's `CODEX_HOME`. Each response is therefore
//! recorded twice: once in BB's `events` table and once in the provider's own
//! log (`~/.claude/projects/**/<session id>.jsonl`, or a rollout under
//! `~/.codex/sessions`). BB stores the link on its rows:
//!
//! - `events.provider_thread_id` (and `providerThreadId` in `data`): the Claude
//!   session id (the transcript's file stem and `sessionId`), or the Codex
//!   thread id (the rollout's `session_meta.id`). Forked threads copy their
//!   history rows with this column cleared, so `data` is the fallback.
//! - `providerCheckpointId` on the turn's `turn/completed` row: the `uuid` of
//!   the turn's last top-level Claude assistant entry, or the Codex turn id
//!   (the rollout's `turn_context.turn_id`).
//!
//! The parser carries that identity in the row's dedup key (see
//! `provider_mirror_from_dedup_key`) so the lanes can drop a BB row when the
//! provider's own record of the same work is counted in the same report.

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
    /// Provider session the event was recorded under. Duplicates the
    /// `events.provider_thread_id` column, and survives in a fork's copied
    /// rows where that column is cleared.
    #[serde(rename = "providerThreadId")]
    provider_thread_id: Option<String>,
    /// On `turn/completed`: the provider's own id for the finished turn.
    #[serde(rename = "providerCheckpointId")]
    provider_checkpoint_id: Option<String>,
}

/// The provider whose own log also records the usage of a BB thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BbMirrorProvider {
    /// `claude-code` threads: Claude Code transcripts.
    Claude,
    /// `codex` threads: Codex rollouts.
    Codex,
}

impl BbMirrorProvider {
    fn for_thread_provider(provider_id: &str) -> Option<Self> {
        match provider_id.trim() {
            "claude-code" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn from_key_segment(segment: &str) -> Option<Self> {
        match segment {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

/// The provider session (and turn, when BB recorded one) a BB usage row
/// mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BbProviderMirror<'a> {
    pub provider: BbMirrorProvider,
    /// Claude session id, or Codex thread id.
    pub session: &'a str,
    /// The turn's `providerCheckpointId`: the last Claude assistant entry's
    /// `uuid`, or the Codex turn id.
    pub turn: Option<&'a str>,
}

/// Dedup-key namespace of a BB row that names its provider session:
/// `bb:mirror:<provider>:<session>:<turn>:` followed by the row's own
/// identity (`bb:<turn id>:<sequence>:<input>:<output>`). The turn segment is
/// empty when BB recorded no checkpoint for the turn. The identity is kept
/// last so fork copies still collapse on it (see [`dedup_bb_messages`]).
pub(crate) const BB_MIRROR_DEDUP_PREFIX: &str = "bb:mirror:";

fn mirror_dedup_key(mirror: Option<BbProviderMirror<'_>>, identity: String) -> String {
    match mirror {
        // The segments are `:`-separated, so an id holding one cannot be
        // read back; such a row keeps its plain key and stays counted.
        Some(mirror)
            if !mirror.session.contains(':')
                && !mirror.turn.is_some_and(|turn| turn.contains(':')) =>
        {
            format!(
                "{BB_MIRROR_DEDUP_PREFIX}{}:{}:{}:{identity}",
                mirror.provider.as_str(),
                mirror.session,
                mirror.turn.unwrap_or_default(),
            )
        }
        _ => identity,
    }
}

/// The provider session a BB row's dedup key names, if any.
pub(crate) fn provider_mirror_from_dedup_key(dedup_key: &str) -> Option<BbProviderMirror<'_>> {
    let mut segments = dedup_key
        .strip_prefix(BB_MIRROR_DEDUP_PREFIX)?
        .splitn(4, ':');
    let provider = BbMirrorProvider::from_key_segment(segments.next()?)?;
    let session = segments.next().filter(|session| !session.is_empty())?;
    let turn = segments.next()?;
    segments.next()?;
    Some(BbProviderMirror {
        provider,
        session,
        turn: (!turn.is_empty()).then_some(turn),
    })
}

/// The part of a BB dedup key that identifies the event itself, without the
/// provider identity in front of it.
fn bb_event_identity(dedup_key: &str) -> &str {
    dedup_key
        .strip_prefix(BB_MIRROR_DEDUP_PREFIX)
        .and_then(|rest| rest.splitn(4, ':').nth(3))
        .unwrap_or(dedup_key)
}

/// How much of the provider's identity a BB row names: nothing, the session,
/// or the session and the turn.
fn provider_identity_rank(message: &UnifiedMessage) -> u8 {
    match message
        .dedup_key
        .as_deref()
        .and_then(provider_mirror_from_dedup_key)
    {
        None => 0,
        Some(mirror) if mirror.turn.is_none() => 1,
        Some(_) => 2,
    }
}

/// Collapse BB rows that record the same event: a forked thread copies its
/// parent's history with the same turn ids and sequence numbers (see
/// [`scan_bb_sqlite`]), and several databases can hold one thread.
///
/// The copy keeps the first row's position. When a later copy names more of
/// the provider's identity, that copy replaces it, so the lanes can still
/// match the event against the provider's own log: BB clears
/// `provider_thread_id` on copied rows, an older copy may lack
/// `providerThreadId` in its data, and a thread forked while a turn was
/// running has the turn's usage row without its `turn/completed` checkpoint.
/// Either way the kept row is a turn start when any copy was.
pub(crate) fn dedup_bb_messages(
    messages: impl IntoIterator<Item = UnifiedMessage>,
) -> Vec<UnifiedMessage> {
    let mut kept: Vec<UnifiedMessage> = Vec::new();
    let mut index_by_identity: HashMap<String, usize> = HashMap::new();
    for message in messages {
        let Some(identity) = message.dedup_key.as_deref().map(bb_event_identity) else {
            kept.push(message);
            continue;
        };
        match index_by_identity.get(identity) {
            Some(&index) => {
                // The scan marks a turn's start on the first row it reads for
                // that turn id, across threads, so the start may be on the
                // copy that is dropped. The kept row inherits it.
                let is_turn_start = kept[index].is_turn_start || message.is_turn_start;
                if provider_identity_rank(&message) > provider_identity_rank(&kept[index]) {
                    kept[index] = message;
                }
                kept[index].is_turn_start = is_turn_start;
            }
            None => {
                index_by_identity.insert(identity.to_string(), kept.len());
                kept.push(message);
            }
        }
    }
    kept
}

/// The Claude sessions BB rows name: the sessions whose transcript rows the
/// Claude lane has to match turn by turn.
pub(crate) fn claude_sessions_named(messages: &[UnifiedMessage]) -> HashSet<String> {
    messages
        .iter()
        .filter_map(|message| message.dedup_key.as_deref())
        .filter_map(provider_mirror_from_dedup_key)
        .filter(|mirror| mirror.provider == BbMirrorProvider::Claude)
        .map(|mirror| mirror.session.to_string())
        .collect()
}

/// What a `turn/completed` row says about the provider side of a BB turn.
#[derive(Debug, Default)]
struct BbTurnCompletion {
    checkpoint: Option<String>,
    provider_thread_id: Option<String>,
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

/// Whether `events` has the `provider_thread_id` column (current BB schema).
fn events_has_provider_thread_column(conn: &Connection) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('events')")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(0)? == "provider_thread_id" {
            return Ok(true);
        }
    }
    Ok(false)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The provider identity of every finished turn, keyed by `(thread id, turn
/// id)`. Usage rows come before their turn's `turn/completed` row, so this is
/// read up front rather than in the event pass.
fn load_turn_completions(
    conn: &Connection,
    has_provider_thread_column: bool,
    buffer: &mut Vec<u8>,
) -> rusqlite::Result<HashMap<(String, String), BbTurnCompletion>> {
    let query = if has_provider_thread_column {
        "SELECT thread_id, turn_id, provider_thread_id, data FROM events \
         WHERE type = 'turn/completed' AND turn_id IS NOT NULL"
    } else {
        "SELECT thread_id, turn_id, NULL, data FROM events \
         WHERE type = 'turn/completed' AND turn_id IS NOT NULL"
    };
    let mut stmt = conn.prepare(query)?;
    let mut rows = stmt.query([])?;
    let mut completions = HashMap::new();
    while let Some(row) = rows.next()? {
        let thread_id: String = row.get(0)?;
        let turn_id: String = row.get(1)?;
        let column_thread: Option<String> = row.get(2)?;
        let data_json: String = row.get(3)?;
        let data = parse_json_line::<BbEventData>(&data_json, buffer);
        let (checkpoint, data_thread) = data
            .map(|data| (data.provider_checkpoint_id, data.provider_thread_id))
            .unwrap_or_default();
        completions.insert(
            (thread_id, turn_id),
            BbTurnCompletion {
                checkpoint: non_empty(checkpoint),
                provider_thread_id: non_empty(column_thread).or_else(|| non_empty(data_thread)),
            },
        );
    }
    Ok(completions)
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

    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let provider_identity = events_has_provider_thread_column(&conn).and_then(|has_column| {
        load_turn_completions(&conn, has_column, &mut buffer)
            .map(|completions| (has_column, completions))
    });
    let (has_provider_thread_column, turn_completions) = match provider_identity {
        Ok(identity) => identity,
        Err(err) => {
            // Without the provider identity every row would be counted on
            // top of the provider's own log, so do not report a partial
            // answer as a finished one.
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to load BB provider turn identity"
            );
            return BbSqliteScan {
                messages: Vec::new(),
                complete: false,
            };
        }
    };

    let db_mtime_ms = file_modified_timestamp_ms(db_path);
    let mut messages: Vec<UnifiedMessage> = Vec::new();
    let mut malformed_rows = 0;
    let mut current_thread_id: Option<String> = None;
    let mut active_fallback_model: Option<String> = None;
    let mut turn_first_message_seen: HashMap<String, bool> = HashMap::new();
    let mut thread_totals: HashMap<String, TokenBreakdown> = HashMap::new();

    let query = if has_provider_thread_column {
        "SELECT id, thread_id, turn_id, sequence, type, data, created_at, provider_thread_id \
         FROM events \
         WHERE type IN ('thread/tokenUsage/updated', 'provider/modelFallback') \
         ORDER BY thread_id, sequence ASC"
    } else {
        "SELECT id, thread_id, turn_id, sequence, type, data, created_at, NULL \
         FROM events \
         WHERE type IN ('thread/tokenUsage/updated', 'provider/modelFallback') \
         ORDER BY thread_id, sequence ASC"
    };

    let scan = sqlite_for_each_row_on(&conn, db_path, query, Some("BB event"), &mut |row| {
        let _event_id: String = row.get(0)?;
        let thread_id: String = row.get(1)?;
        let turn_id: Option<String> = row.get(2)?;
        let sequence: i64 = row.get(3)?;
        let event_type: String = row.get(4)?;
        let data_json: String = row.get(5)?;
        let created_at_ms: Option<i64> = row.get(6)?;
        let column_provider_thread: Option<String> = row.get(7)?;

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
        let identity = if let Some(turn) = &turn_id {
            format!("bb:{turn}:{sequence}:{}:{}", tokens.input, tokens.output)
        } else {
            format!(
                "bb:{timestamp}:{sequence}:{}:{}",
                tokens.input, tokens.output
            )
        };
        let completion = turn_id
            .as_ref()
            .and_then(|turn| turn_completions.get(&(thread_id.clone(), turn.clone())));
        let provider_session = non_empty(column_provider_thread)
            .or_else(|| non_empty(event_data.provider_thread_id.clone()))
            .or_else(|| completion.and_then(|done| done.provider_thread_id.clone()));
        let mirror = thread_metas
            .get(&thread_id)
            .and_then(|thread| BbMirrorProvider::for_thread_provider(&thread.provider_id))
            .zip(provider_session.as_deref())
            .map(|(provider, session)| BbProviderMirror {
                provider,
                session,
                turn: completion.and_then(|done| done.checkpoint.as_deref()),
            });
        let dedup_key = mirror_dedup_key(mirror, identity);

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
        messages.push(msg);

        Ok(())
    });
    let messages = dedup_bb_messages(messages);

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

    /// Current BB schema: `events` carries `provider_thread_id`.
    fn create_current_schema_bb_db(path: &Path) -> Connection {
        let conn = create_test_bb_db(path);
        conn.execute_batch("ALTER TABLE events ADD COLUMN provider_thread_id TEXT;")
            .unwrap();
        conn
    }

    fn insert_thread(conn: &Connection, id: &str, provider: &str, parent: Option<&str>) {
        conn.execute(
            "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
             VALUES (?1, NULL, ?2, 'claude-sonnet-4-5', NULL, ?3, 1000)",
            (id, provider, parent),
        )
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_event(
        conn: &Connection,
        id: &str,
        thread: &str,
        turn: &str,
        sequence: i64,
        kind: &str,
        column_session: Option<&str>,
        data: &str,
    ) {
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at, provider_thread_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1100, ?7)",
            (id, thread, turn, sequence, kind, data, column_session),
        )
        .unwrap();
    }

    const USAGE_DATA: &str = r#"{"providerThreadId":"sess-1","tokenUsage":{"last":{"totalTokens":150,"inputTokens":100,"outputTokens":50}}}"#;

    #[test]
    fn test_scan_bb_sqlite_names_the_provider_session_and_turn() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bb.db");
        let conn = create_current_schema_bb_db(&db_path);
        insert_thread(&conn, "thr_claude", "claude-code", None);
        insert_thread(&conn, "thr_codex", "codex", None);
        insert_thread(&conn, "thr_pi", "pi", None);
        insert_event(
            &conn,
            "e1",
            "thr_claude",
            "turn_1",
            1,
            "thread/tokenUsage/updated",
            Some("sess-1"),
            USAGE_DATA,
        );
        insert_event(
            &conn,
            "e2",
            "thr_claude",
            "turn_1",
            2,
            "turn/completed",
            Some("sess-1"),
            r#"{"providerThreadId":"sess-1","status":"completed","providerCheckpointId":"uuid-last"}"#,
        );
        // Codex: the checkpoint is the provider's turn id.
        insert_event(
            &conn,
            "e3",
            "thr_codex",
            "turn_2",
            1,
            "thread/tokenUsage/updated",
            Some("thread-9"),
            r#"{"providerThreadId":"thread-9","tokenUsage":{"last":{"totalTokens":30,"inputTokens":20,"outputTokens":10}}}"#,
        );
        insert_event(
            &conn,
            "e4",
            "thr_codex",
            "turn_2",
            2,
            "turn/completed",
            Some("thread-9"),
            r#"{"providerThreadId":"thread-9","status":"completed","providerCheckpointId":"codex-turn-2"}"#,
        );
        // A turn still running has no `turn/completed` row yet.
        insert_event(
            &conn,
            "e5",
            "thr_codex",
            "turn_3",
            3,
            "thread/tokenUsage/updated",
            Some("thread-9"),
            r#"{"providerThreadId":"thread-9","tokenUsage":{"last":{"totalTokens":12,"inputTokens":8,"outputTokens":4}}}"#,
        );
        // Not a provider with a log of its own that Tokscale reads.
        insert_event(
            &conn,
            "e6",
            "thr_pi",
            "turn_4",
            1,
            "thread/tokenUsage/updated",
            Some("pi-1"),
            USAGE_DATA,
        );

        let messages = parse_bb_sqlite(&db_path);
        let mirrors: Vec<_> = messages
            .iter()
            .map(|message| provider_mirror_from_dedup_key(message.dedup_key.as_deref().unwrap()))
            .collect();
        assert_eq!(
            mirrors,
            vec![
                Some(BbProviderMirror {
                    provider: BbMirrorProvider::Claude,
                    session: "sess-1",
                    turn: Some("uuid-last"),
                }),
                Some(BbProviderMirror {
                    provider: BbMirrorProvider::Codex,
                    session: "thread-9",
                    turn: Some("codex-turn-2"),
                }),
                Some(BbProviderMirror {
                    provider: BbMirrorProvider::Codex,
                    session: "thread-9",
                    turn: None,
                }),
                None,
            ]
        );
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("bb:mirror:claude:sess-1:uuid-last:bb:turn_1:1:100:50")
        );
        assert_eq!(
            bb_event_identity(messages[0].dedup_key.as_deref().unwrap()),
            "bb:turn_1:1:100:50"
        );
        // Accounting is unchanged: the identity only rides on the key.
        assert_eq!(messages[0].session_id, "thr_claude");
        assert_eq!(messages[0].tokens.total(), 150);
    }

    #[test]
    fn test_scan_bb_sqlite_reads_the_session_from_data_without_the_column() {
        // Databases from before `events.provider_thread_id` (and fork copies,
        // where BB clears the column) still carry `providerThreadId` in `data`.
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bb.db");
        let conn = create_test_bb_db(&db_path);
        insert_thread_without_column(&conn);
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('e1', 'thr_old', 'turn_1', 1, 'thread/tokenUsage/updated', ?1, 1100)",
            [USAGE_DATA],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('e2', 'thr_old', 'turn_1', 2, 'turn/completed', \
             '{\"status\":\"completed\",\"providerCheckpointId\":\"uuid-last\"}', 1100)",
            [],
        )
        .unwrap();

        let scan = scan_bb_sqlite(&db_path);
        assert!(scan.complete);
        assert_eq!(
            provider_mirror_from_dedup_key(scan.messages[0].dedup_key.as_deref().unwrap()),
            Some(BbProviderMirror {
                provider: BbMirrorProvider::Claude,
                session: "sess-1",
                turn: Some("uuid-last"),
            })
        );
    }

    fn insert_thread_without_column(conn: &Connection) {
        conn.execute(
            "INSERT INTO threads (id, project_id, provider_id, model_override, title, parent_thread_id, created_at) \
             VALUES ('thr_old', NULL, 'claude-code', 'claude-sonnet-4-5', NULL, NULL, 1000)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn test_scan_bb_sqlite_without_provider_session_keeps_the_plain_key() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bb.db");
        let conn = create_test_bb_db(&db_path);
        insert_thread_without_column(&conn);
        conn.execute(
            "INSERT INTO events (id, thread_id, turn_id, sequence, type, data, created_at) \
             VALUES ('e1', 'thr_old', 'turn_1', 1, 'thread/tokenUsage/updated', \
             '{\"tokenUsage\":{\"last\":{\"totalTokens\":150,\"inputTokens\":100,\"outputTokens\":50}}}', 1100)",
            [],
        )
        .unwrap();

        let messages = parse_bb_sqlite(&db_path);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("bb:turn_1:1:100:50"));
        assert_eq!(
            provider_mirror_from_dedup_key(messages[0].dedup_key.as_deref().unwrap()),
            None
        );
    }

    #[test]
    fn test_fork_copy_collapses_and_keeps_the_provider_identity() {
        // BB copies a forked thread's history with `provider_thread_id`
        // cleared. Whichever copy is read first, the kept row must still
        // name the provider session, or it could not yield to the transcript.
        for (original_thread, copy_thread) in [("thr_a", "thr_b"), ("thr_b", "thr_a")] {
            let dir = TempDir::new().unwrap();
            let db_path = dir.path().join("bb.db");
            let conn = create_current_schema_bb_db(&db_path);
            insert_thread(&conn, original_thread, "claude-code", None);
            insert_thread(&conn, copy_thread, "claude-code", Some(original_thread));
            insert_event(
                &conn,
                "e1",
                original_thread,
                "turn_1",
                1,
                "thread/tokenUsage/updated",
                Some("sess-1"),
                USAGE_DATA,
            );
            insert_event(
                &conn,
                "e2",
                copy_thread,
                "turn_1",
                1,
                "thread/tokenUsage/updated",
                None,
                r#"{"tokenUsage":{"last":{"totalTokens":150,"inputTokens":100,"outputTokens":50}}}"#,
            );

            let messages = parse_bb_sqlite(&db_path);
            assert_eq!(messages.len(), 1, "fork copy must collapse");
            let mirror =
                provider_mirror_from_dedup_key(messages[0].dedup_key.as_deref().unwrap()).unwrap();
            assert_eq!(mirror.session, "sess-1");
            assert!(
                messages[0].is_turn_start,
                "{original_thread} first: the turn keeps its one start"
            );
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message.is_turn_start)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn test_fork_copy_without_the_completion_keeps_the_turn_identity() {
        // A thread forked while a turn was still running copies the turn's
        // usage row, `providerThreadId` included, but not the
        // `turn/completed` row written afterwards. Whichever copy is read
        // first, the kept row must name the turn, or it could not be matched
        // to the provider's own record of that turn.
        for (original_thread, copy_thread) in [("thr_a", "thr_b"), ("thr_b", "thr_a")] {
            let dir = TempDir::new().unwrap();
            let db_path = dir.path().join("bb.db");
            let conn = create_current_schema_bb_db(&db_path);
            insert_thread(&conn, original_thread, "claude-code", None);
            insert_thread(&conn, copy_thread, "claude-code", Some(original_thread));
            insert_event(
                &conn,
                "e1",
                original_thread,
                "turn_1",
                1,
                "thread/tokenUsage/updated",
                Some("sess-1"),
                USAGE_DATA,
            );
            insert_event(
                &conn,
                "e2",
                original_thread,
                "turn_1",
                2,
                "turn/completed",
                Some("sess-1"),
                r#"{"providerThreadId":"sess-1","status":"completed","providerCheckpointId":"uuid-last"}"#,
            );
            insert_event(
                &conn,
                "e3",
                copy_thread,
                "turn_1",
                1,
                "thread/tokenUsage/updated",
                None,
                USAGE_DATA,
            );

            let messages = parse_bb_sqlite(&db_path);
            assert_eq!(messages.len(), 1, "fork copy must collapse");
            assert_eq!(
                provider_mirror_from_dedup_key(messages[0].dedup_key.as_deref().unwrap()),
                Some(BbProviderMirror {
                    provider: BbMirrorProvider::Claude,
                    session: "sess-1",
                    turn: Some("uuid-last"),
                }),
                "{original_thread} is the original"
            );
            assert!(messages[0].is_turn_start);
        }
    }

    #[test]
    fn test_mirror_dedup_key_rejects_unreadable_ids() {
        let identity = "bb:turn_1:1:100:50".to_string();
        let mirror = BbProviderMirror {
            provider: BbMirrorProvider::Claude,
            session: "a:b",
            turn: None,
        };
        assert_eq!(mirror_dedup_key(Some(mirror), identity.clone()), identity);
        assert_eq!(
            provider_mirror_from_dedup_key("bb:mirror:claude::x:bb:t:1:1:1"),
            None
        );
        assert_eq!(
            provider_mirror_from_dedup_key("bb:mirror:pi:s:x:bb:t:1:1:1"),
            None
        );
        assert_eq!(provider_mirror_from_dedup_key("bb:turn_1:1:100:50"), None);
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
