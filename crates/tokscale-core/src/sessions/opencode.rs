//! OpenCode session parser
//!
//! Parses messages from:
//! - SQLite database (OpenCode 1.2+): ~/.local/share/opencode/opencode.db
//! - Legacy JSON files: ~/.local/share/opencode/storage/message/

use super::utils::{open_readonly_sqlite, read_file_or_none};
use super::{
    normalize_opencode_agent_name, normalize_workspace_key, workspace_label_from_key,
    UnifiedMessage,
};
use crate::{provider_identity, TokenBreakdown};
#[cfg(test)]
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// OpenCode message structure (from JSON files and SQLite data column).
///
/// Handles two on-disk shapes:
/// - **v1** (`opencode.db` `message` table, legacy JSON files): a `role`
///   field, and top-level `modelID` / `providerID` strings.
/// - **v2** (`opencode-next.db` `session_message` table): no `role` field
///   (the row's `type` column carries it), and the model identifiers nested
///   under a `model` object (`model.id` / `model.providerID`).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OpenCodeMessage {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "sessionID", default)]
    pub session_id: Option<String>,
    /// Absent in v2 `session_message` rows (the `type` column is the role
    /// there and the SQL query already filters to `assistant`).
    #[serde(default)]
    pub role: Option<String>,
    #[serde(rename = "modelID", default)]
    pub model_id: Option<String>,
    #[serde(rename = "providerID", default)]
    pub provider_id: Option<String>,
    /// v2 nests model + provider under a `model` object.
    #[serde(default)]
    pub model: Option<OpenCodeModel>,
    pub cost: Option<f64>,
    pub tokens: Option<OpenCodeTokens>,
    pub time: OpenCodeTime,
    pub agent: Option<String>,
    pub mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opencode_path")]
    pub path: Option<OpenCodePath>,
}

impl OpenCodeMessage {
    /// Resolve the model id from the top-level v1 field or the nested v2
    /// `model.id`, preferring the explicit top-level value when both exist.
    fn resolve_model_id(&self) -> Option<String> {
        self.model_id
            .clone()
            .or_else(|| self.model.as_ref().and_then(|m| m.id.clone()))
    }

    /// Resolve the provider id from the top-level v1 field or the nested v2
    /// `model.providerID`, preferring the explicit top-level value.
    fn resolve_provider_id(&self) -> Option<String> {
        self.provider_id
            .clone()
            .or_else(|| self.model.as_ref().and_then(|m| m.provider_id.clone()))
    }

    /// True when this row is an assistant turn. v1 rows carry an explicit
    /// `role`; v2 rows omit it and are pre-filtered by the SQL `type` column,
    /// so a missing role is treated as assistant.
    fn is_assistant(&self) -> bool {
        self.role.as_deref().is_none_or(|role| role == "assistant")
    }
}

/// v2 nested model descriptor: `{"id": "...", "providerID": "...", ...}`.
#[derive(Debug, Deserialize)]
pub struct OpenCodeModel {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "providerID", default)]
    pub provider_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenCodePath {
    pub root: Option<String>,
}

fn deserialize_opencode_path<'de, D>(deserializer: D) -> Result<Option<OpenCodePath>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let root = value
        .get("root")
        .and_then(|root| root.as_str())
        .map(str::to_string);

    Ok(Some(OpenCodePath { root }))
}

#[derive(Debug, Deserialize)]
pub struct OpenCodeTokens {
    pub input: i64,
    pub output: i64,
    pub reasoning: Option<i64>,
    pub cache: OpenCodeCache,
}

#[derive(Debug, Deserialize)]
pub struct OpenCodeCache {
    pub read: i64,
    pub write: i64,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OpenCodeTime {
    pub created: f64, // Unix timestamp in milliseconds (as float)
    pub completed: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OpenCodeSqliteFingerprint {
    created_bits: u64,
    completed_bits: Option<u64>,
    model_id: String,
    provider_id: String,
    input: i64,
    output: i64,
    reasoning: i64,
    cache_read: i64,
    cache_write: i64,
    cost_bits: u64,
    agent: Option<String>,
}

#[derive(Debug, Clone)]
struct OpenCodeSqliteDedupState {
    /// The entry's embedded (`$.id`) message id, if any. Two rows that share
    /// every fingerprint field but carry *different* embedded ids are distinct
    /// messages, not fork copies, and must not be merged. A fork copies the id,
    /// so equal ids (or an id absent on either side) still merge.
    message_id: Option<String>,
    has_workspace_conflict: bool,
}

fn workspace_from_root(root: Option<&str>) -> (Option<String>, Option<String>) {
    let workspace_key = root.and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
    (workspace_key, workspace_label)
}

fn set_workspace_from_root(message: &mut UnifiedMessage, root: Option<&str>) {
    let (workspace_key, workspace_label) = workspace_from_root(root);
    message.set_workspace(workspace_key, workspace_label);
}

fn merge_duplicate_workspace(
    message: &mut UnifiedMessage,
    state: &mut OpenCodeSqliteDedupState,
    root: Option<&str>,
) {
    if state.has_workspace_conflict {
        return;
    }

    let (candidate_key, candidate_label) = workspace_from_root(root);
    match (message.workspace_key.as_deref(), candidate_key) {
        (None, Some(key)) => message.set_workspace(Some(key), candidate_label),
        (Some(existing), Some(candidate)) if existing != candidate => {
            state.has_workspace_conflict = true;
            message.set_workspace(None, None);
        }
        _ => {}
    }
}

fn opencode_duration_ms(time: &OpenCodeTime) -> Option<i64> {
    let duration = time.completed? - time.created;
    if duration.is_finite() && duration > 0.0 {
        Some(duration as i64)
    } else {
        None
    }
}

fn embedded_cost(cost: Option<f64>) -> f64 {
    match cost {
        Some(cost) if cost.is_finite() && cost >= 0.0 => cost,
        _ => 0.0,
    }
}

pub fn parse_opencode_file(path: &Path) -> Option<UnifiedMessage> {
    let data = read_file_or_none(path)?;
    let mut bytes = data;

    let msg: OpenCodeMessage = simd_json::from_slice(&mut bytes).ok()?;

    // OpenCode JSON files (v1) always carry an explicit role, so require it to
    // be "assistant" here. Missing-role acceptance (is_assistant) is reserved
    // for the v2 `session_message` SQLite path, whose SQL already filters
    // `type = 'assistant'`; applying it to files would count a role-less or
    // malformed file as assistant usage (previously it was skipped when the
    // required `role` field failed to deserialize).
    if msg.role.as_deref() != Some("assistant") {
        return None;
    }

    let workspace_root = msg
        .path
        .as_ref()
        .and_then(|path| path.root.as_deref())
        .map(str::to_string);
    // Resolve model + provider before moving any fields out of `msg`, since
    // both borrow the whole struct to fall back onto the nested `model` object.
    let model_id = msg.resolve_model_id()?;
    let provider_id = msg
        .resolve_provider_id()
        .unwrap_or_else(|| "unknown".to_string());
    let provider_id = provider_identity::canonical_provider(&provider_id).unwrap_or(provider_id);

    let tokens = msg.tokens?;
    let agent_or_mode = msg.mode.or(msg.agent);
    let agent = agent_or_mode.map(|a| normalize_opencode_agent_name(&a));

    let session_id = msg.session_id.unwrap_or_else(|| "unknown".to_string());

    // Use message ID from JSON or derive from filename for deduplication
    let dedup_key = msg.id.or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    });
    let cost = embedded_cost(msg.cost);

