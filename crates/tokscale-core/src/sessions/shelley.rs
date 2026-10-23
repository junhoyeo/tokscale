//! Shelley session parser
//!
//! [Shelley](https://shelley.dev) is a Go coding agent that
//! keeps every conversation in one SQLite database rather than a transcript
//! tree. It is the first tokscale client whose only source is a live, WAL-mode
//! database owned by another process, so the file is opened read-only and the
//! `-wal` sidecar participates in cache invalidation (see `load_or_parse_sqlite_source`).
//!
//! # Where the database is
//!
//! Shelley takes `-db <path>`, defaulting to `shelley.db` resolved against the
//! **server process's** working directory, so it has no fixed database location
//! of its own. It defines no environment variable of its own and `shelley.json`
//! has no `db` key, so the registered root is the layout service deployments
//! use — `~/.config/shelley/shelley.db`, where the exe.dev image's unit points
//! it — and a database elsewhere is reached by pointing
//! `scanner.extraScanPaths.shelley` (or `TOKSCALE_EXTRA_DIRS`) at the directory
//! that contains it. Tokscale deliberately invents no `SHELLEY_DB`.
//!
//! # Two tables carry everything
//!
//! `conversations` (one row per chat) and `messages` (append-only). Token
//! accounting lives in two JSON columns:
//!
//! - `messages.usage_data` — one object per agent turn, the direct call.
//! - `messages.other_usage_data` — a JSON **array** of indirect calls (slug
//!   generation, compaction, LLM-backed tools), each entry the same shape plus
//!   a `purpose`.
//!
//! Both are read. A row carrying one of them is one tokscale row: an
//! indirect-only row (a `slug` row, whose `usage_data` is NULL) is a row in its
//! own right. A row carrying both is two, for the reason in "One row per call
//! group" below.
//!
//! # Filters that are load-bearing
//!
//! `usage_data` is **not** NULL only on agent turns. Every `user`, `system`,
//! `modelchange` and `error` row on a probed instance carried it, and every one
//! was all-zero. One `type = 'agent'` row was all-zero too — a subagent's second
//! row, with `model_name` NULL and `excluded_from_context = 1`. The all-zero
//! filter therefore has to apply *inside* `type = 'agent'`, not only outside it,
//! or a phantom zero-token turn appears.
//!
//! `forked_from_message_id` marks a message copied in by a conversation fork.
//! The forked copy repeats the original's usage verbatim, so those rows are
//! excluded rather than deduplicated: the original still carries the call.
//!
//! # Per-call model attribution
//!
//! `messages.model_name` and `usage_data.model` agree and both hold the wire id
//! (`accounts/fireworks/models/deepseek-v4p1-flash`). `conversations.model` is a
//! *different* vocabulary (`deepseek-v4.1-flash-fireworks`) and is mutable — one
//! probed conversation read `grok-4.6` early in its life and
//! `deepseek-v4.1-flash-fireworks` later. Attribution therefore comes from the
//! message, never the conversation.
//!
//! # Provider identity
//!
//! The endpoint host carries none: one gateway (`llm.int.exe.xyz`) fronted four
//! model families in a single session, so `llm_api_url` is never used to derive
//! a provider. The provider comes from the model id, and the scoped-vendor
//! segment has to be read **before** family inference. A naive
//! `inferred_provider_from_model("accounts/fireworks/models/<id>")` returns the
//! model *family* (`deepseek`), and `pricing::lookup`'s
//! `provider_hint_matches_scoped_provider` guard then rejects the scoped path
//! outright — silently disabling provider-scoped pricing for every Fireworks
//! reseller id. See [`provider_for_model`].
//!
//! # Schema
//!
//! The columns this parser reads arrive late in Shelley's migration history:
//! `model_name` in 031, `forked_from_message_id` in 032 and `other_usage_data`
//! in 036. A database older than 036 holds every direct call but no indirect
//! usage at all, so the indirect query is *allowed* to fail to prepare there:
//! that is the legacy schema, not a lost read. A database older than 032 cannot
//! answer the direct query either, because the forked-copy filter has no column
//! to read, and counting forked copies would double every call in a forked
//! conversation — so that schema is reported and yields nothing.
//!
//! `scan_shelley_sqlite` also says whether the database was read to the end. A
//! scan that could not run, or stopped partway, is a subset of the store: worth
//! reporting, not worth caching, or every later scan would replay the shortfall
//! until the file happened to change. A database that is not Shelley's, or is
//! older than 036, is complete — there was nothing more to read.
//!
//! # One row per call group
//!
//! A message carrying both a direct call and indirect ones is recorded as two
//! tokscale rows, not one. The halves are separate calls with their own models
//! and their own cost provenance, and a single row can only price them one way:
//! whichever half did not report a cost would be billed at the other half's
//! model rate, and a reported amount would be discarded outright. The direct
//! call keeps the message id, the `is_turn_start` flag and the message count;
//! the indirect calls follow under `<message_id>:indirect` and are
//! count-neutral, the same split `droid` and `copilot_desktop` apply to their
//! own fragments. A message carrying only one half stays a single tokscale row.
//!
//! # Known limitation
//!
//! Within one half, `other_usage_data` may still name different models, and
//! summing the entries leaves a single `model_id` for the total. The collapse is
//! resolved as: the half's own model first (the row's `model_name` for a direct
//! half), then the first entry's. Every row observed on a real instance carried
//! exactly one entry, so the disagreeing case is only exercised by a fixture.

