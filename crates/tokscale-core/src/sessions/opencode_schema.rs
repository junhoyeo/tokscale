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

use super::utils::{
    open_readonly_sqlite_opt, sqlite_for_each_row_on, sqlite_for_each_row_on_with_params,
    SqliteScan,
};
use super::{
    normalize_opencode_agent_name, normalize_workspace_key, workspace_label_from_key,
    UnifiedMessage,
};

/// Shared base parser version for the OpenCode SQLite session-schema driver.
///
/// OpenCode, MiMo Code, and Kilo all read their SQLite stores through
/// [`parse_opencode_schema_sqlite`], so a change to this driver (or to the
/// shared `Deserialize` types it ingests) alters what byte-identical
/// databases parse to for every adopter at once. Bump this base when that
/// happens; `message_cache::parser_version()` derives each member's version
/// from it (base plus a per-client offset that preserves independent history)
/// so no member can be left serving stale cache entries. Per-client policy
/// differences live in [`OpenCodeSchemaConfig`]; a change scoped to one
/// client's config belongs in that client's offset instead.
pub(crate) const OPENCODE_SCHEMA_PARSER_BASE_VERSION: u32 = 1;
use crate::{provider_identity, TokenBreakdown};
use serde::{Deserialize, Serialize};
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
    /// Incremental-scan support, one entry per `query_groups` entry and in the
    /// same order. `None` leaves the client on full scans only, which is what
    /// every client whose tables carry no `time_updated` column must do.
    pub incremental_groups: Option<&'static [OpenCodeIncrementalGroup]>,
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
            incremental_groups: None,
        }
    }

    pub(crate) const fn opencode() -> Self {
        Self {
            query_groups: OPENCODE_QUERY_GROUPS,
            dual_schema: true,
            incremental_groups: Some(OPENCODE_INCREMENTAL_GROUPS),
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

/// OpenCode v2: per-message rows in `session_message`, role in the `type`
/// column, model + provider nested under `$.model`. Current databases store
/// session metadata in `session_v2`; older v2 databases use `session`.
/// Databases whose metadata table predates the `title` column fall back to a
/// title-less variant, and the final query preserves usage parsing when the
/// metadata table is absent.
const OPENCODE_V2_QUERIES: &[&str] = &[
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, s.title AS session_title, 1 AS eligible
        FROM session_message sm
        LEFT JOIN session_v2 s ON s.id = sm.session_id
        WHERE sm.type = 'assistant'
          AND json_extract(sm.data, '$.tokens') IS NOT NULL
        ORDER BY sm.id, sm.session_id
    "#,
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title, 1 AS eligible
        FROM session_message sm
        LEFT JOIN session_v2 s ON s.id = sm.session_id
        WHERE sm.type = 'assistant'
          AND json_extract(sm.data, '$.tokens') IS NOT NULL
        ORDER BY sm.id, sm.session_id
    "#,
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, s.title AS session_title, 1 AS eligible
        FROM session_message sm
        LEFT JOIN session s ON s.id = sm.session_id
        WHERE sm.type = 'assistant'
          AND json_extract(sm.data, '$.tokens') IS NOT NULL
        ORDER BY sm.id, sm.session_id
    "#,
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title, 1 AS eligible
        FROM session_message sm
        LEFT JOIN session s ON s.id = sm.session_id
        WHERE sm.type = 'assistant'
          AND json_extract(sm.data, '$.tokens') IS NOT NULL
        ORDER BY sm.id, sm.session_id
    "#,
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULL AS workspace_root, NULL AS session_title, 1 AS eligible
        FROM session_message sm
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
        SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root, s.title AS session_title, 1 AS eligible
        FROM message m
        LEFT JOIN session s ON s.id = m.session_id
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#,
    r#"
        SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title, 1 AS eligible
        FROM message m
        LEFT JOIN session s ON s.id = m.session_id
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#,
    r#"
        SELECT m.id, m.session_id, m.data, NULL AS workspace_root, NULL AS session_title, 1 AS eligible
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

/// Incremental spelling of [`OPENCODE_V2_QUERIES`], one variant per full
/// variant and in the same order.
///
/// These queries intentionally select every changed row, including rows that
/// no longer qualify as assistant usage. Qualification belongs to the parser,
/// not to a second SQL predicate that can drift away from it. A changed row
/// that the parser rejects then has an explicit `Rejected` outcome in the row
/// provenance map, so its previously cached message can be removed exactly.
///
/// `>=` and not `>`: a row written in the same millisecond the mark was taken
/// must not be skipped, and re-reading the boundary rows costs nothing because
/// the merge replaces by row provenance.
const OPENCODE_V2_INCREMENTAL_QUERIES: &[&str] = &[
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, s.title AS session_title, sm.type = 'assistant' AS eligible
        FROM session_message sm
        LEFT JOIN session_v2 s ON s.id = sm.session_id
        WHERE sm.time_updated >= ?1
        ORDER BY sm.id, sm.session_id
    "#,
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title, sm.type = 'assistant' AS eligible
        FROM session_message sm
        LEFT JOIN session_v2 s ON s.id = sm.session_id
        WHERE sm.time_updated >= ?1
        ORDER BY sm.id, sm.session_id
    "#,
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, s.title AS session_title, sm.type = 'assistant' AS eligible
        FROM session_message sm
        LEFT JOIN session s ON s.id = sm.session_id
        WHERE sm.time_updated >= ?1
        ORDER BY sm.id, sm.session_id
    "#,
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title, sm.type = 'assistant' AS eligible
        FROM session_message sm
        LEFT JOIN session s ON s.id = sm.session_id
        WHERE sm.time_updated >= ?1
        ORDER BY sm.id, sm.session_id
    "#,
    r#"
        SELECT sm.id, sm.session_id, sm.data, NULL AS workspace_root, NULL AS session_title, sm.type = 'assistant' AS eligible
        FROM session_message sm
        WHERE sm.time_updated >= ?1
        ORDER BY sm.id, sm.session_id
    "#,
];

/// Incremental spelling of [`OPENCODE_V1_QUERIES`]; see
/// [`OPENCODE_V2_INCREMENTAL_QUERIES`] for why the bound leads and why it is
/// inclusive.
const OPENCODE_V1_INCREMENTAL_QUERIES: &[&str] = &[
    r#"
        SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root, s.title AS session_title, 1 AS eligible
        FROM message m
        LEFT JOIN session s ON s.id = m.session_id
        WHERE m.time_updated >= ?1
        ORDER BY m.id, m.session_id
    "#,
    r#"
        SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title, 1 AS eligible
        FROM message m
        LEFT JOIN session s ON s.id = m.session_id
        WHERE m.time_updated >= ?1
        ORDER BY m.id, m.session_id
    "#,
    r#"
        SELECT m.id, m.session_id, m.data, NULL AS workspace_root, NULL AS session_title, 1 AS eligible
        FROM message m
        WHERE m.time_updated >= ?1
        ORDER BY m.id, m.session_id
    "#,
];

/// Every row id and its own update marker, without touching the JSON payload.
///
/// This inventory is the source of truth for incremental safety. A missing id
/// is a deletion, a new id is an insertion, and a changed marker names exactly
/// the rows the delta query must have returned. The previous implementation
/// inferred those facts from counts and several JSON probes; every new schema
/// variation needed another inference. Keeping the row provenance makes the
/// decisions explicit while preserving the cheap, index-only table walk.
const OPENCODE_V2_ROW_INVENTORY_QUERY: &str =
    "SELECT id, time_updated FROM session_message ORDER BY id";
const OPENCODE_V1_ROW_INVENTORY_QUERY: &str = "SELECT id, time_updated FROM message ORDER BY id";

/// Session metadata that changed since a mark, one query per full variant and
/// in the same order. An empty entry means that variant joins no metadata
/// table, so there is nothing that can go stale.
///
/// Sessions are touched alongside their messages -- on a real 14 GB database
/// `session` and `message` high-waters sit 36 ms apart -- so refusing an
/// incremental scan whenever session metadata moved would refuse essentially
/// every rescan. Re-reading only the changed session rows is cheap instead: 13
/// of 23,556 rows changed over a day on that same database.
const OPENCODE_V2_METADATA_QUERIES: &[&str] = &[
    "SELECT s.id, NULLIF(s.directory, '') , s.title, s.time_updated FROM session_v2 s WHERE s.time_updated >= ?1",
    "SELECT s.id, NULLIF(s.directory, '') , NULL, s.time_updated FROM session_v2 s WHERE s.time_updated >= ?1",
    "SELECT s.id, NULLIF(s.directory, '') , s.title, s.time_updated FROM session s WHERE s.time_updated >= ?1",
    "SELECT s.id, NULLIF(s.directory, '') , NULL, s.time_updated FROM session s WHERE s.time_updated >= ?1",
    "",
];

const OPENCODE_V1_METADATA_QUERIES: &[&str] = &[
    "SELECT s.id, NULLIF(s.directory, '') , s.title, s.time_updated FROM session s WHERE s.time_updated >= ?1",
    "SELECT s.id, NULLIF(s.directory, '') , NULL, s.time_updated FROM session s WHERE s.time_updated >= ?1",
    "",
];

/// Highest `time_updated` in the metadata table a variant joins.
const OPENCODE_V2_METADATA_STATS: &[&str] = &[
    "SELECT MAX(time_updated) FROM session_v2",
    "SELECT MAX(time_updated) FROM session_v2",
    "SELECT MAX(time_updated) FROM session",
    "SELECT MAX(time_updated) FROM session",
    "",
];

const OPENCODE_V1_METADATA_STATS: &[&str] = &[
    "SELECT MAX(time_updated) FROM session",
    "SELECT MAX(time_updated) FROM session",
    "",
];

/// Incremental support for one entry of [`OpenCodeSchemaConfig::query_groups`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenCodeIncrementalGroup {
    /// One incremental query per full variant in the matching group, in the
    /// same order, so a variant index resolved against the full list also
    /// selects the incremental spelling of that same variant.
    queries: &'static [&'static str],
    /// Every row id and update marker in the group's base table.
    inventory: &'static str,
    /// Session metadata changed since the mark, per variant.
    metadata: &'static [&'static str],
    /// Metadata-table high-water, per variant.
    metadata_stats: &'static [&'static str],
}