    let mut unified = UnifiedMessage::new_with_agent(
        "opencode",
        model_id,
        provider_id,
        session_id,
        msg.time.created as i64,
        TokenBreakdown {
            input: tokens.input.max(0),
            output: tokens.output.max(0),
            cache_read: tokens.cache.read.max(0),
            cache_write: tokens.cache.write.max(0),
            reasoning: tokens.reasoning.unwrap_or(0).max(0),
        },
        cost,
        agent,
    );
    unified.duration_ms = opencode_duration_ms(&msg.time);
    unified.dedup_key = dedup_key;
    set_workspace_from_root(&mut unified, workspace_root.as_deref());
    mark_opencode_cost_source(&mut unified);
    Some(unified)
}

/// OpenCode computes per-message cost at request time from its own pricing
/// data (models.dev), so a positive `cost` is authoritative and must survive
/// tokscale's LiteLLM repricing pass. A zero cost usually means OpenCode
/// itself had no pricing for the model — leave it `Unknown` so
/// `apply_pricing_if_available` can still estimate.
fn mark_opencode_cost_source(unified: &mut UnifiedMessage) {
    if unified.cost > 0.0 {
        unified.mark_provider_reported_cost();
    }
}

/// Column layout shared by every OpenCode SQLite query variant:
/// `(row_id, session_id, data_json, workspace_root, session_title)`.
type OpenCodeSqliteRow = (String, String, String, Option<String>, Option<String>);

/// Accumulates parsed assistant messages across OpenCode's v1 (`message`) and
/// v2 (`session_message`) tables, applying fingerprint-based deduplication so
/// forked-history copies — and any overlap between the two tables — collapse
/// into a single entry. A fingerprint maps to a *list* of entries, one per
/// distinct embedded message id, so two genuinely different messages that
/// happen to collide on every fingerprint field are kept apart.
#[derive(Default)]
struct OpenCodeSqliteAccumulator {
    messages: Vec<UnifiedMessage>,
    fingerprint_indices: HashMap<OpenCodeSqliteFingerprint, Vec<usize>>,
    dedup_states: Vec<OpenCodeSqliteDedupState>,
}

impl OpenCodeSqliteAccumulator {
    /// Parse one SQLite row's JSON payload and merge it into the accumulator,
    /// deduplicating against previously ingested rows.
    fn ingest_row(&mut self, row: OpenCodeSqliteRow) {
        let (row_id, session_id, data_json, row_workspace_root, row_session_title) = row;

        let mut bytes = data_json.into_bytes();
        let msg: OpenCodeMessage = match simd_json::from_slice(&mut bytes) {
            Ok(m) => m,
            Err(_) => return,
        };

        if !msg.is_assistant() {
            return;
        }

        let message_id = msg.id.clone();
        let embedded_workspace_root = msg
            .path
            .as_ref()
            .and_then(|path| path.root.as_deref())
            .map(str::to_string);

        let tokens = match msg.tokens {
            Some(ref t) => t,
            None => return,
        };

        let model_id = match msg.resolve_model_id() {
            Some(m) => m,
            None => return,
        };

        let provider_id = msg
            .resolve_provider_id()
            .unwrap_or_else(|| "unknown".to_string());
        let provider_id =
            provider_identity::canonical_provider(&provider_id).unwrap_or(provider_id);
        let agent_or_mode = msg.mode.clone().or_else(|| msg.agent.clone());
        let agent = agent_or_mode.map(|a| normalize_opencode_agent_name(&a));
        let input = tokens.input.max(0);
        let output = tokens.output.max(0);
        let reasoning = tokens.reasoning.unwrap_or(0).max(0);
        let cache_read = tokens.cache.read.max(0);
        let cache_write = tokens.cache.write.max(0);
        let cost = embedded_cost(msg.cost);
        let dedup_key = message_id.clone().unwrap_or(row_id);
        let fingerprint = OpenCodeSqliteFingerprint {
            created_bits: msg.time.created.to_bits(),
            completed_bits: msg.time.completed.map(f64::to_bits),
            model_id: model_id.clone(),
            provider_id: provider_id.clone(),
            input,
            output,
            reasoning,
            cache_read,
            cache_write,
            cost_bits: cost.to_bits(),
            agent: agent.clone(),
        };

        let mut unified = UnifiedMessage::new_with_agent(
            "opencode",
            model_id,
            provider_id,
            session_id,
            msg.time.created as i64,
            TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                reasoning,
            },
            cost,
            agent,
        );
        unified.duration_ms = opencode_duration_ms(&msg.time);
        unified.dedup_key = Some(dedup_key);
        let workspace_root = row_workspace_root
            .as_deref()
            .or(embedded_workspace_root.as_deref());
        set_workspace_from_root(&mut unified, workspace_root);
        mark_opencode_cost_source(&mut unified);
        if let Some(ref title) = row_session_title {
            let trimmed = title.trim();
            if !trimmed.is_empty() {
                unified.session_title = Some(trimmed.to_string());
            }
        }

        // Among entries sharing this fingerprint, merge into the first one that
        // is NOT a definitively-different message -- i.e. skip any whose stored
        // embedded id conflicts with this row's. (Cloning the small index list
        // avoids holding a borrow of `fingerprint_indices` while we read
        // `dedup_states`.)
        let candidate = {
            let slots = self
                .fingerprint_indices
                .get(&fingerprint)
                .cloned()
                .unwrap_or_default();
            slots.into_iter().find(|&index| {
                !matches!(
                    (&self.dedup_states[index].message_id, &message_id),
                    (Some(existing), Some(incoming)) if existing != incoming
                )
            })
        };

        if let Some(index) = candidate {
            let dedup_state = &mut self.dedup_states[index];
            // First copy carrying an embedded id promotes the entry's stable
            // dedup key (and records the id so later rows can be told apart).
            if message_id.is_some() && dedup_state.message_id.is_none() {
                dedup_state.message_id = message_id.clone();
                self.messages[index].dedup_key = unified.dedup_key.clone();
            }
            merge_duplicate_workspace(&mut self.messages[index], dedup_state, workspace_root);
            return;
        }

