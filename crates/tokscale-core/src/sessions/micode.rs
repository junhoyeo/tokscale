//! MiMo Code session parser
//!
//! Parses messages from:
//! - SQLite database: ~/.local/share/mimocode/mimocode.db
//!
//! MiMo Code stores assistant turns in OpenCode's message schema, so the parse
//! itself lives in [`super::opencode_schema`]; only the places where MiMo's
//! behaviour departs from OpenCode's are declared here, as
//! `OpenCodeSchemaConfig::micode`.
//!
//! Xiaomi MiMo AI (desktop) and the mimo code CLI share this same store. The
//! desktop engine stamps `session.version` with its InstallationVersion, which
//! starts with `desktop-` (and also sets `MIMOCODE_CLIENT=desktop`). Messages
//! from those sessions are re-stamped as [`MICODE_DESKTOP_CLIENT_ID`] so
//! reports can split the two surfaces.

use super::opencode_schema::{
    parse_opencode_schema_rows_on, OpenCodeRowMetadata, OpenCodeSchemaConfig,
};
use super::utils::open_readonly_sqlite_opt;
use super::UnifiedMessage;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Client id for mimo code CLI / headless engine sessions.
pub const MICODE_CLIENT_ID: &str = "micode";
/// Client id for Xiaomi MiMo AI desktop sessions (shared engine, desktop surface).
pub const MICODE_DESKTOP_CLIENT_ID: &str = "micode-desktop";
/// `session.version` prefix written by the Xiaomi MiMo AI desktop install.
pub const MICODE_DESKTOP_VERSION_PREFIX: &str = "desktop-";

pub fn is_desktop_session_version(version: &str) -> bool {
    version
        .trim()
        .to_ascii_lowercase()
        .starts_with(MICODE_DESKTOP_VERSION_PREFIX)
}

#[derive(Clone, Default)]
struct SessionMetadata {
    desktop: bool,
    created_at: Option<f64>,
}

fn normalize_time(value: f64) -> f64 {
    if value > 1e12 {
        value
    } else {
        value * 1000.0
    }
}

fn read_sessions(conn: &rusqlite::Connection) -> (HashMap<String, SessionMetadata>, bool) {
    let mut stmt = match conn.prepare("SELECT id, version, time_created FROM session") {
        Ok(stmt) => stmt,
        Err(_) => {
            // A missing table/optional column is a compatible legacy schema,
            // but an operational SQL failure is not evidence of absence. Only
            // fall back after inspecting the schema successfully.
            let columns = (|| -> rusqlite::Result<HashSet<String>> {
                let mut stmt = conn.prepare("PRAGMA table_info(session)")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
                rows.collect()
            })();
            let Ok(columns) = columns else {
                return (HashMap::new(), false);
            };
            if columns.is_empty() {
                return (HashMap::new(), true);
            }
            if !columns.contains("id") {
                return (HashMap::new(), false);
            }
            let version = if columns.contains("version") {
                "version"
            } else {
                "NULL"
            };
            let created = if columns.contains("time_created") {
                "time_created"
            } else {
                "NULL"
            };
            let query = format!("SELECT id, {version}, {created} FROM session");
            let Ok(stmt) = conn.prepare(&query) else {
                return (HashMap::new(), false);
            };
            stmt
        }
    };
    let Ok(rows) = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let version: Option<String> = row.get(1)?;
        let created: Option<f64> = row.get(2)?;
        Ok((
            id,
            SessionMetadata {
                desktop: version.as_deref().is_some_and(is_desktop_session_version),
                created_at: created
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .map(normalize_time),
            },
        ))
    }) else {
        return (HashMap::new(), false);
    };
    let mut sessions = HashMap::new();
    let mut complete = true;
    for row in rows {
        match row {
            Ok((id, metadata)) => {
                sessions.insert(id, metadata);
            }
            Err(_) => complete = false,
        }
    }
    (sessions, complete)
}

/// Parallel metadata for one source row, serialized only in MiMo's own cache
/// envelope. Capturing the exact bits during the normal JSON decode avoids a
/// second projection walk and never reconstructs precision from milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MiMoRowMetadata {
    pub session_created_bits: Option<u64>,
    pub row: OpenCodeRowMetadata,
}

