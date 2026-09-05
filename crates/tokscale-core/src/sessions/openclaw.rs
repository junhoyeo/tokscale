//! OpenClaw session parser
//!
//! OpenClaw keeps its transcripts in two stores, and both carry the same event
//! shape (`{"type":"message","id":…,"message":{"role":…,"usage":…}}`), so one
//! ingest path reads them:
//!
//! - Current OpenClaw (2026.x) writes every live transcript into a per-agent
//!   SQLite database, `~/.openclaw/agents/<agentId>/agent/openclaw-agent.sqlite`,
//!   table `transcript_events(session_id, seq, event_json, created_at)`.
//! - Legacy installs wrote one JSONL file per session under
//!   `~/.openclaw/agents/<agentId>/sessions/`, indexed by `sessions.json`.
//!   Current OpenClaw still publishes `<id>.jsonl.deleted.<ts>` /
//!   `<id>.jsonl.reset.<ts>` archives into that directory, and `openclaw
//!   doctor --fix` imports legacy JSONL history into SQLite while leaving the
//!   original files in place.
//!
//! Because a migrated transcript can therefore exist in both stores at once,
//! every assistant message that carries an event id gets a stable dedup key
//! built from the event itself (`openclaw:<event id>:<timestamp>:<input>:
//! <output>`), so the SQLite row and the JSONL line for one event agree on it
//! and so do the copies a `/fork` writes under a new session id; the caller
//! keeps the first copy it sees (SQLite first) and drops the rest. An event
//! without its own timestamp is keyed by its id and usage alone
//! (`openclaw:<event id>:<input>:<output>`), never by the timestamp a store
//! fills in for it, since the two stores fill it in differently.
//!
//! Turns OpenClaw runs through the Codex app-server harness are a special
//! case: the transcript only mirrors the final assistant message of each turn,
//! carrying the usage of the *last* model response, while Codex's own rollout
//! (under the agent's `codex-home`, or a shared user Codex home) records every
//! response. Those mirror rows are keyed `openclaw:codex-mirror:<thread
//! id>:<turn id>:…` so the caller can replace each one with the rollout's
//! record of that turn when it has one.

use super::utils::{
    file_modified_timestamp_ms, for_each_json_line, open_readonly_sqlite, parse_json_line,
    read_file_or_none, sqlite_for_each_row_on, timestamp_secs_to_ms, CamelUsage, SqliteScan,
};
use super::UnifiedMessage;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, warn};

/// Basename of the per-agent SQLite database that holds live transcripts.
pub const OPENCLAW_AGENT_DB_FILENAME: &str = "openclaw-agent.sqlite";

/// Basename OpenClaw reserves for its process-held incognito database. It is a
/// lexical sentinel rather than a file worth parsing, so discovery skips it.
pub const OPENCLAW_INCOGNITO_AGENT_DB_FILENAME: &str = "incognito-openclaw-agent.sqlite";

/// Directory OpenClaw hands Codex app-server as `CODEX_HOME` inside each agent
/// directory (`appServer.homeScope: "agent"`, the default), so the rollouts of
/// the turns it drives through Codex live at
/// `<agents root>/<agentId>/agent/codex-home/sessions/**/rollout-*.jsonl`.
pub const OPENCLAW_CODEX_HOME_DIRNAME: &str = "codex-home";

/// Scope prefix OpenClaw's Codex extension puts on the `idempotencyKey` of
/// every message it mirrors from an app-server thread:
/// `codex-app-server:<thread id>:<turn id>:assistant`.
const CODEX_MIRROR_IDEMPOTENCY_PREFIX: &str = "codex-app-server:";

/// Dedup-key namespace of an assistant row that mirrors a Codex app-server
/// turn: `openclaw:codex-mirror:<thread id>:<turn id>:` followed by the same
/// event identity as any other row. The thread and turn up front let the lane
/// match the row against the rollout's record of that turn without a second
/// field on the message. The turn segment is empty when the mirror named none.
pub(crate) const CODEX_MIRROR_DEDUP_PREFIX: &str = "openclaw:codex-mirror:";

/// The Codex app-server turn a transcript row mirrors: thread id, and the
/// turn id when the row names one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodexMirrorTurn<'a> {
    pub thread: &'a str,
    pub turn: Option<&'a str>,
}

/// Dedup key of one assistant event.
///
/// Keyed by the event's own identity and content rather than by session so
/// that every copy of it collapses: the SQLite row and the retained JSONL
/// line of a migrated transcript, and the copies OpenClaw writes under a new
/// session id when a session is forked (`/fork` copies the visible path with
/// its ids, timestamps and usage intact). An 8-character event id alone could
/// collide across sessions; the millisecond timestamp and token counts make
/// that vanishingly unlikely. An event without a timestamp of its own is
/// keyed by its id and usage alone: the timestamp a store fills in for it
/// differs between the stores and between a session and its fork copy, so it
/// cannot take part, and scoping the key to the session instead would count
/// each fork copy again.
fn openclaw_dedup_key(
    event_id: &str,
    mirror: Option<CodexMirrorTurn<'_>>,
    timestamp: Option<i64>,
    tokens: &crate::TokenBreakdown,
) -> String {
    let identity = match timestamp {
        Some(timestamp) => format!("{event_id}:{timestamp}:{}:{}", tokens.input, tokens.output),
        None => format!("{event_id}:{}:{}", tokens.input, tokens.output),
    };
    match mirror {
        Some(CodexMirrorTurn { thread, turn }) => {
            let turn = turn.unwrap_or_default();
            format!("{CODEX_MIRROR_DEDUP_PREFIX}{thread}:{turn}:{identity}")
        }
        None => format!("openclaw:{identity}"),
    }
}

/// The Codex app-server turn a dedup key mirrors, if any.
pub(crate) fn codex_mirror_turn_from_dedup_key(dedup_key: &str) -> Option<CodexMirrorTurn<'_>> {
    let mut segments = dedup_key
        .strip_prefix(CODEX_MIRROR_DEDUP_PREFIX)?
        .split(':');
    let thread = segments.next().filter(|thread| !thread.is_empty())?;
    let turn = segments.next().filter(|turn| !turn.is_empty());
    Some(CodexMirrorTurn { thread, turn })
}

/// What one JSONL file found under an OpenClaw agents root is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenClawJsonlKind {
    /// A session transcript or published archive: the OpenClaw event format.
    Transcript,
    /// A Codex rollout inside the agent's `codex-home`: Codex's own format,
    /// recording the turns OpenClaw drove through Codex app-server.
    CodexRollout,
    /// Anything else Codex keeps in that home (`history.jsonl`, logs). Not
    /// usage in either format.
    CodexHomeOther,
}

/// Classify a JSONL path the OpenClaw scan produced. The agents root is walked
/// whole, so the per-agent `codex-home` comes along with the transcripts.
pub(crate) fn classify_openclaw_jsonl(path: &Path) -> OpenClawJsonlKind {
    let components: Vec<&std::ffi::OsStr> = path.components().map(|c| c.as_os_str()).collect();
    for index in 1..components.len() {
        if components[index] == OPENCLAW_CODEX_HOME_DIRNAME && components[index - 1] == "agent" {
            return match components.get(index + 1).and_then(|c| c.to_str()) {
                Some("sessions") | Some("archived_sessions") => OpenClawJsonlKind::CodexRollout,
                _ => OpenClawJsonlKind::CodexHomeOther,
            };
        }
    }
    OpenClawJsonlKind::Transcript
}