        let new_index = self.messages.len();
        self.dedup_states.push(OpenCodeSqliteDedupState {
            message_id: message_id.clone(),
            has_workspace_conflict: false,
        });
        self.fingerprint_indices
            .entry(fingerprint)
            .or_default()
            .push(new_index);
        self.messages.push(unified);
    }
}

/// Run one query (whose columns are `id, session_id, data, workspace_root,
/// session_title`) against `conn` and feed every row into `acc`. A prepare/query
/// failure — for example a table that does not exist in this schema variant —
/// is treated as "no rows", so callers can attempt several schema variants
/// against the same database without an error aborting the scan.
fn collect_opencode_rows(
    conn: &rusqlite::Connection,
    query: &str,
    acc: &mut OpenCodeSqliteAccumulator,
) {
    let mut stmt = match conn.prepare(query) {
        Ok(s) => s,
        Err(_) => return,
    };

    let rows = match stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let session_id: String = row.get(1)?;
        let data_json: String = row.get(2)?;
        let workspace_root: Option<String> = row.get(3)?;
        let session_title: Option<String> = row.get(4)?;
        Ok((id, session_id, data_json, workspace_root, session_title))
    }) {
        Ok(r) => r,
        Err(_) => return,
    };

    for row_result in rows.flatten() {
        acc.ingest_row(row_result);
    }
}

pub fn parse_opencode_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite(db_path) else {
        return Vec::new();
    };

    let mut acc = OpenCodeSqliteAccumulator::default();

    // OpenCode v2 (`opencode-next.db`): per-message rows live in
    // `session_message`, keyed by a `type` column, with model + provider nested
    // under `$.model`. Absent in v1 databases, where the prepare fails and this
    // is a no-op.
    //
    // Try the title-bearing query first; older v2 databases whose `session`
    // table predates the `title` column fall back to a title-less variant so
    // they still produce rows (the title is optional, not a gating column).
    let v2_query = r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, s.title AS session_title
        FROM session_message sm
        LEFT JOIN session s ON s.id = sm.session_id
        WHERE sm.type = 'assistant'
          AND json_extract(sm.data, '$.tokens') IS NOT NULL
        ORDER BY sm.id, sm.session_id
    "#;
    let v2_query_no_title = r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title
        FROM session_message sm
        LEFT JOIN session s ON s.id = sm.session_id
        WHERE sm.type = 'assistant'
          AND json_extract(sm.data, '$.tokens') IS NOT NULL
        ORDER BY sm.id, sm.session_id
    "#;
    if conn.prepare(v2_query).is_ok() {
        collect_opencode_rows(&conn, v2_query, &mut acc);
    } else {
        collect_opencode_rows(&conn, v2_query_no_title, &mut acc);
    }

    // OpenCode v1 (`opencode.db`, 1.2+): per-message rows in `message`, role in
    // the JSON `$.role`. The `session` join supplies the workspace directory
    // and title. Three fallback tiers:
    //   1. modern: session table has both `directory` and `title`
    //   2. directory-only: session table has `directory` but not `title`
    //   3. legacy: no `session` table at all (drops workspace + title)
    let v1_modern_query = r#"
        SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root, s.title AS session_title
        FROM message m
        LEFT JOIN session s ON s.id = m.session_id
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#;
    let v1_directory_query = r#"
        SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title
        FROM message m
        LEFT JOIN session s ON s.id = m.session_id
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#;
    let v1_legacy_query = r#"
        SELECT m.id, m.session_id, m.data, NULL AS workspace_root, NULL AS session_title
        FROM message m
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#;
    if conn.prepare(v1_modern_query).is_ok() {
        collect_opencode_rows(&conn, v1_modern_query, &mut acc);
    } else if conn.prepare(v1_directory_query).is_ok() {
        collect_opencode_rows(&conn, v1_directory_query, &mut acc);
    } else {
        collect_opencode_rows(&conn, v1_legacy_query, &mut acc);
    }

    acc.messages
}

// =============================================================================
// Migration cache: skip redundant legacy JSON scanning after full migration
// =============================================================================

const MIGRATION_CACHE_FILENAME: &str = "opencode-migration.json";

/// Persisted migration status for OpenCode JSON → SQLite migration.
/// Stored at <config_dir>/cache/opencode-migration.json.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenCodeMigrationCache {
    /// True when every legacy JSON message was already present in SQLite.
    pub migration_complete: bool,
    /// Number of JSON files in the message directory at detection time.
    pub json_file_count: u64,
    /// Modification time of the JSON directory (Unix seconds) at detection time.
    pub json_dir_mtime_secs: u64,
    /// When this entry was written (Unix seconds).
    pub checked_at_secs: u64,
}

fn migration_cache_dir() -> std::path::PathBuf {
    crate::paths::get_cache_dir()
}

fn migration_cache_path() -> std::path::PathBuf {
    migration_cache_dir().join(MIGRATION_CACHE_FILENAME)
}