impl MiMoRowMetadata {
    pub(crate) fn is_valid(&self) -> bool {
        f64::from_bits(self.row.created_bits).is_finite()
            && self
                .row
                .completed_bits
                .is_none_or(|bits| f64::from_bits(bits).is_finite())
            && self.session_created_bits.is_none_or(|bits| {
                let value = f64::from_bits(bits);
                value.is_finite() && value > 0.0
            })
    }
}

#[derive(Default)]
pub(crate) struct MiMoSource {
    pub messages: Vec<UnifiedMessage>,
    pub metadata: Vec<MiMoRowMetadata>,
    pub complete: bool,
}

#[cfg(test)]
#[path = "micode_io_tests.rs"]
pub(crate) mod io_tests;

#[derive(Clone, PartialEq, Eq, Hash)]
struct UsageFingerprint {
    billing_provider: Option<String>,
    created_bits: u64,
    completed_bits: Option<u64>,
    model: String,
    provider: String,
    tokens: [i64; 5],
    cost: u64,
    agent: Option<String>,
}

impl UsageFingerprint {
    fn new(message: &UnifiedMessage, origin: &MessageOrigin) -> Option<Self> {
        let timing = origin.timing?;
        Some(Self {
            created_bits: timing.created_bits,
            completed_bits: timing.completed_bits,
            model: message.model_id.clone(),
            provider: message.provider_id.clone(),
            billing_provider: message.billing_identity.id().map(str::to_string),
            tokens: [
                message.tokens.input,
                message.tokens.output,
                message.tokens.cache_read,
                message.tokens.cache_write,
                message.tokens.reasoning,
            ],
            cost: message.cost.to_bits(),
            agent: message.agent.clone(),
        })
    }
}

#[derive(Clone)]
struct MessageOrigin {
    database: String,
    session_created_at: Option<f64>,
    timing: Option<OpenCodeRowMetadata>,
    has_embedded_id: bool,
    workspace_conflicted: bool,
}

impl MessageOrigin {
    fn owns_turn(&self) -> Option<bool> {
        Some(f64::from_bits(self.timing?.created_bits) >= self.session_created_at?)
    }

    fn preference<'a>(
        &'a self,
        message: &'a UnifiedMessage,
    ) -> (u8, u64, &'a str, &'a str, &'a str) {
        let rank = match self.owns_turn() {
            Some(true) => 0,
            None => 1,
            Some(false) => 2,
        };
        (
            rank,
            self.session_created_at
                .map(f64::to_bits)
                .unwrap_or(u64::MAX),
            &message.session_id,
            &message.client,
            &self.database,
        )
    }
}

/// One accounting identity for the shared store, used by both public parse
/// lanes. A surface is presentation metadata, never a second charge for an
/// embedded message id. Different ids require evidence before cross-surface
/// fingerprint merging: MiMo forks rewrite ids but preserve timestamps, so a
/// historical copy predates its new session while its original does not.
type SourceObservation = (String, String, Option<bool>);
type FingerprintObservations = HashMap<UsageFingerprint, HashSet<SourceObservation>>;

#[derive(Default)]
pub(crate) struct MiMoMessages {
    pending: Vec<(UnifiedMessage, MessageOrigin)>,
    messages: Vec<UnifiedMessage>,
    origins: Vec<MessageOrigin>,
    source_surfaces: Vec<FingerprintObservations>,
    redirects: Vec<usize>,
    keys: HashMap<String, usize>,
    fingerprints: HashMap<UsageFingerprint, Vec<usize>>,
}

impl MiMoMessages {
    pub(crate) fn extend(&mut self, path: &Path, source: MiMoSource) {
        let database = std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned();
        // Callers rebuild unusable cache metadata; never reopen the DB here.
        // A malformed pair cannot silently drop the tail of the message list.
        let valid_metadata = source.metadata.len() == source.messages.len();
        let mut metadata = source.metadata.into_iter();
        for message in source.messages {
            let row = metadata
                .next()
                .filter(|row| valid_metadata && row.is_valid());
            let origin = MessageOrigin {
                database: database.clone(),
                session_created_at: row
                    .as_ref()
                    .and_then(|row| row.session_created_bits)
                    .map(f64::from_bits),
                timing: row.as_ref().map(|row| row.row),
                has_embedded_id: row.as_ref().is_some_and(|row| row.row.has_embedded_id),
                workspace_conflicted: false,
            };
            self.pending.push((message, origin));
        }
    }