/// Incremental support for [`OPENCODE_QUERY_GROUPS`], in the same order.
const OPENCODE_INCREMENTAL_GROUPS: &[OpenCodeIncrementalGroup] = &[
    OpenCodeIncrementalGroup {
        queries: OPENCODE_V2_INCREMENTAL_QUERIES,
        inventory: OPENCODE_V2_ROW_INVENTORY_QUERY,
        metadata: OPENCODE_V2_METADATA_QUERIES,
        metadata_stats: OPENCODE_V2_METADATA_STATS,
    },
    OpenCodeIncrementalGroup {
        queries: OPENCODE_V1_INCREMENTAL_QUERIES,
        inventory: OPENCODE_V1_ROW_INVENTORY_QUERY,
        metadata: OPENCODE_V1_METADATA_QUERIES,
        metadata_stats: OPENCODE_V1_METADATA_STATS,
    },
];

/// MiMo Code: `message` table, with the `session` join dropped on databases
/// that predate it.
const MICODE_QUERIES: &[&str] = &[
    r#"
        SELECT m.id, m.session_id, m.data, NULLIF(s.directory, '') AS workspace_root, NULL AS session_title, 1 AS eligible
        FROM message m
        LEFT JOIN session s ON s.id = m.session_id
        WHERE json_extract(m.data, '$.role') = 'assistant'
          AND json_extract(m.data, '$.tokens') IS NOT NULL
        ORDER BY m.id, m.session_id
    "#,
    r#"
        SELECT m.id, m.session_id, m.data, NULL AS workspace_root, NULL AS session_title, 1 AS eligible
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
        SELECT m.id, m.session_id, m.data, NULL AS workspace_root, NULL AS session_title, 1 AS eligible
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

/// Column layout shared by every query variant.
struct OpenCodeSchemaRow {
    row_id: String,
    session_id: String,
    data_json: String,
    workspace_root: Option<String>,
    session_title: Option<String>,
    /// False for a v2 row whose SQL `type` is not `assistant`. V1 carries the
    /// role inside JSON and therefore passes true here and is checked below.
    eligible: bool,
}

#[derive(Default)]
struct SchemaAccumulator {
    messages: Vec<UnifiedMessage>,
    fingerprint_indices: HashMap<OpenCodeSchemaFingerprint, Vec<usize>>,
    dedup_states: Vec<SchemaDedupState>,
    /// Dedup keys of every row that took part in a fingerprint merge, on both
    /// sides of it.
    ///
    /// A merged entry is the only trace two rows left, so a later scan that
    /// re-reads one of them cannot tell what the other contributed and cannot
    /// reproduce the collapse. Only collected for a client with an incremental
    /// lane, which is the only thing that reads it.
    merged_dedup_keys: std::collections::HashSet<String>,
}

impl SchemaAccumulator {
    /// Decode one row's JSON payload and merge it into the accumulator.
    ///
    /// Returns the message slot this physical row backs, or `None` when the
    /// parser rejects it. Keeping that outcome is what lets an incremental
    /// scan remove a previously accepted row without guessing which message it
    /// produced.
    fn ingest(
        &mut self,
        row: OpenCodeSchemaRow,
        cfg: &OpenCodeSchemaConfig,
        db_namespace: &str,
    ) -> Option<usize> {
        let OpenCodeSchemaRow {
            row_id,
            session_id: row_session_id,
            data_json,
            workspace_root: row_workspace_root,
            session_title: row_session_title,
            eligible,
        } = row;

        if !eligible {
            return None;
        }

        let mut bytes = data_json.into_bytes();
        let msg: OpenCodeSchemaMessage = match simd_json::from_slice(&mut bytes) {
            Ok(m) => m,
            Err(_) => return None,
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
            return None;
        }

        let tokens = msg.tokens.as_ref()?;
        let (cache_read, cache_write) = resolve_cache(tokens.cache.as_ref(), cfg.strict_cache)?;

        let resolved_model_id = if cfg.dual_schema {
            msg.resolve_model_id()
        } else {
            msg.model_id.clone()
        };
        let model_id = resolved_model_id?;

        let provider_id = resolve_provider(&msg, &model_id, cfg);

        // A payload with no `$.time` is dropped unless the client supplies a
        // fallback, matching the mandatory `time` field its own type declared.
        let (created_ms, completed_ms) = match msg.time {
            Some(ref time) => (
                normalize_epoch(time.created, cfg),
                time.completed
                    .map(|completed| normalize_epoch(completed, cfg)),
            ),
            None => (cfg.fallback_timestamp? as f64, None),
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
            let index = self.messages.len();
            self.messages.push(unified);
            return Some(index);
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
            let absorbed_dedup_key = if cfg.incremental_groups.is_some() {
                unified.dedup_key.clone()
            } else {
                None
            };
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
            if cfg.incremental_groups.is_some() {
                if let Some(key) = self.messages[index].dedup_key.clone() {
                    self.merged_dedup_keys.insert(key);
                }
                if let Some(key) = absorbed_dedup_key {
                    self.merged_dedup_keys.insert(key);
                }
            }
            return Some(index);
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
        Some(new_index)
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
) -> SqliteScan {
    // Quiet: these queries are schema probes — the caller tries each spelling
    // in turn, so a query the database does not understand is expected.
    sqlite_for_each_row_on(conn, db_path, query, None, &mut |row| {
        let id: String = row.get(0)?;
        let session_id: String = row.get(1)?;
        let data_json: String = row.get(2)?;
        let workspace_root: Option<String> = row.get(3)?;
        let session_title: Option<String> = row.get(4)?;
        let eligible: bool = row.get(5)?;
        on_row(OpenCodeSchemaRow {
            row_id: id,
            session_id,
            data_json,
            workspace_root,
            session_title,
            eligible,
        });
        Ok(())
    })
}

/// Run `query` with `since` bound to `?1`, handing every row to `on_row`.
/// Returns whether the statement prepared, matching [`collect_rows`].
fn collect_rows_since(
    db_path: &Path,
    conn: &rusqlite::Connection,
    query: &str,
    since: i64,
    on_row: &mut dyn FnMut(OpenCodeSchemaRow),
) -> bool {
    let scan =
        sqlite_for_each_row_on_with_params(conn, db_path, query, &[&since], None, &mut |row| {
            let id: String = row.get(0)?;
            let session_id: String = row.get(1)?;
            let data_json: String = row.get(2)?;
            let workspace_root: Option<String> = row.get(3)?;
            let session_title: Option<String> = row.get(4)?;
            let eligible: bool = row.get(5)?;
            on_row(OpenCodeSchemaRow {
                row_id: id,
                session_id,
                data_json,
                workspace_root,
                session_title,
                eligible,
            });
            Ok(())
        });
    // `ran()`, not `prepared()`: the delta is only complete if the statement
    // finished. `prepared()` also accepts a statement that failed while
    // stepping -- json_extract on a malformed payload, say -- and a truncated
    // delta read as a complete one keeps cached messages a cold parse drops.
    scan.completed()
}

/// Highest `time_updated` in a variant's metadata table, or `i64::MIN` when the
/// variant joins none.
fn read_metadata_high_water(
    db_path: &Path,
    conn: &rusqlite::Connection,
    query: &str,
) -> Option<i64> {
    if query.is_empty() {
        return Some(i64::MIN);
    }
    let mut high_water = i64::MIN;
    let scan = sqlite_for_each_row_on(conn, db_path, query, None, &mut |row| {
        high_water = row.get::<_, Option<i64>>(0)?.unwrap_or(i64::MIN);
        Ok(())
    });
    scan.completed().then_some(high_water)
}

/// Re-apply session metadata that changed since `since` to already-cached
/// messages, returning the new metadata high-water.
///
/// A rename or a moved directory advances the session row's `time_updated`
/// without touching any message row, so the incremental message scan cannot
/// see it and the cached messages keep the old title and workspace forever.
///
/// Returns `None` when the refresh cannot be trusted -- the statement did not
/// prepare, or a changed row no longer supplies a directory. The row's
/// directory is only half the answer in that case: [`SchemaAccumulator::ingest`]
/// falls back to the payload's own `path.root`, which is not available here
/// without re-reading the message. A full scan is the honest answer, and it is
/// rare: 13 of 23,556 session rows changed over a day on a real database.
fn refresh_changed_session_metadata(
    db_path: &Path,
    conn: &rusqlite::Connection,
    query: &str,
    stats_query: &str,
    since: i64,
    cached: &mut [UnifiedMessage],
    already_refreshed: &mut std::collections::HashSet<String>,
) -> Option<i64> {
    if query.is_empty() {
        return Some(i64::MIN);
    }

    let mut changed: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    let mut usable = true;
    let scan =
        sqlite_for_each_row_on_with_params(conn, db_path, query, &[&since], None, &mut |row| {
            let session_id: String = row.get(0)?;
            let workspace_root: Option<String> = row.get(1)?;
            let session_title: Option<String> = row.get(2)?;
            if workspace_root.is_none() {
                usable = false;
            }
            changed.insert(session_id, (workspace_root, session_title));
            Ok(())
        });
    if !scan.completed() || !usable {
        return None;
    }

    // A session already re-stamped by another generation's table would be
    // overwritten here with a different generation's metadata.
    if changed.keys().any(|id| already_refreshed.contains(id)) {
        return None;
    }
    already_refreshed.extend(changed.keys().cloned());

    if !changed.is_empty() {
        for message in cached.iter_mut() {
            let Some((workspace_root, session_title)) = changed.get(&message.session_id) else {
                continue;
            };
            set_workspace_from_root(message, workspace_root.as_deref());
            message.session_title = session_title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_string);
        }
    }

    read_metadata_high_water(db_path, conn, stats_query)
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
    scan_opencode_schema_sqlite(db_path, cfg).messages
}

// =============================================================================
// Incremental scan
// =============================================================================

/// What one query group looked like on the scan that filled the cache.
///
/// Deliberately keyed on `time_updated` and not on the row id. OpenCode
/// rewrites a message row long after inserting it -- on a real 14 GB database
/// 434,851 of 434,955 rows carry `time_updated > time_created`, with lags of up
/// to 79 days -- so an id high-water would skip the later rewrite of almost
/// every row and permanently under-report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OpenCodeGroupMark {
    /// Digest of the full query variant the cached rows came from. Any edit to
    /// the SQL changes it, which discards the mark rather than pairing new SQL
    /// with rows the old SQL produced.
    pub query_digest: u64,
    /// Highest `time_updated`. The incremental scan reads from here.
    pub updated_high_water: i64,
    /// Highest `time_updated` in the joined metadata table. A rename moves this
    /// without touching a single message row, so the message high-water cannot
    /// stand in for it. `i64::MIN` when the variant joins no metadata.
    pub metadata_high_water: i64,
    /// Every physical row and the cached message it currently backs.
    ///
    /// Rows rejected by the parser are retained too. That makes a later
    /// accepted/rejected transition explicit and lets the rescan remove a
    /// stale cached message without a SQL predicate that tries to duplicate
    /// parser semantics.
    pub rows: Vec<OpenCodeRowProvenance>,
}

/// How one physical SQLite row contributes to the parsed message list.
///
/// `RowId` avoids storing the overwhelmingly common dedup key twice. On the
/// profiled OpenCode database no row carried an embedded `$.id`, so this keeps
/// the map close to the measured 13 MB of row-id bytes rather than doubling it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum OpenCodeRowMessageKey {
    Rejected,
    RowId,
    Override(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OpenCodeRowProvenance {
    pub row_id: String,
    pub updated_at: i64,
    pub message_key: OpenCodeRowMessageKey,
}

impl OpenCodeRowProvenance {
    fn dedup_key(&self) -> Option<&str> {
        match &self.message_key {
            OpenCodeRowMessageKey::Rejected => None,
            OpenCodeRowMessageKey::RowId => Some(self.row_id.as_str()),
            OpenCodeRowMessageKey::Override(key) => Some(key.as_str()),
        }
    }

    fn set_dedup_key(&mut self, key: Option<&str>) {
        self.message_key = match key {
            None => OpenCodeRowMessageKey::Rejected,
            Some(key) if key == self.row_id => OpenCodeRowMessageKey::RowId,
            Some(key) => OpenCodeRowMessageKey::Override(key.to_string()),
        };
    }
}

/// One mark per query group, in [`OpenCodeSchemaConfig::query_groups`] order.
/// A group whose table this database does not have contributes `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OpenCodeIncrementalState {
    pub groups: Vec<Option<OpenCodeGroupMark>>,
    /// Dedup keys backed by more than one physical row, including fingerprint
    /// merges and repeated embedded ids. A rescan that re-reads only one side
    /// cannot reconstruct which message a full scan would retain, so it falls
    /// back instead.
    pub merged_dedup_keys: Vec<String>,
}

/// Ceiling on the merged-key set a mark will carry.
///
/// Forked history is a small fraction of a real database -- 2,711 of 342,927
/// rows on the 14 GB profile this was measured against -- so a set anywhere
/// near this size means the assumption does not hold for that database, and
/// full scans are the honest answer rather than a mark whose bookkeeping is
/// larger than the delta it saves.
const MAX_MERGED_DEDUP_KEYS: usize = 250_000;

/// Counts the rescans that stayed incremental, so a test can tell an
/// incremental scan apart from a full re-parse that happened to agree with it.
#[cfg(test)]
pub(crate) static INCREMENTAL_RESCANS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// A scan's messages plus the state a later scan needs to resume from it.
pub(crate) struct OpenCodeSchemaScan {
    pub messages: Vec<UnifiedMessage>,
    /// `None` for a client with no incremental support, and for a database
    /// that could not be opened.
    pub incremental: Option<OpenCodeIncrementalState>,
}

impl OpenCodeSchemaScan {
    fn empty() -> Self {
        Self {
            messages: Vec::new(),
            incremental: None,
        }
    }
}

/// FNV-1a over a query variant's text.
///
/// Only ever compared against itself, so the algorithm matters less than that
/// it is stable across runs and covers every byte of the SQL.
fn query_digest(query: &str) -> u64 {
    let mut digest: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in query.as_bytes() {
        digest ^= u64::from(*byte);
        digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
    }
    digest
}

/// Read every physical row id and its own update marker without parsing JSON.
/// `None` means the table is absent or the scan stopped before completion.
fn read_row_inventory(
    db_path: &Path,
    conn: &rusqlite::Connection,
    query: &str,
) -> Option<Vec<OpenCodeRowProvenance>> {
    let mut rows = Vec::new();
    let scan = sqlite_for_each_row_on(conn, db_path, query, None, &mut |row| {
        rows.push(OpenCodeRowProvenance {
            row_id: row.get(0)?,
            updated_at: row.get(1)?,
            message_key: OpenCodeRowMessageKey::Rejected,
        });
        Ok(())
    });
    scan.completed().then_some(rows)
}

/// Full scan, also recording the state an incremental rescan resumes from.
///
/// The inventory is read *before* the messages on purpose. A mark taken after
/// the messages could name a row the scan never saw, and the next incremental
/// scan would then skip that row forever. A row that lands between the two
/// reads is parsed but deliberately prevents a resumable mark.
pub(crate) fn scan_opencode_schema_sqlite(
    db_path: &Path,
    cfg: OpenCodeSchemaConfig,
) -> OpenCodeSchemaScan {
    let Some(conn) = open_readonly_sqlite_opt(db_path) else {
        return OpenCodeSchemaScan::empty();
    };

    let db_namespace = if cfg.namespace_rowid_dedup_key {
        db_path.to_string_lossy().into_owned()
    } else {
        String::new()
    };

    let mut acc = SchemaAccumulator::default();
    let mut marks: Vec<Option<OpenCodeGroupMark>> = Vec::with_capacity(cfg.query_groups.len());
    let mut parsed_rows: Vec<Vec<(String, usize)>> = Vec::with_capacity(cfg.query_groups.len());
    // A group that produced rows but no invariants -- an older database whose
    // table predates the `time_updated` column -- has no way to be rescanned
    // incrementally, and a mark that silently skipped it would serve those rows
    // stale forever. One such group disqualifies the whole database.
    let mut resumable = true;

    for (group_index, group) in cfg.query_groups.iter().enumerate() {
        let incremental = cfg
            .incremental_groups
            .and_then(|groups| groups.get(group_index));
        let inventory = incremental
            .and_then(|incremental| read_row_inventory(db_path, &conn, incremental.inventory));

        // `prepared()` selects the variant -- it is the schema probe, and a
        // variant that prepared is the one this database understands, so
        // falling through to an older query would read the wrong columns.
        // `completed()` is a separate question: a variant that matched but
        // stopped on a step error read only a prefix, and a mark written over
        // that prefix would tell the next rescan those rows were already seen.
        let mut chosen = None;
        let mut read_every_row = false;
        let mut group_parsed_rows = Vec::new();
        for (index, query) in group.iter().enumerate() {
            let scan = collect_rows(db_path, &conn, query, &mut |row| {
                let row_id = row.row_id.clone();
                if let Some(message_index) = acc.ingest(row, &cfg, &db_namespace) {
                    group_parsed_rows.push((row_id, message_index));
                }
            });
            if scan.prepared() {
                chosen = Some(index);
                read_every_row = scan.completed();
                break;
            }
        }
        if chosen.is_some() && !read_every_row {
            resumable = false;
        }
        parsed_rows.push(group_parsed_rows);

        marks.push(match (chosen, inventory) {
            (Some(index), Some(rows)) if read_every_row => {
                let metadata_high_water = cfg
                    .incremental_groups
                    .and_then(|groups| groups.get(group_index))
                    .and_then(|incremental| incremental.metadata_stats.get(index))
                    .and_then(|query| read_metadata_high_water(db_path, &conn, query));
                match metadata_high_water {
                    Some(metadata_high_water) => Some(OpenCodeGroupMark {
                        query_digest: query_digest(group[index]),
                        updated_high_water: rows
                            .iter()
                            .map(|row| row.updated_at)
                            .max()
                            .unwrap_or(i64::MIN),
                        metadata_high_water,
                        rows,
                    }),
                    // Without a metadata high-water a later rescan cannot tell
                    // whether a rename happened, so this scan is not resumable.
                    None => {
                        resumable = false;
                        None
                    }
                }
            }
            (Some(_), None) if incremental.is_some() => {
                resumable = false;
                None
            }
            _ => None,
        });
    }

    // Resolve each physical row to the final message slot after all
    // fingerprint merges and cross-generation promotions have completed.
    for (group_index, group_rows) in parsed_rows.into_iter().enumerate() {
        let Some(Some(mark)) = marks.get_mut(group_index) else {
            continue;
        };
        let row_index: HashMap<String, usize> = mark
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.row_id.clone(), index))
            .collect();
        for (row_id, message_index) in group_rows {
            let Some(&index) = row_index.get(row_id.as_str()) else {
                // The row landed between the inventory and message reads. The
                // messages are correct, but a mark that omitted its source
                // could not update or delete it safely on the next scan.
                resumable = false;
                continue;
            };
            let key = acc.messages[message_index].dedup_key.as_deref();
            mark.rows[index].set_dedup_key(key);
        }
    }

    // Any key backed by several rows has the same reconstruction problem as a
    // fingerprint merge: changing one source cannot reveal what another source
    // contributed. Keep the conservative full-scan fallback for those keys.
    let mut backing_counts: HashMap<String, usize> = HashMap::new();
    for row in marks
        .iter()
        .filter_map(Option::as_ref)
        .flat_map(|mark| mark.rows.iter())
    {
        if let Some(key) = row.dedup_key() {
            *backing_counts.entry(key.to_string()).or_default() += 1;
        }
    }
    for (key, count) in backing_counts {
        if count > 1 {
            acc.merged_dedup_keys.insert(key);
        }
    }

    resumable &= acc.merged_dedup_keys.len() <= MAX_MERGED_DEDUP_KEYS;
    let merged_dedup_keys = acc.merged_dedup_keys.into_iter().collect();

    OpenCodeSchemaScan {
        messages: acc.messages,
        incremental: cfg.incremental_groups.filter(|_| resumable).map(|_| {
            OpenCodeIncrementalState {
                groups: marks,
                merged_dedup_keys,
            }
        }),
    }
}

