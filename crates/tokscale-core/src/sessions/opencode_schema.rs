//! Shared driver for the OpenCode SQLite session schema.
//!
//! OpenCode, MiMo Code, and Kilo all persist assistant turns as a JSON payload
//! in a `data` column keyed by `(id, session_id)`. The payloads share one
//! schema — `modelID` / `providerID` / `tokens.{input,output,reasoning,cache}`
//! / `time.{created,completed}` — so this module owns a single set of
//! `Deserialize` types and a single row-ingest loop for all three, instead of
//! each client re-declaring the schema and re-implementing the loop.
//!
//! The clients differ only in *policy* (which tables exist, how duplicates
//! collapse, whether epochs are seconds or milliseconds, which fallbacks
//! apply). Every such difference is an explicit field on
//! [`OpenCodeSchemaConfig`], which is `Copy` and built from a per-client
//! `const fn` constructor. The driver is a plain `fn` taking that config by
//! value rather than a generic over the message type or the row callback:
//! generics would monomorphize per client and *grow* the binary, which is the
//! opposite of the point.

use super::utils::{open_readonly_sqlite_opt, sqlite_for_each_row_on};
use super::{
    normalize_opencode_agent_name, normalize_workspace_key, workspace_label_from_key,
    UnifiedMessage,
};
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

// =============================================================================
// Schema types
// =============================================================================

/// An assistant turn as stored in the `data` column (and in OpenCode's legacy
/// JSON message files).
///
/// The shape is the permissive union of every variant the OpenCode-schema
/// clients emit: a field that is mandatory for one client is optional here, and
/// the per-client strictness is re-applied at parse time from
/// [`OpenCodeSchemaConfig`]. Keeping the strictness in the config rather than in
/// the type is what lets one `Deserialize` impl serve all three clients without
/// changing what any of them accept.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OpenCodeSchemaMessage {
    #[serde(default)]
    pub id: Option<String>,
    /// OpenCode's camelCase session id, used by its legacy JSON files.
    #[serde(rename = "sessionID", default)]
    pub session_id: Option<String>,
    /// Kilo's snake_case session id.
    ///
    /// Deliberately a second field rather than a `serde(alias)` on
    /// `session_id`: an alias would also make OpenCode's file parser start
    /// honouring `session_id`, silently widening a client that has only ever
    /// read `sessionID`.
    #[serde(rename = "session_id", default)]
    pub snake_session_id: Option<String>,
    /// Absent in OpenCode v2 `session_message` rows, where the row's `type`
    /// column carries the role and the SQL already filters to `assistant`.
    #[serde(default)]
    pub role: Option<String>,
    #[serde(rename = "modelID", default)]
    pub model_id: Option<String>,
    #[serde(rename = "providerID", default)]
    pub provider_id: Option<String>,
    /// OpenCode v2 nests model + provider under a `model` object.
    #[serde(default)]
    pub model: Option<OpenCodeSchemaModel>,
    pub cost: Option<f64>,
    pub tokens: Option<OpenCodeSchemaTokens>,
    pub time: Option<OpenCodeSchemaTime>,
    pub agent: Option<String>,
    pub mode: Option<String>,
    #[serde(default, deserialize_with = "deserialize_schema_path")]
    pub path: Option<OpenCodeSchemaPath>,
}

impl OpenCodeSchemaMessage {
    /// Resolve the model id from the top-level v1 field or the nested v2
    /// `model.id`, preferring the explicit top-level value when both exist.
    pub(crate) fn resolve_model_id(&self) -> Option<String> {
        self.model_id
            .clone()
            .or_else(|| self.model.as_ref().and_then(|m| m.id.clone()))
    }

    /// Resolve the provider id from the top-level v1 field or the nested v2
    /// `model.providerID`, preferring the explicit top-level value.
    pub(crate) fn resolve_provider_id(&self) -> Option<String> {
        self.provider_id
            .clone()
            .or_else(|| self.model.as_ref().and_then(|m| m.provider_id.clone()))
    }