fn legacy_migration_cache_paths() -> Vec<std::path::PathBuf> {
    if crate::paths::is_config_dir_overridden() {
        return Vec::new();
    }

    [
        crate::paths::legacy_dirs_cache_dir().map(|d| d.join(MIGRATION_CACHE_FILENAME)),
        crate::paths::legacy_dot_cache_tokscale_dir().map(|d| d.join(MIGRATION_CACHE_FILENAME)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Load the migration cache from disk. Returns `None` if the file is missing or
/// unparseable.
pub fn load_opencode_migration_cache() -> Option<OpenCodeMigrationCache> {
    let canonical = migration_cache_path();
    match std::fs::read_to_string(&canonical) {
        Ok(content) => serde_json::from_str(&content).ok(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            legacy_migration_cache_paths().into_iter().find_map(|path| {
                let content = std::fs::read_to_string(path).ok()?;
                serde_json::from_str(&content).ok()
            })
        }
        Err(_) => None,
    }
}

/// Persist the migration cache atomically (write to temp file, then rename).
pub fn save_opencode_migration_cache(cache: &OpenCodeMigrationCache) {
    use std::io::Write as _;

    let dir = migration_cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let content = match serde_json::to_string(cache) {
        Ok(c) => c,
        Err(_) => return,
    };

    let final_path = migration_cache_path();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let tmp_name = format!(".opencode-migration.{}.{:x}.tmp", std::process::id(), nanos);
    let tmp_path = dir.join(tmp_name);

    // INVARIANT: All cache writes use atomic temp-file rename. NEVER delete
    // the canonical cache file before writing — a partial save or process
    // crash between delete and rename would lose the cache. The temp-file
    // pattern makes corruption-on-crash impossible.
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        crate::fs_atomic::replace_file(&tmp_path, &final_path)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
}

/// Return the modification time of `json_dir` as Unix seconds, or `None` on
/// error (directory absent, permissions, etc.).
pub fn get_json_dir_mtime(json_dir: &Path) -> Option<u64> {
    std::fs::metadata(json_dir)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Current Unix timestamp in seconds.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, previous) in self.0.drain(..) {
                unsafe {
                    match previous {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    fn create_opencode_sqlite_db(db_path: &Path) -> Connection {
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                data TEXT NOT NULL
            );",
        )
        .unwrap();
        conn
    }

    /// Build a database shaped like OpenCode v2 (`opencode-next.db`): an empty
    /// `message` table plus the `session_message` + `session` tables that hold
    /// the real per-message data. Mirrors the columns tokscale actually reads.
    fn create_opencode_v2_sqlite_db(db_path: &Path) -> Connection {
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                data TEXT NOT NULL
            );
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                directory TEXT NOT NULL,
                title TEXT
            );
            CREATE TABLE session_message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                type TEXT NOT NULL,
                data TEXT NOT NULL
            );",
        )
        .unwrap();
        conn
    }

    /// A representative v2 assistant payload: no `role` field, model + provider
    /// nested under `$.model`, integer timestamps.
    const V2_ASSISTANT_DATA: &str = r#"{
        "time": { "created": 1783882279705, "completed": 1783882279943 },
        "agent": "build",
        "model": { "id": "claude-sonnet-4", "providerID": "anthropic", "variant": "default" },
        "content": [],
        "finish": "stop",
        "cost": 0.0123,
        "tokens": {
            "input": 5519,
            "output": 20,
            "reasoning": 23,
            "cache": { "read": 100, "write": 50 }
        }
    }"#;

    #[test]
    fn test_deserialize_v2_message_resolves_nested_model() {
        let mut bytes = V2_ASSISTANT_DATA.as_bytes().to_vec();
        let msg: OpenCodeMessage = simd_json::from_slice(&mut bytes).unwrap();

        assert_eq!(msg.role, None, "v2 payloads carry no role field");
        assert!(msg.is_assistant(), "missing role defaults to assistant");
        assert_eq!(msg.resolve_model_id().as_deref(), Some("claude-sonnet-4"));
        assert_eq!(msg.resolve_provider_id().as_deref(), Some("anthropic"));
        assert_eq!(msg.agent.as_deref(), Some("build"));
    }

    #[test]
    fn test_top_level_model_id_takes_precedence_over_nested() {
        let json = r#"{
            "role": "assistant",
            "modelID": "top-level-model",
            "providerID": "top-level-provider",
            "model": { "id": "nested-model", "providerID": "nested-provider" },
            "tokens": { "input": 1, "output": 1, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;
        let mut bytes = json.as_bytes().to_vec();
        let msg: OpenCodeMessage = simd_json::from_slice(&mut bytes).unwrap();

        assert_eq!(msg.resolve_model_id().as_deref(), Some("top-level-model"));
        assert_eq!(
            msg.resolve_provider_id().as_deref(),
            Some("top-level-provider")
        );
    }

    #[test]
    fn test_parse_v2_session_message_reads_tokens_and_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode-next.db");

        let conn = create_opencode_v2_sqlite_db(&db_path);
        conn.execute(
            "INSERT INTO session (id, directory) VALUES (?1, ?2)",
            rusqlite::params!["ses_v2", "/Users/alice/opencode-v2-repo"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["msg_v2_001", "ses_v2", "assistant", V2_ASSISTANT_DATA],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(messages.len(), 1, "v2 assistant row should be parsed");
        let msg = &messages[0];
        assert_eq!(msg.model_id, "claude-sonnet-4");
        assert_eq!(msg.provider_id, "anthropic");
        assert_eq!(msg.tokens.input, 5519);
        assert_eq!(msg.tokens.output, 20);
        assert_eq!(msg.tokens.reasoning, 23);
        assert_eq!(msg.tokens.cache_read, 100);
        assert_eq!(msg.tokens.cache_write, 50);
        assert_eq!(msg.duration_ms, Some(238));
        assert_eq!(
            msg.workspace_key.as_deref(),
            Some("/Users/alice/opencode-v2-repo"),
            "workspace should come from session.directory"
        );
        assert_eq!(msg.workspace_label.as_deref(), Some("opencode-v2-repo"));
        assert_eq!(
            msg.dedup_key.as_deref(),
            Some("msg_v2_001"),
            "v2 dedup_key falls back to the session_message row id"
        );
        assert_eq!(
            msg.cost_source,
            crate::sessions::CostSource::ProviderReported
        );
    }

    #[test]
    fn test_parse_v2_skips_non_assistant_and_tokenless_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode-next.db");

        let conn = create_opencode_v2_sqlite_db(&db_path);
        let user_data = r#"{ "time": { "created": 1783882279705 }, "content": [] }"#;
        let tokenless = r#"{ "time": { "created": 1783882279705 }, "model": { "id": "m", "providerID": "p" } }"#;
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["msg_ok", "ses_v2", "assistant", V2_ASSISTANT_DATA],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["msg_user", "ses_v2", "user", user_data],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["msg_synthetic", "ses_v2", "synthetic", V2_ASSISTANT_DATA],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["msg_no_tokens", "ses_v2", "assistant", tokenless],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            1,
            "only the assistant row with tokens should parse"
        );
        assert_eq!(messages[0].dedup_key.as_deref(), Some("msg_ok"));
    }

    #[test]
    fn test_parse_v2_negative_tokens_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode-next.db");

        let conn = create_opencode_v2_sqlite_db(&db_path);
        let negative = r#"{
            "time": { "created": 1783882279705 },
            "model": { "id": "claude-sonnet-4", "providerID": "anthropic" },
            "cost": -1.0,
            "tokens": { "input": -100, "output": -50, "reasoning": -25, "cache": { "read": -200, "write": -10 } }
        }"#;
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["msg_neg", "ses_v2", "assistant", negative],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.tokens.input, 0);
        assert_eq!(msg.tokens.output, 0);
        assert_eq!(msg.tokens.reasoning, 0);
        assert_eq!(msg.tokens.cache_read, 0);
        assert_eq!(msg.tokens.cache_write, 0);
        assert!(msg.cost >= 0.0);
    }

    #[test]
    fn test_parse_v2_deduplicates_forked_session_message_history() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode-next.db");

        let conn = create_opencode_v2_sqlite_db(&db_path);
        // Same payload copied into a forked session must collapse to one entry.
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["root_row", "root_session", "assistant", V2_ASSISTANT_DATA],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["fork_row", "fork_session", "assistant", V2_ASSISTANT_DATA],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            1,
            "forked copies of the same assistant turn collapse inside v2 parsing"
        );
    }

    #[test]
    fn test_distinct_embedded_ids_are_not_merged_despite_fingerprint_collision() {
        // Two genuinely different assistant messages can share every fingerprint
        // field (timestamp, model, tokens, cost, agent). When both carry an
        // embedded `$.id` and the ids DIFFER, they are distinct messages -- not
        // fork copies -- and must be kept separate rather than collapsed.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode-next.db");
        let conn = create_opencode_v2_sqlite_db(&db_path);

        let payload = |id: &str| {
            format!(
                r#"{{
                    "id": "{id}",
                    "time": {{ "created": 1783882279705, "completed": 1783882279943 }},
                    "agent": "build",
                    "model": {{ "id": "claude-sonnet-4", "providerID": "anthropic" }},
                    "cost": 0.0123,
                    "tokens": {{ "input": 10, "output": 5, "reasoning": 0, "cache": {{ "read": 0, "write": 0 }} }}
                }}"#
            )
        };

        conn.execute(
            "INSERT INTO session_message (id, session_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["row_a", "ses_v2", "assistant", payload("msg_a")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["row_b", "ses_v2", "assistant", payload("msg_b")],
        )
        .unwrap();
        // A true fork of msg_a (same embedded id, different session/row) must
        // still collapse into msg_a rather than becoming a third entry.
        conn.execute(
            "INSERT INTO session_message (id, session_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["row_a_fork", "fork_session", "assistant", payload("msg_a")],
        )
        .unwrap();
        drop(conn);

        let mut dedup_keys: Vec<String> = parse_opencode_sqlite(&db_path)
            .into_iter()
            .filter_map(|m| m.dedup_key)
            .collect();
        dedup_keys.sort();
        assert_eq!(
            dedup_keys,
            vec!["msg_a".to_string(), "msg_b".to_string()],
            "distinct embedded ids stay separate; a same-id fork collapses"
        );
    }

    #[test]
    fn test_parse_opencode_structure() {
        let json = r#"{
            "id": "msg_123",
            "sessionID": "ses_456",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 100,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let mut bytes = json.as_bytes().to_vec();
        let msg: OpenCodeMessage = simd_json::from_slice(&mut bytes).unwrap();

        assert_eq!(msg.model_id, Some("claude-sonnet-4".to_string()));
        assert_eq!(msg.tokens.unwrap().input, 1000);
        assert_eq!(msg.agent, None);
    }

    #[test]
    fn test_parse_opencode_with_agent() {
        let json = r#"{
            "id": "msg_123",
            "sessionID": "ses_456",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "agent": "OmO",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 100,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let mut bytes = json.as_bytes().to_vec();
        let msg: OpenCodeMessage = simd_json::from_slice(&mut bytes).unwrap();

        assert_eq!(msg.agent, Some("OmO".to_string()));
    }

    /// Verify negative token values are clamped to 0 (defense-in-depth for PR #147)
    #[test]
    fn test_negative_values_clamped_to_zero() {
        use std::io::Write;

        let json = r#"{
            "id": "msg_negative",
            "sessionID": "ses_negative",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": -0.05,
            "tokens": {
                "input": -100,
                "output": -50,
                "reasoning": -25,
                "cache": { "read": -200, "write": -10 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let mut temp_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        temp_file.write_all(json.as_bytes()).unwrap();

        let result = parse_opencode_file(temp_file.path());
        assert!(result.is_some(), "Should parse file with negative values");

        let msg = result.unwrap();
        assert_eq!(msg.tokens.input, 0, "Negative input should be clamped to 0");
        assert_eq!(
            msg.tokens.output, 0,
            "Negative output should be clamped to 0"
        );
        assert_eq!(
            msg.tokens.cache_read, 0,
            "Negative cache_read should be clamped to 0"
        );
        assert_eq!(
            msg.tokens.cache_write, 0,
            "Negative cache_write should be clamped to 0"
        );
        assert_eq!(
            msg.tokens.reasoning, 0,
            "Negative reasoning should be clamped to 0"
        );
        assert!(
            msg.cost >= 0.0,
            "Negative cost should be clamped to 0.0, got {}",
            msg.cost
        );
    }

    #[test]
    fn test_parse_opencode_file_requires_explicit_assistant_role() {
        use std::io::Write;
        // Regression: making `role` optional for the v2 SQLite path must NOT
        // loosen file parsing. A file without a `role` (or a non-assistant one)
        // is not assistant usage and must be skipped -- the missing-role =>
        // assistant shortcut applies only to the type-filtered session_message
        // SQLite query, never to JSON files.
        let role_less = r#"{
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 10, "output": 5, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;
        let mut f1 = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        f1.write_all(role_less.as_bytes()).unwrap();
        assert!(
            parse_opencode_file(f1.path()).is_none(),
            "a role-less OpenCode JSON file must not be counted as assistant usage"
        );

        let user_role = r#"{
            "role": "user",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 10, "output": 5, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;
        let mut f2 = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        f2.write_all(user_role.as_bytes()).unwrap();
        assert!(
            parse_opencode_file(f2.path()).is_none(),
            "a non-assistant OpenCode JSON file must be skipped"
        );
    }

    /// JSON dedup_key uses msg.id when present
    #[test]
    fn test_dedup_key_from_json_message_id() {
        use std::io::Write;

        let json = r#"{
            "id": "msg_dedup_001",
            "sessionID": "ses_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.01,
            "tokens": {
                "input": 100,
                "output": 50,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let mut temp_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        temp_file.write_all(json.as_bytes()).unwrap();

        let msg = parse_opencode_file(temp_file.path()).expect("Should parse");
        assert_eq!(
            msg.dedup_key,
            Some("msg_dedup_001".to_string()),
            "dedup_key should use msg.id from JSON"
        );
    }

    #[test]
    fn test_parse_opencode_file_sets_duration_from_completed_time() {
        use std::io::Write;

        let json = r#"{
            "id": "msg_timed",
            "sessionID": "ses_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.01,
            "tokens": {
                "input": 100,
                "output": 50,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000001234.0 }
        }"#;

        let mut temp_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        temp_file.write_all(json.as_bytes()).unwrap();

        let msg = parse_opencode_file(temp_file.path()).expect("Should parse");
        assert_eq!(msg.duration_ms, Some(1234));
    }

    /// JSON dedup_key falls back to file stem when msg.id is absent
    #[test]
    fn test_dedup_key_falls_back_to_file_stem() {
        let json = r#"{
            "sessionID": "ses_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.01,
            "tokens": {
                "input": 100,
                "output": 50,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("msg_fallback_999.json");
        std::fs::write(&file_path, json).unwrap();

        let msg = parse_opencode_file(&file_path).expect("Should parse");
        assert_eq!(
            msg.dedup_key,
            Some("msg_fallback_999".to_string()),
            "dedup_key should fall back to file stem when id is missing"
        );
    }

    /// Non-assistant messages are skipped (no dedup_key produced)
    #[test]
    fn test_dedup_key_skips_non_assistant() {
        let json = r#"{
            "id": "msg_user_001",
            "sessionID": "ses_001",
            "role": "user",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": {
                "input": 100,
                "output": 50,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("msg_user_001.json");
        std::fs::write(&file_path, json).unwrap();

        let result = parse_opencode_file(&file_path);
        assert!(result.is_none(), "User messages should be skipped");
    }

    /// SQLite dedup_key falls back to the database row id when the message has no embedded id.
    #[test]
    fn test_sqlite_dedup_key_falls_back_to_row_id() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");

        let conn = create_opencode_sqlite_db(&db_path);

        let data_json = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 0,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_sqlite_001", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key,
            Some("msg_sqlite_001".to_string()),
            "SQLite dedup_key should fall back to the row id when no embedded id exists"
        );
        assert_eq!(messages[0].model_id, "claude-sonnet-4");
        assert_eq!(messages[0].tokens.input, 1000);
    }

    #[test]
    fn test_parse_opencode_file_marks_positive_cost_as_provider_reported() {
        use std::io::Write;
        let json = r#"{
            "id": "msg_cost_001",
            "sessionID": "ses_cost",
            "role": "assistant",
            "modelID": "z-ai/glm-4.6",
            "providerID": "openrouter",
            "cost": 0.0025158,
            "tokens": {
                "input": 2675,
                "output": 28,
                "reasoning": 1,
                "cache": { "read": 7700, "write": 0 }
            },
            "time": { "created": 1765915142201.0 }
        }"#;

        let mut temp_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        temp_file.write_all(json.as_bytes()).unwrap();

        let msg = parse_opencode_file(temp_file.path()).expect("Should parse");
        assert_eq!(
            msg.cost_source,
            crate::sessions::CostSource::ProviderReported,
            "positive embedded cost must survive the LiteLLM repricing pass"
        );
        assert!((msg.cost - 0.0025158).abs() < 1e-12);
    }

    #[test]
    fn test_parse_opencode_file_keeps_zero_cost_unknown_for_estimation() {
        use std::io::Write;
        let json = r#"{
            "id": "msg_cost_002",
            "sessionID": "ses_cost",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.0,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        let mut temp_file = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        temp_file.write_all(json.as_bytes()).unwrap();

        let msg = parse_opencode_file(temp_file.path()).expect("Should parse");
        assert_eq!(
            msg.cost_source,
            crate::sessions::CostSource::Unknown,
            "zero cost means OpenCode had no pricing — leave repricing enabled"
        );
    }

    #[test]
    fn test_parse_opencode_sqlite_marks_positive_cost_as_provider_reported() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_opencode_cost.db");
        let conn = create_opencode_sqlite_db(&db_path);

        let costed = r#"{
            "role": "assistant",
            "modelID": "z-ai/glm-4.6",
            "providerID": "openrouter",
            "cost": 0.0025158,
            "tokens": {
                "input": 2675,
                "output": 28,
                "reasoning": 1,
                "cache": { "read": 7700, "write": 0 }
            },
            "time": { "created": 1765915142201.0 }
        }"#;
        let free = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.0,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_costed", "ses_cost", costed],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_free", "ses_cost", free],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(messages.len(), 2);

        let costed_msg = messages
            .iter()
            .find(|m| m.dedup_key.as_deref() == Some("msg_costed"))
            .unwrap();
        assert_eq!(
            costed_msg.cost_source,
            crate::sessions::CostSource::ProviderReported
        );

        let free_msg = messages
            .iter()
            .find(|m| m.dedup_key.as_deref() == Some("msg_free"))
            .unwrap();
        assert_eq!(free_msg.cost_source, crate::sessions::CostSource::Unknown);
    }

    #[test]
    fn test_parse_opencode_file_uses_explicit_path_root_as_workspace() {
        let json = r#"{
            "id": "msg_workspace_001",
            "sessionID": "ses_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.01,
            "tokens": {
                "input": 100,
                "output": 50,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0 },
            "path": { "root": "/Users/alice/opencode-json-repo" }
        }"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("msg_workspace_001.json");
        std::fs::write(&file_path, json).unwrap();

        let msg = parse_opencode_file(&file_path).expect("Should parse");
        assert_eq!(
            msg.workspace_key.as_deref(),
            Some("/Users/alice/opencode-json-repo")
        );
        assert_eq!(msg.workspace_label.as_deref(), Some("opencode-json-repo"));
    }

    #[test]
    fn test_parse_opencode_file_ignores_non_object_path_without_rejecting_message() {
        let json = r#"{
            "id": "msg_path_string_001",
            "sessionID": "ses_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.01,
            "tokens": {
                "input": 100,
                "output": 50,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0 },
            "path": "/Users/alice/not-object"
        }"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("msg_path_string_001.json");
        std::fs::write(&file_path, json).unwrap();

        let msg = parse_opencode_file(&file_path).expect("Should parse");
        assert_eq!(msg.workspace_key, None);
        assert_eq!(msg.workspace_label, None);
    }

    #[test]
    fn test_parse_opencode_sqlite_uses_session_directory_as_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");

        let conn = create_opencode_sqlite_db(&db_path);
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                directory TEXT NOT NULL,
                title TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory) VALUES (?1, ?2)",
            rusqlite::params!["ses_001", "/Users/alice/opencode-sqlite-repo"],
        )
        .unwrap();

        let data_json = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 0,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_sqlite_workspace", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/alice/opencode-sqlite-repo")
        );
        assert_eq!(
            messages[0].workspace_label.as_deref(),
            Some("opencode-sqlite-repo")
        );
    }

    #[test]
    fn test_parse_opencode_sqlite_legacy_fallback_uses_path_root_when_session_table_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");

        let conn = create_opencode_sqlite_db(&db_path);

        let data_json = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 0,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0 },
            "path": { "root": "/Users/alice/legacy-fallback-repo" }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_sqlite_legacy_workspace", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/alice/legacy-fallback-repo")
        );
        assert_eq!(
            messages[0].workspace_label.as_deref(),
            Some("legacy-fallback-repo")
        );
        assert_eq!(messages[0].tokens.input, 1000);
    }

    #[test]
    fn test_parse_opencode_sqlite_duplicate_workspace_conflict_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");

        let conn = create_opencode_sqlite_db(&db_path);
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                directory TEXT NOT NULL,
                title TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory) VALUES (?1, ?2)",
            rusqlite::params!["ses_root", "/Users/alice/root-workspace"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory) VALUES (?1, ?2)",
            rusqlite::params!["ses_fork", "/Users/alice/fork-workspace"],
        )
        .unwrap();

        let data_json = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 0,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000000500.0 },
            "mode": "build"
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["z_root_copy", "ses_root", data_json],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["a_fork_copy", "ses_fork", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key, None);
        assert_eq!(messages[0].workspace_label, None);
        assert_eq!(messages[0].tokens.input, 1000);
    }

    /// SQLite prefers the embedded message id when present so JSON/SQLite overlap keeps deduplicating.
    #[test]
    fn test_sqlite_dedup_key_prefers_embedded_message_id() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");

        let conn = create_opencode_sqlite_db(&db_path);

        let valid = r#"{
            "id": "embedded_msg_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 100, "output": 50, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["row_msg_001", "ses_001", valid],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key,
            Some("embedded_msg_001".to_string()),
            "SQLite dedup_key should prefer the embedded message id for cross-source overlap"
        );
    }

    /// SQLite skips rows without tokens or with non-assistant role
    #[test]
    fn test_sqlite_skips_invalid_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");

        let conn = create_opencode_sqlite_db(&db_path);

        let valid = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 100, "output": 50, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;

        let user_msg = r#"{
            "role": "user",
            "modelID": "claude-sonnet-4",
            "time": { "created": 1700000000000.0 }
        }"#;

        let no_tokens = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_valid", "ses_001", valid],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_user", "ses_001", user_msg],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_no_tokens", "ses_001", no_tokens],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            1,
            "Should only parse valid assistant message"
        );
        assert_eq!(messages[0].dedup_key, Some("msg_valid".to_string()));
    }

    /// Forked SQLite sessions should not count copied history more than once.
    #[test]
    fn test_sqlite_deduplicates_forked_history_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");
        let conn = create_opencode_sqlite_db(&db_path);

        let root_msg = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 25,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000000500.0 },
            "mode": "build"
        }"#;

        let new_msg = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.08,
            "tokens": {
                "input": 1300,
                "output": 650,
                "reasoning": 40,
                "cache": { "read": 100, "write": 0 }
            },
            "time": { "created": 1700000001000.0, "completed": 1700000001500.0 },
            "mode": "build"
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["root_row", "root_session", root_msg],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["fork_copy_row", "fork_session", root_msg],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["fork_new_row", "fork_session", new_msg],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            2,
            "Forked copies of the same assistant history should collapse inside SQLite parsing"
        );
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[1].tokens.input, 1300);
    }

    /// Same-timestamp messages with different payloads should remain distinct.
    #[test]
    fn test_sqlite_same_timestamp_distinct_payloads_survive() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_opencode.db");
        let conn = create_opencode_sqlite_db(&db_path);

        let first = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000000100.0 },
            "mode": "build"
        }"#;

        let second = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "cost": 0.05,
            "tokens": {
                "input": 1500,
                "output": 750,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000000100.0 },
            "mode": "build"
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["row_one", "session_one", first],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["row_two", "session_two", second],
        )
        .unwrap();
        drop(conn);

        let messages = parse_opencode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            2,
            "Distinct assistant calls should survive even when they share the same creation timestamp"
        );
    }

    /// Cross-source dedup: matching IDs between SQLite and JSON should deduplicate
    #[test]
    fn test_cross_source_dedup_by_message_id() {
        use std::collections::HashSet;

        let dir = tempfile::tempdir().unwrap();

        // --- SQLite source ---
        let db_path = dir.path().join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                data TEXT NOT NULL
            );",
        )
        .unwrap();

        let shared_data_json = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 500, "output": 200, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;
        let sqlite_only_data_json = r#"{
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 700, "output": 250, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000001000.0 }
        }"#;

        // Insert two messages into SQLite
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_shared_001", "ses_001", shared_data_json],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_sqlite_only", "ses_001", sqlite_only_data_json],
        )
        .unwrap();
        drop(conn);

        // --- JSON source ---
        let json_dir = dir.path().join("json");
        std::fs::create_dir_all(&json_dir).unwrap();

        // Duplicate of SQLite msg_shared_001
        let json_shared = r#"{
            "id": "msg_shared_001",
            "sessionID": "ses_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 500, "output": 200, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;
        std::fs::write(json_dir.join("msg_shared_001.json"), json_shared).unwrap();

        // JSON-only message (not in SQLite)
        let json_only = r#"{
            "id": "msg_json_only",
            "sessionID": "ses_001",
            "role": "assistant",
            "modelID": "claude-sonnet-4",
            "providerID": "anthropic",
            "tokens": { "input": 100, "output": 50, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000000000.0 }
        }"#;
        std::fs::write(json_dir.join("msg_json_only.json"), json_only).unwrap();

        // --- Simulate the dedup logic from lib.rs ---
        let sqlite_messages = parse_opencode_sqlite(&db_path);
        assert_eq!(sqlite_messages.len(), 2);

        // Build seen set from SQLite (same as lib.rs)
        let mut seen: HashSet<String> = HashSet::new();
        for msg in &sqlite_messages {
            if let Some(ref key) = msg.dedup_key {
                seen.insert(key.clone());
            }
        }
        assert_eq!(seen.len(), 2);

        // Parse JSON files
        let json_msg_shared = parse_opencode_file(&json_dir.join("msg_shared_001.json")).unwrap();
        let json_msg_only = parse_opencode_file(&json_dir.join("msg_json_only.json")).unwrap();

        // Filter JSON through seen set (same logic as lib.rs)
        let json_messages = vec![json_msg_shared, json_msg_only];
        let deduped: Vec<UnifiedMessage> = json_messages
            .into_iter()
            .filter(|msg| {
                msg.dedup_key
                    .as_ref()
                    .is_none_or(|key| seen.insert(key.clone()))
            })
            .collect();

        // msg_shared_001 should be filtered (duplicate), msg_json_only should survive
        assert_eq!(
            deduped.len(),
            1,
            "Only the JSON-only message should survive dedup"
        );
        assert_eq!(
            deduped[0].dedup_key,
            Some("msg_json_only".to_string()),
            "Surviving message should be the JSON-only one"
        );

        // Total unique messages = 2 from SQLite + 1 from JSON
        let total = sqlite_messages.len() + deduped.len();
        assert_eq!(total, 3, "Should have 3 unique messages total");
    }

    // -------------------------------------------------------------------------
    // Migration cache tests
    // -------------------------------------------------------------------------

    /// Round-trip: save then load returns identical data.
    #[test]
    fn test_migration_cache_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        // Point the cache at a temp dir by overriding via a temporary env var is
        // impractical here; instead we test the structs and serde directly.
        let cache = OpenCodeMigrationCache {
            migration_complete: true,
            json_file_count: 42,
            json_dir_mtime_secs: 1_700_000_000,
            checked_at_secs: 1_700_100_000,
        };

        let json = serde_json::to_string(&cache).unwrap();
        let loaded: OpenCodeMigrationCache = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded, cache);

        // Ensure the JSON contains all expected keys
        assert!(json.contains("migration_complete"));
        assert!(json.contains("json_file_count"));
        assert!(json.contains("json_dir_mtime_secs"));
        assert!(json.contains("checked_at_secs"));

        drop(dir);
    }

    /// Cache is valid when file count and mtime are unchanged.
    #[test]
    fn test_migration_cache_valid_when_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let json_dir = dir.path().join("message");
        std::fs::create_dir_all(&json_dir).unwrap();

        // Write a dummy file so the directory exists and has a stable mtime
        std::fs::write(json_dir.join("msg.json"), b"{}").unwrap();

        let current_mtime = get_json_dir_mtime(&json_dir).expect("should stat dir");
        let current_file_count = 1u64;

        let cache = OpenCodeMigrationCache {
            migration_complete: true,
            json_file_count: current_file_count,
            json_dir_mtime_secs: current_mtime, // same mtime
            checked_at_secs: now_secs(),
        };

        // Simulate the validity check from lib.rs
        let is_valid = cache.migration_complete
            && current_file_count == cache.json_file_count
            && get_json_dir_mtime(&json_dir).is_some_and(|m| m <= cache.json_dir_mtime_secs);

        assert!(is_valid, "Cache should be valid when count and mtime match");
    }

    /// Cache is invalid when file count has changed.
    #[test]
    fn test_migration_cache_invalid_when_file_count_changes() {
        let dir = tempfile::tempdir().unwrap();
        let json_dir = dir.path().join("message");
        std::fs::create_dir_all(&json_dir).unwrap();
        std::fs::write(json_dir.join("msg1.json"), b"{}").unwrap();

        let current_mtime = get_json_dir_mtime(&json_dir).unwrap();

        let cache = OpenCodeMigrationCache {
            migration_complete: true,
            json_file_count: 1,
            json_dir_mtime_secs: current_mtime,
            checked_at_secs: now_secs(),
        };

        // Simulate: a new file was added → current_file_count = 2
        let current_file_count = 2u64; // changed
        let is_valid = cache.migration_complete
            && current_file_count == cache.json_file_count
            && get_json_dir_mtime(&json_dir).is_some_and(|m| m <= cache.json_dir_mtime_secs);

        assert!(!is_valid, "Cache should be invalid when file count changes");
    }

    /// Cache is invalid when directory mtime is newer than cached value.
    #[test]
    fn test_migration_cache_invalid_when_mtime_newer() {
        let dir = tempfile::tempdir().unwrap();
        let json_dir = dir.path().join("message");
        std::fs::create_dir_all(&json_dir).unwrap();
        std::fs::write(json_dir.join("msg.json"), b"{}").unwrap();

        let current_mtime = get_json_dir_mtime(&json_dir).unwrap();

        // Simulate: cache recorded an older mtime → directory is now newer
        let stale_mtime = current_mtime.saturating_sub(1);
        let cache = OpenCodeMigrationCache {
            migration_complete: true,
            json_file_count: 1,
            json_dir_mtime_secs: stale_mtime, // older than current
            checked_at_secs: now_secs(),
        };

        let is_valid = cache.migration_complete
            && 1u64 == cache.json_file_count
            && get_json_dir_mtime(&json_dir).is_some_and(|m| m <= cache.json_dir_mtime_secs);

        assert!(
            !is_valid,
            "Cache should be invalid when directory mtime is newer than cached value"
        );
    }

    /// Cache is not loaded when the file is missing (load returns None).
    #[test]
    fn test_migration_cache_missing_returns_none() {
        // load_opencode_migration_cache reads from ~/.cache/tokscale/opencode-migration.json
        // We can't easily override the path in a unit test, but we can verify that
        // serde_json::from_str returns None for invalid input (simulating missing file).
        let result: Option<OpenCodeMigrationCache> = serde_json::from_str("").ok();
        assert!(
            result.is_none(),
            "Empty/missing content should produce None"
        );
    }

    /// migration_complete=false disables the cache even if count/mtime match.
    #[test]
    fn test_migration_cache_not_skipped_when_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let json_dir = dir.path().join("message");
        std::fs::create_dir_all(&json_dir).unwrap();
        std::fs::write(json_dir.join("msg.json"), b"{}").unwrap();

        let current_mtime = get_json_dir_mtime(&json_dir).unwrap();

        let cache = OpenCodeMigrationCache {
            migration_complete: false, // migration not complete
            json_file_count: 1,
            json_dir_mtime_secs: current_mtime,
            checked_at_secs: now_secs(),
        };

        let is_valid = cache.migration_complete
            && 1u64 == cache.json_file_count
            && get_json_dir_mtime(&json_dir).is_some_and(|m| m <= cache.json_dir_mtime_secs);

        assert!(
            !is_valid,
            "Cache should not allow skipping when migration_complete=false"
        );
    }

    #[test]
    #[serial_test::serial]
    fn migration_record_falls_back_to_legacy_path() {
        use std::env;

        let temp_home = tempfile::tempdir().unwrap();
        let temp_xdg_cache = tempfile::tempdir().unwrap();
        let prev_home = env::var_os("HOME");
        let prev_xdg_cache = env::var_os("XDG_CACHE_HOME");
        let prev_override = env::var_os("TOKSCALE_CONFIG_DIR");
        let _guard = EnvGuard(vec![
            ("TOKSCALE_CONFIG_DIR", prev_override),
            ("XDG_CACHE_HOME", prev_xdg_cache),
            ("HOME", prev_home),
        ]);
        unsafe {
            env::set_var("HOME", temp_home.path());
            env::set_var("XDG_CACHE_HOME", temp_xdg_cache.path());
            env::remove_var("TOKSCALE_CONFIG_DIR");
        }

        let legacy_path = crate::paths::legacy_dirs_cache_dir()
            .unwrap()
            .join(MIGRATION_CACHE_FILENAME);
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        std::fs::write(
            &legacy_path,
            r#"{"migration_complete":true,"json_file_count":2,"json_dir_mtime_secs":3,"checked_at_secs":4}"#,
        )
        .unwrap();

        let loaded = load_opencode_migration_cache().unwrap();
        assert!(loaded.migration_complete);
        assert_eq!(loaded.json_file_count, 2);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[ignore] // Run manually with: cargo test integration -- --ignored
    fn test_parse_real_sqlite_db() {
        let home = std::env::var("HOME").unwrap();
        let db_path = PathBuf::from(format!("{}/.local/share/opencode/opencode.db", home));

        if !db_path.exists() {
            println!("Skipping: OpenCode database not found at {:?}", db_path);
            return;
        }

        let messages = parse_opencode_sqlite(&db_path);
        println!("Parsed {} messages from SQLite", messages.len());

        if !messages.is_empty() {
            let first = &messages[0];
            println!(
                "First message: model={}, provider={}, tokens={:?}",
                first.model_id, first.provider_id, first.tokens
            );
        }

        assert!(
            !messages.is_empty(),
            "Expected to parse some messages from SQLite"
        );
    }
}
