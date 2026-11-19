//! Devin session parser
//!
//! Parses local session data from:
//! - Devin CLI SQLite database (`~/.local/share/devin/cli/sessions.db`)
//! - Devin CLI per-session logs (`<cli_root>/sessions/<session-id>/{meta.json, log.jsonl}`)
//! - Devin Desktop NDJSON event streams (`.../Devin/User/acp-events/*.ndjson`)
//! - Devin Desktop per-session SQLite databases (`.../devin/User/acp-messages/*.db`)

use super::utils::{
    back_anchor_timestamp, file_modified_timestamp_ms, for_each_json_line,
    open_readonly_sqlite_opt, parse_timestamp_str, parse_timestamp_value, sqlite_for_each_row_on,
    sqlite_for_each_row_on_with_params,
};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Devin CLI (SQLite + per-session JSONL)
// ---------------------------------------------------------------------------

/// `sessions.model` and `meta.json`'s `model` can be set to a Devin routing
/// mode such as `"adaptive"` or a composite Fusion profile identifier like
/// `"fusion-claude-opus-5-5-medium-sidekick-swe-2-medium"` rather than a real
/// model id. Exclude these from the session-model fallback so rows missing
/// `generation_model` are skipped (or fall back to `"devin"` on Desktop)
/// instead of being reported under a fictitious composite model.
fn is_devin_routing_mode(s: &str) -> bool {
    let trimmed = s.trim();
    trimmed.eq_ignore_ascii_case("adaptive")
        || trimmed.eq_ignore_ascii_case("fusion")
        || trimmed
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("fusion-"))
}

pub(crate) fn is_devin_cli_session_log_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "log.jsonl")
}

pub(crate) fn is_devin_cli_meta_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "meta.json")
}

pub(crate) fn devin_cli_session_meta_path(log_path: &Path) -> PathBuf {
    log_path.with_file_name("meta.json")
}

pub(crate) fn is_devin_desktop_sqlite_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("db"))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "role")]
enum DevinChatMessage {
    #[serde(rename = "assistant")]
    Assistant {
        #[serde(default)]
        metadata: Option<DevinNodeMetadata>,
    },
    // User/tool metadata need not follow the assistant usage schema.
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize, Default)]
struct DevinNodeMetadata {
    #[serde(default)]
    num_tokens: Option<i64>,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    metrics: Option<DevinMetrics>,
    #[serde(default)]
    generation_model: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DevinMetrics {
    #[serde(default)]
    input_tokens: Option<i64>,
    #[serde(default)]
    output_tokens: Option<i64>,
    #[serde(default)]
    cache_read_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_tokens: Option<i64>,
    #[serde(default)]
    total_time_ms: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct DevinCliSessionMeta {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    workspace_dirs: Option<serde_json::Value>,
}

impl DevinCliSessionMeta {
    fn effective_id(&self, source_path: &Path) -> String {
        self.id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .or_else(|| session_id_from_session_dir(source_path))
            .unwrap_or_else(|| "unknown".to_string())
    }

    fn effective_model(&self) -> Option<String> {
        self.model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string)
    }

    fn effective_title(&self) -> Option<String> {
        self.title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string)
    }

    fn effective_workspace(&self) -> Option<String> {
        self.working_directory
            .as_deref()
            .map(str::trim)
            .filter(|ws| !ws.is_empty())
            .map(str::to_string)
            .or_else(|| {
                self.workspace_dirs
                    .as_ref()
                    .and_then(|value| value.as_array())
                    .and_then(|dirs| {
                        dirs.iter().find_map(|dir| {
                            dir.as_str()
                                .map(str::trim)
                                .filter(|ws| !ws.is_empty())
                                .map(str::to_string)
                        })
                    })
            })
    }
}

fn session_id_from_session_dir(source_path: &Path) -> Option<String> {
    source_path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn load_devin_cli_session_meta(meta_path: &Path) -> (Option<DevinCliSessionMeta>, bool) {
    match std::fs::read_to_string(meta_path) {
        Ok(contents) => match serde_json::from_str::<DevinCliSessionMeta>(&contents) {
            Ok(meta) => (Some(meta), true),
            Err(_) => (None, false),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, true),
        Err(_) => (None, false),
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind")]
enum DevinLogRecord {
    #[serde(rename = "node")]
    Node {
        #[serde(default)]
        at: Option<serde_json::Value>,
        node: Box<DevinLogNode>,
    },
    // Non-node events (`tool_call_state`, `tool_call_update`, `live_row`, etc.)
    // do not carry assistant usage records.
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct DevinLogNode {
    id: i64,
    #[serde(default)]
    message: Option<DevinChatMessage>,
}

struct DevinLogNodeEntry {
    at_ms: Option<i64>,
    message: Option<DevinChatMessage>,
}

fn parse_devin_at_ms(value: &serde_json::Value) -> Option<i64> {
    if let Some(ms) = parse_timestamp_value(value) {
        return Some(ms);
    }
    if let Some(f) = value.as_f64() {
        if f.is_finite() && f > 0.0 {
            return Some(if f >= 1_000_000_000_000.0 {
                f.round() as i64
            } else {
                (f * 1000.0).round() as i64
            });
        }
    }
    None
}

/// Metadata from the authoritative Devin CLI session store (SQLite `sessions`
/// table or `sessions/*/meta.json`) that lets ACP Desktop stores recover a
/// stable session id and model. Desktop ACP file names are independent UUIDs
/// or hashes, so they cannot be compared directly with CLI session ids.
#[derive(Debug, Clone)]
struct DevinDesktopSession {
    session_id: String,
    model_id: Option<String>,
    workspace: Option<String>,
}

/// Title- and id-to-session lookup for Desktop ACP stores. A title shared by
/// more than one CLI session is deliberately treated as ambiguous: using an
/// arbitrary match could suppress unrelated Desktop usage when CLI data is
/// also present.
#[derive(Debug, Default)]
pub struct DevinDesktopSessionLookup {
    by_title: HashMap<String, Option<DevinDesktopSession>>,
    by_id: HashMap<String, DevinDesktopSession>,
}

impl DevinDesktopSessionLookup {
    fn insert_session(&mut self, title: Option<String>, mut session: DevinDesktopSession) {
        session.model_id = session
            .model_id
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty() && !is_devin_routing_mode(model))
            .map(str::to_string);

        if !session.session_id.is_empty() {
            self.by_id
                .entry(session.session_id.clone())
                .and_modify(|existing| {
                    if existing.model_id.is_none() {
                        existing.model_id = session.model_id.clone();
                    }
                    if existing.workspace.is_none() {
                        existing.workspace = session.workspace.clone();
                    }
                })
                .or_insert_with(|| session.clone());
        }

        let Some(title) = title else {
            return;
        };

        match self.by_title.entry(title) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Some(session));
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if let Some(existing) = entry.get_mut().as_mut() {
                    if existing.session_id != session.session_id {
                        entry.insert(None);
                    } else {
                        if existing.model_id.is_none() {
                            existing.model_id = session.model_id;
                        }
                        if existing.workspace.is_none() {
                            existing.workspace = session.workspace;
                        }
                    }
                }
            }
        }
    }

    fn resolve(&self, title: &str) -> Option<&DevinDesktopSession> {
        self.by_title.get(title)?.as_ref()
    }

    fn resolve_by_id(&self, session_id: &str) -> Option<&DevinDesktopSession> {
        self.by_id.get(session_id)
    }
}

fn load_devin_desktop_session_lookup_from_meta(
    meta_path: &Path,
    lookup: &mut DevinDesktopSessionLookup,
) {
    let (Some(meta), true) = load_devin_cli_session_meta(meta_path) else {
        return;
    };
    let session_id = meta.effective_id(meta_path);
    let title = meta.effective_title();
    let model_id = meta.effective_model();
    let workspace = meta.effective_workspace();
    lookup.insert_session(
        title,
        DevinDesktopSession {
            session_id,
            model_id,
            workspace,
        },
    );
}

fn load_devin_desktop_session_lookup_from_sqlite(
    db_path: &Path,
    lookup: &mut DevinDesktopSessionLookup,
) {
    let Some(conn) = open_readonly_sqlite_opt(db_path) else {
        return;
    };
    // Quiet: a Devin database without a `sessions` table or `title` column is
    // one of the candidate paths this scan probes, not a fault.
    sqlite_for_each_row_on(
        &conn,
        db_path,
        "SELECT id, title, model, working_directory FROM sessions",
        None,
        &mut |row| {
            let session_id: String = row.get(0)?;
            let title: Option<String> = row.get(1)?;
            let model_id: Option<String> = row.get(2)?;
            let workspace: Option<String> = row.get(3)?;

            let title = title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_string);
            lookup.insert_session(
                title,
                DevinDesktopSession {
                    session_id,
                    model_id: model_id.filter(|model| !model.is_empty()),
                    workspace,
                },
            );
            Ok(())
        },
    );
}

/// Load the CLI-session metadata needed to resolve Desktop ACP stores.
///
/// Accepts both legacy `sessions.db` SQLite databases and new per-session
/// `sessions/*/meta.json` files (or `log.jsonl` files, resolving their sibling
/// `meta.json`). Older or partial databases may not expose `sessions.title`;
/// those remain usable for CLI usage while Desktop stores fall back to their
/// file-based identity instead of failing the whole scan.
pub fn load_devin_desktop_session_lookup(source_paths: &[PathBuf]) -> DevinDesktopSessionLookup {
    let mut lookup = DevinDesktopSessionLookup::default();

    for source_path in source_paths {
        if is_devin_cli_meta_path(source_path) {
            load_devin_desktop_session_lookup_from_meta(source_path, &mut lookup);
        } else if is_devin_cli_session_log_path(source_path) {
            let meta_path = devin_cli_session_meta_path(source_path);
            if meta_path.is_file() {
                load_devin_desktop_session_lookup_from_meta(&meta_path, &mut lookup);
            }
        } else {
            load_devin_desktop_session_lookup_from_sqlite(source_path, &mut lookup);
        }
    }

    lookup
}

/// True when `key` is a Devin CLI dedup key derived from `metadata.request_id`
/// rather than from a database row id or log node id.
///
/// Parser generation 1 keyed every row by row id, so two persisted copies of
/// one API request both counted; generation 2 collapses them on this key.
/// Which key a dropped duplicate carried is therefore what decides whether the
/// drop is a generation difference worth proving to the receiver.
pub fn is_devin_cli_request_dedup_key(key: Option<&str>) -> bool {
    key.is_some_and(|key| key.starts_with("devin-cli:") && key.contains(":request:"))
}