    /// True when this row is an assistant turn under OpenCode's dual-schema
    /// rules. v1 rows carry an explicit `role`; v2 rows omit it and are
    /// pre-filtered by the SQL `type` column, so a missing role is assistant.
    pub(crate) fn is_assistant(&self) -> bool {
        self.role.as_deref().is_none_or(|role| role == "assistant")
    }

    /// The workspace root embedded in the payload's `path` object, if any.
    fn embedded_workspace_root(&self) -> Option<&str> {
        self.path.as_ref().and_then(|path| path.root.as_deref())
    }
}

/// OpenCode v2 nested model descriptor: `{"id": "...", "providerID": "..."}`.
#[derive(Debug, Deserialize)]
pub struct OpenCodeSchemaModel {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "providerID", default)]
    pub provider_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenCodeSchemaPath {
    pub root: Option<String>,
}

/// Accept any JSON value for `path` and keep only a string `root`.
///
/// Some builds write a non-object `path`, which a plain derive would reject —
/// dropping the whole message rather than just the workspace hint.
fn deserialize_schema_path<'de, D>(deserializer: D) -> Result<Option<OpenCodeSchemaPath>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let root = value
        .get("root")
        .and_then(|root| root.as_str())
        .map(str::to_string);

    Ok(Some(OpenCodeSchemaPath { root }))
}

#[derive(Debug, Deserialize)]
pub struct OpenCodeSchemaTokens {
    pub input: i64,
    pub output: i64,
    pub reasoning: Option<i64>,
    /// Optional in the union type. Clients that require a well-formed cache
    /// object set [`OpenCodeSchemaConfig::strict_cache`], which restores the
    /// drop-the-message behaviour their own derive used to produce.
    #[serde(default)]
    pub cache: Option<OpenCodeSchemaCache>,
}

#[derive(Debug, Default, Deserialize)]
pub struct OpenCodeSchemaCache {
    #[serde(default)]
    pub read: Option<i64>,
    #[serde(default)]
    pub write: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OpenCodeSchemaTime {
    /// Unix epoch, normally in milliseconds (as a float).
    pub created: f64,
    pub completed: Option<f64>,
}

// =============================================================================
// Per-client policy
// =============================================================================

/// When an embedded `cost` marks a message as carrying a provider-reported
/// price that tokscale's repricing pass must not overwrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CostProvenance {
    /// Never mark; the client's costs are always re-derived.
    Never,
    /// Mark when the resolved cost is strictly positive. A zero usually means
    /// the client itself had no pricing for the model, so leaving it unmarked
    /// lets tokscale estimate.
    WhenPositive,
    /// Mark whenever the payload carried a usable `cost`, including an
    /// explicit `0.0`.
    WhenReported,
}

/// How rows that describe the same assistant turn collapse together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DedupMode {
    /// Emit every row; the client has no duplicate sources.
    Off,
    /// Collapse every row sharing a fingerprint into one entry.
    Merge,
    /// Collapse rows sharing a fingerprint unless their embedded message ids
    /// disagree — that marks them as genuinely distinct turns that merely
    /// collided on every fingerprint field, not as forked copies.
    MergeUnlessIdConflict,
}