    fn canonical_index(&self, mut index: usize) -> usize {
        while self.redirects[index] != index {
            index = self.redirects[index];
        }
        index
    }

    fn insert(&mut self, message: UnifiedMessage, origin: MessageOrigin) {
        let fingerprint = UsageFingerprint::new(&message, &origin);
        let by_key = message
            .dedup_key
            .as_ref()
            .and_then(|key| self.keys.get(key))
            .map(|&index| self.canonical_index(index));
        let by_fingerprint = fingerprint.as_ref().and_then(|fingerprint| {
            self.fingerprints.get(fingerprint).and_then(|indices| {
                indices
                    .iter()
                    .map(|&index| self.canonical_index(index))
                    .find(|&index| {
                        // Every edge must be witnessed by an immutable source
                        // observation, never a canonical row whose cost/origin
                        // came from different databases during reduction.
                        self.source_surfaces[index]
                            .get(fingerprint)
                            .is_some_and(|sources| {
                                sources.iter().any(|(database, client, owns_turn)| {
                                    (database == &origin.database && client == &message.client)
                                        || (client != &message.client
                                            && matches!(
                                                (origin.owns_turn(), *owns_turn),
                                                (Some(true), Some(false))
                                                    | (Some(false), Some(true))
                                            ))
                                })
                            })
                    })
            })
        });
        let candidate = match (by_key, by_fingerprint) {
            (Some(key), Some(fingerprint)) if key != fingerprint => {
                // An id-only copy without chronology can arrive before the
                // row proving that id belongs to another slot's fork history.
                // Join both identities rather than allowing the early key hit
                // to hide later ownership evidence. Redirects preserve aliases.
                let (keep, discard) = if self.origins[key].preference(&self.messages[key])
                    <= self.origins[fingerprint].preference(&self.messages[fingerprint])
                {
                    (key, fingerprint)
                } else {
                    (fingerprint, key)
                };
                let original_workspace = (origin.owns_turn() == Some(false)
                    && self.origins[keep].owns_turn() == Some(true))
                .then(|| {
                    (
                        self.messages[keep].workspace_key.clone(),
                        self.messages[keep].workspace_label.clone(),
                        self.origins[keep].workspace_conflicted,
                    )
                });
                // Joining canonical slots transfers their actual observations
                // below; the merged representative is not another source row.
                self.merge(
                    keep,
                    self.messages[discard].clone(),
                    self.origins[discard].clone(),
                );
                // The incoming row connects the metadata-less alias to proven
                // copied history. Its temporary unknown status is not evidence
                // that the original owner's workspace conflicts.
                if let Some((key, label, conflicted)) = original_workspace {
                    self.messages[keep].set_workspace(key, label);
                    self.origins[keep].workspace_conflicted = conflicted;
                }
                let sources = std::mem::take(&mut self.source_surfaces[discard]);
                for (fingerprint, sources) in sources {
                    self.source_surfaces[keep]
                        .entry(fingerprint)
                        .or_default()
                        .extend(sources);
                }
                self.redirects[discard] = keep;
                Some(keep)
            }
            (key, fingerprint) => key.or(fingerprint),
        };
        if let Some(index) = candidate {
            if let Some(key) = &message.dedup_key {
                self.keys.insert(key.clone(), index);
            }
            self.merge_observation(index, message, origin);
        } else {
            let index = self.messages.len();
            if let Some(key) = &message.dedup_key {
                self.keys.insert(key.clone(), index);
            }
            let mut sources = HashMap::new();
            if let Some(fingerprint) = fingerprint {
                sources.insert(
                    fingerprint.clone(),
                    HashSet::from([(
                        origin.database.clone(),
                        message.client.clone(),
                        origin.owns_turn(),
                    )]),
                );
                self.fingerprints
                    .entry(fingerprint)
                    .or_default()
                    .push(index);
            }
            self.source_surfaces.push(sources);
            self.messages.push(message);
            self.origins.push(origin);
            self.redirects.push(index);
        }
    }