/// The row query [`parse_devin_cli_sqlite`] runs, shared with the readability
/// probe so the probe can only pass when this exact statement prepares and
/// steps against the database.
///
/// Token usage metrics live inside the `chat_message` JSON blob under
/// `$.metadata.metrics`, NOT in the separate `metadata` SQL column (which is
/// always NULL in real Devin CLI databases). The per-message model is
/// `$.metadata.generation_model`; `sessions.model` is only a fallback because
/// it can be "adaptive" or a composite Fusion routing profile, not a real
/// model id.
///
/// message_nodes.created_at is stored as Unix seconds; convert to ms.
const DEVIN_CLI_MESSAGE_QUERY: &str = r#"
        SELECT
            m.row_id,
            m.session_id,
            m.chat_message,
            m.created_at * 1000 AS created_at_ms,
            s.model,
            s.working_directory
        FROM message_nodes m
        JOIN sessions s ON m.session_id = s.id
        ORDER BY m.row_id
    "#;

/// [`DEVIN_CLI_MESSAGE_QUERY`] limited to rows appended after a resume
/// row id. This public parser convenience is not a safe cache-resume
/// protocol: existing rows may change or disappear between calls.
const DEVIN_CLI_MESSAGE_QUERY_AFTER: &str = r#"
        SELECT
            m.row_id,
            m.session_id,
            m.chat_message,
            m.created_at * 1000 AS created_at_ms,
            s.model,
            s.working_directory
        FROM message_nodes m
        JOIN sessions s ON m.session_id = s.id
        WHERE m.row_id > ?1
        ORDER BY m.row_id
    "#;

/// Whether a discovered Devin CLI source (`sessions.db` or
/// `sessions/*/log.jsonl`) can actually be read end to end.
///
/// For SQLite databases, this validates the same SQL row types and assistant
/// JSON as the parser, not merely COUNT(*). A missing `message_nodes` table is
/// readable-and-empty because no parser generation counted it. Once the table
/// exists, query, stepping and decoding failures all mean incomplete history.
/// For JSONL session logs, unreadable files, invalid UTF-8, corrupt JSON lines,
/// or a corrupt sibling `meta.json` all mean incomplete history.
///
/// Cached scans propagate their own read verdict instead of calling this API
/// again: a second read cannot certify the snapshot that supplied their rows.
pub fn devin_cli_source_is_readable(path: &Path) -> bool {
    if is_devin_cli_session_log_path(path) {
        scan_devin_cli_session_log(path).1
    } else {
        scan_devin_cli_sqlite(path).1
    }
}

pub fn parse_devin_cli_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    scan_devin_cli_sqlite(db_path).0
}

pub fn parse_devin_cli_session_log(log_path: &Path) -> Vec<UnifiedMessage> {
    scan_devin_cli_session_log(log_path).0
}

/// Parse rows after a caller-supplied row id. This does not prove earlier rows
/// are unchanged, and must not be used alone to resume a persistent cache.
pub fn parse_devin_cli_sqlite_after(db_path: &Path, after_row_id: i64) -> Vec<UnifiedMessage> {
    scan_devin_cli_sqlite_inner(db_path, Some(after_row_id)).0
}

/// Read all raw rows in one statement/snapshot. The completion bit prevents a
/// partial SQLite read from becoming an apparently complete persistent cache.
pub(crate) fn scan_devin_cli_sqlite(db_path: &Path) -> (Vec<UnifiedMessage>, bool) {
    scan_devin_cli_sqlite_inner(db_path, None)
}

/// Read a per-session `sessions/<session-id>/log.jsonl` (paired with sibling
/// `meta.json` when present). Append-only overwrites for the same `node.id`
/// collapse to the latest record before emitting messages, and distinct
/// `node.id`s sharing a `request_id` deduplicate while preserving the
/// `devin-cli:{session_id}:request:{request_id}` key for cross-store dedup.
pub(crate) fn scan_devin_cli_session_log(log_path: &Path) -> (Vec<UnifiedMessage>, bool) {
    #[cfg(test)]
    cache_test_hooks::before_read(log_path);
    let fallback_timestamp = file_modified_timestamp_ms(log_path);
    let result = scan_devin_cli_session_log_inner(log_path, fallback_timestamp);
    #[cfg(test)]
    cache_test_hooks::after_read(log_path);
    result
}

fn scan_devin_cli_session_log_inner(
    log_path: &Path,
    fallback_timestamp: i64,
) -> (Vec<UnifiedMessage>, bool) {
    let Ok(file) = std::fs::File::open(log_path) else {
        return (Vec::new(), false);
    };

    let meta_path = devin_cli_session_meta_path(log_path);
    let (meta, meta_complete) = load_devin_cli_session_meta(&meta_path);
    let session_id = meta
        .as_ref()
        .map(|m| m.effective_id(log_path))
        .or_else(|| session_id_from_session_dir(log_path))
        .unwrap_or_else(|| "unknown".to_string());
    let session_model = meta.as_ref().and_then(DevinCliSessionMeta::effective_model);
    let workspace = meta
        .as_ref()
        .and_then(DevinCliSessionMeta::effective_workspace);

    let mut reader = std::io::BufReader::new(file);
    let mut buf = Vec::new();
    let mut at_start = true;
    let mut complete = meta_complete;
    let mut nodes_by_id: BTreeMap<i64, DevinLogNodeEntry> = BTreeMap::new();

    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                    if buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                }
                let start = if std::mem::take(&mut at_start) && buf.starts_with(b"\xef\xbb\xbf") {
                    3
                } else {
                    0
                };
                let Ok(text) = std::str::from_utf8(&buf[start..]) else {
                    complete = false;
                    continue;
                };
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<DevinLogRecord>(trimmed) {
                    Ok(DevinLogRecord::Node { at, node }) => {
                        let at_ms = at.as_ref().and_then(parse_devin_at_ms);
                        nodes_by_id.insert(
                            node.id,
                            DevinLogNodeEntry {
                                at_ms,
                                message: node.message,
                            },
                        );
                    }
                    Ok(DevinLogRecord::Other) => {}
                    Err(_) => {
                        complete = false;
                    }
                }
            }
            Err(_) => {
                complete = false;
                break;
            }
        }
    }

    let mut messages = Vec::new();
    let mut seen_dedup_keys: HashSet<String> = HashSet::new();
    for (node_id, entry) in nodes_by_id {
        let Some(DevinChatMessage::Assistant { metadata }) = entry.message else {
            continue;
        };
        // Prefer the assistant message's own `metadata.created_at` completion
        // timestamp over the outer event append time `at` so a later append-only
        // update to an existing node cannot drift its date bucket, falling back
        // to `at` when `created_at` is absent.
        let recorded_end_ms = metadata
            .as_ref()
            .and_then(|m| m.created_at.as_deref())
            .and_then(parse_timestamp_str)
            .or(entry.at_ms);
        let fallback_dedup_key = format!("devin-cli:{session_id}:node:{node_id}");
        let Some(unified) = build_devin_cli_message(
            &session_id,
            metadata.as_ref(),
            recorded_end_ms,
            fallback_timestamp,
            session_model.as_deref(),
            workspace.as_deref(),
            fallback_dedup_key,
        ) else {
            continue;
        };
        if let Some(key) = unified.dedup_key.as_ref() {
            if !seen_dedup_keys.insert(key.clone()) {
                continue;
            }
        }
        messages.push(unified);
    }

    (messages, complete)
}

fn scan_devin_cli_sqlite_inner(
    db_path: &Path,
    after_row_id: Option<i64>,
) -> (Vec<UnifiedMessage>, bool) {
    #[cfg(test)]
    cache_test_hooks::before_read(db_path);
    let fallback_timestamp = file_modified_timestamp_ms(db_path);
    let Some(mut conn) = open_readonly_sqlite_opt(db_path) else {
        return (Vec::new(), false);
    };
    // The legacy-table check and message SELECT must observe one schema and
    // data snapshot. An absent table is empty history, not a query failure.
    let result = match conn.transaction() {
        Ok(tx) => scan_devin_cli_snapshot(&tx, db_path, after_row_id, fallback_timestamp),
        Err(_) => (Vec::new(), false),
    };
    #[cfg(test)]
    cache_test_hooks::after_read(db_path);
    result
}

fn scan_devin_cli_snapshot(
    conn: &rusqlite::Connection,
    db_path: &Path,
    after_row_id: Option<i64>,
    fallback_timestamp: i64,
) -> (Vec<UnifiedMessage>, bool) {
    let has_messages: rusqlite::Result<bool> = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type IN ('table', 'view') AND name = 'message_nodes' COLLATE NOCASE)",
        [],
        |row| row.get(0),
    );
    match has_messages {
        Ok(true) => {}
        Ok(false) => return (Vec::new(), true),
        Err(_) => return (Vec::new(), false),
    }
    let after = after_row_id.unwrap_or(0);
    let (query, params): (&str, &[&dyn rusqlite::ToSql]) = match after_row_id {
        Some(_) => (DEVIN_CLI_MESSAGE_QUERY_AFTER, &[&after]),
        None => (DEVIN_CLI_MESSAGE_QUERY, &[]),
    };
    let mut messages = Vec::new();
    let mut rows_complete = true;
    // Quiet: older Devin databases may not yet have message_nodes.
    let scan = sqlite_for_each_row_on_with_params(conn, db_path, query, params, None, &mut |row| {
        match parse_devin_cli_row(row, fallback_timestamp) {
            Ok(Some(message)) => {
                messages.push(message);
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(error) => {
                rows_complete = false;
                Err(error)
            }
        }
    });
    (messages, scan.completed() && rows_complete)
}