/// Per-client policy for [`parse_opencode_schema_sqlite`].
///
/// `Copy` and built from `const fn` constructors so a call site reads as
/// `parse_opencode_schema_sqlite(db, OpenCodeSchemaConfig::micode())`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenCodeSchemaConfig {
    /// tokscale client id stamped on every emitted message.
    pub client: &'static str,
    /// Query groups to run, in order. Within a group the first query that
    /// prepares successfully wins, so a client can offer several schema
    /// variants (modern / legacy) and let the database pick.
    pub query_groups: &'static [&'static [&'static str]],
    /// Provider used when the payload names none and inference is off or
    /// finds nothing.
    pub fallback_provider: &'static str,
    /// Infer the provider from the model id before using `fallback_provider`.
    pub infer_provider_from_model: bool,
    /// Accept OpenCode's v1/v2 variance: model + provider nested under
    /// `$.model`, and a missing `$.role` meaning assistant.
    pub dual_schema: bool,
    /// Prefer the payload's own session id over the row's `session_id` column.
    pub payload_session_id: bool,
    /// Run the resolved agent/mode through `normalize_opencode_agent_name`.
    pub normalize_agent: bool,
    /// Resolve the agent as `mode` then `agent`; when false, `agent` then
    /// `mode`.
    pub prefer_mode_over_agent: bool,
    /// Require `$.tokens.cache` to be present with both `read` and `write`,
    /// dropping the message otherwise.
    pub strict_cache: bool,
    /// Treat an epoch at or below 1e12 as seconds and scale it to milliseconds.
    pub normalize_epoch_seconds: bool,
    /// Timestamp to use when the payload has no `$.time`. `None` drops such a
    /// message, matching a client whose own type made `time` mandatory.
    pub fallback_timestamp: Option<i64>,
    /// Record `completed - created` as the message duration.
    pub record_duration: bool,
    /// When an embedded cost marks the message as provider-reported.
    pub cost_provenance: CostProvenance,
    /// Capture the workspace root from the session join and `$.path.root`.
    pub capture_workspace: bool,
    /// Namespace the row-id dedup fallback by the database path. Needed by
    /// clients that keep several databases whose rowids are not comparable.
    pub namespace_rowid_dedup_key: bool,
    /// How duplicate rows collapse.
    pub dedup: DedupMode,
}

impl OpenCodeSchemaConfig {
    /// Baseline shared by every OpenCode-schema client. Each constructor below
    /// states only the fields where that client departs from OpenCode itself.
    const fn base(client: &'static str) -> Self {
        Self {
            client,
            query_groups: &[],
            fallback_provider: "unknown",
            infer_provider_from_model: false,
            dual_schema: false,
            payload_session_id: false,
            normalize_agent: true,
            prefer_mode_over_agent: true,
            strict_cache: true,
            normalize_epoch_seconds: false,
            fallback_timestamp: None,
            record_duration: true,
            cost_provenance: CostProvenance::WhenPositive,
            capture_workspace: true,
            namespace_rowid_dedup_key: false,
            dedup: DedupMode::MergeUnlessIdConflict,
        }
    }

    pub(crate) const fn opencode() -> Self {
        Self {
            query_groups: OPENCODE_QUERY_GROUPS,
            dual_schema: true,
            ..Self::base("opencode")
        }
    }

    pub(crate) const fn micode() -> Self {
        Self {
            query_groups: MICODE_QUERY_GROUPS,
            // MiMo assistant messages may omit `cache` (or its read/write);
            // requiring it would silently drop the message.
            strict_cache: false,
            // Some MiMo builds write epoch seconds where OpenCode writes
            // milliseconds, which landed dates ~1000x in the past.
            normalize_epoch_seconds: true,
            // An explicit `"cost": 0.0` is a real MiMo-reported price, not a
            // missing one, so it must survive repricing.
            cost_provenance: CostProvenance::WhenReported,
            // MiMo uses channel-suffixed databases whose rowids are only
            // unique per file.
            namespace_rowid_dedup_key: true,
            dedup: DedupMode::Merge,
            ..Self::base("micode")
        }
    }

    /// Kilo reads a single `message` table and has no duplicate sources, so it
    /// keeps neither fingerprints nor workspace/duration metadata.
    pub(crate) const fn kilo(fallback_timestamp: i64) -> Self {
        Self {
            query_groups: KILO_QUERY_GROUPS,
            fallback_provider: "kilo",
            infer_provider_from_model: true,
            payload_session_id: true,
            normalize_agent: false,
            prefer_mode_over_agent: false,
            fallback_timestamp: Some(fallback_timestamp),
            record_duration: false,
            cost_provenance: CostProvenance::Never,
            capture_workspace: false,
            dedup: DedupMode::Off,
            ..Self::base("kilo")
        }
    }
}

// =============================================================================
// Query variants
// =============================================================================