/// How long a read waits on a transient lock. OpenClaw holds the database open
/// in WAL mode while the gateway runs, so readers normally never block, but a
/// checkpoint or recovery can hold an exclusive lock for a moment.
const OPENCLAW_SQLITE_BUSY_TIMEOUT: Duration = Duration::from_millis(1500);

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
    /// Stable per-session event id. OpenClaw assigns one to every transcript
    /// entry and preserves it when it migrates a JSONL transcript into SQLite,
    /// which is what makes cross-store deduplication possible.
    id: Option<String>,
    message: Option<OpenClawMessage>,
    #[serde(rename = "customType")]
    custom_type: Option<String>,
    data: Option<OpenClawModelData>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
    provider: Option<String>,
}

/// `api` OpenClaw stamps on assistant rows it writes itself for transcript
/// bookkeeping (channel delivery mirrors, gateway-injected notices). They
/// carry `provider: "openclaw"`, a model of [`OPENCLAW_TRANSCRIPT_ONLY_MODELS`]
/// and an all-zero usage block; they are not model output and count nothing.
/// See `src/shared/transcript-only-openclaw-assistant.ts` upstream.
const OPENCLAW_TRANSCRIPT_ARTIFACT_API: &str = "openclaw-transcript";
const OPENCLAW_TRANSCRIPT_ARTIFACT_PROVIDER: &str = "openclaw";
const OPENCLAW_TRANSCRIPT_ONLY_MODELS: [&str; 2] = ["delivery-mirror", "gateway-injected"];

#[derive(Debug, Deserialize)]
struct OpenClawMessage {
    role: Option<String>,
    usage: Option<CamelUsage>,
    timestamp: Option<i64>,
    provider: Option<String>,
    model: Option<String>,
    api: Option<String>,
    /// Set by OpenClaw's Codex extension on the assistant message it mirrors
    /// from an app-server turn (see [`CODEX_MIRROR_IDEMPOTENCY_PREFIX`]).
    #[serde(rename = "idempotencyKey")]
    idempotency_key: Option<String>,
}

impl OpenClawMessage {
    /// True for an assistant row OpenClaw authored as transcript bookkeeping
    /// rather than as a model response.
    fn is_transcript_artifact(&self) -> bool {
        if self.api.as_deref() == Some(OPENCLAW_TRANSCRIPT_ARTIFACT_API) {
            return true;
        }
        self.provider.as_deref() == Some(OPENCLAW_TRANSCRIPT_ARTIFACT_PROVIDER)
            && self
                .model
                .as_deref()
                .is_some_and(|model| OPENCLAW_TRANSCRIPT_ONLY_MODELS.contains(&model))
    }

    /// The Codex app-server turn this message mirrors, if any. The key is
    /// `codex-app-server:<thread id>:<turn id>:assistant`; a key that stops
    /// after the thread still identifies the thread.
    fn codex_mirror_turn(&self) -> Option<CodexMirrorTurn<'_>> {
        let mut segments = self
            .idempotency_key
            .as_deref()?
            .strip_prefix(CODEX_MIRROR_IDEMPOTENCY_PREFIX)?
            .split(':');
        let thread = segments.next().filter(|thread| !thread.is_empty())?;
        let turn = segments
            .next()
            .filter(|turn| !turn.is_empty() && *turn != "assistant");
        Some(CodexMirrorTurn { thread, turn })
    }
}

#[derive(Debug, Deserialize)]
struct OpenClawModelData {
    provider: Option<String>,
    #[serde(rename = "modelId")]
    model_id: Option<String>,
}

/// Model and provider carried forward between the events of one session, fed
/// by `model_change` / `model-snapshot` events and by each emitted message.
#[derive(Debug, Default)]
struct OpenClawSessionState {
    current_model: Option<String>,
    current_provider: Option<String>,
}

/// The inputs the two stores resolve differently for one event.
struct OpenClawEventContext<'a> {
    session_id: &'a str,
    /// Used when the assistant message carries no `timestamp` of its own: the
    /// file mtime for JSONL, the row's `created_at` for SQLite.
    fallback_timestamp: i64,
    /// Session-level provider/model recorded by the store itself (SQLite
    /// `session_windows`). Consulted only after the message and the
    /// in-transcript model events, because they describe the session's
    /// *current* model while earlier messages may have used another one.
    session_provider: Option<&'a str>,
    session_model: Option<&'a str>,
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Feed one transcript event through the shared interpretation. Returns the
/// usage message for an assistant event with usage, and `None` for everything
/// else (user/tool events, model bookkeeping, unknown types).
fn ingest_openclaw_entry(
    entry: OpenClawEntry,
    state: &mut OpenClawSessionState,
    ctx: &OpenClawEventContext<'_>,
) -> Option<UnifiedMessage> {
    match entry.entry_type.as_str() {
        "model_change" => {
            if let Some(model) = entry.model_id {
                state.current_model = Some(model);
            }
            if let Some(provider) = entry.provider {
                state.current_provider = Some(provider);
            }
            None
        }
        "custom" => {
            if entry.custom_type.as_deref() != Some("model-snapshot") {
                return None;
            }

            if let Some(data) = entry.data {
                if let Some(model) = data.model_id {
                    state.current_model = Some(model);
                }
                if let Some(provider) = data.provider {
                    state.current_provider = Some(provider);
                }
            }
            None
        }
        "message" => {
            let msg = entry.message?;
            if msg.role.as_deref() != Some("assistant") || msg.is_transcript_artifact() {
                return None;
            }

            let mirror: Option<(String, Option<String>)> = msg
                .codex_mirror_turn()
                .map(|turn| (turn.thread.to_string(), turn.turn.map(str::to_string)));
            let usage = msg.usage?;

            let model = msg
                .model
                .and_then(non_empty)
                .or_else(|| state.current_model.clone().and_then(non_empty))
                .or_else(|| ctx.session_model.map(str::to_string).and_then(non_empty))?;
            let provider = msg
                .provider
                .and_then(non_empty)
                .or_else(|| state.current_provider.clone().and_then(non_empty))
                .or_else(|| ctx.session_provider.map(str::to_string).and_then(non_empty))
                .unwrap_or_else(|| "unknown".to_string());

            state.current_model = Some(model.clone());
            state.current_provider = Some(provider.clone());
            let timestamp = msg.timestamp.unwrap_or(ctx.fallback_timestamp);
            let cost = usage.cost.as_ref().and_then(|c| c.total).unwrap_or(0.0);
            let tokens = usage.to_breakdown_with_reasoning();
            let dedup_key = entry.id.and_then(non_empty).map(|id| {
                openclaw_dedup_key(
                    &id,
                    mirror.as_ref().map(|(thread, turn)| CodexMirrorTurn {
                        thread,
                        turn: turn.as_deref(),
                    }),
                    msg.timestamp,
                    &tokens,
                )
            });

            Some(UnifiedMessage::new_with_dedup(
                "openclaw",
                model,
                provider,
                ctx.session_id.to_string(),
                timestamp,
                tokens,
                cost.max(0.0),
                dedup_key,
            ))
        }
        _ => None,
    }
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

    let ctx = OpenClawEventContext {
        session_id,
        fallback_timestamp: file_mtime_ms,
        session_provider: None,
        session_model: None,
    };
    let mut messages = Vec::with_capacity(64);
    let mut state = OpenClawSessionState::default();
    let mut buffer = Vec::with_capacity(4096);

    for_each_json_line(session_path, &mut |_index, trimmed| {
        let Some(entry) = parse_json_line::<OpenClawEntry>(trimmed, &mut buffer) else {
            return;
        };
        if let Some(message) = ingest_openclaw_entry(entry, &mut state, &ctx) {
            messages.push(message);
        }
    });

    messages
}