    fn merge_observation(&mut self, index: usize, incoming: UnifiedMessage, origin: MessageOrigin) {
        if let Some(observed) = UsageFingerprint::new(&incoming, &origin) {
            self.source_surfaces[index]
                .entry(observed.clone())
                .or_default()
                .insert((
                    origin.database.clone(),
                    incoming.client.clone(),
                    origin.owns_turn(),
                ));
            let indices = self.fingerprints.entry(observed).or_default();
            if !indices.contains(&index) {
                indices.push(index);
            }
        }
        // Authoritative cost promotion cannot mutate the identity graph: only
        // the raw observation indexed above can establish another match.
        self.merge(index, incoming, origin);
    }

    fn merge(&mut self, index: usize, incoming: UnifiedMessage, incoming_origin: MessageOrigin) {
        let retained = &mut self.messages[index];
        let retained_origin = &mut self.origins[index];
        // into_messages sorted original owners first, so no later copy can
        // change the chosen client/session or replace an original workspace.
        let identified_original = matches!(
            (retained_origin.owns_turn(), incoming_origin.owns_turn()),
            (Some(true), Some(false))
        );
        if !retained.has_authoritative_cost() && incoming.has_authoritative_cost() {
            retained.cost = incoming.cost;
            retained.mark_provider_reported_cost();
        }
        // The shared schema namespaces a missing embedded id by DB path. If a
        // fingerprint-equivalent copy supplies one, preserve that globally
        // useful identity just as the previous schema accumulator did.
        if !retained_origin.has_embedded_id && incoming_origin.has_embedded_id {
            retained.dedup_key = incoming.dedup_key.clone();
            retained_origin.has_embedded_id = true;
        }
        if !identified_original {
            retained_origin.workspace_conflicted |= retained.workspace_key.is_some()
                && incoming.workspace_key.is_some()
                && retained.workspace_key != incoming.workspace_key;
            merge_workspace(retained, &incoming, retained_origin.workspace_conflicted);
        }
    }

    pub(crate) fn into_messages(mut self) -> Vec<UnifiedMessage> {
        let mut pending = std::mem::take(&mut self.pending);
        // Establish original owners before matching their historical copies.
        // This also handles several copies arriving before the only original.
        pending.sort_by(|(a, a_origin), (b, b_origin)| {
            a_origin
                .preference(a)
                .cmp(&b_origin.preference(b))
                .then_with(|| a.dedup_key.cmp(&b.dedup_key))
        });
        for (message, origin) in pending {
            self.insert(message, origin);
        }
        self.messages
            .into_iter()
            .enumerate()
            .filter_map(|(index, message)| (self.redirects[index] == index).then_some(message))
            .collect()
    }
}

fn merge_workspace(retained: &mut UnifiedMessage, incoming: &UnifiedMessage, conflicted: bool) {
    if conflicted
        || (retained.workspace_key.is_some()
            && incoming.workspace_key.is_some()
            && retained.workspace_key != incoming.workspace_key)
    {
        retained.set_workspace(None, None);
    } else if retained.workspace_key.is_none() {
        retained.set_workspace(
            incoming.workspace_key.clone(),
            incoming.workspace_label.clone(),
        );
    }
}

/// Read session attribution and physical message rows from one SQLite
/// snapshot. The shared decoder emits exact raw timing with each accepted row,
/// so the reducer and its cache never need to query the database again.
pub(crate) fn parse_micode_source(db_path: &Path) -> MiMoSource {
    #[cfg(test)]
    io_tests::record_open();
    let Some(conn) = open_readonly_sqlite_opt(db_path) else {
        return MiMoSource::default();
    };
    if conn.execute_batch("BEGIN DEFERRED").is_err() {
        return MiMoSource::default();
    }
    #[cfg(test)]
    io_tests::record_sessions();
    let (sessions, sessions_complete) = read_sessions(&conn);
    let session_clients: HashMap<String, String> = sessions
        .iter()
        .filter(|(_, info)| info.desktop)
        .map(|(id, _)| (id.clone(), MICODE_DESKTOP_CLIENT_ID.to_string()))
        .collect();
    let (messages, rows, complete) = parse_opencode_schema_rows_on(
        db_path,
        &conn,
        OpenCodeSchemaConfig::micode(),
        &session_clients,
    );
    let metadata = messages
        .iter()
        .zip(rows)
        .map(|(message, row)| MiMoRowMetadata {
            session_created_bits: sessions
                .get(&message.session_id)
                .and_then(|session| session.created_at)
                .map(f64::to_bits),
            row,
        })
        .collect();
    drop(conn);
    #[cfg(test)]
    io_tests::run_after_read_hook();
    MiMoSource {
        messages,
        metadata,
        complete: complete && sessions_complete,
    }
}