/// OpenCode v2 (`opencode-next.db`): per-message rows in `session_message`,
/// role in the `type` column, model + provider nested under `$.model`.
/// Databases whose `session` table predates the `title` column fall back to the
/// title-less variant — the title is optional, not a gating column.
const OPENCODE_V2_QUERIES: &[&str] = &[
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, s.title AS session_title
        FROM session_message sm
        LEFT JOIN session s ON s.id = sm.session_id
        WHERE sm.type = 'assistant'
          AND json_extract(sm.data, '$.tokens') IS NOT NULL
        ORDER BY sm.id, sm.session_id
    "#,
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title
        FROM session_message sm
        LEFT JOIN session s ON s.id = sm.session_id
        WHERE sm.type = 'assistant'
          AND json_extract(sm.data, '$.tokens') IS NOT NULL
        ORDER BY sm.id, sm.session_id
    "#,
];

/// OpenCode v1 (`opencode.db`, 1.2+): per-message rows in `message`, role in
/// `$.role`. Three tiers: `session` has `directory` and `title`; `directory`
/// only; no `session` table at all (drops workspace and title).
const OPENCODE_V1_QUERIES: &[&str] = &[
    r#"
        SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root, s.title AS session_title
        FROM message m
        LEFT JOIN session s ON s.id = m.session_id
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#,
    r#"
        SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title
        FROM message m
        LEFT JOIN session s ON s.id = m.session_id
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#,
    r#"
        SELECT m.id, m.session_id, m.data, NULL AS workspace_root, NULL AS session_title
        FROM message m
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#,
];

/// Both OpenCode generations are probed against the same database; whichever
/// tables exist contribute rows, and the fingerprint dedup collapses any
/// overlap between them.
const OPENCODE_QUERY_GROUPS: &[&[&str]] = &[OPENCODE_V2_QUERIES, OPENCODE_V1_QUERIES];

/// MiMo Code: `message` table, with the `session` join dropped on databases
/// that predate it.
const MICODE_QUERIES: &[&str] = &[
    r#"
        SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title
        FROM message m
        LEFT JOIN session s ON s.id = m.session_id
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#,
    r#"
        SELECT m.id, m.session_id, m.data, NULL AS workspace_root, NULL AS session_title
        FROM message m
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#,
];

const MICODE_QUERY_GROUPS: &[&[&str]] = &[MICODE_QUERIES];