use super::utils::{
    file_modified_timestamp_ms, open_readonly_sqlite_opt, parse_timestamp_value,
    sqlite_for_each_row_on,
};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use tracing::{debug, warn};

/// Registered client id, used as the last-resort provider and as `client`.
const CLIENT_ID: &str = "shelley";

/// Bucket label for a conversation Shelley itself marks as not user-initiated
/// (`user_initiated = 0`): a subagent or tool conversation.
const SUBAGENT_AGENT_NAME: &str = "Shelley Subagent";

/// Dedup-key suffix for the indirect half of a message that carries both a
/// direct call and indirect ones.
const INDIRECT_KEY_SUFFIX: &str = ":indirect";

/// One `usage_data` object, or one entry of an `other_usage_data` array.
///
/// Both spellings serialize from the same Go `llm.Usage` / `PurposedUsage`
/// structs, so one shape covers them; `purpose` is the only field an indirect
/// entry adds and the parser does not need it.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct ShelleyUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
    #[serde(default)]
    cost_usd: f64,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    start_time: Option<String>,
    #[serde(default)]
    end_time: Option<String>,
}

impl ShelleyUsage {
    /// Whether this call moved any tokens or reported any money.
    ///
    /// Shelley writes a fully zeroed `usage_data` on rows that are not calls —
    /// including one `type = 'agent'` row on a probed instance — so this is the
    /// filter that keeps phantom rows out.
    fn is_billable(&self) -> bool {
        self.input_tokens > 0
            || self.output_tokens > 0
            || self.cache_read_input_tokens > 0
            || self.cache_creation_input_tokens > 0
            || self.cost_usd > 0.0
    }

    fn tokens(&self) -> TokenBreakdown {
        TokenBreakdown {
            input: self.input_tokens.max(0),
            output: self.output_tokens.max(0),
            cache_read: self.cache_read_input_tokens.max(0),
            cache_write: self.cache_creation_input_tokens.max(0),
            ..Default::default()
        }
    }
}

/// Conversation-level metadata, joined onto every message by `conversation_id`.
#[derive(Debug, Clone, Default)]
struct ConversationMeta {
    slug: Option<String>,
    parent_conversation_id: Option<String>,
    user_initiated: bool,
    cwd: Option<String>,
}

/// Everything one `messages` row contributes, before the direct and indirect
/// halves are combined.
#[derive(Debug, Clone, Default)]
struct MessageRow {
    conversation_id: String,
    sequence_id: i64,
    /// `messages.created_at`, SQLite's timezone-less `YYYY-MM-DD HH:MM:SS`.
    created_at: Option<String>,
    model_name: Option<String>,
    direct: Option<ShelleyUsage>,
    indirect: Vec<ShelleyUsage>,
}