fn parse_devin_cli_row(
    row: &rusqlite::Row<'_>,
    fallback_timestamp: i64,
) -> rusqlite::Result<Option<UnifiedMessage>> {
    let row_id: i64 = row.get(0)?;
    let session_id: String = row.get(1)?;
    let chat_json: String = row.get(2)?;
    let created_at_ms: Option<i64> = row.get(3)?;
    let session_model: Option<String> = row.get(4)?;
    let workspace: Option<String> = row.get(5)?;

    // Keep local skip behavior for corrupt JSON, but report the decode
    // failure to the scan so omitted rows cannot certify complete history.
    let decode_error = |error: serde_json::Error| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
    };
    let DevinChatMessage::Assistant { metadata } =
        serde_json::from_str(&chat_json).map_err(decode_error)?
    else {
        return Ok(None);
    };

    let fallback_dedup_key = format!("devin-cli:{session_id}:row:{row_id}");
    Ok(build_devin_cli_message(
        &session_id,
        metadata.as_ref(),
        created_at_ms,
        fallback_timestamp,
        session_model.as_deref(),
        workspace.as_deref(),
        fallback_dedup_key,
    ))
}

fn build_devin_cli_message(
    session_id: &str,
    metadata: Option<&DevinNodeMetadata>,
    recorded_end_ms: Option<i64>,
    fallback_timestamp: i64,
    session_model: Option<&str>,
    workspace: Option<&str>,
    fallback_dedup_key: String,
) -> Option<UnifiedMessage> {
    let metrics = metadata.and_then(|m| m.metrics.as_ref());

    // Prefer the per-message generation_model over the session-level model,
    // which can be "adaptive", a composite Fusion routing string, or empty —
    // none of which is a real model id.
    let model_id = metadata
        .and_then(|m| m.generation_model.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty() && !is_devin_routing_mode(s))
        .or_else(|| {
            session_model
                .map(str::trim)
                .filter(|s| !s.is_empty() && !is_devin_routing_mode(s))
        })
        .unwrap_or_default()
        .to_string();
    if model_id.is_empty() {
        return None;
    }

    let provider = provider_identity::inferred_provider_from_model(&model_id)
        .map(str::to_string)
        .unwrap_or_else(|| "devin".to_string());

    let tokens = match metrics {
        Some(m) => TokenBreakdown {
            input: m.input_tokens.unwrap_or(0).max(0),
            output: m.output_tokens.unwrap_or(0).max(0),
            cache_read: m.cache_read_tokens.unwrap_or(0).max(0),
            cache_write: m.cache_creation_tokens.unwrap_or(0).max(0),
            cache_write_1h: 0,
            reasoning: 0,
        },
        None => TokenBreakdown::default(),
    };

    // Fallback: if metrics are missing but num_tokens is present, attribute
    // everything to output so the message is still counted.
    let tokens = if tokens.total() == 0 {
        if let Some(num_tokens) = metadata.and_then(|m| m.num_tokens) {
            TokenBreakdown {
                output: num_tokens.max(0),
                ..TokenBreakdown::default()
            }
        } else {
            tokens
        }
    } else {
        tokens
    };
    // Assistant rows without any attributable usage must not become
    // precedence markers for a matching Desktop ACP session. Otherwise a
    // zero-metric CLI row could suppress the only real usage record.
    if tokens.total() == 0 {
        return None;
    }

    let recorded_timestamp = recorded_end_ms.unwrap_or(fallback_timestamp);
    // `message_nodes.created_at` and `log.jsonl`'s `at` / `metadata.created_at`
    // are stamped when the assistant message (including `metrics`) is
    // finalized, i.e. the turn's *end*, not its start. `total_time_ms` is
    // that turn's elapsed generation time, so sessionize()'s
    // `[timestamp, timestamp + duration_ms]` span would otherwise project
    // forward past the actual completion into phantom idle time.
    // Back-calculate the start anchor the same way #890 did for
    // Copilot's `endTime`-only records.
    let duration_ms = metrics
        .and_then(|m| m.total_time_ms)
        .map(|total_time_ms| total_time_ms.max(0));
    // Only back-calculate when `recorded_end_ms` is this row's own recorded
    // completion time: when it's absent, `recorded_timestamp` is
    // `fallback_timestamp` (the file's mtime), not this message's own end time,
    // and subtracting `total_time_ms` from it would shift the message into the
    // wrong day rather than anchor it correctly.
    let timestamp = match (
        recorded_end_ms,
        duration_ms.filter(|duration| *duration > 0),
    ) {
        (Some(end), Some(duration)) => back_anchor_timestamp(end, duration),
        _ => recorded_timestamp,
    };
    // Devin CLI can persist the same assistant response in multiple
    // `message_nodes` rows or distinct `log.jsonl` node ids (for example, a
    // transcript copy with and without UI-only tool-call metadata). `row_id`
    // and `node.id` differ between those copies, while `metadata.request_id`
    // identifies the underlying API request. Keep row_id / node_id as a
    // fallback for older records that lack `request_id`.
    let request_id = metadata
        .and_then(|metadata| metadata.request_id.as_deref())
        .map(str::trim)
        .filter(|request_id| !request_id.is_empty());
    let dedup_key = match request_id {
        Some(request_id) => format!("devin-cli:{session_id}:request:{request_id}"),
        None => fallback_dedup_key,
    };
    let mut unified = UnifiedMessage::new_with_dedup(
        "devin-cli",
        model_id,
        provider,
        session_id,
        timestamp,
        tokens,
        0.0,
        Some(dedup_key),
    );

    unified.duration_ms = duration_ms;

    if let Some(ws) = workspace {
        let workspace_key = normalize_workspace_key(ws);
        let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
        unified.set_workspace(workspace_key, workspace_label);
    }

    Some(unified)
}

#[cfg(test)]
pub(crate) mod cache_test_hooks {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{LazyLock, Mutex};

    #[derive(Default)]
    struct State {
        reads: usize,
        before_sql: Option<String>,
        after_sql: Option<String>,
        after_append_bytes: Option<Vec<u8>>,
        remove_before_read: bool,
    }

    static STATES: LazyLock<Mutex<HashMap<PathBuf, State>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    /// Path-scoped hooks keep parallel parser tests independent of cache tests.
    pub(crate) struct Probe(PathBuf);

    pub(crate) fn watch(path: &Path) -> Probe {
        STATES
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), State::default());
        Probe(path.to_path_buf())
    }

    impl Probe {
        pub(crate) fn reads(&self) -> usize {
            STATES.lock().unwrap()[&self.0].reads
        }

        pub(crate) fn remove_before_read(&self) {
            STATES
                .lock()
                .unwrap()
                .get_mut(&self.0)
                .unwrap()
                .remove_before_read = true;
        }

        pub(crate) fn before_read_sql(&self, sql: &str) {
            STATES.lock().unwrap().get_mut(&self.0).unwrap().before_sql = Some(sql.to_string());
        }

        pub(crate) fn after_read_sql(&self, sql: &str) {
            STATES.lock().unwrap().get_mut(&self.0).unwrap().after_sql = Some(sql.to_string());
        }

        pub(crate) fn after_read_append(&self, bytes: impl Into<Vec<u8>>) {
            STATES
                .lock()
                .unwrap()
                .get_mut(&self.0)
                .unwrap()
                .after_append_bytes = Some(bytes.into());
        }
    }

    impl Drop for Probe {
        fn drop(&mut self) {
            STATES.lock().unwrap().remove(&self.0);
        }
    }

    pub(super) fn before_read(path: &Path) {
        run(path, true);
    }

    pub(super) fn after_read(path: &Path) {
        run(path, false);
    }

    fn run(path: &Path, before: bool) {
        let (sql, append_bytes) = {
            let mut states = STATES.lock().unwrap();
            let Some(state) = states.get_mut(path) else {
                return;
            };
            if before {
                state.reads += 1;
                if std::mem::take(&mut state.remove_before_read) {
                    std::fs::remove_file(path).unwrap();
                }
                (state.before_sql.take(), None)
            } else {
                (state.after_sql.take(), state.after_append_bytes.take())
            }
        };
        if let Some(sql) = sql {
            rusqlite::Connection::open(path)
                .unwrap()
                .execute_batch(&sql)
                .unwrap();
        }
        if let Some(bytes) = append_bytes {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
            file.write_all(&bytes).unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// Devin Desktop (NDJSON `acp-events` + SQLite `acp-messages`)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DevinDesktopEvent {
    #[serde(default)]
    notification: Option<serde_json::Value>,
}

#[derive(Debug, Default)]
struct DevinDesktopAcpUsage {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    model_id: Option<String>,
    timestamp: Option<i64>,
}

#[derive(Debug, Default)]
struct DevinDesktopDbMetaInfo {
    session_id: Option<String>,
    title: Option<String>,
    model: Option<String>,
    workspace: Option<String>,
    timestamp: Option<i64>,
}

fn nonnegative_number(value: Option<&serde_json::Value>) -> Option<i64> {
    value
        .and_then(|value| value.as_i64())
        .map(|value| value.max(0))
}

fn notification_timestamp(notification: &serde_json::Value) -> Option<i64> {
    notification
        .pointer("/content/metadata/created_at")
        .or_else(|| notification.pointer("/metadata/created_at"))
        .or_else(|| notification.get("created_at"))
        .or_else(|| notification.get("createdAt"))
        .or_else(|| notification.get("timestamp"))
        .and_then(parse_devin_at_ms)
}

fn notification_model(notification: &serde_json::Value) -> Option<String> {
    [
        notification.pointer("/content/metadata/generation_model"),
        notification.pointer("/metadata/generation_model"),
        notification
            .get("_meta")
            .and_then(|meta| meta.get("cognition.ai/model")),
        notification.get("cognition.ai/model"),
    ]
    .into_iter()
    .flatten()
    .filter_map(|value| value.as_str())
    .map(str::trim)
    .find(|value| !value.is_empty() && !is_devin_routing_mode(value))
    .map(str::to_string)
}

struct DevinDesktopMessage<'a> {
    file_session_id: &'a str,
    explicit_session_id: Option<&'a str>,
    title: Option<&'a str>,
    model_hint: Option<&'a str>,
    session_model_hint: Option<&'a str>,
    workspace_hint: Option<&'a str>,
    timestamp: i64,
    tokens: TokenBreakdown,
}