/// Transcript rows joined with the session window that owns them.
///
/// `instr` prefilters in SQLite so only rows that can matter — assistant
/// messages with a `usage` block and the model bookkeeping events — leave the
/// database; user prompts and tool results, which are the bulk of a live
/// transcript store, are never copied out or JSON-decoded. Every key it looks
/// for is a JSON string token, so the check does not depend on how the writer
/// spaced or escaped the document. `ORDER BY` follows the primary key, so the
/// scan streams in storage order without a sort.
const TRANSCRIPT_EVENTS_QUERY_WITH_SESSION_WINDOWS: &str = r#"
        SELECT e.session_id, e.event_json, e.created_at, w.model_provider, w.model
        FROM transcript_events e
        LEFT JOIN session_windows w ON w.session_id = e.session_id
        WHERE instr(e.event_json, '"usage"') > 0
           OR instr(e.event_json, '"model_change"') > 0
           OR instr(e.event_json, '"model-snapshot"') > 0
        ORDER BY e.session_id, e.seq
"#;

/// Same projection against the pre-`session_windows` schema, which kept the
/// session rows in a `sessions` table.
const TRANSCRIPT_EVENTS_QUERY_WITH_SESSIONS: &str = r#"
        SELECT e.session_id, e.event_json, e.created_at, s.model_provider, s.model
        FROM transcript_events e
        LEFT JOIN sessions s ON s.session_id = e.session_id
        WHERE instr(e.event_json, '"usage"') > 0
           OR instr(e.event_json, '"model_change"') > 0
           OR instr(e.event_json, '"model-snapshot"') > 0
        ORDER BY e.session_id, e.seq
"#;

/// Last resort for a schema whose session table is missing or lacks the
/// model columns: the transcript alone, with NULL session fallbacks.
const TRANSCRIPT_EVENTS_QUERY_BARE: &str = r#"
        SELECT e.session_id, e.event_json, e.created_at, NULL, NULL
        FROM transcript_events e
        WHERE instr(e.event_json, '"usage"') > 0
           OR instr(e.event_json, '"model_change"') > 0
           OR instr(e.event_json, '"model-snapshot"') > 0
        ORDER BY e.session_id, e.seq
"#;

/// Whether the store has a `transcript_events` table. `Err` when the probe
/// itself failed — a store another process holds exclusively, an unreadable
/// file — which says nothing about the schema.
fn has_transcript_events_table(conn: &rusqlite::Connection) -> rusqlite::Result<bool> {
    match conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'transcript_events'",
        [],
        |row| row.get::<_, i64>(0),
    ) {
        Ok(_) => Ok(true),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(err) => Err(err),
    }
}

/// What one read of a per-agent store produced.
#[derive(Debug, Default)]
pub(crate) struct OpenClawSqliteScan {
    pub messages: Vec<UnifiedMessage>,
    /// False when the store was not read to the end: it could not be opened,
    /// or iteration stopped partway. `messages` is then a prefix of the
    /// store, worth reporting for this scan (a lower bound beats nothing)
    /// but not worth caching, or every later scan would replay the shortfall
    /// until the file happens to change. A store with no transcript table is
    /// complete: there was nothing to read.
    pub complete: bool,
}

/// Parse every assistant usage event in one per-agent OpenClaw database.
///
/// Sessions whose `agent_harness_id` is `codex` (OpenClaw running Codex
/// app-server as its agent runtime) are read like any other: OpenClaw mirrors
/// the final assistant message of each Codex turn into its own transcript with
/// the usage of that turn's last model response. Those rows come out keyed by
/// the Codex thread and turn (see [`CODEX_MIRROR_DEDUP_PREFIX`]); the caller
/// swaps each for the rollout's record of that turn when it has read one, and
/// keeps it otherwise, so the usage is never silently dropped. The legacy JSONL
/// parser never filtered on the harness either.
pub fn parse_openclaw_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    scan_openclaw_sqlite(db_path).messages
}

/// [`parse_openclaw_sqlite`], also saying whether the store was read to the
/// end. The cached lane needs the distinction; see [`OpenClawSqliteScan`].
pub(crate) fn scan_openclaw_sqlite(db_path: &Path) -> OpenClawSqliteScan {
    let conn = match open_readonly_sqlite(db_path) {
        Ok(conn) => conn,
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to open OpenClaw agent database"
            );
            return OpenClawSqliteScan {
                messages: Vec::new(),
                complete: false,
            };
        }
    };
    if let Err(err) = conn.busy_timeout(OPENCLAW_SQLITE_BUSY_TIMEOUT) {
        debug!(
            db_path = %db_path.display(),
            error = %err,
            "Failed to set OpenClaw agent database busy timeout"
        );
    }

    // A partial install or a pre-transcript schema has nothing to read; that
    // is expected, not a fault worth logging on every scan. A probe that
    // could not run at all is a store that was not read.
    match has_transcript_events_table(&conn) {
        Ok(true) => {}
        Ok(false) => {
            debug!(
                db_path = %db_path.display(),
                "OpenClaw agent database has no transcript_events table; skipping"
            );
            return OpenClawSqliteScan {
                messages: Vec::new(),
                complete: true,
            };
        }
        Err(err) => {
            warn!(
                db_path = %db_path.display(),
                error = %err,
                "Failed to probe OpenClaw transcript_events table"
            );
            return OpenClawSqliteScan {
                messages: Vec::new(),
                complete: false,
            };
        }
    }

    let db_mtime_ms = file_modified_timestamp_ms(db_path);
    let mut messages: Vec<UnifiedMessage> = Vec::new();
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let mut current_session: Option<String> = None;
    let mut state = OpenClawSessionState::default();
    let mut malformed_rows: usize = 0;

    let query = transcript_events_query(&conn);
    let scan = sqlite_for_each_row_on(
        &conn,
        db_path,
        query,
        Some("OpenClaw transcript event"),
        &mut |row| {
            let session_id: String = row.get(0)?;
            let event_json: String = row.get(1)?;
            // Read as f64 so an INTEGER or a REAL column both decode; a row
            // that failed here would be skipped silently.
            let created_at: Option<f64> = row.get(2)?;
            let session_provider: Option<String> = row.get(3)?;
            let session_model: Option<String> = row.get(4)?;

            if current_session.as_deref() != Some(session_id.as_str()) {
                current_session = Some(session_id.clone());
                state = OpenClawSessionState::default();
            }

            // A row that does not decode is skipped on its own; it must not
            // end the scan, and it is not worth a warning per row.
            let Some(entry) = parse_json_line::<OpenClawEntry>(&event_json, &mut buffer) else {
                malformed_rows += 1;
                return Ok(());
            };

            let fallback_timestamp = created_at
                .filter(|value| *value > 0.0)
                .map(timestamp_secs_to_ms)
                .unwrap_or(db_mtime_ms);
            let ctx = OpenClawEventContext {
                session_id: &session_id,
                fallback_timestamp,
                session_provider: session_provider.as_deref(),
                session_model: session_model.as_deref(),
            };
            if let Some(message) = ingest_openclaw_entry(entry, &mut state, &ctx) {
                messages.push(message);
            }
            Ok(())
        },
    );

    if malformed_rows > 0 {
        debug!(
            db_path = %db_path.display(),
            malformed_rows,
            "Skipped OpenClaw transcript rows that were not valid JSON"
        );
    }

    // The driver has already logged what went wrong under the label above;
    // what is left to decide is whether `messages` is the store or a prefix
    // of it. Only a scan that iterated to the end is the store.
    let complete = match scan {
        SqliteScan::Ran => true,
        SqliteScan::Incomplete
        | SqliteScan::NotExecuted
        | SqliteScan::NotPrepared
        | SqliteScan::NotOpened => false,
    };
    OpenClawSqliteScan { messages, complete }
}