/// Re-scan only the rows that changed since `cached_state`, merged into
/// `cached_messages`.
///
/// Returns `None` whenever the cached state cannot be trusted, and the caller
/// then runs [`scan_opencode_schema_sqlite`] instead. That covers a database
/// that will not open, a schema variant that no longer matches the one the
/// mark came from, a query variant whose SQL has since been edited, and -- the
/// case that matters for correctness -- a changed row the delta did not return.
/// Deletions and parser rejections are handled directly through row provenance;
/// a non-monotonic update that falls below the SQL high-water is detected and
/// answered with a full scan rather than silently omitted.
pub(crate) fn rescan_opencode_schema_sqlite(
    db_path: &Path,
    cfg: OpenCodeSchemaConfig,
    cached_state: &OpenCodeIncrementalState,
    mut cached_messages: Vec<UnifiedMessage>,
) -> Option<OpenCodeSchemaScan> {
    let incremental_groups = cfg.incremental_groups?;
    if cached_state.groups.len() != cfg.query_groups.len() {
        return None;
    }

    let conn = open_readonly_sqlite_opt(db_path)?;
    let db_namespace = if cfg.namespace_rowid_dedup_key {
        db_path.to_string_lossy().into_owned()
    } else {
        String::new()
    };

    let mut acc = SchemaAccumulator::default();
    let mut marks: Vec<Option<OpenCodeGroupMark>> = Vec::with_capacity(cached_state.groups.len());
    // (mark index, metadata query, metadata stats query, previous high-water)
    let mut pending_metadata: Vec<(usize, &'static str, &'static str, i64)> = Vec::new();
    // Per group: `(row slot in the new inventory, message slot in `acc`)`.
    let mut parsed_row_slots: Vec<Vec<(usize, usize)>> =
        Vec::with_capacity(cached_state.groups.len());
    let previously_merged: std::collections::HashSet<&str> = cached_state
        .merged_dedup_keys
        .iter()
        .map(String::as_str)
        .collect();

    for (group_index, group) in cfg.query_groups.iter().enumerate() {
        let incremental = incremental_groups.get(group_index)?;
        let cached_mark = cached_state.groups[group_index].as_ref();
        let inventory = read_row_inventory(db_path, &conn, incremental.inventory);

        let (mark, mut rows) = match (cached_mark, inventory) {
            (Some(mark), Some(rows)) => (mark, rows),
            // The group's table was absent when the cache was written, and the
            // inventory query still does not run. That is *usually* the same
            // table still missing -- but the inventory also fails on a table that
            // exists without `time_created`/`time_updated`, and the usage
            // queries do not need those columns. Skipping such a table would
            // omit its rows from every warm scan while a cold parse reads them,
            // so the variants are probed before concluding it is absent.
            (None, None) => {
                if group.iter().any(|query| conn.prepare(query).is_ok()) {
                    return None;
                }
                marks.push(None);
                parsed_row_slots.push(Vec::new());
                continue;
            }
            // The table appeared or vanished, or no variant prepared when the
            // cache was written. Either way the cached rows no longer describe
            // this database.
            _ => return None,
        };

        // A full scan takes the first variant that prepares; the cached rows
        // carry that variant's workspace and title columns, so the mark is
        // only reusable while the same variant still wins.
        let chosen = group.iter().position(|query| conn.prepare(query).is_ok())?;
        if query_digest(group[chosen]) != mark.query_digest {
            return None;
        }

        // Both inventories are ordered by id. Diff them in one linear pass so
        // the safety check does not allocate a second 434k-entry hash map (or,
        // worse, turn into an O(n^2) lookup on large databases).
        let mut expected_changed: HashMap<String, usize> = HashMap::new();
        let mut old_index = 0;
        for (row_slot, row) in rows.iter_mut().enumerate() {
            while old_index < mark.rows.len()
                && mark.rows[old_index].row_id.as_str() < row.row_id.as_str()
            {
                let deleted = &mark.rows[old_index];
                if deleted
                    .dedup_key()
                    .is_some_and(|key| previously_merged.contains(key))
                {
                    return None;
                }
                old_index += 1;
            }

            if old_index < mark.rows.len() && mark.rows[old_index].row_id == row.row_id {
                let old = &mark.rows[old_index];
                if old.updated_at == row.updated_at {
                    row.message_key = old.message_key.clone();
                } else {
                    if old
                        .dedup_key()
                        .is_some_and(|key| previously_merged.contains(key))
                    {
                        return None;
                    }
                    expected_changed.insert(row.row_id.clone(), row_slot);
                }
                old_index += 1;
            } else {
                expected_changed.insert(row.row_id.clone(), row_slot);
            }
        }
        for deleted in &mark.rows[old_index..] {
            if deleted
                .dedup_key()
                .is_some_and(|key| previously_merged.contains(key))
            {
                return None;
            }
        }

        let mut seen_changed = std::collections::HashSet::new();
        let mut group_parsed_slots = Vec::new();
        let mut unsafe_delta = false;

        let query = incremental.queries.get(chosen)?;
        if !collect_rows_since(db_path, &conn, query, mark.updated_high_water, &mut |row| {
            // Inclusive boundary rows are intentionally re-read by SQL. Their
            // per-row marker proves they did not change, so they need no merge.
            let Some(&row_slot) = expected_changed.get(&row.row_id) else {
                if rows
                    .binary_search_by(|candidate| candidate.row_id.cmp(&row.row_id))
                    .is_err()
                {
                    // Inserted after the inventory read. This scan cannot
                    // persist a map that omits the source it just observed.
                    unsafe_delta = true;
                }
                return;
            };
            seen_changed.insert(row.row_id.clone());
            if let Some(message_slot) = acc.ingest(row, &cfg, &db_namespace) {
                group_parsed_slots.push((row_slot, message_slot));
            }
        }) {
            return None;
        }
        if unsafe_delta || seen_changed.len() != expected_changed.len() {
            // A new or changed row whose own timestamp fell below the global
            // high-water is real but not incrementally readable. The inventory
            // detects it; a full scan recovers it.
            return None;
        }
        parsed_row_slots.push(group_parsed_slots);

        // Defer metadata refresh until every row group has been validated, so
        // a later unsafe delta cannot leave a partially re-stamped result.
        pending_metadata.push((
            marks.len(),
            incremental.metadata.get(chosen).copied().unwrap_or(""),
            incremental
                .metadata_stats
                .get(chosen)
                .copied()
                .unwrap_or(""),
            mark.metadata_high_water,
        ));

        marks.push(Some(OpenCodeGroupMark {
            query_digest: mark.query_digest,
            updated_high_water: rows
                .iter()
                .map(|row| row.updated_at)
                .max()
                .unwrap_or(i64::MIN),
            metadata_high_water: mark.metadata_high_water,
            rows,
        }));
    }

    // Resolve parsed physical rows after all changed-row fingerprint merges
    // have chosen their final message slots.
    for (group_index, slots) in parsed_row_slots.iter().enumerate() {
        let Some(Some(mark)) = marks.get_mut(group_index) else {
            continue;
        };
        for &(row_slot, message_slot) in slots {
            let key = acc.messages[message_slot].dedup_key.as_deref();
            mark.rows[row_slot].set_dedup_key(key);
            if key.is_some_and(|key| previously_merged.contains(key)) {
                return None;
            }
        }
    }

    // Both generations are scanned into one message list, and a cached message
    // does not record which group produced it. So a session id that exists in
    // more than one generation's metadata table cannot be re-stamped safely:
    // the later group would overwrite the earlier group's messages with its own
    // title and workspace, and a cold parse would disagree. Refusing is rare --
    // it needs the same id in both `session` and `session_v2`, which is the
    // half-migrated database -- and a full scan is correct there.
    let mut refreshed_sessions: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for (index, query, stats_query, since) in pending_metadata {
        // A rename moves session metadata without touching a message row, so
        // the cached messages are re-stamped rather than left stale.
        let metadata_high_water = refresh_changed_session_metadata(
            db_path,
            &conn,
            query,
            stats_query,
            since,
            &mut cached_messages,
            &mut refreshed_sessions,
        )?;
        if let Some(Some(mark)) = marks.get_mut(index) {
            mark.metadata_high_water = metadata_high_water;
        }
    }

    let mut backing_counts: HashMap<String, usize> = HashMap::new();
    let mut active_keys = std::collections::HashSet::new();
    for row in marks
        .iter()
        .filter_map(Option::as_ref)
        .flat_map(|mark| mark.rows.iter())
    {
        if let Some(key) = row.dedup_key() {
            active_keys.insert(key.to_string());
            *backing_counts.entry(key.to_string()).or_default() += 1;
        }
    }

    // A newly introduced multi-row key needs one full scan to establish its
    // canonical merged message and provenance. Existing multi-row keys were
    // already rejected above if any of their sources changed.
    if backing_counts
        .iter()
        .any(|(key, &count)| count > 1 && !previously_merged.contains(key.as_str()))
    {
        return None;
    }

    // Deletions, parser rejections, and A -> B key changes all become the same
    // operation: remove cached messages no physical row backs in the new map.
    cached_messages.retain(|message| {
        message
            .dedup_key
            .as_deref()
            .is_some_and(|key| active_keys.contains(key))
    });

    let merged_dedup_keys: std::collections::HashSet<String> =
        cached_state.merged_dedup_keys.iter().cloned().collect();
    let messages = merge_incremental_messages(cached_messages, acc.messages, &merged_dedup_keys)?;
    #[cfg(test)]
    INCREMENTAL_RESCANS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Some(OpenCodeSchemaScan {
        messages,
        incremental: Some(OpenCodeIncrementalState {
            groups: marks,
            merged_dedup_keys: merged_dedup_keys.into_iter().collect(),
        }),
    })
}