fn desktop_message(
    path: &Path,
    lookup: &DevinDesktopSessionLookup,
    message: DevinDesktopMessage<'_>,
    dedup_suffix: impl std::fmt::Display,
) -> UnifiedMessage {
    let resolved = message
        .explicit_session_id
        .and_then(|id| lookup.resolve_by_id(id))
        .or_else(|| message.title.and_then(|title| lookup.resolve(title)));
    let session_id = resolved
        .map(|session| session.session_id.clone())
        .or_else(|| message.explicit_session_id.map(str::to_string))
        .unwrap_or_else(|| message.file_session_id.to_string());
    let model_id = message
        .model_hint
        .map(str::trim)
        .filter(|model| !model.is_empty() && !is_devin_routing_mode(model))
        .or_else(|| {
            resolved
                .and_then(|session| session.model_id.as_deref())
                .map(str::trim)
                .filter(|model| !model.is_empty() && !is_devin_routing_mode(model))
        })
        .or_else(|| {
            message
                .session_model_hint
                .map(str::trim)
                .filter(|model| !model.is_empty() && !is_devin_routing_mode(model))
        })
        .unwrap_or("devin")
        .to_string();
    let provider = provider_identity::inferred_provider_from_model(&model_id)
        .map(str::to_string)
        .unwrap_or_else(|| "devin".to_string());
    let source_key = path.to_string_lossy();
    let mut unified = UnifiedMessage::new_with_dedup(
        "devin-desktop",
        model_id,
        provider,
        session_id,
        message.timestamp,
        message.tokens,
        0.0,
        Some(format!("devin-desktop:{source_key}:{dedup_suffix}")),
    );

    if let Some(workspace) = resolved
        .and_then(|session| session.workspace.as_deref())
        .or(message.workspace_hint)
    {
        let workspace_key = normalize_workspace_key(workspace);
        let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
        unified.set_workspace(workspace_key, workspace_label);
    }

    unified
}

struct DevinDesktopAccumulator {
    file_session_id: String,
    explicit_session_id: Option<String>,
    title: Option<String>,
    session_model_hint: Option<String>,
    workspace_hint: Option<String>,
    fallback_timestamp: i64,
    acp_usage: Option<DevinDesktopAcpUsage>,
    legacy_messages: Vec<UnifiedMessage>,
}

impl DevinDesktopAccumulator {
    fn new(path: &Path, meta: DevinDesktopDbMetaInfo) -> Self {
        let fallback_timestamp = meta
            .timestamp
            .unwrap_or_else(|| file_modified_timestamp_ms(path));
        Self {
            file_session_id: session_id_from_desktop_path(path),
            explicit_session_id: meta.session_id,
            title: meta.title,
            session_model_hint: meta.model,
            workspace_hint: meta.workspace,
            fallback_timestamp,
            acp_usage: None,
            legacy_messages: Vec::new(),
        }
    }

    fn ingest_notification(
        &mut self,
        path: &Path,
        lookup: &DevinDesktopSessionLookup,
        notification: &serde_json::Value,
        kind_hint: Option<&str>,
        allow_direct_acp: bool,
        dedup_index: impl std::fmt::Display,
    ) {
        let session_update = notification
            .get("sessionUpdate")
            .and_then(|value| value.as_str());
        let effective_kind = session_update.or(kind_hint);

        if effective_kind == Some("session_info_update") {
            if let Some(updated_title) = notification
                .get("title")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_string)
            {
                self.title = Some(updated_title);
            }
            return;
        }

        // Canonical ACP usage_update fields live inside `_meta` (or directly on
        // the payload object in newer `acp-messages` rows). Never fabricate
        // tokens from ACU-only payloads such as `cognition.ai/acuAmount`.
        let acp_field = |key: &str| {
            nonnegative_number(
                notification
                    .get("_meta")
                    .and_then(|meta| meta.get(key))
                    .or_else(|| notification.get(key)),
            )
        };
        let input = acp_field("cognition.ai/inputTokens");
        let cache_read = acp_field("cognition.ai/cachedReadTokens");
        let cache_write = acp_field("cognition.ai/cachedWriteTokens");
        let output = acp_field("cognition.ai/outputTokens");
        let has_canonical_acp_tokens =
            input.is_some() || cache_read.is_some() || cache_write.is_some() || output.is_some();

        // In legacy NDJSON, `usage_update` events carry `sessionUpdate:
        // "usage_update"`. In `acp-messages/*.db`, rows carry a SQL `kind` and
        // direct ACP JSON payloads without the outer notification wrapper.
        // Only claim the event for ACP aggregation when at least one canonical
        // token field is present; otherwise let embedded-metrics extraction
        // below handle historical captures.
        if has_canonical_acp_tokens && (effective_kind == Some("usage_update") || allow_direct_acp)
        {
            let usage = self
                .acp_usage
                .get_or_insert_with(DevinDesktopAcpUsage::default);
            if let Some(input) = input {
                usage.input = input;
            }
            if let Some(cache_read) = cache_read {
                usage.cache_read = cache_read;
            }
            if let Some(cache_write) = cache_write {
                usage.cache_write = cache_write;
            }
            if let Some(output) = output {
                usage.output = usage.output.saturating_add(output);
            }
            if usage.model_id.is_none() {
                usage.model_id = notification_model(notification);
            }
            if let Some(timestamp) = notification_timestamp(notification) {
                usage.timestamp = Some(timestamp);
            }
            return;
        }

        // Look for usage metrics nested inside the event/payload. Devin Desktop
        // stores them either under a `metrics` object or directly on `metadata`.
        let usage = notification
            .pointer("/content/metadata/metrics")
            .or_else(|| notification.pointer("/metadata/metrics"))
            .or_else(|| notification.pointer("/metrics"))
            .or_else(|| notification.pointer("/content/metadata"))
            .or_else(|| notification.pointer("/metadata"));

        let Some(usage) = usage else {
            return;
        };