/// One tokscale row's share of a Shelley message row.
///
/// A message that carries both a direct call and indirect ones is recorded as
/// two tokscale rows; every other message is one.
struct MessageHalf<'a> {
    /// Dedup key: the message id, or the message id plus a role suffix.
    key: String,
    /// The calls this half accounts for, in the order they are summed.
    calls: Vec<&'a ShelleyUsage>,
    /// The half's own model, when it names one.
    model: Option<&'a str>,
    /// True for the half that is the Shelley message itself. It carries the
    /// row's `is_turn_start` flag and the message count; an appended fragment is
    /// count-neutral, so one Shelley message never counts twice.
    primary: bool,
}

/// Resolve the provider for a wire model id.
///
/// The order matters and is not interchangeable:
///
/// 1. A provider-scoped path (`accounts/<vendor>/(models|routers)/<model>`)
///    names its serving vendor in the path, so canonicalize **that**. Doing
///    family inference first would yield `deepseek` for a Fireworks reseller
///    id, whose tags do not intersect the scoped vendor's, and
///    `provider_hint_matches_scoped_provider` would then refuse the
///    provider-scoped lookup entirely.
/// 2. Otherwise a `vendor/model` id canonicalizes its leading segment.
/// 3. Otherwise infer the family from the model id.
/// 4. Failing all of that, the client id, which prices nothing but labels the
///    row honestly.
fn provider_for_model(model: &str) -> String {
    if let Some(vendor) = provider_scoped_vendor(model) {
        if let Some(canonical) = provider_identity::canonical_provider(vendor) {
            return canonical;
        }
    }
    if let Some((vendor, _)) = model.split_once('/') {
        if let Some(canonical) = provider_identity::canonical_provider(vendor) {
            return canonical;
        }
    }
    if let Some(inferred) = provider_identity::inferred_provider_from_model(model) {
        // The inference returns a family name that is usually already a
        // canonical provider id, but not always (`mistral` -> `mistralai`), so
        // it goes through the same canonicalizer.
        if let Some(canonical) = provider_identity::canonical_provider(inferred) {
            return canonical;
        }
        return inferred.to_string();
    }
    CLIENT_ID.to_string()
}

/// The `<vendor>` of an `accounts/<vendor>/(models|routers)/<model>` id.
///
/// Mirrors `pricing::lookup`'s own recogniser so the parser and the resolver
/// cannot disagree about what counts as a scoped path.
fn provider_scoped_vendor(model: &str) -> Option<&str> {
    let rest = model.strip_prefix("accounts/")?;
    let (vendor, rest) = rest.split_once('/')?;
    let (scope, terminal_model_id) = rest.split_once('/')?;
    if vendor.is_empty() || terminal_model_id.is_empty() {
        return None;
    }
    matches!(scope, "models" | "routers").then_some(vendor)
}

/// Non-empty, trimmed `Option<&str>`.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Parse one JSON blob into the shape it is expected to hold.
///
/// `usage_data` is an object and `other_usage_data` an array; a blob of the
/// wrong shape is a corrupt row, not a row with no usage.
fn parse_usage_object(raw: &str) -> Option<ShelleyUsage> {
    serde_json::from_str::<ShelleyUsage>(raw)
        .ok()
        .filter(ShelleyUsage::is_billable)
}