/// The transcript query this store's schema supports.
///
/// Prefers the join against the current `session_windows` table, then the
/// older `sessions` table, and reads the transcript alone when neither has
/// the model columns. Each candidate is only *prepared* here: a missing
/// table or column is the normal shape of an older store, not a fault worth
/// a warning on every scan, so the probe is silent and the query that runs
/// is the one that logs.
fn transcript_events_query(conn: &rusqlite::Connection) -> &'static str {
    for (table, query) in [
        (
            "session_windows",
            TRANSCRIPT_EVENTS_QUERY_WITH_SESSION_WINDOWS,
        ),
        ("sessions", TRANSCRIPT_EVENTS_QUERY_WITH_SESSIONS),
    ] {
        if conn
            .prepare(&format!(
                "SELECT model_provider, model FROM {table} LIMIT 0"
            ))
            .is_ok()
        {
            return query;
        }
    }
    TRANSCRIPT_EVENTS_QUERY_BARE
}

/// Synthetic per-agent database fixtures shared by the parser, scanner and
/// lane tests. The schema is the subset of OpenClaw's `openclaw-agent-schema.sql`
/// the parser touches, spelled the way OpenClaw spells it (`STRICT`, composite
/// primary keys) so a query that works here works on the real store.
#[cfg(test)]
pub(crate) mod test_fixtures {
    use rusqlite::{params, Connection};
    use std::path::Path;

    pub(crate) const AGENT_DB_SCHEMA: &str = r#"
        CREATE TABLE IF NOT EXISTS session_windows (
          session_id TEXT NOT NULL PRIMARY KEY,
          session_key TEXT NOT NULL,
          previous_session_id TEXT,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL,
          model_provider TEXT,
          model TEXT,
          agent_harness_id TEXT
        ) STRICT;
        CREATE TABLE IF NOT EXISTS transcript_events (
          session_id TEXT NOT NULL,
          seq INTEGER NOT NULL,
          event_json TEXT NOT NULL,
          created_at INTEGER NOT NULL,
          PRIMARY KEY (session_id, seq)
        ) STRICT;
        CREATE TABLE IF NOT EXISTS transcript_event_identities (
          session_id TEXT NOT NULL,
          event_id TEXT NOT NULL,
          seq INTEGER NOT NULL,
          event_type TEXT,
          parent_id TEXT,
          message_idempotency_key TEXT,
          created_at INTEGER NOT NULL,
          PRIMARY KEY (session_id, event_id)
        ) STRICT;
    "#;