        let input = usage
            .get("input_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
        let output = usage
            .get("output_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
        let cache_read = usage
            .get("cache_read_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
        let cache_write = usage
            .get("cache_creation_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);

        if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 {
            return;
        }

        let model_hint = notification_model(notification);
        self.legacy_messages.push(desktop_message(
            path,
            lookup,
            DevinDesktopMessage {
                file_session_id: &self.file_session_id,
                explicit_session_id: self.explicit_session_id.as_deref(),
                title: self.title.as_deref(),
                model_hint: model_hint.as_deref(),
                session_model_hint: self.session_model_hint.as_deref(),
                workspace_hint: self.workspace_hint.as_deref(),
                timestamp: notification_timestamp(notification).unwrap_or(self.fallback_timestamp),
                tokens: TokenBreakdown {
                    input,
                    output,
                    cache_read,
                    cache_write,
                    cache_write_1h: 0,
                    reasoning: 0,
                },
            },
            dedup_index,
        ));
    }

    fn finish(self, path: &Path, lookup: &DevinDesktopSessionLookup) -> Vec<UnifiedMessage> {
        if let Some(usage) = self.acp_usage {
            let tokens = TokenBreakdown {
                // ACP's inputTokens is the complete prompt, including the
                // cachedReadTokens subset. Tokscale stores uncached input and
                // cache reads separately, so subtract the overlap before totals
                // and pricing add both fields.
                input: usage.input.saturating_sub(usage.cache_read),
                output: usage.output,
                cache_read: usage.cache_read,
                cache_write: usage.cache_write,
                cache_write_1h: 0,
                reasoning: 0,
            };
            if tokens.total() == 0 {
                return Vec::new();
            }
            return vec![desktop_message(
                path,
                lookup,
                DevinDesktopMessage {
                    file_session_id: &self.file_session_id,
                    explicit_session_id: self.explicit_session_id.as_deref(),
                    title: self.title.as_deref(),
                    model_hint: usage.model_id.as_deref(),
                    session_model_hint: self.session_model_hint.as_deref(),
                    workspace_hint: self.workspace_hint.as_deref(),
                    timestamp: usage.timestamp.unwrap_or(self.fallback_timestamp),
                    tokens,
                },
                "usage",
            )];
        }

        self.legacy_messages
    }
}

pub fn parse_devin_desktop_ndjson(path: &Path) -> Vec<UnifiedMessage> {
    parse_devin_desktop_ndjson_with_lookup(path, &DevinDesktopSessionLookup::default())
}

pub fn parse_devin_desktop_sqlite(path: &Path) -> Vec<UnifiedMessage> {
    parse_devin_desktop_sqlite_with_lookup(path, &DevinDesktopSessionLookup::default())
}

/// Parse either a legacy Devin Desktop `acp-events/*.ndjson` stream or a new
/// `acp-messages/*.db` per-session SQLite database.
pub fn parse_devin_desktop_source_with_lookup(
    path: &Path,
    lookup: &DevinDesktopSessionLookup,
) -> Vec<UnifiedMessage> {
    if is_devin_desktop_sqlite_path(path) {
        parse_devin_desktop_sqlite_with_lookup(path, lookup)
    } else {
        parse_devin_desktop_ndjson_with_lookup(path, lookup)
    }
}

/// Parse a Devin Desktop ACP event stream (`acp-events/*.ndjson`).
///
/// Canonical ACP `usage_update` events contain cumulative input/cache counts
/// and per-step output counts. They are therefore reduced to one aggregate
/// message per file. The older embedded-metrics shape remains supported as a
/// best-effort fallback for historical captures.
pub fn parse_devin_desktop_ndjson_with_lookup(
    path: &Path,
    lookup: &DevinDesktopSessionLookup,
) -> Vec<UnifiedMessage> {
    let mut acc = DevinDesktopAccumulator::new(path, DevinDesktopDbMetaInfo::default());

    for_each_json_line(path, &mut |line_index, line| {
        let Ok(event) = serde_json::from_str::<DevinDesktopEvent>(line) else {
            return;
        };

        // The Desktop app streams ACP events. Usage is not reliably present in
        // the NDJSON itself; the authoritative usage lives in the CLI store.
        // We extract any embedded usage blocks we can find, but most files will
        // yield no messages. This keeps the parser future-proof and avoids
        // double-counting CLI data.
        let Some(notification) = event.notification else {
            return;
        };

        acc.ingest_notification(path, lookup, &notification, None, false, line_index);
    });

    acc.finish(path, lookup)
}

fn read_devin_desktop_db_meta_info(conn: &rusqlite::Connection) -> DevinDesktopDbMetaInfo {
    let raw: rusqlite::Result<String> =
        conn.query_row("SELECT value FROM meta WHERE key = 'info'", [], |row| {
            row.get(0)
        });
    let Ok(raw) = raw else {
        return DevinDesktopDbMetaInfo::default();
    };
    let Ok(info) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return DevinDesktopDbMetaInfo::default();
    };
    let non_empty_str = |key: &str| {
        info.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    DevinDesktopDbMetaInfo {
        session_id: non_empty_str("sessionId")
            .or_else(|| non_empty_str("session_id"))
            .or_else(|| non_empty_str("id")),
        title: non_empty_str("title"),
        model: non_empty_str("generation_model")
            .filter(|m| !is_devin_routing_mode(m))
            .or_else(|| non_empty_str("model").filter(|m| !is_devin_routing_mode(m))),
        workspace: non_empty_str("working_directory")
            .or_else(|| non_empty_str("workingDirectory"))
            .or_else(|| non_empty_str("cwd")),
        timestamp: info
            .get("updated_at")
            .or_else(|| info.get("updatedAt"))
            .or_else(|| info.get("created_at"))
            .or_else(|| info.get("createdAt"))
            .and_then(parse_devin_at_ms),
    }
}

/// Parse a Devin Desktop `User/acp-messages/<hash>.db` SQLite database.
///
/// The `meta` table holds session metadata (`info`, `schema_version`, etc.),
/// and `messages(position, kind, payload)` holds direct ACP JSON payloads (or
/// legacy `{"notification": ...}` wrappers). ACU-only rows without token counts
/// are ignored cleanly; authoritative per-request token usage for local/Fusion
/// sessions lives in `cli/sessions/<id>/log.jsonl`.
pub fn parse_devin_desktop_sqlite_with_lookup(
    path: &Path,
    lookup: &DevinDesktopSessionLookup,
) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite_opt(path) else {
        return Vec::new();
    };

    let meta_info = read_devin_desktop_db_meta_info(&conn);
    let mut acc = DevinDesktopAccumulator::new(path, meta_info);

    // Quiet: unrelated `.db` files in a user-configured extra scan root may not
    // have an ACP `messages` table.
    sqlite_for_each_row_on(
        &conn,
        path,
        "SELECT position, kind, payload FROM messages ORDER BY position",
        None,
        &mut |row| {
            let position: i64 = row.get(0)?;
            let kind: Option<String> = row.get(1)?;
            let Some(payload_str) = row.get::<_, Option<String>>(2)? else {
                return Ok(());
            };

            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&payload_str) else {
                return Ok(());
            };
            let event = payload
                .get("notification")
                .filter(|value| value.is_object())
                .unwrap_or(&payload);
            acc.ingest_notification(path, lookup, event, kind.as_deref(), true, position);
            Ok(())
        },
    );

    acc.finish(path, lookup)
}

fn session_id_from_desktop_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_devin_cli_db(dir: &TempDir) -> std::path::PathBuf {
        let db_path = dir.path().join("sessions.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                working_directory TEXT NOT NULL,
                backend_type TEXT NOT NULL,
                model TEXT NOT NULL,
                title TEXT,
                agent_mode TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_activity_at INTEGER NOT NULL
            );
            CREATE TABLE message_nodes (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                node_id INTEGER NOT NULL,
                parent_node_id INTEGER,
                chat_message TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                metadata TEXT
            );
            "#,
        )
        .unwrap();
        db_path
    }

    fn insert_session(conn: &Connection, id: &str, working_directory: &str, model: &str) {
        conn.execute(
            "INSERT INTO sessions (id, working_directory, backend_type, model, title, agent_mode, created_at, last_activity_at) VALUES (?1, ?2, 'windsurf', ?3, NULL, 'accept-edits', 1, 1)",
            rusqlite::params![id, working_directory, model],
        )
        .unwrap();
    }

    fn set_session_title(conn: &Connection, id: &str, title: &str) {
        conn.execute(
            "UPDATE sessions SET title = ?2 WHERE id = ?1",
            rusqlite::params![id, title],
        )
        .unwrap();
    }

    /// Insert a message_nodes row. In real Devin CLI databases the SQL
    /// `metadata` column is always NULL; token metrics and generation_model
    /// live inside the `chat_message` JSON blob under `$.metadata`.
    fn insert_message(
        conn: &Connection,
        session_id: &str,
        chat_message: &str,
        created_at: i64,
    ) -> i64 {
        conn.execute(
        "INSERT INTO message_nodes (session_id, node_id, chat_message, metadata, created_at) VALUES (?1, 1, ?2, NULL, ?3)",
        rusqlite::params![session_id, chat_message, created_at],
    )
    .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn test_parse_devin_cli_after_keeps_distinct_request_rows() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        insert_session(&conn, "sess-1", "/tmp/project", "gpt-5");
        let chat = r#"{"role":"assistant","metadata":{"request_id":"req-1","metrics":{"input_tokens":10,"output_tokens":5}}}"#;
        let first = insert_message(&conn, "sess-1", chat, 1_700_000_000);
        insert_message(&conn, "sess-1", chat, 1_700_000_000);
        insert_message(&conn, "sess-1", chat, 1_700_000_000);
        let suffix = parse_devin_cli_sqlite_after(&db_path, first);
        assert_eq!(suffix.len(), 2);
        assert_eq!(suffix[0].dedup_key, suffix[1].dedup_key);
        assert!(parse_devin_cli_sqlite_after(&db_path, i64::MAX).is_empty());
    }

    #[test]
    fn test_parse_devin_cli_sqlite_reads_assistant_metrics() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        // sessions.model is "adaptive" (a routing mode), but the real model
        // is in chat_message.metadata.generation_model.
        insert_session(&conn, "sess-1", "/Users/alice/project", "adaptive");
        let chat = r#"{"role":"assistant","content":"hello","metadata":{"num_tokens":147,"generation_model":"glm-5-2-max-1m","metrics":{"input_tokens":31134,"output_tokens":147,"cache_read_tokens":8,"cache_creation_tokens":null,"total_time_ms":2846}}}"#;
        insert_message(&conn, "sess-1", chat, 1_700_000_000);
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(msg.client, "devin-cli");
        assert_eq!(msg.session_id, "sess-1");
        assert_eq!(msg.model_id, "glm-5-2-max-1m");
        // `inferred_provider_from_model` recognizes "glm" and infers "zai"
        // (Zhipu AI), taking precedence over the "devin" fallback below —
        // the same convention this file already applies to Claude/GPT
        // models (see the "anthropic" assertion further down). "devin" is
        // only used when inference can't identify the model at all.
        assert_eq!(msg.provider_id, "zai");
        assert_eq!(msg.tokens.input, 31134);
        assert_eq!(msg.tokens.output, 147);
        assert_eq!(msg.tokens.cache_read, 8);
        assert_eq!(msg.tokens.cache_write, 0);
        // `created_at` is the message row's write time (the turn's end), so
        // the message timestamp is back-calculated to the turn start:
        // created_at_ms - total_time_ms. See #890 (follow-up).
        assert_eq!(msg.timestamp, 1_700_000_000_000 - 2846);
        assert_eq!(msg.duration_ms, Some(2846));
        assert_eq!(msg.workspace_key.as_deref(), Some("/Users/alice/project"));
    }

    #[test]
    fn test_parse_devin_cli_sqlite_deduplicates_rows_by_request_id() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        insert_session(&conn, "sess-1", "/Users/alice/project", "gpt-5");

        // Devin stores some assistant responses twice, with the duplicate
        // carrying UI-only metadata. Both rows represent the same API request.
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","metadata":{"request_id":"req-1","generation_model":"gpt-5","metrics":{"input_tokens":67,"output_tokens":103}}}"#,
            1_700_000_000,
        );
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","metadata":{"request_id":"req-1","generation_model":"gpt-5","metrics":{"input_tokens":67,"output_tokens":103},"extensions":{"chisel/tool_call_content":{}}}}"#,
            1_700_000_000,
        );
        // Older records without request_id must retain distinct row-based keys.
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","metadata":{"generation_model":"gpt-5","metrics":{"input_tokens":10,"output_tokens":5}}}"#,
            1_700_000_001,
        );
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","metadata":{"generation_model":"gpt-5","metrics":{"input_tokens":20,"output_tokens":5}}}"#,
            1_700_000_002,
        );
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].tokens.input, messages[1].tokens.input);
        assert_eq!(messages[0].tokens.output, messages[1].tokens.output);
        assert_eq!(messages[0].dedup_key, messages[1].dedup_key);
        assert_ne!(messages[2].dedup_key, messages[3].dedup_key);
        // Only the request_id key marks a generation difference.
        assert!(is_devin_cli_request_dedup_key(
            messages[0].dedup_key.as_deref()
        ));
        assert!(!is_devin_cli_request_dedup_key(
            messages[2].dedup_key.as_deref()
        ));
        assert!(!is_devin_cli_request_dedup_key(None));
        assert!(!is_devin_cli_request_dedup_key(Some(
            "devin-desktop:/path:request:1"
        )));
    }

    #[test]
    fn test_devin_cli_source_is_readable_distinguishes_empty_from_broken() {
        let dir = TempDir::new().unwrap();

        // A well-formed database is readable whether or not it holds rows.
        let db_path = create_devin_cli_db(&dir);
        assert!(devin_cli_source_is_readable(&db_path));
        let conn = Connection::open(&db_path).unwrap();
        insert_session(&conn, "sess-1", "/Users/alice/project", "gpt-5");
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","metadata":{"generation_model":"gpt-5","metrics":{"input_tokens":1,"output_tokens":1}}}"#,
            1_700_000_000,
        );
        drop(conn);
        assert!(devin_cli_source_is_readable(&db_path));

        // A database predating `message_nodes` never contributed rows to any
        // generation, so it is readable-and-empty rather than a failure.
        let legacy_path = dir.path().join("legacy.db");
        Connection::open(&legacy_path)
            .unwrap()
            .execute_batch("CREATE TABLE sessions (id TEXT PRIMARY KEY)")
            .unwrap();
        assert!(devin_cli_source_is_readable(&legacy_path));

        // `message_nodes` without `sessions`, or without a column the parser
        // selects, is a schema the parser's statement cannot read: it yields
        // no rows, but not because the history is empty.
        let orphan_path = dir.path().join("orphan.db");
        Connection::open(&orphan_path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE message_nodes (row_id INTEGER PRIMARY KEY, session_id TEXT, chat_message TEXT, created_at INTEGER)",
            )
            .unwrap();
        assert!(!devin_cli_source_is_readable(&orphan_path));
        assert!(parse_devin_cli_sqlite(&orphan_path).is_empty());

        let narrow_path = dir.path().join("narrow.db");
        Connection::open(&narrow_path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, model TEXT);
                 CREATE TABLE message_nodes (row_id INTEGER PRIMARY KEY, session_id TEXT, created_at INTEGER)",
            )
            .unwrap();
        assert!(!devin_cli_source_is_readable(&narrow_path));
        assert!(parse_devin_cli_sqlite(&narrow_path).is_empty());

        // The parser returns no rows for either of these too, which is exactly
        // why a separate probe has to tell them apart from the cases above.
        let garbage_path = dir.path().join("garbage.db");
        let mut garbage = std::fs::File::create(&garbage_path).unwrap();
        garbage.write_all(b"not a sqlite database").unwrap();
        drop(garbage);
        assert!(!devin_cli_source_is_readable(&garbage_path));
        assert!(parse_devin_cli_sqlite(&garbage_path).is_empty());

        assert!(!devin_cli_source_is_readable(
            &dir.path().join("missing.db")
        ));
    }

    #[test]
    fn test_total_time_ms_timestamp_is_start_anchored() {
        // Regression (follow-up to #890): `message_nodes.created_at` is
        // stamped when the row is written, which happens once the assistant
        // message (including `metrics`) is finalized, i.e. the turn's *end*,
        // not its start. `total_time_ms` is that turn's elapsed generation
        // time, so sessionize()'s `[timestamp, timestamp + duration_ms]` span
        // would otherwise project forward past the actual completion into
        // phantom idle time. The parser must back-calculate the start anchor
        // instead.
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "claude-sonnet-4");
        let chat = r#"{"role":"assistant","content":"hello","metadata":{"generation_model":"claude-sonnet-4","metrics":{"input_tokens":100,"output_tokens":50,"total_time_ms":5000}}}"#;
        insert_message(&conn, "sess-1", chat, 1_700_000_010);
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 1);

        let msg = &messages[0];
        assert_eq!(
            msg.timestamp,
            1_700_000_010_000 - 5000,
            "timestamp must be back-calculated to the turn start (end - duration)"
        );
        assert_eq!(
            msg.duration_ms,
            Some(5000),
            "duration_ms must still span from start to the recorded end timestamp"
        );
    }

    #[test]
    fn test_parse_devin_cli_sqlite_skips_non_assistant_and_missing_model() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "glm-5-2-max-1m");
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"user","content":"hi","metadata":{"metrics":{"input_tokens":1}}}"#,
            1_700_000_000,
        );
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","content":"ok","metadata":{"generation_model":"glm-5-2","metrics":{"input_tokens":10,"output_tokens":5}}}"#,
            1_700_000_001,
        );
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 5);
    }

    #[test]
    fn test_parse_devin_cli_sqlite_falls_back_to_session_model() {
        // When generation_model is absent, fall back to sessions.model.
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "kimi-k2-7");
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","content":"ok","metadata":{"metrics":{"input_tokens":10,"output_tokens":5}}}"#,
            1_700_000_000,
        );
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "kimi-k2-7");
    }

    #[test]
    fn test_parse_devin_cli_sqlite_skips_adaptive_session_model() {
        // When generation_model is absent and sessions.model is "adaptive"
        // (a routing mode), the row should be skipped rather than reported
        // under a fictitious "adaptive" model.
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "adaptive");
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","metadata":{"metrics":{"input_tokens":10,"output_tokens":5}}}"#,
            1_700_000_000,
        );
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_devin_cli_sqlite_skips_zero_usage() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "glm-5-2-max-1m");
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","metadata":{"generation_model":"glm-5-2","metrics":{"input_tokens":-100,"output_tokens":-50,"cache_read_tokens":-10,"cache_creation_tokens":-5,"total_time_ms":-1}}}"#,
            1_700_000_000,
        );
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_devin_cli_sqlite_skips_malformed_rows_without_losing_later_usage() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();

        insert_session(&conn, "sess-1", "/Users/alice/project", "gpt-5");
        insert_message(&conn, "sess-1", "{not valid json", 1_700_000_000);
        insert_message(
            &conn,
            "sess-1",
            r#"{"role":"assistant","metadata":{"metrics":{"input_tokens":10,"output_tokens":5}}}"#,
            1_700_000_001,
        );
        drop(conn);

        let messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 5);
    }

    #[test]
    fn test_parse_devin_desktop_ndjson_extracts_usage() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("event.ndjson");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"providerId":"devin-cli","notification":{{"content":{{"text":"hello"}},"metadata":{{"input_tokens":100,"output_tokens":50,"generation_model":"claude-sonnet-4","created_at":"2026-06-16T12:00:00Z"}}}}}}"#
        ).unwrap();
        writeln!(
            file,
            r#"{{"providerId":"devin-cli","notification":{{"content":{{"text":"hi"}},"metadata":{{"input_tokens":0,"output_tokens":0}}}}}}"#
        ).unwrap();
        drop(file);

        let messages = parse_devin_desktop_ndjson(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "devin-desktop");
        assert_eq!(messages[0].model_id, "claude-sonnet-4");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].timestamp, 1_781_611_200_000);
    }

    #[test]
    fn test_parse_devin_desktop_usage_update_without_acp_fields_keeps_legacy_metrics() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("legacy-usage-update.ndjson");
        std::fs::write(
            &path,
            r#"{"notification":{"sessionUpdate":"usage_update","metadata":{"input_tokens":12,"output_tokens":3,"generation_model":"gpt-5"}}}
"#,
        )
        .unwrap();

        let messages = parse_devin_desktop_ndjson(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5");
        assert_eq!(messages[0].tokens.input, 12);
        assert_eq!(messages[0].tokens.output, 3);
    }

    #[test]
    fn test_parse_devin_desktop_ndjson_keeps_distinct_events_with_identical_usage() {
        // Two events with identical model/tokens/timestamp at different line
        // positions must both survive — they represent distinct API calls.
        // The line-index dedup key prevents collision without undercounting.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("event.ndjson");
        std::fs::write(
            &path,
            r#"{"providerId":"devin-cli","notification":{"metadata":{"input_tokens":10,"output_tokens":5,"generation_model":"gpt-5","created_at":"2026-06-16T12:00:00Z"}}}
{"providerId":"devin-cli","notification":{"metadata":{"input_tokens":10,"output_tokens":5,"generation_model":"gpt-5","created_at":"2026-06-16T12:00:00Z"}}}
"#,
        )
        .unwrap();

        let messages = parse_devin_desktop_ndjson(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[1].tokens.input, 10);
    }

    #[test]
    fn test_parse_devin_desktop_acp_usage_aggregates_and_resolves_cli_title() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        insert_session(&conn, "cli-session-1", "/Users/alice/project", "gpt-5");
        set_session_title(&conn, "cli-session-1", "Build the release");
        drop(conn);

        let path = dir.path().join("desktop-file-id.ndjson");
        std::fs::write(
            &path,
            concat!(
                r#"{"notification":{"sessionUpdate":"session_info_update","title":"Build the release"}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"session_info_update"}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"usage_update","_meta":{"cognition.ai/inputTokens":100,"cognition.ai/outputTokens":7,"cognition.ai/cachedReadTokens":20}}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"usage_update","_meta":{"cognition.ai/inputTokens":150,"cognition.ai/outputTokens":8,"cognition.ai/cachedReadTokens":30}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let lookup = load_devin_desktop_session_lookup(std::slice::from_ref(&db_path));
        let messages = parse_devin_desktop_ndjson_with_lookup(&path, &lookup);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "cli-session-1");
        assert_eq!(messages[0].model_id, "gpt-5");
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].tokens.output, 15);
        assert_eq!(messages[0].tokens.cache_read, 30);
        assert_eq!(messages[0].tokens.total(), 165);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/alice/project")
        );
    }

    #[test]
    fn test_parse_devin_desktop_does_not_resolve_an_ambiguous_title() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        insert_session(&conn, "cli-session-1", "/Users/alice/project-a", "gpt-5");
        insert_session(
            &conn,
            "cli-session-2",
            "/Users/alice/project-b",
            "claude-sonnet-4",
        );
        set_session_title(&conn, "cli-session-1", "Untitled task");
        set_session_title(&conn, "cli-session-2", "Untitled task");
        drop(conn);

        let path = dir.path().join("desktop-file-id.ndjson");
        std::fs::write(
            &path,
            concat!(
                r#"{"notification":{"sessionUpdate":"session_info_update","title":"Untitled task"}}"#,
                "\n",
                r#"{"notification":{"sessionUpdate":"usage_update","_meta":{"cognition.ai/inputTokens":100,"cognition.ai/outputTokens":7}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let lookup = load_devin_desktop_session_lookup(std::slice::from_ref(&db_path));
        let messages = parse_devin_desktop_ndjson_with_lookup(&path, &lookup);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "desktop-file-id");
        assert_eq!(messages[0].model_id, "devin");
    }

    #[test]
    fn test_parse_devin_cli_sqlite_returns_empty_for_missing_db() {
        let messages = parse_devin_cli_sqlite(Path::new("/nonexistent/devin/sessions.db"));
        assert!(messages.is_empty());
    }

    fn write_devin_cli_session_dir(
        base: &Path,
        dir_name: &str,
        meta_json: Option<&str>,
        log_lines: &[&str],
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let session_dir = base.join("sessions").join(dir_name);
        std::fs::create_dir_all(&session_dir).unwrap();
        let meta_path = session_dir.join("meta.json");
        if let Some(meta) = meta_json {
            std::fs::write(&meta_path, meta).unwrap();
        }
        let log_path = session_dir.join("log.jsonl");
        let mut content = log_lines.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        std::fs::write(&log_path, content).unwrap();
        (log_path, meta_path)
    }

    fn create_devin_desktop_db(
        dir: &Path,
        filename: &str,
        info_json: Option<&str>,
        rows: &[(i64, Option<&str>, Option<&str>)],
    ) -> std::path::PathBuf {
        let db_path = dir.join(filename);
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE meta (
                key TEXT PRIMARY KEY,
                value TEXT
            );
            CREATE TABLE messages (
                position INTEGER PRIMARY KEY,
                kind TEXT,
                payload TEXT
            );
            "#,
        )
        .unwrap();
        if let Some(info) = info_json {
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('info', ?1)",
                rusqlite::params![info],
            )
            .unwrap();
        }
        for (pos, kind, payload) in rows {
            conn.execute(
                "INSERT INTO messages (position, kind, payload) VALUES (?1, ?2, ?3)",
                rusqlite::params![pos, kind, payload],
            )
            .unwrap();
        }
        db_path
    }

    #[test]
    fn test_parse_devin_cli_session_log_reads_metrics_meta_and_start_anchors_timestamp() {
        let dir = TempDir::new().unwrap();
        let (log_path, _) = write_devin_cli_session_dir(
            dir.path(),
            "session-dir-fallback",
            Some(
                r#"{"id":"sess-jsonl-1","title":"Refactor parser","model":"claude-sonnet-4","workspace_dirs":["/Users/alice/devin-repo"]}"#,
            ),
            &[
                r#"{"kind":"tool_call_state","at":1775645520.0,"tool_call_id":"tc-1","status":"running"}"#,
                r#"{"kind":"live_row","at":1775645521.0,"text":"working"}"#,
                r#"{"kind":"node","at":1775645522.0,"node":{"id":1,"parent_id":null,"message":{"role":"user","content":"hello"}}}"#,
                r#"{"kind":"node","at":1775645530.5,"node":{"id":2,"parent_id":1,"message":{"role":"assistant","metadata":{"request_id":"req-1","generation_model":"claude-opus-4-6","created_at":"2026-04-08T10:52:06.259Z","metrics":{"input_tokens":1200,"output_tokens":340,"cache_read_tokens":500,"cache_creation_tokens":80,"total_time_ms":2259}}}}}"#,
                r#"{"kind":"node","at":1775645540.25,"node":{"id":3,"parent_id":2,"message":{"role":"assistant","metadata":{"num_tokens":420}}}}"#,
            ],
        );

        assert!(devin_cli_source_is_readable(&log_path));
        let messages = parse_devin_cli_session_log(&log_path);
        assert_eq!(messages.len(), 2);

        let first = &messages[0];
        assert_eq!(first.client, "devin-cli");
        assert_eq!(first.session_id, "sess-jsonl-1");
        assert_eq!(first.model_id, "claude-opus-4-6");
        assert_eq!(first.provider_id, "anthropic");
        assert_eq!(first.tokens.input, 1200);
        assert_eq!(first.tokens.output, 340);
        assert_eq!(first.tokens.cache_read, 500);
        assert_eq!(first.tokens.cache_write, 80);
        // 2026-04-08T10:52:06.259Z == 1775645526259 ms; back-anchored by 2259 ms.
        assert_eq!(first.timestamp, 1_775_645_524_000);
        assert_eq!(first.duration_ms, Some(2259));
        assert_eq!(
            first.dedup_key.as_deref(),
            Some("devin-cli:sess-jsonl-1:request:req-1")
        );
        assert_eq!(
            first.workspace_key.as_deref(),
            Some("/Users/alice/devin-repo")
        );
        assert_eq!(first.workspace_label.as_deref(), Some("devin-repo"));

        let second = &messages[1];
        assert_eq!(second.model_id, "claude-sonnet-4");
        assert_eq!(second.tokens.input, 0);
        assert_eq!(second.tokens.output, 420);
        // Falls back to outer float `at` (seconds -> ms).
        assert_eq!(second.timestamp, 1_775_645_540_250);
        assert_eq!(
            second.dedup_key.as_deref(),
            Some("devin-cli:sess-jsonl-1:node:3")
        );
    }

    #[test]
    fn test_parse_devin_cli_session_log_collapses_append_only_node_overwrites() {
        let dir = TempDir::new().unwrap();
        let (log_path, _) = write_devin_cli_session_dir(
            dir.path(),
            "sess-overwrite",
            Some(r#"{"id":"sess-overwrite","working_directory":"/tmp/repo","model":"gpt-5"}"#),
            &[
                r#"{"kind":"node","at":1775645526.0,"node":{"id":5,"message":{"role":"assistant","metadata":{"request_id":"req-5","generation_model":"gpt-5","created_at":"2026-04-08T10:52:06.000Z","metrics":{"input_tokens":50,"output_tokens":10,"total_time_ms":1000}}}}}"#,
                r#"{"kind":"node","at":1775731926.0,"node":{"id":5,"message":{"role":"assistant","metadata":{"request_id":"req-5","generation_model":"gpt-5","created_at":"2026-04-08T10:52:06.000Z","metrics":{"input_tokens":150,"output_tokens":45,"total_time_ms":2000}}}}}"#,
            ],
        );

        let messages = parse_devin_cli_session_log(&log_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 150);
        assert_eq!(messages[0].tokens.output, 45);
        // Uses metadata.created_at (2026-04-08T10:52:06Z) - 2000ms, NOT the later overwrite `at`.
        assert_eq!(messages[0].timestamp, 1_775_645_524_000);
        assert_eq!(messages[0].duration_ms, Some(2000));
    }

    #[test]
    fn test_parse_devin_cli_session_log_dedups_request_id_and_falls_back_to_node_key() {
        let dir = TempDir::new().unwrap();
        let (log_path, _) = write_devin_cli_session_dir(
            dir.path(),
            "sess-dedup",
            Some(r#"{"id":"sess-dedup","model":"claude-opus-4-6"}"#),
            &[
                r#"{"kind":"node","at":1775645526.0,"node":{"id":10,"message":{"role":"assistant","metadata":{"request_id":"req-shared","metrics":{"input_tokens":100,"output_tokens":20}}}}}"#,
                r#"{"kind":"node","at":1775645527.0,"node":{"id":11,"message":{"role":"assistant","metadata":{"request_id":"req-shared","metrics":{"input_tokens":100,"output_tokens":20}}}}}"#,
                r#"{"kind":"node","at":1775645528.0,"node":{"id":12,"message":{"role":"assistant","metadata":{"metrics":{"input_tokens":30,"output_tokens":15}}}}}"#,
            ],
        );

        let messages = parse_devin_cli_session_log(&log_path);
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("devin-cli:sess-dedup:request:req-shared")
        );
        assert_eq!(
            messages[1].dedup_key.as_deref(),
            Some("devin-cli:sess-dedup:node:12")
        );
    }

    #[test]
    fn test_devin_cli_rejects_fusion_routing_profile_but_honors_generation_model() {
        let dir = TempDir::new().unwrap();

        // 1. New JSONL session log with Fusion composite model in meta.json.
        let (log_path, _) = write_devin_cli_session_dir(
            dir.path(),
            "sess-fusion-log",
            Some(
                r#"{"id":"sess-fusion-log","model":"fusion-claude-opus-5-5-medium-sidekick-swe-2-medium"}"#,
            ),
            &[
                // No generation_model -> must be skipped instead of attributing to Fusion profile.
                r#"{"kind":"node","at":1775645526.0,"node":{"id":1,"message":{"role":"assistant","metadata":{"metrics":{"input_tokens":200,"output_tokens":40}}}}}"#,
                // Explicit generation_model -> kept with the real underlying model.
                r#"{"kind":"node","at":1775645527.0,"node":{"id":2,"message":{"role":"assistant","metadata":{"generation_model":"claude-opus-4-6","metrics":{"input_tokens":300,"output_tokens":60}}}}}"#,
            ],
        );
        let log_messages = parse_devin_cli_session_log(&log_path);
        assert_eq!(log_messages.len(), 1);
        assert_eq!(log_messages[0].model_id, "claude-opus-4-6");
        assert_eq!(log_messages[0].provider_id, "anthropic");

        // 2. Legacy SQLite sessions.db with Fusion composite model in sessions.model.
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        insert_session(
            &conn,
            "sess-fusion-db",
            "/tmp/project",
            "fusion-claude-opus-5-5-medium-sidekick-swe-2-medium",
        );
        insert_message(
            &conn,
            "sess-fusion-db",
            r#"{"role":"assistant","metadata":{"metrics":{"input_tokens":100,"output_tokens":20}}}"#,
            1_700_000_000,
        );
        insert_message(
            &conn,
            "sess-fusion-db",
            r#"{"role":"assistant","metadata":{"generation_model":"gpt-5","metrics":{"input_tokens":150,"output_tokens":30}}}"#,
            1_700_000_010,
        );
        drop(conn);

        let db_messages = parse_devin_cli_sqlite(&db_path);
        assert_eq!(db_messages.len(), 1);
        assert_eq!(db_messages[0].model_id, "gpt-5");
        assert_eq!(db_messages[0].provider_id, "openai");
    }

    #[test]
    fn test_parse_devin_cli_session_log_completeness_on_missing_or_corrupt_files() {
        let dir = TempDir::new().unwrap();

        // Missing meta.json is allowed (complete = true) and falls back to dir name.
        let (no_meta_log, _) = write_devin_cli_session_dir(
            dir.path(),
            "dir-session-id",
            None,
            &[
                r#"{"kind":"node","at":1775645526.0,"node":{"id":1,"message":{"role":"assistant","metadata":{"generation_model":"gpt-5","metrics":{"input_tokens":10,"output_tokens":5}}}}}"#,
            ],
        );
        let (msgs, complete) = scan_devin_cli_session_log(&no_meta_log);
        assert!(complete);
        assert!(devin_cli_source_is_readable(&no_meta_log));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].session_id, "dir-session-id");

        // Corrupt meta.json revokes completeness while keeping readable log lines.
        let (bad_meta_log, _) = write_devin_cli_session_dir(
            dir.path(),
            "bad-meta-session",
            Some("{not json"),
            &[
                r#"{"kind":"node","at":1775645526.0,"node":{"id":1,"message":{"role":"assistant","metadata":{"generation_model":"gpt-5","metrics":{"input_tokens":12,"output_tokens":4}}}}}"#,
            ],
        );
        let (msgs, complete) = scan_devin_cli_session_log(&bad_meta_log);
        assert!(!complete);
        assert!(!devin_cli_source_is_readable(&bad_meta_log));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].session_id, "bad-meta-session");

        // Corrupt JSON line in log.jsonl revokes completeness while keeping valid lines.
        let (bad_line_log, _) = write_devin_cli_session_dir(
            dir.path(),
            "bad-line-session",
            Some(r#"{"id":"bad-line-session"}"#),
            &[
                "{broken json",
                r#"{"kind":"node","at":1775645526.0,"node":{"id":1,"message":{"role":"assistant","metadata":{"generation_model":"gpt-5","metrics":{"input_tokens":20,"output_tokens":8}}}}}"#,
            ],
        );
        let (msgs, complete) = scan_devin_cli_session_log(&bad_line_log);
        assert!(!complete);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].tokens.input, 20);

        // Invalid UTF-8 line in log.jsonl revokes completeness while keeping valid lines.
        let utf8_dir = dir.path().join("sessions/bad-utf8");
        std::fs::create_dir_all(&utf8_dir).unwrap();
        let utf8_log = utf8_dir.join("log.jsonl");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\xff\xfe\n");
        bytes.extend_from_slice(
            br#"{"kind":"node","at":1775645526.0,"node":{"id":1,"message":{"role":"assistant","metadata":{"generation_model":"gpt-5","metrics":{"input_tokens":9,"output_tokens":3}}}}}
"#,
        );
        std::fs::write(&utf8_log, bytes).unwrap();
        let (msgs, complete) = scan_devin_cli_session_log(&utf8_log);
        assert!(!complete);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].tokens.input, 9);

        // Missing log file is incomplete.
        assert!(!devin_cli_source_is_readable(
            &dir.path().join("sessions/missing/log.jsonl")
        ));
    }

    #[test]
    fn test_load_devin_desktop_session_lookup_combines_sqlite_and_meta_json() {
        let dir = TempDir::new().unwrap();
        let db_path = create_devin_cli_db(&dir);
        let conn = Connection::open(&db_path).unwrap();
        insert_session(&conn, "shared-sess", "/tmp/from-db", "gpt-5");
        set_session_title(&conn, "shared-sess", "Shared task");
        insert_session(&conn, "untitled-db", "/tmp/untitled", "claude-sonnet-4");
        insert_session(&conn, "dup-a", "/tmp/a", "gpt-5");
        set_session_title(&conn, "dup-a", "Duplicate title");
        drop(conn);

        // Same session id mirrored in meta.json with Fusion model must NOT mark
        // the title ambiguous and must preserve the non-routing model from SQLite.
        let (_, meta_shared) = write_devin_cli_session_dir(
            dir.path(),
            "shared-sess",
            Some(
                r#"{"id":"shared-sess","title":"Shared task","model":"fusion-claude-opus-5-5-medium","working_directory":"/tmp/from-meta"}"#,
            ),
            &[],
        );
        let (_, meta_dup_b) = write_devin_cli_session_dir(
            dir.path(),
            "dup-b",
            Some(r#"{"id":"dup-b","title":"Duplicate title","model":"claude-opus-4-6"}"#),
            &[],
        );
        let (_, meta_fusion_only) = write_devin_cli_session_dir(
            dir.path(),
            "fusion-only",
            Some(
                r#"{"id":"fusion-only","title":"Fusion only task","model":"fusion-swe-2-medium","working_directory":"/tmp/fusion"}"#,
            ),
            &[],
        );

        let lookup = load_devin_desktop_session_lookup(&[
            db_path,
            meta_shared,
            meta_dup_b,
            meta_fusion_only,
        ]);

        let shared = lookup
            .resolve("Shared task")
            .expect("same session across db and meta.json is not ambiguous");
        assert_eq!(shared.session_id, "shared-sess");
        assert_eq!(shared.model_id.as_deref(), Some("gpt-5"));

        // Untitled DB session is still resolvable by id.
        let untitled = lookup
            .resolve_by_id("untitled-db")
            .expect("untitled session resolves by id");
        assert_eq!(untitled.model_id.as_deref(), Some("claude-sonnet-4"));

        // Distinct sessions sharing a title are ambiguous by title, unambiguous by id.
        assert!(lookup.resolve("Duplicate title").is_none());
        assert_eq!(
            lookup.resolve_by_id("dup-b").unwrap().model_id.as_deref(),
            Some("claude-opus-4-6")
        );

        // Fusion routing profile is stripped from lookup model_id.
        let fusion = lookup.resolve("Fusion only task").unwrap();
        assert_eq!(fusion.session_id, "fusion-only");
        assert_eq!(fusion.model_id, None);
        assert_eq!(fusion.workspace.as_deref(), Some("/tmp/fusion"));
    }

    #[test]
    fn test_parse_devin_desktop_sqlite_direct_acp_wrapped_and_acu_only() {
        let dir = TempDir::new().unwrap();
        let (_, meta_path) = write_devin_cli_session_dir(
            dir.path(),
            "cli-from-meta",
            Some(
                r#"{"id":"cli-from-meta","title":"Desktop SQLite task","model":"claude-opus-4-6","working_directory":"/Users/alice/app"}"#,
            ),
            &[],
        );
        let lookup = load_devin_desktop_session_lookup(&[meta_path]);

        // Case A: Direct ACP payloads (no outer `notification` wrapper) + SQL kind
        // for `session_info_update` + cumulative input/cache & additive output.
        let direct_db = create_devin_desktop_db(
            dir.path(),
            "0123456789abcdef0123456789abcdef01234567.db",
            Some(r#"{"createdAt":1775645526.259}"#),
            &[
                (
                    1,
                    Some("session_info_update"),
                    Some(r#"{"title":"Desktop SQLite task"}"#),
                ),
                (
                    2,
                    Some("usage_update"),
                    Some(
                        r#"{"_meta":{"cognition.ai/inputTokens":200,"cognition.ai/cachedReadTokens":50,"cognition.ai/cachedWriteTokens":10,"cognition.ai/outputTokens":25}}"#,
                    ),
                ),
                (
                    3,
                    Some("usage_update"),
                    Some(
                        r#"{"cognition.ai/inputTokens":300,"cognition.ai/cachedReadTokens":80,"cognition.ai/outputTokens":15}"#,
                    ),
                ),
                (4, None, None),
            ],
        );
        let direct_msgs = parse_devin_desktop_source_with_lookup(&direct_db, &lookup);
        assert_eq!(direct_msgs.len(), 1);
        assert_eq!(direct_msgs[0].client, "devin-desktop");
        assert_eq!(direct_msgs[0].session_id, "cli-from-meta");
        assert_eq!(direct_msgs[0].model_id, "claude-opus-4-6");
        assert_eq!(direct_msgs[0].provider_id, "anthropic");
        // 300 total prompt - 80 cachedRead = 220 uncached input; output = 25 + 15 = 40.
        assert_eq!(direct_msgs[0].tokens.input, 220);
        assert_eq!(direct_msgs[0].tokens.output, 40);
        assert_eq!(direct_msgs[0].tokens.cache_read, 80);
        assert_eq!(direct_msgs[0].tokens.cache_write, 10);
        assert_eq!(direct_msgs[0].timestamp, 1_775_645_526_259);
        assert_eq!(
            direct_msgs[0].workspace_key.as_deref(),
            Some("/Users/alice/app")
        );

        // Case B: Wrapped notification with embedded metrics + meta.info sessionId/workspace
        // and Fusion model in meta.info ignored in favor of per-event generation_model.
        let wrapped_db = create_devin_desktop_db(
            dir.path(),
            "wrapped.db",
            Some(
                r#"{"sessionId":"explicit-sess-2","model":"fusion-swe-2-medium","cwd":"/Users/alice/wrapped"}"#,
            ),
            &[(
                1,
                Some("event"),
                Some(
                    r#"{"notification":{"metadata":{"generation_model":"gpt-5","created_at":"2026-04-08T12:00:00Z","metrics":{"input_tokens":90,"output_tokens":30}}}}"#,
                ),
            )],
        );
        let wrapped_msgs = parse_devin_desktop_sqlite(&wrapped_db);
        assert_eq!(wrapped_msgs.len(), 1);
        assert_eq!(wrapped_msgs[0].session_id, "explicit-sess-2");
        assert_eq!(wrapped_msgs[0].model_id, "gpt-5");
        assert_eq!(wrapped_msgs[0].provider_id, "openai");
        assert_eq!(wrapped_msgs[0].tokens.input, 90);
        assert_eq!(wrapped_msgs[0].tokens.output, 30);
        assert_eq!(
            wrapped_msgs[0].workspace_key.as_deref(),
            Some("/Users/alice/wrapped")
        );

        // Case C: ACU-only rows (cloud Devin sessions) must yield 0 messages.
        let acu_db = create_devin_desktop_db(
            dir.path(),
            "acu-only.db",
            Some(r#"{"title":"Cloud task"}"#),
            &[(
                1,
                Some("usage_update"),
                Some(
                    r#"{"_meta":{"cognition.ai/eventType":"acu_consumption_at_last_user_interaction","cognition.ai/acuAmount":0.42}}"#,
                ),
            )],
        );
        assert!(parse_devin_desktop_sqlite(&acu_db).is_empty());

        // Case D: Unrelated SQLite DB without `messages` table or corrupt bytes returns empty.
        let non_acp_db = dir.path().join("unrelated.db");
        let conn = Connection::open(&non_acp_db).unwrap();
        conn.execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);")
            .unwrap();
        drop(conn);
        assert!(parse_devin_desktop_sqlite(&non_acp_db).is_empty());

        let corrupt_db = dir.path().join("corrupt.db");
        std::fs::write(&corrupt_db, b"not a sqlite file").unwrap();
        assert!(parse_devin_desktop_sqlite(&corrupt_db).is_empty());

        // Case E: An earlier routing-mode model hint must not block a later real model.
        let routing_then_real_db = create_devin_desktop_db(
            dir.path(),
            "routing-then-real.db",
            Some(r#"{"sessionId":"sess-routing-then-real","model":"fusion-pro"}"#),
            &[
                (
                    1,
                    Some("usage_update"),
                    Some(
                        r#"{"_meta":{"cognition.ai/model":"fusion-pro","cognition.ai/inputTokens":100,"cognition.ai/outputTokens":10}}"#,
                    ),
                ),
                (
                    2,
                    Some("usage_update"),
                    Some(
                        r#"{"_meta":{"cognition.ai/model":"claude-sonnet-4-6","cognition.ai/inputTokens":150,"cognition.ai/outputTokens":20}}"#,
                    ),
                ),
            ],
        );
        let routing_msgs = parse_devin_desktop_sqlite(&routing_then_real_db);
        assert_eq!(routing_msgs.len(), 1);
        assert_eq!(routing_msgs[0].model_id, "claude-sonnet-4-6");
        assert_eq!(routing_msgs[0].provider_id, "anthropic");
        assert_eq!(routing_msgs[0].tokens.input, 150);
        assert_eq!(routing_msgs[0].tokens.output, 30);
    }
}