fn parse_usage_array(raw: &str) -> Vec<ShelleyUsage> {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| match value {
            Value::Array(entries) => Some(
                entries
                    .into_iter()
                    .filter_map(|entry| serde_json::from_value::<ShelleyUsage>(entry).ok())
                    .filter(ShelleyUsage::is_billable)
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

/// Read `conversations` into memory, keyed by `conversation_id`.
fn load_conversations(conn: &Connection, db_path: &Path) -> HashMap<String, ConversationMeta> {
    let mut conversations = HashMap::new();
    // Quiet: a SQLite file named `shelley.db` that is not Shelley's has no
    // `conversations` table, which is a probe miss rather than a fault.
    sqlite_for_each_row_on(
        conn,
        db_path,
        "SELECT conversation_id, slug, parent_conversation_id, user_initiated, cwd \
         FROM conversations",
        None,
        &mut |row| {
            let conversation_id: String = row.get(0)?;
            let slug: Option<String> = row.get(1)?;
            let parent_conversation_id: Option<String> = row.get(2)?;
            let user_initiated: Option<i64> = row.get(3)?;
            let cwd: Option<String> = row.get(4)?;
            conversations.insert(
                conversation_id,
                ConversationMeta {
                    slug: non_empty(slug.as_deref()).map(String::from),
                    parent_conversation_id: non_empty(parent_conversation_id.as_deref())
                        .map(String::from),
                    // Absent counts as user-initiated: only an explicit 0 is
                    // Shelley's own "this is a subagent" signal.
                    user_initiated: user_initiated.unwrap_or(1) != 0,
                    cwd: non_empty(cwd.as_deref()).map(String::from),
                },
            );
            Ok(())
        },
    );
    conversations
}

/// Read every `user` row's `sequence_id`, per conversation.
///
/// Only used to decide `is_turn_start`; a conversation with no user rows simply
/// has no turn starts.
fn load_user_sequences(conn: &Connection, db_path: &Path) -> HashMap<String, Vec<i64>> {
    let mut sequences: HashMap<String, Vec<i64>> = HashMap::new();
    sqlite_for_each_row_on(
        conn,
        db_path,
        "SELECT conversation_id, sequence_id FROM messages WHERE type = 'user'",
        None,
        &mut |row| {
            let conversation_id: String = row.get(0)?;
            let sequence_id: i64 = row.get(1)?;
            sequences
                .entry(conversation_id)
                .or_default()
                .push(sequence_id);
            Ok(())
        },
    );
    for ids in sequences.values_mut() {
        ids.sort_unstable();
    }
    sequences
}

/// Read every row that carries usage, direct or indirect, keyed by `message_id`.
///
/// Both halves are read in one pass over one query each, on the single
/// read-only connection the caller opened.
/// The returned flag is false when the database was not read to the end: a
/// query that could not run, or one that stopped partway through its rows.
fn load_message_rows(conn: &Connection, db_path: &Path) -> (HashMap<String, MessageRow>, bool) {
    let mut rows: HashMap<String, MessageRow> = HashMap::new();

    // Direct calls: agent turns only, excluding forked copies. The all-zero
    // filter runs in Rust (see `ShelleyUsage::is_billable`) rather than in SQL
    // so one definition of "billable" covers both halves.
    //
    // `forked_from_message_id` arrived in migration 032, so a prepare failure
    // here is a schema that cannot answer the question at all rather than a
    // table with no rows. Stop instead of counting forked copies twice.
    let direct = sqlite_for_each_row_on(
        conn,
        db_path,
        "SELECT message_id, conversation_id, sequence_id, created_at, model_name, usage_data \
         FROM messages \
         WHERE type = 'agent' AND usage_data IS NOT NULL AND forked_from_message_id IS NULL",
        Some("Shelley agent usage"),
        &mut |row| {
            let message_id: String = row.get(0)?;
            let conversation_id: String = row.get(1)?;
            let sequence_id: i64 = row.get(2)?;
            let created_at: Option<String> = row.get(3)?;
            let model_name: Option<String> = row.get(4)?;
            let usage_data: Option<String> = row.get(5)?;

            let Some(direct) = usage_data.as_deref().and_then(parse_usage_object) else {
                return Ok(());
            };

            rows.insert(
                message_id,
                MessageRow {
                    conversation_id,
                    sequence_id,
                    created_at,
                    model_name: non_empty(model_name.as_deref()).map(String::from),
                    direct: Some(direct),
                    indirect: Vec::new(),
                },
            );
            Ok(())
        },
    );
    if !direct.prepared() {
        warn!(
            db_path = %db_path.display(),
            "Shelley database is older than migration 032; direct usage cannot be read"
        );
        return (rows, false);
    }
    let mut complete = direct.completed();

    // Indirect calls: slug generation, compaction and LLM-backed tools are
    // recorded against the message they were affiliated with. `usage_data` is
    // NULL on those rows, so nothing here is counted twice.
    //
    // `other_usage_data` arrived in migration 036. Before it Shelley recorded
    // no indirect usage anywhere, so a prepare failure is the legacy schema
    // and the direct half is still the whole answer.
    let indirect = sqlite_for_each_row_on(
        conn,
        db_path,
        "SELECT message_id, conversation_id, sequence_id, created_at, model_name, other_usage_data \
         FROM messages \
         WHERE other_usage_data IS NOT NULL AND forked_from_message_id IS NULL",
        Some("Shelley indirect usage"),
        &mut |row| {
            let message_id: String = row.get(0)?;
            let conversation_id: String = row.get(1)?;
            let sequence_id: i64 = row.get(2)?;
            let created_at: Option<String> = row.get(3)?;
            let model_name: Option<String> = row.get(4)?;
            let other_usage_data: Option<String> = row.get(5)?;

            let indirect = other_usage_data
                .as_deref()
                .map(parse_usage_array)
                .unwrap_or_default();
            if indirect.is_empty() {
                return Ok(());
            }

            // A row can in principle carry both halves; keep the direct half
            // and append rather than overwrite it.
            let entry = rows.entry(message_id).or_insert_with(|| MessageRow {
                conversation_id,
                sequence_id,
                created_at,
                model_name: non_empty(model_name.as_deref()).map(String::from),
                direct: None,
                indirect: Vec::new(),
            });
            entry.indirect.extend(indirect);
            Ok(())
        },
    );
    if !indirect.prepared() {
        debug!(
            db_path = %db_path.display(),
            "Shelley database is older than migration 036; indirect usage is not recorded"
        );
    } else {
        complete &= indirect.completed();
    }

    (rows, complete)
}

/// The first billable row after each user turn, per conversation.
///
/// A conversation's agent turns are separated by its `user` rows, so a row is a
/// turn start when it is the lowest-`sequence_id` billable row in its own gap
/// between consecutive user rows. A row that follows no user row at all is not
/// a turn start: the rule is "the first assistant response after a user turn".
fn turn_start_keys(
    rows: &HashMap<String, MessageRow>,
    user_sequences: &HashMap<String, Vec<i64>>,
) -> BTreeSet<String> {
    let empty: Vec<i64> = Vec::new();
    // conversation -> gap index -> (lowest sequence_id, message_id)
    let mut per_gap: HashMap<&str, HashMap<usize, (i64, String)>> = HashMap::new();

    for (message_id, row) in rows {
        let user_ids = user_sequences.get(&row.conversation_id).unwrap_or(&empty);
        // The gap a row falls in is the number of user turns that precede it,
        // so rows between the same two user turns share an index.
        let Some(gap) = user_ids
            .iter()
            .filter(|id| **id < row.sequence_id)
            .count()
            .checked_sub(1)
        else {
            continue;
        };
        let slot = per_gap
            .entry(row.conversation_id.as_str())
            .or_default()
            .entry(gap)
            .or_insert_with(|| (row.sequence_id, message_id.clone()));
        if row.sequence_id < slot.0 {
            *slot = (row.sequence_id, message_id.clone());
        }
    }

    per_gap
        .into_values()
        .flat_map(|gaps| gaps.into_values().map(|(_, message_id)| message_id))
        .collect()
}

/// Parse Shelley's SQLite database into unified messages.
///
/// One tokscale row per Shelley message that carries usage, or two when the
/// message carries both a direct call and indirect ones (see the module's "One
/// row per call group"). Conversations are counted whether or not they are
/// archived or still drafts: archiving is a UI action and the tokens were billed
/// either way.
pub fn parse_shelley_sqlite(db_path: &Path) -> Vec<UnifiedMessage> {
    scan_shelley_sqlite(db_path).messages
}

/// One read of a Shelley database.
#[derive(Debug, Default)]
pub(crate) struct ShelleySqliteScan {
    pub messages: Vec<UnifiedMessage>,
    /// False when the database was not read to the end: it could not be opened,
    /// a query did not prepare on a schema that should have answered it, or
    /// iteration stopped partway. `messages` is then a subset of the store.
    pub complete: bool,
}

/// [`parse_shelley_sqlite`], also saying whether the database was read to the
/// end. The cached lane needs the distinction; see [`ShelleySqliteScan`].
pub(crate) fn scan_shelley_sqlite(db_path: &Path) -> ShelleySqliteScan {
    let Some(conn) = open_readonly_sqlite_opt(db_path) else {
        return ShelleySqliteScan {
            messages: Vec::new(),
            complete: false,
        };
    };

    let conversations = load_conversations(&conn, db_path);
    // A database without a `conversations` table is not Shelley's; stop before
    // reporting an empty parse as if it were. Nothing to read is complete.
    if conversations.is_empty() {
        return ShelleySqliteScan {
            messages: Vec::new(),
            complete: true,
        };
    }

    let (mut rows, complete) = load_message_rows(&conn, db_path);
    if rows.is_empty() {
        return ShelleySqliteScan {
            messages: Vec::new(),
            complete,
        };
    }

    let user_sequences = load_user_sequences(&conn, db_path);
    let turn_starts = turn_start_keys(&rows, &user_sequences);

    // Deterministic output: `HashMap` iteration order is not stable, and two
    // scans of an unchanged database must produce identical vectors.
    let mut keys: Vec<String> = rows.keys().cloned().collect();
    keys.sort_unstable();

    let mut messages = Vec::with_capacity(keys.len());
    for message_id in keys {
        let Some(row) = rows.remove(&message_id) else {
            continue;
        };
        let meta = conversations
            .get(&row.conversation_id)
            .cloned()
            .unwrap_or_default();

        // A message that carries both a direct call and indirect ones is two
        // tokscale rows; every other message is one. See the module's "One row
        // per call group".
        let halves: Vec<MessageHalf<'_>> = match (row.direct.as_ref(), row.indirect.is_empty()) {
            (Some(direct), false) => vec![
                MessageHalf {
                    key: message_id.clone(),
                    calls: vec![direct],
                    model: row
                        .model_name
                        .as_deref()
                        .or_else(|| non_empty(direct.model.as_deref())),
                    primary: true,
                },
                MessageHalf {
                    key: format!("{message_id}{INDIRECT_KEY_SUFFIX}"),
                    calls: row.indirect.iter().collect(),
                    model: row
                        .indirect
                        .iter()
                        .find_map(|entry| non_empty(entry.model.as_deref())),
                    primary: false,
                },
            ],
            _ => vec![MessageHalf {
                key: message_id.clone(),
                calls: row.direct.iter().chain(row.indirect.iter()).collect(),
                model: row
                    .model_name
                    .as_deref()
                    .or_else(|| {
                        row.direct
                            .as_ref()
                            .and_then(|direct| non_empty(direct.model.as_deref()))
                    })
                    .or_else(|| {
                        row.indirect
                            .iter()
                            .find_map(|entry| non_empty(entry.model.as_deref()))
                    }),
                primary: true,
            }],
        };

        for half in halves {
            let mut tokens = TokenBreakdown::default();
            let mut reported_cost = 0.0f64;
            // A half's total is authoritative only when every call in it
            // reported an amount. A partially reported half is priced locally
            // instead, because the engine prices the half's whole token total
            // and adding a reported sub-amount on top would double count.
            let mut all_calls_reported = true;

            for call in &half.calls {
                let call_tokens = call.tokens();
                tokens.input += call_tokens.input;
                tokens.output += call_tokens.output;
                tokens.cache_read += call_tokens.cache_read;
                tokens.cache_write += call_tokens.cache_write;
                reported_cost += call.cost_usd;
                all_calls_reported &= call.cost_usd > 0.0;
            }

            let model = half
                .model
                .map(String::from)
                .unwrap_or_else(|| CLIENT_ID.to_string());

            let timestamp = half
                .calls
                .first()
                .and_then(|call| call.start_time.as_deref())
                .and_then(|raw| parse_timestamp_value(&Value::String(raw.to_string())))
                .or_else(|| {
                    row.created_at
                        .as_deref()
                        .and_then(|raw| parse_timestamp_value(&Value::String(raw.to_string())))
                })
                .unwrap_or_else(|| file_modified_timestamp_ms(db_path));

            let duration_ms = half.calls.first().and_then(|call| call_duration_ms(call));

            let mut message = UnifiedMessage::new_with_dedup(
                CLIENT_ID,
                &model,
                provider_for_model(&model),
                &row.conversation_id,
                timestamp,
                tokens,
                0.0,
                Some(half.key.clone()),
            );

            if all_calls_reported && reported_cost > 0.0 {
                message.cost = reported_cost;
                message.mark_provider_reported_cost();
            }

            message.message_count = if half.primary {
                half.calls.len().max(1) as i32
            } else {
                0
            };
            message.duration_ms = duration_ms;
            message.session_title = meta.slug.clone();
            message.parent_session_id = meta.parent_conversation_id.clone();
            message.is_turn_start = half.primary && turn_starts.contains(&message_id);

            if !meta.user_initiated {
                message.agent = Some(SUBAGENT_AGENT_NAME.to_string());
            }

            if let Some(cwd) = meta.cwd.as_deref() {
                let workspace_key = normalize_workspace_key(cwd);
                let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
                message.set_workspace(workspace_key, workspace_label);
            }

            messages.push(message);
        }
    }

    ShelleySqliteScan { messages, complete }
}

/// Elapsed time of one call, from its own recorded start and end.
fn call_duration_ms(call: &ShelleyUsage) -> Option<i64> {
    let start = call
        .start_time
        .as_deref()
        .and_then(|raw| parse_timestamp_value(&Value::String(raw.to_string())))?;
    let end = call
        .end_time
        .as_deref()
        .and_then(|raw| parse_timestamp_value(&Value::String(raw.to_string())))?;
    (end >= start).then_some(end - start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_scoped_vendor_matches_only_the_scoped_shape() {
        assert_eq!(
            provider_scoped_vendor("accounts/fireworks/models/deepseek-v4-pro"),
            Some("fireworks")
        );
        assert_eq!(
            provider_scoped_vendor("accounts/fireworks/routers/llama-v3p1-8b"),
            Some("fireworks")
        );
        // A non-scoped scope segment is not a scoped path.
        assert_eq!(
            provider_scoped_vendor("accounts/fireworks/whatever/deepseek-v4-pro"),
            None
        );
        // Missing terminal model id.
        assert_eq!(provider_scoped_vendor("accounts/fireworks/models/"), None);
        assert_eq!(provider_scoped_vendor("grok-4.6"), None);
        assert_eq!(provider_scoped_vendor("deepseek/deepseek-v4-pro"), None);
    }

    #[test]
    fn provider_for_model_prefers_the_scoped_vendor_over_family_inference() {
        // The trap: family inference would say `deepseek`, which makes
        // `provider_hint_matches_scoped_provider` refuse the scoped lookup and
        // silently drops the price.
        assert_eq!(
            provider_for_model("accounts/fireworks/models/deepseek-v4p1-flash"),
            "fireworks_ai"
        );
        assert_ne!(
            provider_for_model("accounts/fireworks/models/deepseek-v4p1-flash"),
            provider_identity::inferred_provider_from_model("deepseek-v4p1-flash").unwrap()
        );
    }

    #[test]
    fn provider_for_model_falls_back_through_vendor_then_family_then_client() {
        assert_eq!(provider_for_model("deepseek/deepseek-v4-pro"), "deepseek");
        assert_eq!(provider_for_model("grok-4.6"), "xai");
        // Nothing recognisable: the client id, so the row is still labelled.
        assert_eq!(provider_for_model("totally-unknown-model"), CLIENT_ID);
    }

    #[test]
    fn all_zero_usage_is_not_billable() {
        let zero = ShelleyUsage::default();
        assert!(!zero.is_billable());
        let cost_only = ShelleyUsage {
            cost_usd: 0.000056,
            ..Default::default()
        };
        assert!(cost_only.is_billable());
        let tokens_only = ShelleyUsage {
            input_tokens: 1,
            ..Default::default()
        };
        assert!(tokens_only.is_billable());
    }

    #[test]
    fn parse_usage_object_rejects_the_all_zero_agent_row() {
        let raw = r#"{"input_tokens":0,"cache_creation_input_tokens":0,
            "cache_read_input_tokens":0,"output_tokens":0,"cost_usd":0}"#;
        assert!(parse_usage_object(raw).is_none());
    }

    /// A Shelley-shaped database, with the usage columns a given migration
    /// era provides.
    fn fixture_db(path: &Path, forked_from: bool, other_usage: bool) {
        let mut columns = vec![
            "message_id TEXT PRIMARY KEY",
            "conversation_id TEXT NOT NULL",
            "sequence_id INTEGER NOT NULL",
            "type TEXT NOT NULL",
            "usage_data TEXT",
            "created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP",
            "model_name TEXT",
        ];
        if forked_from {
            columns.push("forked_from_message_id TEXT");
        }
        if other_usage {
            columns.push("other_usage_data TEXT");
        }
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE conversations (conversation_id TEXT PRIMARY KEY, slug TEXT, \
             user_initiated BOOLEAN NOT NULL DEFAULT TRUE, cwd TEXT, \
             parent_conversation_id TEXT);\nCREATE TABLE messages ({});",
            columns.join(", ")
        ))
        .unwrap();
        conn.execute(
            "INSERT INTO conversations (conversation_id, user_initiated) VALUES ('c-1', TRUE)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn scan_reports_whether_the_database_was_read_to_the_end() {
        let dir = tempfile::TempDir::new().unwrap();

        // A database that is not Shelley's has nothing to read, so it is
        // complete rather than a failed read.
        let foreign = dir.path().join("foreign.db");
        Connection::open(&foreign)
            .unwrap()
            .execute_batch("CREATE TABLE unrelated (id INTEGER);")
            .unwrap();
        let scan = scan_shelley_sqlite(&foreign);
        assert!(scan.messages.is_empty());
        assert!(scan.complete, "a non-Shelley database has nothing to read");

        // Migration 036 and later: both halves readable.
        let current = dir.path().join("current.db");
        fixture_db(&current, true, true);
        assert!(scan_shelley_sqlite(&current).complete);

        // Pre-036: indirect usage was never recorded, so the direct half is
        // the whole answer and the read is complete.
        let pre_other = dir.path().join("pre-036.db");
        fixture_db(&pre_other, true, false);
        assert!(scan_shelley_sqlite(&pre_other).complete);

        // Pre-032: the direct query cannot run, so the read is not complete
        // and must not be cached.
        let pre_fork = dir.path().join("pre-032.db");
        fixture_db(&pre_fork, false, false);
        let scan = scan_shelley_sqlite(&pre_fork);
        assert!(scan.messages.is_empty());
        assert!(
            !scan.complete,
            "an unreadable direct query is not a complete read"
        );
    }

    #[test]
    fn parse_usage_array_keeps_only_billable_entries() {
        let raw = r#"[{"purpose":"slug","input_tokens":146,"output_tokens":10,"cost_usd":0.000056},
            {"purpose":"compaction","input_tokens":0,"output_tokens":0,"cost_usd":0}]"#;
        let entries = parse_usage_array(raw);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].input_tokens, 146);
        // An object where an array is expected is a corrupt row, not one entry.
        assert!(parse_usage_array(r#"{"input_tokens":5}"#).is_empty());
        assert!(parse_usage_array("not json").is_empty());
    }
}