    /// Create `path` with the transcript schema in WAL mode, the journal mode
    /// a running OpenClaw gateway keeps its store in.
    pub(crate) fn create_agent_db(path: &Path) -> Connection {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let conn = Connection::open(path).unwrap();
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode=WAL;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_lowercase(), "wal");
        conn.execute_batch(AGENT_DB_SCHEMA).unwrap();
        conn
    }

    pub(crate) fn insert_session_window(
        conn: &Connection,
        session_id: &str,
        provider: Option<&str>,
        model: Option<&str>,
        agent_harness_id: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO session_windows (session_id, session_key, created_at, updated_at, model_provider, model, agent_harness_id)
             VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                format!("agent:main:{session_id}"),
                1_756_548_000_000_i64,
                provider,
                model,
                agent_harness_id
            ],
        )
        .unwrap();
    }

    pub(crate) fn insert_event(
        conn: &Connection,
        session_id: &str,
        seq: i64,
        event_json: &str,
        created_at: i64,
    ) {
        conn.execute(
            "INSERT INTO transcript_events (session_id, seq, event_json, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, seq, event_json, created_at],
        )
        .unwrap();
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(event_json) {
            if let Some(event_id) = value.get("id").and_then(|id| id.as_str()) {
                conn.execute(
                    "INSERT OR IGNORE INTO transcript_event_identities (session_id, event_id, seq, event_type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        session_id,
                        event_id,
                        seq,
                        value.get("type").and_then(|kind| kind.as_str()),
                        created_at
                    ],
                )
                .unwrap();
            }
        }
    }

    /// The first row of every transcript: the session header.
    pub(crate) fn header_event(session_id: &str) -> String {
        format!(
            r#"{{"type":"session","version":3,"id":"{session_id}","timestamp":"2026-08-30T10:00:00.000Z","cwd":"/tmp"}}"#
        )
    }

    pub(crate) fn user_event(id: &str, text: &str) -> String {
        format!(
            r#"{{"type":"message","id":"{id}","parentId":null,"timestamp":"2026-08-30T10:00:00.500Z","message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}],"timestamp":1756548000500}}}}"#
        )
    }

    /// An assistant event with the usage block current OpenClaw writes,
    /// including provider/model on the message itself.
    pub(crate) fn assistant_event(
        id: &str,
        provider: &str,
        model: &str,
        usage_json: &str,
        timestamp_ms: i64,
    ) -> String {
        format!(
            r#"{{"type":"message","id":"{id}","parentId":"u1","timestamp":"2026-08-30T10:00:01.000Z","message":{{"role":"assistant","content":[{{"type":"text","text":"ok"}}],"api":"anthropic-messages","provider":"{provider}","model":"{model}","usage":{usage_json},"stopReason":"stop","timestamp":{timestamp_ms}}}}}"#
        )
    }
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
    fn test_parse_openclaw_transcript_derives_session_id_from_doctor_backup_filename() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"model_change","provider":"openai-codex","modelId":"gpt-5.4"}
{"type":"message","id":"msg1","message":{"role":"assistant","content":[],"usage":{"input":10,"output":5},"timestamp":1700000000000}}"#;

        let session_path = create_test_session(
            &dir,
            "my-session-123.jsonl.pre-doctor-openai-codex-repair-2026-07-01T15-35-38-171Z.bak",
            content,
        );
        let messages = parse_openclaw_transcript(Path::new(&session_path));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "my-session-123");
        assert_eq!(messages[0].provider_id, "openai-codex");
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
    // ---- SQLite transcript store ------------------------------------------------

    use super::test_fixtures::{
        assistant_event, create_agent_db, header_event, insert_event, insert_session_window,
        user_event,
    };

    const USAGE_FULL: &str = r#"{"input":100,"output":50,"cacheRead":200,"cacheWrite":10,"reasoningTokens":20,"totalTokens":360,"cost":{"input":0.001,"output":0.002,"cacheRead":0.0005,"cacheWrite":0.0001,"total":0.0036}}"#;

    #[test]
    fn test_parse_openclaw_sqlite_parses_assistant_usage_fields() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("agent").join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        insert_session_window(
            &conn,
            "sess-a",
            Some("anthropic"),
            Some("claude-opus-4-6"),
            Some("openclaw"),
        );
        insert_event(
            &conn,
            "sess-a",
            0,
            &header_event("sess-a"),
            1_756_548_000_000,
        );
        insert_event(
            &conn,
            "sess-a",
            1,
            &user_event("u1", "hello"),
            1_756_548_000_500,
        );
        insert_event(
            &conn,
            "sess-a",
            2,
            &assistant_event(
                "a1",
                "anthropic",
                "claude-opus-4-6",
                USAGE_FULL,
                1_756_548_001_000,
            ),
            1_756_548_001_000,
        );
        drop(conn);

        let messages = parse_openclaw_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "openclaw");
        assert_eq!(message.session_id, "sess-a");
        assert_eq!(message.provider_id, "anthropic");
        assert_eq!(message.model_id, "claude-opus-4-6");
        assert_eq!(message.timestamp, 1_756_548_001_000);
        assert_eq!(message.tokens.input, 100);
        // `reasoningTokens` is a subset of `output`, so it is moved out of it.
        assert_eq!(message.tokens.output, 30);
        assert_eq!(message.tokens.reasoning, 20);
        assert_eq!(message.tokens.cache_read, 200);
        assert_eq!(message.tokens.cache_write, 10);
        assert_eq!(message.cost, 0.0036);
        assert_eq!(
            message.dedup_key.as_deref(),
            Some("openclaw:a1:1756548001000:100:30")
        );
    }

    #[test]
    fn test_parse_openclaw_sqlite_ignores_user_and_tool_result_events() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        insert_session_window(
            &conn,
            "sess-a",
            Some("anthropic"),
            Some("claude-opus-4-6"),
            None,
        );
        insert_event(&conn, "sess-a", 0, &header_event("sess-a"), 1);
        // A user prompt that happens to quote a usage block must not count.
        insert_event(
            &conn,
            "sess-a",
            1,
            r#"{"type":"message","id":"u1","message":{"role":"user","content":[{"type":"text","text":"paste: {\"usage\":{\"input\":9,\"output\":9}}"}],"usage":{"input":9,"output":9},"timestamp":1756548000500}}"#,
            1_756_548_000_500,
        );
        insert_event(
            &conn,
            "sess-a",
            2,
            r#"{"type":"message","id":"t1","message":{"role":"toolResult","toolCallId":"call_1","toolName":"read","content":[{"type":"text","text":"file body"}],"usage":{"input":5,"output":5},"timestamp":1756548000700}}"#,
            1_756_548_000_700,
        );
        insert_event(
            &conn,
            "sess-a",
            3,
            &assistant_event(
                "a1",
                "anthropic",
                "claude-opus-4-6",
                r#"{"input":10,"output":5}"#,
                1_756_548_001_000,
            ),
            1_756_548_001_000,
        );
        // An assistant event without usage carries nothing to count.
        insert_event(
            &conn,
            "sess-a",
            4,
            r#"{"type":"message","id":"a2","message":{"role":"assistant","content":[],"provider":"anthropic","model":"claude-opus-4-6","timestamp":1756548002000}}"#,
            1_756_548_002_000,
        );
        drop(conn);

        let messages = parse_openclaw_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("openclaw:a1:1756548001000:10:5")
        );
        assert_eq!(messages[0].tokens.input, 10);
    }

    #[test]
    fn test_parse_openclaw_sqlite_skips_malformed_rows_and_keeps_the_rest() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        insert_session_window(
            &conn,
            "sess-a",
            Some("anthropic"),
            Some("claude-opus-4-6"),
            None,
        );
        insert_event(
            &conn,
            "sess-a",
            0,
            &assistant_event(
                "a1",
                "anthropic",
                "claude-opus-4-6",
                r#"{"input":1,"output":1}"#,
                1_756_548_001_000,
            ),
            1_756_548_001_000,
        );
        insert_event(
            &conn,
            "sess-a",
            1,
            r#"{"type":"message","id":"broken","message":{"role":"assistant","usage":{"input":"#,
            1_756_548_001_500,
        );
        insert_event(
            &conn,
            "sess-a",
            2,
            "not json at all \"usage\"",
            1_756_548_001_600,
        );
        insert_event(
            &conn,
            "sess-a",
            3,
            &assistant_event(
                "a3",
                "anthropic",
                "claude-opus-4-6",
                r#"{"input":2,"output":2}"#,
                1_756_548_002_000,
            ),
            1_756_548_002_000,
        );
        drop(conn);

        let messages = parse_openclaw_sqlite(&db_path);
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("openclaw:a1:1756548001000:1:1")
        );
        assert_eq!(
            messages[1].dedup_key.as_deref(),
            Some("openclaw:a3:1756548002000:2:2")
        );
    }

    #[test]
    fn test_parse_openclaw_sqlite_tolerates_missing_database_and_tables() {
        let dir = TempDir::new().unwrap();

        // No file at all.
        assert!(parse_openclaw_sqlite(&dir.path().join("missing.sqlite")).is_empty());

        // A database from before transcripts moved into SQLite: auth tables
        // only, no transcript_events.
        let old_db = dir.path().join("old.sqlite");
        let conn = rusqlite::Connection::open(&old_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE auth_profile_store (profile_id TEXT PRIMARY KEY, payload TEXT NOT NULL);",
        )
        .unwrap();
        drop(conn);
        assert!(parse_openclaw_sqlite(&old_db).is_empty());

        // Not a SQLite file.
        let garbage = dir.path().join("garbage.sqlite");
        std::fs::write(&garbage, b"this is not a database").unwrap();
        assert!(parse_openclaw_sqlite(&garbage).is_empty());
    }

    #[test]
    fn test_parse_openclaw_sqlite_reads_older_sessions_table_and_bare_transcripts() {
        let dir = TempDir::new().unwrap();

        // Pre-`session_windows` schema: the session rows lived in `sessions`.
        let sessions_db = dir.path().join("sessions-schema.sqlite");
        let conn = rusqlite::Connection::open(&sessions_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (session_id TEXT PRIMARY KEY, session_key TEXT NOT NULL, model_provider TEXT, model TEXT);
             CREATE TABLE transcript_events (session_id TEXT NOT NULL, seq INTEGER NOT NULL, event_json TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY (session_id, seq));",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions VALUES ('sess-old', 'agent:main:main', 'openai', 'gpt-5.2')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcript_events VALUES ('sess-old', 0, ?1, 1756548001000)",
            rusqlite::params![
                r#"{"type":"message","id":"a1","message":{"role":"assistant","content":[],"usage":{"input":7,"output":3},"timestamp":1756548001000}}"#
            ],
        )
        .unwrap();
        drop(conn);
        let messages = parse_openclaw_sqlite(&sessions_db);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].model_id, "gpt-5.2");

        // No session table at all: the transcript alone still parses, and a
        // message that names its own model keeps the legacy `unknown`
        // provider fallback.
        let bare_db = dir.path().join("bare.sqlite");
        let conn = rusqlite::Connection::open(&bare_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE transcript_events (session_id TEXT NOT NULL, seq INTEGER NOT NULL, event_json TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY (session_id, seq));",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcript_events VALUES ('sess-bare', 0, ?1, 1756548001000)",
            rusqlite::params![
                r#"{"type":"message","id":"a1","message":{"role":"assistant","model":"claude-sonnet-4-6","content":[],"usage":{"input":7,"output":3},"timestamp":1756548001000}}"#
            ],
        )
        .unwrap();
        drop(conn);
        let messages = parse_openclaw_sqlite(&bare_db);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "unknown");
        assert_eq!(messages[0].model_id, "claude-sonnet-4-6");
    }

    #[test]
    fn test_parse_openclaw_sqlite_provider_and_model_fallback_order() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        insert_session_window(
            &conn,
            "sess-a",
            Some("window-provider"),
            Some("window-model"),
            None,
        );
        insert_event(&conn, "sess-a", 0, &header_event("sess-a"), 1);
        // 1. Nothing on the message, no model event yet: the session window
        //    supplies both.
        insert_event(
            &conn,
            "sess-a",
            1,
            r#"{"type":"message","id":"a1","message":{"role":"assistant","content":[],"usage":{"input":1,"output":1},"timestamp":1756548001000}}"#,
            1_756_548_001_000,
        );
        // 2. A model_change event outranks the window.
        insert_event(
            &conn,
            "sess-a",
            2,
            r#"{"type":"model_change","id":"m1","provider":"event-provider","modelId":"event-model","timestamp":"2026-08-30T10:00:01.500Z"}"#,
            1_756_548_001_500,
        );
        insert_event(
            &conn,
            "sess-a",
            3,
            r#"{"type":"message","id":"a2","message":{"role":"assistant","content":[],"usage":{"input":1,"output":1},"timestamp":1756548002000}}"#,
            1_756_548_002_000,
        );
        // 3. The message's own fields outrank everything; empty strings do not
        //    count as present.
        insert_event(
            &conn,
            "sess-a",
            4,
            r#"{"type":"message","id":"a3","message":{"role":"assistant","provider":"message-provider","model":"message-model","content":[],"usage":{"input":1,"output":1},"timestamp":1756548003000}}"#,
            1_756_548_003_000,
        );
        insert_event(
            &conn,
            "sess-a",
            5,
            r#"{"type":"message","id":"a4","message":{"role":"assistant","provider":"","model":"","content":[],"usage":{"input":1,"output":1},"timestamp":1756548004000}}"#,
            1_756_548_004_000,
        );
        // A second session with no window row and no model anywhere: the
        // message is dropped rather than attributed to a guess, and state
        // from the first session must not leak into it.
        insert_event(
            &conn,
            "sess-b",
            0,
            r#"{"type":"message","id":"b1","message":{"role":"assistant","content":[],"usage":{"input":1,"output":1},"timestamp":1756548005000}}"#,
            1_756_548_005_000,
        );
        drop(conn);

        let messages = parse_openclaw_sqlite(&db_path);
        let by_key: Vec<(&str, &str, &str)> = messages
            .iter()
            .map(|m| {
                (
                    m.dedup_key.as_deref().unwrap(),
                    m.provider_id.as_str(),
                    m.model_id.as_str(),
                )
            })
            .collect();
        assert_eq!(
            by_key,
            vec![
                (
                    "openclaw:a1:1756548001000:1:1",
                    "window-provider",
                    "window-model"
                ),
                (
                    "openclaw:a2:1756548002000:1:1",
                    "event-provider",
                    "event-model"
                ),
                (
                    "openclaw:a3:1756548003000:1:1",
                    "message-provider",
                    "message-model"
                ),
                // Carries the last emitted message's attribution forward.
                (
                    "openclaw:a4:1756548004000:1:1",
                    "message-provider",
                    "message-model"
                ),
            ]
        );
    }

    #[test]
    fn test_parse_openclaw_sqlite_counts_codex_harness_sessions() {
        // OpenClaw running Codex app-server as its agent runtime mirrors the
        // final assistant message of every turn, with the token usage Codex
        // reported (reasoning as a subset of output, cost zeroed), into its
        // own transcript. By default that Codex turn's rollout lives in a
        // per-agent CODEX_HOME nothing else scans, so this row is the only
        // local record of the usage and must be counted, not filtered on
        // `agent_harness_id`.
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        insert_session_window(
            &conn,
            "sess-codex",
            Some("openai"),
            Some("gpt-5.2-codex"),
            Some("codex"),
        );
        insert_event(&conn, "sess-codex", 0, &header_event("sess-codex"), 1);
        insert_event(&conn, "sess-codex", 1, &user_event("u1", "hi"), 2);
        insert_event(
            &conn,
            "sess-codex",
            2,
            r#"{"type":"message","id":"c1","message":{"role":"assistant","content":[{"type":"text","text":"done"}],"api":"openai-chatgpt-responses","provider":"openai","model":"gpt-5.2-codex","usage":{"input":1200,"output":300,"cacheRead":800,"cacheWrite":0,"reasoningTokens":120,"totalTokens":2300,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"stop","timestamp":1756548001000}}"#,
            1_756_548_001_000,
        );
        drop(conn);

        let messages = parse_openclaw_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "openclaw");
        assert_eq!(message.provider_id, "openai");
        assert_eq!(message.model_id, "gpt-5.2-codex");
        assert_eq!(message.tokens.input, 1200);
        assert_eq!(message.tokens.output, 180);
        assert_eq!(message.tokens.reasoning, 120);
        assert_eq!(message.tokens.cache_read, 800);
        // A zero provider cost is kept as zero here; the pricing pass
        // estimates it later exactly as it does for legacy JSONL rows.
        assert_eq!(message.cost, 0.0);
    }

    #[test]
    fn test_parse_openclaw_sqlite_falls_back_to_row_created_at_for_timestamp() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        insert_session_window(
            &conn,
            "sess-a",
            Some("anthropic"),
            Some("claude-opus-4-6"),
            None,
        );
        insert_event(
            &conn,
            "sess-a",
            0,
            r#"{"type":"message","id":"a1","message":{"role":"assistant","content":[],"usage":{"input":1,"output":1}}}"#,
            1_756_548_001_000,
        );
        // Seconds-resolution `created_at` is normalized to milliseconds.
        insert_event(
            &conn,
            "sess-a",
            1,
            r#"{"type":"message","id":"a2","message":{"role":"assistant","content":[],"usage":{"input":1,"output":1}}}"#,
            1_756_548_002,
        );
        drop(conn);

        let messages = parse_openclaw_sqlite(&db_path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].timestamp, 1_756_548_001_000);
        assert_eq!(messages[1].timestamp, 1_756_548_002_000);
    }

    #[test]
    fn test_parse_openclaw_sqlite_degrades_when_the_store_is_exclusively_locked() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        insert_session_window(
            &conn,
            "sess-a",
            Some("anthropic"),
            Some("claude-opus-4-6"),
            None,
        );
        insert_event(
            &conn,
            "sess-a",
            0,
            &assistant_event(
                "a1",
                "anthropic",
                "claude-opus-4-6",
                r#"{"input":1,"output":1}"#,
                1_756_548_001_000,
            ),
            1_756_548_001_000,
        );

        // A writer that holds the store exclusively (locking_mode=EXCLUSIVE
        // plus an open write) shuts every other connection out. The parser
        // must give up on that store for this scan instead of failing it.
        conn.pragma_update(None, "locking_mode", "EXCLUSIVE")
            .unwrap();
        conn.execute_batch(
            "BEGIN IMMEDIATE;
             INSERT INTO transcript_events VALUES ('sess-a', 1, '{}', 1);",
        )
        .unwrap();
        let locked = scan_openclaw_sqlite(&db_path);
        assert!(locked.messages.is_empty());
        // Nothing was read, and the caller must not remember it as nothing
        // to read.
        assert!(!locked.complete);

        conn.execute_batch("ROLLBACK;").unwrap();
        drop(conn);
        let released = scan_openclaw_sqlite(&db_path);
        assert_eq!(released.messages.len(), 1);
        assert!(released.complete);
    }

    #[test]
    fn test_scan_openclaw_sqlite_reports_a_store_it_could_not_read_to_the_end() {
        // A read that stops partway (here the file is cut off under a table
        // that spans many pages) hands back the rows it got, marked as not
        // the whole store: worth reporting for this scan, not worth caching
        // as if it were complete.
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        let sessions = 400;
        for index in 0..sessions {
            let session = format!("sess-{index:04}");
            insert_session_window(
                &conn,
                &session,
                Some("anthropic"),
                Some("claude-opus-4-6"),
                None,
            );
            insert_event(
                &conn,
                &session,
                0,
                &assistant_event(
                    &format!("a{index:04}"),
                    "anthropic",
                    "claude-opus-4-6",
                    r#"{"input":10,"output":5,"cacheRead":0,"cacheWrite":0}"#,
                    1_756_548_000_000 + index,
                ),
                1_756_548_000_000 + index,
            );
        }
        // Fold the WAL into the main file so cutting the file off is what
        // is read.
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        drop(conn);

        let intact = scan_openclaw_sqlite(&db_path);
        assert_eq!(intact.messages.len(), sessions as usize);
        assert!(intact.complete);

        let size = std::fs::metadata(&db_path).unwrap().len();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&db_path)
            .unwrap();
        file.set_len(size / 3).unwrap();
        drop(file);

        // Whether SQLite fails before the first row or after some depends on
        // where the b-tree's pages ended up; either way the scan says what
        // it returned is not the store.
        let cut = scan_openclaw_sqlite(&db_path);
        assert!(!cut.complete);
        assert!(
            cut.messages.len() < sessions as usize,
            "read {} rows from a store cut to a third",
            cut.messages.len()
        );
    }

    #[test]
    fn test_scan_openclaw_sqlite_without_a_transcript_table_is_complete() {
        // Nothing to read is a complete read: there is nothing a later scan
        // would find that this one missed.
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE unrelated (id INTEGER PRIMARY KEY);")
            .unwrap();
        drop(conn);
        let scan = scan_openclaw_sqlite(&db_path);
        assert!(scan.messages.is_empty());
        assert!(scan.complete);
    }

    #[test]
    fn test_parse_openclaw_sqlite_and_jsonl_agree_on_the_same_events() {
        // The two stores share one interpretation: the same events yield the
        // same messages and the same dedup keys, which is what lets the lane
        // collapse a migrated transcript against its retained JSONL original.
        let dir = TempDir::new().unwrap();
        let events = [
            header_event("sess-a"),
            r#"{"type":"model_change","id":"m1","provider":"anthropic","modelId":"claude-opus-4-6"}"#.to_string(),
            user_event("u1", "hi"),
            assistant_event("a1", "anthropic", "claude-opus-4-6", USAGE_FULL, 1_756_548_001_000),
            r#"{"type":"message","id":"a2","message":{"role":"assistant","content":[],"usage":{"input":3,"output":4,"reasoningTokens":1},"timestamp":1756548002000}}"#.to_string(),
        ];

        let jsonl_path = dir.path().join("sess-a.jsonl");
        std::fs::write(&jsonl_path, events.join("\n")).unwrap();
        let from_jsonl = parse_openclaw_transcript(&jsonl_path);

        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        insert_session_window(
            &conn,
            "sess-a",
            Some("anthropic"),
            Some("claude-opus-4-6"),
            None,
        );
        for (seq, event) in events.iter().enumerate() {
            insert_event(
                &conn,
                "sess-a",
                seq as i64,
                event,
                1_756_548_000_000 + seq as i64,
            );
        }
        drop(conn);
        let from_sqlite = parse_openclaw_sqlite(&db_path);

        assert_eq!(from_jsonl.len(), 2);
        assert_eq!(from_jsonl, from_sqlite);
        assert_eq!(
            from_jsonl[0].dedup_key.as_deref(),
            Some("openclaw:a1:1756548001000:100:30")
        );
        assert_eq!(
            from_jsonl[1].dedup_key.as_deref(),
            Some("openclaw:a2:1756548002000:3:3")
        );
        assert_eq!(from_jsonl[1].tokens.output, 3);
        assert_eq!(from_jsonl[1].tokens.reasoning, 1);
    }

    #[test]
    fn test_codex_mirror_rows_are_keyed_by_thread_and_turn_in_both_stores() {
        // The assistant row OpenClaw mirrors from a Codex app-server turn
        // carries `idempotencyKey: codex-app-server:<thread>:<turn>:assistant`.
        // Its dedup key leads with the thread and turn so the lane can replace
        // it with the rollout's record of exactly that turn, and the two
        // stores agree on the key.
        let dir = TempDir::new().unwrap();
        let mirror = r#"{"type":"message","id":"m1","message":{"role":"assistant","content":[{"type":"text","text":"done"}],"api":"openai-chatgpt-responses","provider":"openai","model":"gpt-5.2-codex","usage":{"input":400,"output":300,"cacheRead":800,"cacheWrite":0,"totalTokens":1500,"cost":{"total":0}},"idempotencyKey":"codex-app-server:0192f3a4-5b6c-7d8e-9f01-23456789abcd:turn-7:assistant","__openclaw":{"mirrorOrigin":"codex-app-server","mirrorIdentity":"turn-7:assistant"},"stopReason":"stop","timestamp":1756548001000}}"#;
        let plain = r#"{"type":"message","id":"m2","message":{"role":"assistant","content":[],"provider":"anthropic","model":"claude-opus-4-6","usage":{"input":10,"output":5},"timestamp":1756548002000}}"#;
        let events = [
            header_event("sess-a"),
            mirror.to_string(),
            plain.to_string(),
        ];

        let jsonl_path = dir.path().join("sess-a.jsonl");
        std::fs::write(&jsonl_path, events.join("\n")).unwrap();
        let from_jsonl = parse_openclaw_transcript(&jsonl_path);

        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        insert_session_window(
            &conn,
            "sess-a",
            Some("openai"),
            Some("gpt-5.2-codex"),
            Some("codex"),
        );
        for (seq, event) in events.iter().enumerate() {
            insert_event(
                &conn,
                "sess-a",
                seq as i64,
                event,
                1_756_548_000_000 + seq as i64,
            );
        }
        drop(conn);
        let from_sqlite = parse_openclaw_sqlite(&db_path);

        assert_eq!(from_jsonl, from_sqlite);
        assert_eq!(from_jsonl.len(), 2);
        assert_eq!(
            from_jsonl[0].dedup_key.as_deref(),
            Some("openclaw:codex-mirror:0192f3a4-5b6c-7d8e-9f01-23456789abcd:turn-7:m1:1756548001000:400:300")
        );
        assert_eq!(
            codex_mirror_turn_from_dedup_key(from_jsonl[0].dedup_key.as_deref().unwrap()),
            Some(CodexMirrorTurn {
                thread: "0192f3a4-5b6c-7d8e-9f01-23456789abcd",
                turn: Some("turn-7"),
            })
        );
        assert_eq!(
            from_jsonl[1].dedup_key.as_deref(),
            Some("openclaw:m2:1756548002000:10:5")
        );
        assert_eq!(
            codex_mirror_turn_from_dedup_key("openclaw:m2:1756548002000:10:5"),
            None
        );

        // A mirror that names the thread but no turn keeps an empty turn
        // segment, and reads back as thread-only.
        let thread_only = mirror.replace(
            "codex-app-server:0192f3a4-5b6c-7d8e-9f01-23456789abcd:turn-7:assistant",
            "codex-app-server:0192f3a4-5b6c-7d8e-9f01-23456789abcd",
        );
        let thread_only_path = dir.path().join("sess-b.jsonl");
        std::fs::write(
            &thread_only_path,
            [header_event("sess-b"), thread_only].join("\n"),
        )
        .unwrap();
        let from_thread_only = parse_openclaw_transcript(&thread_only_path);
        assert_eq!(
            from_thread_only[0].dedup_key.as_deref(),
            Some("openclaw:codex-mirror:0192f3a4-5b6c-7d8e-9f01-23456789abcd::m1:1756548001000:400:300")
        );
        assert_eq!(
            codex_mirror_turn_from_dedup_key(from_thread_only[0].dedup_key.as_deref().unwrap()),
            Some(CodexMirrorTurn {
                thread: "0192f3a4-5b6c-7d8e-9f01-23456789abcd",
                turn: None,
            })
        );
    }

    #[test]
    fn test_fork_copies_share_a_dedup_key_across_sessions() {
        // `/fork` copies the visible path of a transcript into a new session
        // with the same ids, timestamps and usage. Those tokens were spent
        // once, so both copies must key the same; a different event that
        // merely reuses an id (a later timestamp) must not.
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        insert_session_window(
            &conn,
            "sess-a",
            Some("anthropic"),
            Some("claude-opus-4-6"),
            None,
        );
        insert_session_window(
            &conn,
            "sess-fork",
            Some("anthropic"),
            Some("claude-opus-4-6"),
            None,
        );
        let original = assistant_event(
            "a1",
            "anthropic",
            "claude-opus-4-6",
            r#"{"input":100,"output":50}"#,
            1_756_548_001_000,
        );
        insert_event(&conn, "sess-a", 0, &original, 1_756_548_001_000);
        insert_event(&conn, "sess-fork", 0, &original, 1_756_548_900_000);
        insert_event(
            &conn,
            "sess-fork",
            1,
            &assistant_event(
                "a1",
                "anthropic",
                "claude-opus-4-6",
                r#"{"input":100,"output":50}"#,
                1_756_548_950_000,
            ),
            1_756_548_950_000,
        );
        // A timestamp-less event is keyed by its id and usage alone: the
        // fork copy carries the same id and usage, and the timestamp a store
        // fills in (the row's `created_at` here) differs per copy, so it must
        // not take part — and neither may the session, or the copy would
        // count again.
        let timeless = r#"{"type":"message","id":"t1","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-4-6","content":[],"usage":{"input":1,"output":1}}}"#;
        insert_event(&conn, "sess-a", 1, timeless, 1_756_548_002_000);
        insert_event(&conn, "sess-fork", 2, timeless, 1_756_548_960_000);
        drop(conn);

        let messages = parse_openclaw_sqlite(&db_path);
        let keys: Vec<(&str, &str)> = messages
            .iter()
            .map(|m| (m.session_id.as_str(), m.dedup_key.as_deref().unwrap()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("sess-a", "openclaw:a1:1756548001000:100:50"),
                ("sess-a", "openclaw:t1:1:1"),
                ("sess-fork", "openclaw:a1:1756548001000:100:50"),
                ("sess-fork", "openclaw:a1:1756548950000:100:50"),
                ("sess-fork", "openclaw:t1:1:1"),
            ]
        );
        // The store's own timestamp still reaches the message.
        assert_eq!(messages[1].timestamp, 1_756_548_002_000);
        assert_eq!(messages[4].timestamp, 1_756_548_960_000);
    }

    #[test]
    fn test_transcript_bookkeeping_assistant_rows_are_ignored() {
        // OpenClaw writes its own assistant rows for channel delivery mirrors
        // and gateway-injected notices (`api: "openclaw-transcript"`,
        // provider `openclaw`, model `delivery-mirror` / `gateway-injected`,
        // zero usage). They are not model output and must not count, not
        // even as messages.
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"message","id":"d1","message":{"role":"assistant","api":"openclaw-transcript","provider":"openclaw","model":"delivery-mirror","content":[{"type":"text","text":"delivered"}],"usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"total":0}},"timestamp":1756548001000}}
{"type":"message","id":"g1","message":{"role":"assistant","provider":"openclaw","model":"gateway-injected","content":[],"usage":{"input":0,"output":0},"timestamp":1756548002000}}
{"type":"message","id":"a1","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-4-6","content":[],"usage":{"input":10,"output":5},"timestamp":1756548003000}}"#;
        let session_path = create_test_session(&dir, "session.jsonl", content);
        let messages = parse_openclaw_session(Path::new(&session_path), "test-session");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-opus-4-6");

        let db_path = dir.path().join("openclaw-agent.sqlite");
        let conn = create_agent_db(&db_path);
        insert_session_window(
            &conn,
            "sess-a",
            Some("anthropic"),
            Some("claude-opus-4-6"),
            None,
        );
        for (seq, line) in content.lines().enumerate() {
            insert_event(
                &conn,
                "sess-a",
                seq as i64,
                line,
                1_756_548_000_000 + seq as i64,
            );
        }
        drop(conn);
        let from_sqlite = parse_openclaw_sqlite(&db_path);
        assert_eq!(from_sqlite.len(), 1);
        assert_eq!(from_sqlite[0].model_id, "claude-opus-4-6");
    }

    #[test]
    fn test_classify_openclaw_jsonl_paths() {
        use OpenClawJsonlKind::*;
        let root = Path::new("/home/u/.openclaw/agents");
        assert_eq!(
            classify_openclaw_jsonl(&root.join("main/sessions/abc.jsonl")),
            Transcript
        );
        assert_eq!(
            classify_openclaw_jsonl(
                &root.join("main/sessions/abc.jsonl.reset.2026-08-30T10-00-00.000Z")
            ),
            Transcript
        );
        assert_eq!(
            classify_openclaw_jsonl(
                &root.join("main/agent/codex-home/sessions/2026/08/30/rollout-2026-08-30T10-00-00-0192f3a4-5b6c-7d8e-9f01-23456789abcd.jsonl")
            ),
            CodexRollout
        );
        assert_eq!(
            classify_openclaw_jsonl(
                &root.join("main/agent/codex-home/archived_sessions/rollout-x.jsonl")
            ),
            CodexRollout
        );
        assert_eq!(
            classify_openclaw_jsonl(&root.join("main/agent/codex-home/history.jsonl")),
            CodexHomeOther
        );
        // A session that happens to be named like the directory is still a transcript.
        assert_eq!(
            classify_openclaw_jsonl(&root.join("main/sessions/codex-home/sessions/x.jsonl")),
            Transcript
        );
    }

    #[test]
    fn test_parse_openclaw_session_dedup_key_requires_an_event_id() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"model_change","provider":"anthropic","modelId":"claude-opus-4-6"}
{"type":"message","message":{"role":"assistant","content":[],"usage":{"input":10,"output":5},"timestamp":1700000000000}}
{"type":"message","id":"","message":{"role":"assistant","content":[],"usage":{"input":10,"output":5},"timestamp":1700000001000}}
{"type":"message","id":"abc12345","message":{"role":"assistant","content":[],"usage":{"input":10,"output":5},"timestamp":1700000002000}}"#;

        let session_path = create_test_session(&dir, "session.jsonl", content);
        let messages = parse_openclaw_session(Path::new(&session_path), "test-session");

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].dedup_key, None);
        assert_eq!(messages[1].dedup_key, None);
        assert_eq!(
            messages[2].dedup_key.as_deref(),
            Some("openclaw:abc12345:1700000002000:10:5")
        );
    }

    #[test]
    fn test_parse_openclaw_session_reasoning_tokens_never_exceed_output() {
        let dir = TempDir::new().unwrap();
        let content = r#"{"type":"message","id":"a1","message":{"role":"assistant","provider":"openai","model":"gpt-5.2","content":[],"usage":{"input":10,"output":5,"reasoningTokens":9},"timestamp":1700000000000}}"#;

        let session_path = create_test_session(&dir, "session.jsonl", content);
        let messages = parse_openclaw_session(Path::new(&session_path), "test-session");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.output, 0);
        assert_eq!(messages[0].tokens.reasoning, 5);
    }
}