/// A digest of everything [`OpenCodeSchemaFingerprint`] compares, taken from
/// the parsed message instead of the row.
///
/// Only ever used to ask "could a full scan have collapsed these two?", and it
/// is deliberately the coarser of the two: `timestamp` and `duration_ms` are
/// whole milliseconds where the row fingerprint keeps the raw float bits. A
/// coarser digest can only claim a collapse that would not have happened,
/// which costs a full re-parse -- the safe direction. It can never miss one.
fn content_digest(message: &UnifiedMessage) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    message.timestamp.hash(&mut hasher);
    message.duration_ms.hash(&mut hasher);
    message.model_id.hash(&mut hasher);
    message.provider_id.hash(&mut hasher);
    message.tokens.input.hash(&mut hasher);
    message.tokens.output.hash(&mut hasher);
    message.tokens.reasoning.hash(&mut hasher);
    message.tokens.cache_read.hash(&mut hasher);
    message.tokens.cache_write.hash(&mut hasher);
    message.cost.to_bits().hash(&mut hasher);
    message.agent.hash(&mut hasher);
    hasher.finish()
}

/// Fold the rows a rescan re-read back into the cached message list.
///
/// A changed row replaces the cached message carrying its dedup key and a new
/// row is appended, so the cached relative order survives and the result holds
/// one entry per key -- the same set a full scan of the same database state
/// produces.
///
/// The fingerprint dedup is what makes that non-trivial, because it lets one
/// cached entry stand for more than one row (2,711 of 342,927 rows on the
/// database this was measured against). Two rules keep the merge faithful to
/// what a full scan would have produced:
///
/// * A row that already took part in a merge forces a full re-parse. The
///   collapsed entry is the only trace the other row left, so re-reading one
///   side cannot reconstruct what the other contributed.
/// * A row whose content matches a cached message it is not itself replacing
///   forces one too. That is a collapse a full scan would perform and this
///   merge cannot see -- a forked copy arriving after the mark, or a rewrite
///   that happens to land on another row's content.
///
/// `None` also when any message lacks a dedup key. The OpenCode driver always
/// sets one, so that cannot happen; if it ever did, appending an unkeyed
/// message would double count it on the next scan.
fn merge_incremental_messages(
    cached: Vec<UnifiedMessage>,
    changed: Vec<UnifiedMessage>,
    merged_dedup_keys: &std::collections::HashSet<String>,
) -> Option<Vec<UnifiedMessage>> {
    let mut merged = cached;
    let mut index_by_key: HashMap<String, usize> = HashMap::with_capacity(merged.len());
    let mut index_by_digest: HashMap<u64, usize> = HashMap::with_capacity(merged.len());
    for (index, message) in merged.iter().enumerate() {
        // First index wins, matching the cross-database suppression the caller
        // applies to this list: it keeps the first message carrying a key and
        // drops the rest, so refreshing a later one would refresh a message
        // nothing downstream reads.
        index_by_key
            .entry(message.dedup_key.clone()?)
            .or_insert(index);
        index_by_digest
            .entry(content_digest(message))
            .or_insert(index);
    }

    for message in changed {
        let key = message.dedup_key.clone()?;
        if merged_dedup_keys.contains(&key) {
            return None;
        }
        let digest = content_digest(&message);
        match index_by_key.get(&key).copied() {
            Some(index) => {
                if index_by_digest
                    .get(&digest)
                    .is_some_and(|&other| other != index)
                {
                    return None;
                }
                index_by_digest.insert(digest, index);
                merged[index] = message;
            }
            None => {
                if index_by_digest.contains_key(&digest) {
                    return None;
                }
                index_by_digest.insert(digest, merged.len());
                index_by_key.insert(key, merged.len());
                merged.push(message);
            }
        }
    }

    Some(merged)
}