pub fn parse_micode_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    let mut messages = MiMoMessages::default();
    messages.extend(db_path, parse_micode_source(db_path));
    messages.into_messages()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn create_micode_sqlite_db(db_path: &Path) -> Connection {
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

    #[test]
    fn test_parse_micode_sqlite_basic() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");

        let conn = create_micode_sqlite_db(&db_path);

        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 100,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000001234.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_001", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "micode");
        assert_eq!(messages[0].model_id, "mimo-v2.5-pro");
        assert_eq!(messages[0].provider_id, "mimo");
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[0].tokens.output, 500);
        assert_eq!(messages[0].tokens.reasoning, 100);
        assert_eq!(messages[0].tokens.cache_read, 200);
        assert_eq!(messages[0].tokens.cache_write, 50);
        assert!((messages[0].cost - 0.05).abs() < 1e-9);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );
        assert_eq!(messages[0].duration_ms, Some(1234));
    }

    #[test]
    fn test_parse_micode_sqlite_skips_user_messages() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");

        let conn = create_micode_sqlite_db(&db_path);

        let user_msg = r#"{
            "role": "user",
            "modelID": "mimo-v2.5-pro",
            "time": { "created": 1700000000000.0 }
        }"#;

        let assistant_msg = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "tokens": { "input": 100, "output": 50, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1700000001000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_user", "ses_001", user_msg],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_assistant", "ses_001", assistant_msg],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        // This message carries no embedded JSON id, so the dedup key falls back
        // to the SQLite row id and is namespaced by the database path.
        assert!(messages[0]
            .dedup_key
            .as_deref()
            .is_some_and(|key| key.ends_with(":msg_assistant")));
    }

    /// Regression: MiMo Code uses channel-suffixed databases (mimocode.db and
    /// mimocode-<channel>.db). A mid-session channel switch can write the SAME
    /// message (same embedded id) to both files. The embedded id must NOT be
    /// namespaced by the database, otherwise the cross-file dedup set produces
    /// two distinct keys and the message's cost + tokens get counted twice.
    #[test]
    fn embedded_message_id_is_not_namespaced_by_database() {
        let dir = tempfile::tempdir().unwrap();
        let db_a = dir.path().join("mimocode.db");
        let db_b = dir.path().join("mimocode-beta.db");
        // Embedded JSON "id" is the globally unique message id.
        let msg = r#"{
            "id": "msg_shared",
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000000.0 }
        }"#;
        // Different SQLite row ids prove the collapse is driven by the embedded
        // id (not the row id), exactly as a mid-session channel switch records.
        for (db, row_id) in [(&db_a, "row_a"), (&db_b, "row_b")] {
            let conn = create_micode_sqlite_db(db);
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params![row_id, "ses_1", msg],
            )
            .unwrap();
            drop(conn);
        }

        let a = parse_micode_sqlite(&db_a);
        let b = parse_micode_sqlite(&db_b);
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        // Same embedded id across both channel databases yields IDENTICAL,
        // un-namespaced dedup keys, so a shared dedup set collapses the
        // duplicate to a single count.
        assert_eq!(a[0].dedup_key, Some("msg_shared".to_string()));
        assert_eq!(b[0].dedup_key, Some("msg_shared".to_string()));

        // Prove the collapse end-to-end with the same HashSet logic used by the
        // cross-file aggregation in lib.rs.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let kept: Vec<_> = a
            .into_iter()
            .chain(b)
            .filter(|m| m.dedup_key.as_ref().is_none_or(|k| seen.insert(k.clone())))
            .collect();
        assert_eq!(kept.len(), 1, "shared embedded id must be counted once");
    }

    /// Two DIFFERENT messages that happen to share a SQLite rowid across two
    /// databases (rowids are per-database, not globally unique) must NOT be
    /// collapsed by the cross-file dedup set. The row-id fallback path is
    /// namespaced by database precisely to keep them distinct.
    #[test]
    fn rowid_fallback_is_namespaced_by_database() {
        let dir = tempfile::tempdir().unwrap();
        let db_a = dir.path().join("a.db");
        let db_b = dir.path().join("b.db");
        // No embedded "id" field -> the parser falls back to the SQLite rowid.
        let msg = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000000.0 }
        }"#;
        for db in [&db_a, &db_b] {
            let conn = create_micode_sqlite_db(db);
            // Same SQLite row id ("id" column) in both databases. With no
            // embedded JSON id, the parser falls back to this row id.
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params!["row_shared", "ses_1", msg],
            )
            .unwrap();
            drop(conn);
        }

        let a = parse_micode_sqlite(&db_a);
        let b = parse_micode_sqlite(&db_b);
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        // Same row id ("row_shared") in two databases must yield DISTINCT,
        // db-namespaced keys so the two unrelated messages are not merged.
        assert_ne!(a[0].dedup_key, b[0].dedup_key);
        assert!(a[0].dedup_key.as_deref().unwrap().ends_with(":row_shared"));
        assert!(b[0].dedup_key.as_deref().unwrap().ends_with(":row_shared"));

        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let kept: Vec<_> = a
            .into_iter()
            .chain(b)
            .filter(|m| m.dedup_key.as_ref().is_none_or(|k| seen.insert(k.clone())))
            .collect();
        assert_eq!(
            kept.len(),
            2,
            "rowid collisions across DBs must stay distinct"
        );
    }

    #[test]
    fn test_parse_micode_sqlite_negative_values_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");

        let conn = create_micode_sqlite_db(&db_path);

        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": -0.05,
            "tokens": {
                "input": -100,
                "output": -50,
                "reasoning": -25,
                "cache": { "read": -200, "write": -10 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_negative", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 0);
        assert_eq!(messages[0].tokens.output, 0);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.cache_write, 0);
        assert_eq!(messages[0].tokens.reasoning, 0);
        assert!(messages[0].cost >= 0.0);
    }

    #[test]
    fn test_parse_micode_sqlite_dedup_forked_history() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);

        let root_msg = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 25,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0, "completed": 1700000000500.0 }
        }"#;

        let new_msg = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.08,
            "tokens": {
                "input": 1300,
                "output": 650,
                "reasoning": 40,
                "cache": { "read": 100, "write": 0 }
            },
            "time": { "created": 1700000001000.0, "completed": 1700000001500.0 }
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

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[1].tokens.input, 1300);
    }

    #[test]
    fn duplicate_explicit_zero_cost_upgrades_retained_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);
        let without_cost = r#"{
            "role": "assistant",
            "modelID": "unknown-model",
            "providerID": "mimo",
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000000.0 }
        }"#;
        let with_zero_cost = r#"{
            "role": "assistant",
            "modelID": "unknown-model",
            "providerID": "mimo",
            "cost": 0,
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000000.0 }
        }"#;
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["row_a", "session_a", without_cost],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["row_b", "session_b", with_zero_cost],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].cost, 0.0);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );
    }

    #[test]
    fn test_parse_micode_sqlite_workspace_from_session() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                directory TEXT NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory) VALUES (?1, ?2)",
            rusqlite::params!["ses_001", "/Users/alice/micode-repo"],
        )
        .unwrap();

        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
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
            rusqlite::params!["msg_ws", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/alice/micode-repo")
        );
        assert_eq!(messages[0].workspace_label.as_deref(), Some("micode-repo"));
    }

    #[test]
    fn test_parse_micode_sqlite_splits_desktop_sessions_by_version_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                directory TEXT,
                version TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory, version) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                "ses_desktop",
                "/Users/alice/desktop-repo",
                "desktop-5198ff5"
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory, version) VALUES (?1, ?2, ?3)",
            rusqlite::params!["ses_cli", "/Users/alice/cli-repo", "1.2.3"],
        )
        .unwrap();

        let desktop_msg = r#"{
            "id": "msg_desktop",
            "role": "assistant",
            "modelID": "mimo-x-pro-preview",
            "providerID": "xiaomi",
            "cost": 0,
            "tokens": {
                "input": 1000,
                "output": 200,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;
        let cli_msg = r#"{
            "id": "msg_cli",
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0,
            "tokens": {
                "input": 500,
                "output": 100,
                "reasoning": 0,
                "cache": { "read": 0, "write": 0 }
            },
            "time": { "created": 1700000005000.0 }
        }"#;
        for (row_id, session_id, data) in [
            ("msg_desktop", "ses_desktop", desktop_msg),
            ("msg_cli", "ses_cli", cli_msg),
        ] {
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params![row_id, session_id, data],
            )
            .unwrap();
        }
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            2,
            "desktop and CLI assistant rows must both survive parse: {:?}",
            messages
                .iter()
                .map(|m| (m.session_id.as_str(), m.client.as_str()))
                .collect::<Vec<_>>()
        );
        let by_session: std::collections::HashMap<&str, &str> = messages
            .iter()
            .map(|m| (m.session_id.as_str(), m.client.as_str()))
            .collect();
        assert_eq!(
            by_session.get("ses_desktop"),
            Some(&MICODE_DESKTOP_CLIENT_ID)
        );
        assert_eq!(by_session.get("ses_cli"), Some(&MICODE_CLIENT_ID));
    }

    #[test]
    fn test_is_desktop_session_version() {
        assert!(is_desktop_session_version("desktop-5198ff5"));
        assert!(is_desktop_session_version("Desktop-abc"));
        assert!(is_desktop_session_version("  desktop-abc  "));
        assert!(!is_desktop_session_version("1.2.3"));
        assert!(!is_desktop_session_version("latest"));
        assert!(!is_desktop_session_version("desktopapp"));
    }

    #[test]
    fn test_parse_micode_sqlite_keeps_cross_surface_identical_fingerprints() {
        // Regression: desktop and CLI turns can share every fingerprint field
        // (model, tokens, cost, agent, timestamps). Surface-aware merge must
        // keep both; post-hoc remap after Merge would label only the retained
        // row and hide the other surface from its client filter.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                directory TEXT,
                version TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory, version) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                "ses_desktop",
                "/Users/alice/desktop-repo",
                "desktop-5198ff5"
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory, version) VALUES (?1, ?2, ?3)",
            rusqlite::params!["ses_cli", "/Users/alice/cli-repo", "1.0.0"],
        )
        .unwrap();
        // Identical usage fingerprint; distinct message ids.
        for (row_id, session_id, msg_id) in [
            ("row_desktop", "ses_desktop", "msg_desktop"),
            ("row_cli", "ses_cli", "msg_cli"),
        ] {
            let data = format!(
                r#"{{
                    "id": "{msg_id}",
                    "role": "assistant",
                    "modelID": "mimo-x-pro-preview",
                    "providerID": "xiaomi",
                    "cost": 0,
                    "agent": "build",
                    "tokens": {{
                        "input": 1000,
                        "output": 200,
                        "reasoning": 0,
                        "cache": {{ "read": 0, "write": 0 }}
                    }},
                    "time": {{ "created": 1700000000000.0 }}
                }}"#
            );
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params![row_id, session_id, data],
            )
            .unwrap();
        }
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            2,
            "identical fingerprints on different surfaces must not merge: {:?}",
            messages
                .iter()
                .map(|m| (m.session_id.as_str(), m.client.as_str()))
                .collect::<Vec<_>>()
        );
        let by_session: std::collections::HashMap<&str, &str> = messages
            .iter()
            .map(|m| (m.session_id.as_str(), m.client.as_str()))
            .collect();
        assert_eq!(
            by_session.get("ses_desktop"),
            Some(&MICODE_DESKTOP_CLIENT_ID)
        );
        assert_eq!(by_session.get("ses_cli"), Some(&MICODE_CLIENT_ID));
    }

    #[test]
    fn test_parse_micode_sqlite_with_agent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);

        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "agent": "build",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 100,
                "cache": { "read": 200, "write": 50 }
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_agent", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent, Some("Build".to_string()));
    }

    /// Regression for PR #710: `time.created` was hard-assumed to be epoch
    /// milliseconds. If MiMo writes epoch *seconds*, the date landed ~1000x in
    /// the past (1970-era). A ms-valued and a seconds-valued `time.created` that
    /// denote the SAME instant must normalize to the same date and the same
    /// (millisecond-scale) timestamp. Without `micode_timestamp_to_ms`, the
    /// seconds variant would yield 1970-01-20 instead of 2023-11-14.
    #[test]
    fn test_parse_micode_sqlite_normalizes_seconds_and_milliseconds() {
        let dir = tempfile::tempdir().unwrap();
        let db_ms = dir.path().join("ms.db");
        let db_secs = dir.path().join("secs.db");

        // 1_700_000_000 s == 1_700_000_000_000 ms == 2023-11-14T22:13:20Z.
        let msg_ms = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000000.0, "completed": 1700000001234.0 }
        }"#;
        // Same instant, expressed in epoch SECONDS (the bugged-input shape).
        let msg_secs = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 10, "output": 5 },
            "time": { "created": 1700000000.0, "completed": 1700000001.234 }
        }"#;

        for (db, data) in [(&db_ms, msg_ms), (&db_secs, msg_secs)] {
            let conn = create_micode_sqlite_db(db);
            conn.execute(
                "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
                rusqlite::params!["msg_1", "ses_1", data],
            )
            .unwrap();
            drop(conn);
        }

        let ms = parse_micode_sqlite(&db_ms);
        let secs = parse_micode_sqlite(&db_secs);
        assert_eq!(ms.len(), 1);
        assert_eq!(secs.len(), 1);

        // Both inputs resolve to the SAME instant: identical timestamp (ms) and
        // identical, non-empty (i.e. not 1970-era-then-formatted) date.
        assert_eq!(ms[0].timestamp, 1_700_000_000_000);
        assert_eq!(secs[0].timestamp, 1_700_000_000_000);
        assert_eq!(ms[0].date, secs[0].date);
        assert!(!ms[0].date.is_empty());

        // Duration is in milliseconds for BOTH representations (~1234 ms), not
        // ~1 (which is what the seconds input would have produced unnormalized).
        assert_eq!(ms[0].duration_ms, Some(1234));
        assert_eq!(secs[0].duration_ms, Some(1234));
    }

    /// A non-object `path` field (e.g. a bare string instead of `{ "root": .. }`)
    /// must not crash deserialization or fail the whole message: the custom
    /// `deserialize_micode_path` extracts `root` defensively, leaving it `None`.
    /// The message must still parse and have no embedded-path workspace.
    #[test]
    fn test_parse_micode_sqlite_non_object_path_field() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);

        // `path` is a string, not an object — the deserializer's `.get("root")`
        // returns None rather than erroring, so the message survives.
        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 100, "output": 50 },
            "path": "/some/string/not/an/object",
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_badpath", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(
            messages.len(),
            1,
            "non-object path must not drop the message"
        );
        assert_eq!(messages[0].tokens.input, 100);
        // No usable root -> no workspace derived from the embedded path.
        assert_eq!(messages[0].workspace_key, None);
        assert_eq!(messages[0].workspace_label, None);
    }

    /// Legacy-query fallback: when the database has no `session` table, the
    /// modern query (which JOINs `session`) fails to prepare and the parser
    /// falls back to `legacy_query`. In that path `workspace_root` from the row
    /// is NULL, so the workspace must come from the message's EMBEDDED `path.root`.
    #[test]
    fn test_parse_micode_sqlite_legacy_fallback_embedded_path_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        // Note: create_micode_sqlite_db creates ONLY the `message` table, so the
        // modern query's `LEFT JOIN session` cannot prepare and we exercise the
        // legacy fallback.
        let conn = create_micode_sqlite_db(&db_path);

        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": { "input": 100, "output": 50 },
            "path": { "root": "/Users/bob/embedded-repo" },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_embedded", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        // Row workspace_root is NULL on the legacy path, so the embedded
        // `path.root` supplies the workspace.
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/bob/embedded-repo")
        );
        assert_eq!(
            messages[0].workspace_label.as_deref(),
            Some("embedded-repo")
        );
    }

    #[test]
    fn test_parse_micode_sqlite_missing_cache_defaults_to_zero() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_micode.db");
        let conn = create_micode_sqlite_db(&db_path);

        // Assistant payload with no `cache` object at all — must parse (not be
        // dropped) with cache tokens defaulting to 0.
        let data_json = r#"{
            "role": "assistant",
            "modelID": "mimo-v2.5-pro",
            "providerID": "mimo",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 500,
                "reasoning": 100
            },
            "time": { "created": 1700000000000.0 }
        }"#;

        conn.execute(
            "INSERT INTO message (id, session_id, data) VALUES (?1, ?2, ?3)",
            rusqlite::params!["msg_no_cache", "ses_001", data_json],
        )
        .unwrap();
        drop(conn);

        let messages = parse_micode_sqlite(&db_path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 1000);
        assert_eq!(messages[0].tokens.output, 500);
        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.cache_write, 0);
    }
}