/// Kilo: a single `message` table with no session join.
///
/// The `json_valid` guard is load-bearing here and deliberately absent from the
/// other clients: without it a single malformed `data` blob makes SQLite's
/// `json_extract` abort the whole statement rather than skip the row.
const KILO_QUERIES: &[&str] = &[r#"
        SELECT m.id, m.session_id, m.data, NULL AS workspace_root, NULL AS session_title
        FROM message m
        WHERE json_valid(m.data)
          AND json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
    "#];

const KILO_QUERY_GROUPS: &[&[&str]] = &[KILO_QUERIES];

// =============================================================================
// Workspace helpers
// =============================================================================

fn workspace_from_root(root: Option<&str>) -> (Option<String>, Option<String>) {
    let workspace_key = root.and_then(normalize_workspace_key);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
    (workspace_key, workspace_label)
}

pub(crate) fn set_workspace_from_root(message: &mut UnifiedMessage, root: Option<&str>) {
    let (workspace_key, workspace_label) = workspace_from_root(root);
    message.set_workspace(workspace_key, workspace_label);
}

fn merge_duplicate_workspace(
    message: &mut UnifiedMessage,
    state: &mut SchemaDedupState,
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

// =============================================================================
// Field resolution
// =============================================================================

/// Clamp `$.tokens.cache` to `(read, write)`, or reject the message when the
/// client requires a well-formed cache object and the payload lacks one.
fn resolve_cache(cache: Option<&OpenCodeSchemaCache>, strict: bool) -> Option<(i64, i64)> {
    match cache {
        Some(cache) => match (cache.read, cache.write) {
            (Some(read), Some(write)) => Some((read.max(0), write.max(0))),
            _ if strict => None,
            (read, write) => Some((read.unwrap_or(0).max(0), write.unwrap_or(0).max(0))),
        },
        None if strict => None,
        None => Some((0, 0)),
    }
}

/// Normalize an epoch `time.created`/`time.completed` to milliseconds.
///
/// A recent epoch is ~1.7e12 in milliseconds versus ~1.7e9 in seconds, so a
/// value at or under the `1e12` threshold is treated as seconds and scaled up
/// for clients known to emit both.
fn normalize_epoch(timestamp: f64, cfg: &OpenCodeSchemaConfig) -> f64 {
    if !cfg.normalize_epoch_seconds || timestamp > 1e12 {
        timestamp
    } else {
        timestamp * 1000.0
    }
}

/// Both endpoints arrive already normalized, so a seconds/milliseconds mismatch
/// still yields a millisecond duration rather than one 1000x too small.
fn duration_ms(created_ms: f64, completed_ms: Option<f64>) -> Option<i64> {
    let duration = completed_ms? - created_ms;
    if duration.is_finite() && duration > 0.0 {
        Some(duration as i64)
    } else {
        None
    }
}

fn resolve_provider(
    msg: &OpenCodeSchemaMessage,
    model_id: &str,
    cfg: &OpenCodeSchemaConfig,
) -> String {
    let explicit = if cfg.dual_schema {
        msg.resolve_provider_id()
    } else {
        msg.provider_id.clone()
    };

    let provider = explicit
        .or_else(|| {
            if cfg.infer_provider_from_model {
                provider_identity::inferred_provider_from_model(model_id).map(str::to_string)
            } else {
                None
            }
        })
        .unwrap_or_else(|| cfg.fallback_provider.to_string());

    provider_identity::canonical_provider(&provider).unwrap_or(provider)
}

/// A payload `cost` is usable only when it is a finite, non-negative number.
pub(crate) fn reported_cost(cost: Option<f64>) -> Option<f64> {
    cost.filter(|cost| cost.is_finite() && *cost >= 0.0)
}

// =============================================================================
// Deduplication
// =============================================================================

/// The immutable content of an assistant turn. Two rows agreeing on every field
/// describe the same turn, whether they came from a forked session copy, a
/// channel-suffixed sibling database, or an overlap between schema generations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OpenCodeSchemaFingerprint {
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
struct SchemaDedupState {
    /// The entry's embedded (`$.id`) message id, if any. Under
    /// [`DedupMode::MergeUnlessIdConflict`] two rows that share every
    /// fingerprint field but carry *different* embedded ids are distinct
    /// messages, not fork copies, and must not be merged.
    message_id: Option<String>,
    has_workspace_conflict: bool,
}

/// Column layout shared by every query variant:
/// `(row_id, session_id, data_json, workspace_root, session_title)`.
type OpenCodeSchemaRow = (String, String, String, Option<String>, Option<String>);

#[derive(Default)]
struct SchemaAccumulator {
    messages: Vec<UnifiedMessage>,
    fingerprint_indices: HashMap<OpenCodeSchemaFingerprint, Vec<usize>>,
    dedup_states: Vec<SchemaDedupState>,
}

impl SchemaAccumulator {
    /// Decode one row's JSON payload and merge it into the accumulator.
    fn ingest(&mut self, row: OpenCodeSchemaRow, cfg: &OpenCodeSchemaConfig, db_namespace: &str) {
        let (row_id, row_session_id, data_json, row_workspace_root, row_session_title) = row;

        let mut bytes = data_json.into_bytes();
        let msg: OpenCodeSchemaMessage = match simd_json::from_slice(&mut bytes) {
            Ok(m) => m,
            Err(_) => return,
        };

        // v1 rows carry an explicit role; v2 rows omit it and are pre-filtered
        // by the SQL `type` column, so only a dual-schema client may treat a
        // missing role as assistant.
        let is_assistant = if cfg.dual_schema {
            msg.is_assistant()
        } else {
            msg.role.as_deref() == Some("assistant")
        };
        if !is_assistant {
            return;
        }

        let tokens = match msg.tokens {
            Some(ref tokens) => tokens,
            None => return,
        };
        let Some((cache_read, cache_write)) =
            resolve_cache(tokens.cache.as_ref(), cfg.strict_cache)
        else {
            return;
        };

        let resolved_model_id = if cfg.dual_schema {
            msg.resolve_model_id()
        } else {
            msg.model_id.clone()
        };
        let model_id = match resolved_model_id {
            Some(model_id) => model_id,
            None => return,
        };

        let provider_id = resolve_provider(&msg, &model_id, cfg);

        // A payload with no `$.time` is dropped unless the client supplies a
        // fallback, matching the mandatory `time` field its own type declared.
        let (created_ms, completed_ms) = match msg.time {
            Some(ref time) => (
                normalize_epoch(time.created, cfg),
                time.completed
                    .map(|completed| normalize_epoch(completed, cfg)),
            ),
            None => match cfg.fallback_timestamp {
                Some(fallback) => (fallback as f64, None),
                None => return,
            },
        };

        let agent_or_mode = if cfg.prefer_mode_over_agent {
            msg.mode.clone().or_else(|| msg.agent.clone())
        } else {
            msg.agent.clone().or_else(|| msg.mode.clone())
        };
        let agent = agent_or_mode.map(|agent| {
            if cfg.normalize_agent {
                normalize_opencode_agent_name(&agent)
            } else {
                agent
            }
        });

        let input = tokens.input.max(0);
        let output = tokens.output.max(0);
        let reasoning = tokens.reasoning.unwrap_or(0).max(0);
        let reported = reported_cost(msg.cost);
        let cost = reported.unwrap_or(0.0);

        let session_id = if cfg.payload_session_id {
            msg.snake_session_id.clone().unwrap_or(row_session_id)
        } else {
            row_session_id
        };

        let message_id = msg.id.clone();
        let dedup_key = match message_id.clone() {
            // Embedded ids are globally unique: keep them un-namespaced so the
            // same message in sibling databases collapses.
            Some(id) => id,
            // Rowids are per-database: namespace to avoid false cross-file
            // merges when the client keeps more than one database.
            None if cfg.namespace_rowid_dedup_key => format!("{db_namespace}:{row_id}"),
            None => row_id,
        };

        let mut unified = UnifiedMessage::new_with_agent(
            cfg.client,
            model_id.clone(),
            provider_id.clone(),
            session_id,
            created_ms as i64,
            TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                reasoning,
            },
            cost,
            agent.clone(),
        );
        if cfg.record_duration {
            unified.duration_ms = duration_ms(created_ms, completed_ms);
        }
        match cfg.cost_provenance {
            CostProvenance::Never => {}
            CostProvenance::WhenPositive => {
                if cost > 0.0 {
                    unified.mark_provider_reported_cost();
                }
            }
            CostProvenance::WhenReported => {
                if reported.is_some() {
                    unified.mark_provider_reported_cost();
                }
            }
        }
        unified.dedup_key = Some(dedup_key);

        let workspace_root = if cfg.capture_workspace {
            row_workspace_root
                .as_deref()
                .or_else(|| msg.embedded_workspace_root())
        } else {
            None
        };
        if cfg.capture_workspace {
            set_workspace_from_root(&mut unified, workspace_root);
        }

        if let Some(ref title) = row_session_title {
            let trimmed = title.trim();
            if !trimmed.is_empty() {
                unified.session_title = Some(trimmed.to_string());
            }
        }

        if cfg.dedup == DedupMode::Off {
            self.messages.push(unified);
            return;
        }

        let fingerprint = OpenCodeSchemaFingerprint {
            created_bits: created_ms.to_bits(),
            completed_bits: completed_ms.map(f64::to_bits),
            model_id,
            provider_id,
            input,
            output,
            reasoning,
            cache_read,
            cache_write,
            cost_bits: cost.to_bits(),
            agent,
        };

        // Cloning the small index list avoids holding a borrow of
        // `fingerprint_indices` while reading `dedup_states`.
        let candidate = {
            let slots = self
                .fingerprint_indices
                .get(&fingerprint)
                .cloned()
                .unwrap_or_default();
            match cfg.dedup {
                DedupMode::Off => None,
                DedupMode::Merge => slots.first().copied(),
                // Merge into the first entry that is NOT a definitively
                // different message -- skip any whose stored embedded id
                // conflicts with this row's.
                DedupMode::MergeUnlessIdConflict => slots.into_iter().find(|&index| {
                    !matches!(
                        (&self.dedup_states[index].message_id, &message_id),
                        (Some(existing), Some(incoming)) if existing != incoming
                    )
                }),
            }
        };

        if let Some(index) = candidate {
            // A duplicate carrying an authoritative cost upgrades the retained
            // entry's provenance. This is inert for clients that derive
            // provenance from the cost value alone: `cost_bits` is part of the
            // fingerprint, so every row in a slot already agrees on it.
            if unified.has_authoritative_cost() {
                self.messages[index].mark_provider_reported_cost();
            }
            let dedup_state = &mut self.dedup_states[index];
            // The first copy carrying an embedded id promotes the entry's
            // stable dedup key, and records the id so later rows can be told
            // apart.
            if message_id.is_some() && dedup_state.message_id.is_none() {
                dedup_state.message_id = message_id;
                self.messages[index].dedup_key = unified.dedup_key;
            }
            merge_duplicate_workspace(&mut self.messages[index], dedup_state, workspace_root);
            return;
        }

        let new_index = self.messages.len();
        self.dedup_states.push(SchemaDedupState {
            message_id,
            has_workspace_conflict: false,
        });
        self.fingerprint_indices
            .entry(fingerprint)
            .or_default()
            .push(new_index);
        self.messages.push(unified);
    }
}

// =============================================================================
// Driver
// =============================================================================

/// Run `query` and hand every row to `on_row`. Returns whether the statement
/// prepared, so the caller can fall through to the next schema variant when a
/// table or column does not exist in this database.
///
/// `on_row` is a `&mut dyn FnMut` rather than an `impl FnMut` on purpose: a
/// generic callback would monomorphize this function once per client and grow
/// the binary, which is what consolidating these parsers exists to avoid.
fn collect_rows(
    db_path: &Path,
    conn: &rusqlite::Connection,
    query: &str,
    on_row: &mut dyn FnMut(OpenCodeSchemaRow),
) -> bool {
    // Quiet: these queries are schema probes — the caller tries each spelling
    // in turn, so a query the database does not understand is expected.
    let scan = sqlite_for_each_row_on(conn, db_path, query, None, &mut |row| {
        let id: String = row.get(0)?;
        let session_id: String = row.get(1)?;
        let data_json: String = row.get(2)?;
        let workspace_root: Option<String> = row.get(3)?;
        let session_title: Option<String> = row.get(4)?;
        on_row((id, session_id, data_json, workspace_root, session_title));
        Ok(())
    });
    scan.prepared()
}

/// Parse assistant turns out of a SQLite database that uses the OpenCode
/// message schema, applying `cfg`'s per-client policy.
///
/// A missing or unreadable database yields no messages rather than an error, so
/// callers can probe candidate paths without special-casing absence.
pub(crate) fn parse_opencode_schema_sqlite(
    db_path: &Path,
    cfg: OpenCodeSchemaConfig,
) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite_opt(db_path) else {
        return Vec::new();
    };

    let db_namespace = if cfg.namespace_rowid_dedup_key {
        db_path.to_string_lossy().into_owned()
    } else {
        String::new()
    };

    let mut acc = SchemaAccumulator::default();
    for group in cfg.query_groups {
        for query in *group {
            if collect_rows(db_path, &conn, query, &mut |row| {
                acc.ingest(row, &cfg, &db_namespace)
            }) {
                break;
            }
        }
    }

    acc.messages
}
