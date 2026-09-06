//! Antigravity CLI session parser
//!
//! The Antigravity CLI (the terminal agent, distinct from the Antigravity IDE)
//! stores each conversation as a SQLite database under
//! `~/.gemini/antigravity-cli/conversations/<uuid>.db`. Unlike the IDE-backed
//! [`super::antigravity`] source — which depends on a *running* language server
//! reachable over RPC and caches JSONL under the config dir — the CLI usage is
//! already on disk and can be read directly. No RPC, no `antigravity sync`.
//!
//! Each `gen_metadata` row is one generation encoded as the same
//! `GeneratorMetadata` protobuf the IDE returns over
//! `GetCascadeTrajectoryGeneratorMetadata`. The repository has no `.proto` /
//! prost decoder (the IDE path receives JSON because the language server does
//! the proto→JSON conversion), so this module ships a tiny wire-format reader
//! and pulls only the few fields it needs. The field numbers below were
//! reverse-engineered from real databases and cross-checked across 6 sessions
//! / 140 turns (`#9 + #10 == #3`, i.e. output + thinking == total output;
//! `#5`/cacheRead only appears once a cached prefix exists and grows with the
//! conversation):
//!
//! - `gen_metadata.#1`            → chatModel message
//!   - `#19` (string, optional)  → responseModel (e.g. `gemini-3-flash-a`)
//!   - `#21` (string, optional)  → model display label (`Gemini 3.6 Flash (High)`)
//!   - `#9` (message)            → per-generation wall-clock time. Two layouts
//!     exist depending on the agy version, both handled by
//!     [`generation_timestamp_ms`]:
//!     - agy ≤ 1.1.17: `#9.#4` = `{#1: seconds, #2: nanos}` Timestamp.
//!     - agy 1.1.18: `#4` is gone. `#9` instead carries `#2` = `u64::MAX` (an
//!       `int64` -1 "unset" sentinel, never a time) and a new `#10` holding 8
//!       length-delimited bytes. Unlike every other field number here, `#10`'s
//!       encoding was *not* read off a real database — it is inferred from a
//!       field dump in issue #1184 — so each candidate reading is range-checked
//!       *and* pinned to the lifetime of the session that contains it before it
//!       is accepted, and is declined outright when that lifetime is unknown —
//!       i.e. `trajectory_metadata_blob` held no decodable created-at. An
//!       absolute "is this a believable date" window is not a meaningful test
//!       for eight opaque bytes: read as a nanosecond count it alone covers
//!       roughly 2% of the `u64` range, so trying both byte orders leaves an
//!       arbitrary payload (an id, a hash, a duration) a few percent
//!       chance of passing as a date. Constraining it to the session's own span
//!       cuts that by orders of magnitude, because a turn cannot happen before
//!       the conversation it belongs to nor after the moment we read the file.
//!   - `#4`                      → usage message
//!     - `#1` (varint, const)    → fixed system-prompt tokens (≈1132)
//!     - `#2` (varint)           → newly-processed (non-cached) input tokens
//!     - `#5` (varint)           → cacheRead tokens
//!     - `#9` (varint)           → output (text) tokens
//!     - `#10` (varint)          → thinking / reasoning tokens
//!     - `#11` (string)          → responseId (dedup key)
//! - `trajectory_metadata_blob.#2` = `{#1: seconds, #2: nanos}` → created-at
//! - `trajectory_metadata_blob.#1.#1` (string)                  → workspace URI
//!
//! `#19` is optional in practice: some continuation turns omit it while still
//! writing `#21`. [`SessionModels`] recovers the machine id for those rows from
//! the rest of the same conversation. `#21` was present on every row observed so
//! far, including the ones missing `#19`, but nothing here requires it — a row
//! carrying neither field is handled too. `#21` serves only as a join key
//! between rows of one file and is never used as a pricing key: it is a
//! server-supplied name that gets renamed (`Gemini 3 Flash` → `Gemini 3.5 Flash
//! (High)`) and could be localized.

use super::utils::{open_readonly_sqlite_opt, sqlite_for_each_row_on};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{pricing, provider_identity, TokenBreakdown};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::ops::RangeInclusive;
use std::path::Path;

pub fn parse_antigravity_cli_file(path: &Path) -> Vec<UnifiedMessage> {
    let Some(conn) = open_readonly_sqlite_opt(path) else {
        return Vec::new();
    };

    let session_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("unknown")
        .to_string();

    let meta = read_trajectory_meta(&conn, path);

    // Buffered rather than streamed so a row missing its own `#19` can borrow
    // attribution from anywhere in the conversation, not just from rows that
    // happen to precede it.
    //
    // Quiet: a database without `gen_metadata` is not an Antigravity CLI
    // database at all, so there is nothing to warn about.
    let mut blobs: Vec<Vec<u8>> = Vec::new();
    sqlite_for_each_row_on(
        &conn,
        path,
        "SELECT data FROM gen_metadata ORDER BY idx",
        None,
        &mut |row| {
            blobs.push(row.get::<_, Vec<u8>>(0)?);
            Ok(())
        },
    );
    let session_models = SessionModels::from_blobs(&blobs);

    let mut messages = Vec::new();
    let mut seen_response_ids: HashSet<String> = HashSet::new();
    for blob in &blobs {
        // `fallback_ms` is the per-row timestamp fallback; each row prefers its
        // own per-generation wall-clock stamp (see `parse_gen_metadata`).
        // `created_ms` travels separately because only a genuinely decoded
        // session created-at may anchor the inferred 1.1.18 reading.
        if let Some(mut message) = parse_gen_metadata(
            blob,
            &session_id,
            meta.fallback_ms,
            meta.created_ms,
            &session_models,
            &mut seen_response_ids,
        ) {
            if meta.workspace_key.is_some() {
                message.set_workspace(meta.workspace_key.clone(), meta.workspace_label.clone());
            }
            messages.push(message);
        }
    }

    messages
}

/// Model attribution recovered from the conversation as a whole, for rows whose
/// `chatModel.#19` (responseModel) is missing.
///
/// Antigravity CLI leaves `#19` out of some generations — observed on
/// continuation / tool turns, which also carry a large `cacheRead` and a tiny
/// output — while still writing the `#21` display label. Those rows are not
/// information-poor: sibling rows in the same database carry the machine id for
/// the very same label, so the id is recoverable from the file itself. Without
/// this the rows resolve to `antigravity/unknown`, which has no price, and a
/// single such row aborts the whole submission.
///
/// The label is only ever a join key between rows of one file; the value handed
/// back is always a `#19` machine id observed in that same file. Display labels
/// are never used as pricing keys — see the alias table's note on renamed and
/// localizable labels.
#[derive(Default)]
struct SessionModels {
    /// `#21` display label → the `#19` machine id seen alongside it. A label
    /// that appears with two ids naming *different* priced models is dropped:
    /// an ambiguous label is no better evidence than no label at all. Ids that
    /// differ only in spelling but share an alias target (Antigravity's
    /// `gemini-pro-default` / `gemini-pro-agent`) are not ambiguous, and the
    /// first id observed is kept.
    by_display: HashMap<String, String>,
    /// The file's only `#19` value — set only when every row carrying one
    /// agrees *and* every label in the file was identified by some row. Serves
    /// rows that have neither `#19` nor `#21`.
    sole_model: Option<String>,
}

impl SessionModels {
    fn from_blobs(blobs: &[Vec<u8>]) -> Self {
        let mut by_display: HashMap<&str, Option<&str>> = HashMap::new();
        let mut distinct: HashSet<&str> = HashSet::new();
        let mut unresolved_labels: Vec<&str> = Vec::new();

        for blob in blobs {
            let Some(chat_model) = message_field(blob, 1) else {
                continue;
            };
            let label = non_empty_string_field(chat_model, 21);
            let Some(model) = non_empty_string_field(chat_model, 19) else {
                // Kept so `sole_model` below can tell whether this row was
                // identified by some other row carrying the same label.
                unresolved_labels.extend(label);
                continue;
            };
            if is_antigravity_routing_label(model) {
                // A routing label names the router that served the request, never
                // the model that answered it, so it is no evidence of a concrete
                // model for this display label. Treat it like a row with no #19:
                // it must not occupy the by_display slot, which would trip the
                // ambiguity check against a real sibling id and drop the mapping.
                unresolved_labels.extend(label);
                continue;
            }
            distinct.insert(model);
            if let Some(label) = label {
                by_display
                    .entry(label)
                    .and_modify(|resolved| {
                        if let Some(existing) = *resolved {
                            // Antigravity swaps between several `#19` machine ids
                            // under one display label within a single conversation
                            // (`gemini-pro-default` and `gemini-pro-agent` both
                            // appear as "Gemini 3.1 Pro (High)"). Those are the same
                            // priced model, so comparing the raw strings reports a
                            // false ambiguity and drops the label — which later
                            // leaves `#19`-less rows as `unknown` and aborts
                            // `submit`. Compare canonical alias targets instead, so
                            // only a genuinely different model clears the mapping.
                            if existing != model {
                                let existing_canon =
                                    pricing::aliases::resolve_alias(existing).unwrap_or(existing);
                                let new_canon =
                                    pricing::aliases::resolve_alias(model).unwrap_or(model);
                                if existing_canon != new_canon {
                                    *resolved = None;
                                }
                            }
                        }
                    })
                    .or_insert(Some(model));
            }
        }

        let by_display: HashMap<String, String> = by_display
            .into_iter()
            .filter_map(|(label, model)| Some((label.to_string(), model?.to_string())))
            .collect();

        // A label that no row ever identified is proof the conversation ran a
        // model this file never names — one *identified* model is not one
        // model. Counting only the ids would let an unlabelled row inherit the
        // single named id and bill a model switch under the wrong model, so
        // withhold the fallback entirely and let those rows stay `unknown`.
        let every_label_identified = unresolved_labels
            .iter()
            .all(|label| by_display.contains_key(*label));
        let sole_model = match (distinct.len(), every_label_identified) {
            (1, true) => distinct.iter().next().map(|model| (*model).to_string()),
            _ => None,
        };

        Self {
            by_display,
            sole_model,
        }
    }

    /// Best available `#19` for a row that has none of its own.
    fn recover(&self, chat_model: &[u8]) -> Option<&str> {
        match non_empty_string_field(chat_model, 21) {
            // A label joined elsewhere in this file resolves to its machine id.
            // A label that never appeared next to a `#19` is positive evidence
            // of a model this file never identified, so falling through to
            // another row's model would be a guess against the evidence —
            // `unknown` is the honest answer there.
            Some(label) => self.by_display.get(label).map(String::as_str),
            None => self.sole_model.as_deref(),
        }
    }
}

/// Antigravity CLI's generic routing label. It names which router served the
/// request, never which model did, so a row carrying it carries no model
/// identity of its own. Kept in sync with `is_generic_routing_label` in lib.rs,
/// which excludes the label from submission when no concrete model is recovered
/// here.
fn is_antigravity_routing_label(model: &str) -> bool {
    model.trim().eq_ignore_ascii_case("gemini-default")
}

/// Map an Antigravity `#21` display label to a concrete model id.
///
/// Display labels are server-supplied names (the app's `_getModelLabel` switch
/// emits them from `MODEL_PLACEHOLDER_*` ids) and could be renamed or localized,
/// so only labels verified against real user databases are mapped here; anything
/// else returns `None` and the routing label is preserved for the submission-time
/// exclusion rather than guessed at.
fn display_label_to_model_id(label: &str) -> Option<&'static str> {
    match label.trim() {
        "Gemini 3.5 Flash (Low)" => Some("gemini-3.5-flash-extra-low"),
        "Gemini 3.5 Flash (Medium)" => Some("gemini-3.5-flash-medium"),
        "Gemini 3.5 Flash (High)" => Some("gemini-3.5-flash-high"),
        _ => None,
    }
}

fn parse_gen_metadata(
    blob: &[u8],
    session_id: &str,
    session_timestamp: i64,
    session_anchor: Option<i64>,
    session_models: &SessionModels,
    seen_response_ids: &mut HashSet<String>,
) -> Option<UnifiedMessage> {
    let chat_model = message_field(blob, 1)?;
    let usage = message_field(chat_model, 4)?;

    // Per-generation wall-clock time for this turn, so each turn is dated when
    // it actually happened rather than at conversation start. Falls back to
    // `session_timestamp` when `chatModel.#9` is absent or no candidate field
    // in it decodes to a believable time (older databases, malformed rows, or a
    // `#9` layout this module does not recognise).
    //
    // `session_anchor` is the created-at actually decoded from
    // `trajectory_metadata_blob`, and it alone bounds the inferred 1.1.18
    // reading, so a turn can only be re-dated to somewhere inside its own
    // session's lifetime. It is deliberately not `session_timestamp`: that one
    // degrades to the file mtime, which dates the last write to the database
    // rather than the start of the conversation and so vouches for nothing.
    let timestamp = message_field(chat_model, 9)
        .and_then(|gen| generation_timestamp_ms(gen, session_anchor))
        .unwrap_or(session_timestamp);

    // input = fixed system prompt (#1) + newly-processed input (#2). The
    // constant #1 is, to the best of our reverse-engineering, the agent's fixed
    // system prompt and counts as billable input; if an official schema later
    // contradicts this, only the input total needs revisiting.
    // Clamp untrusted u64 varints into i64 (a corrupt/malicious blob could
    // encode a value > i64::MAX, which `as i64` would wrap to a negative count)
    // and combine with saturating_add so totals never overflow.
    let to_i64 = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    let input = to_i64(varint_field(usage, 1).unwrap_or(0))
        .saturating_add(to_i64(varint_field(usage, 2).unwrap_or(0)));
    let cache_read = to_i64(varint_field(usage, 5).unwrap_or(0));
    let output = to_i64(varint_field(usage, 9).unwrap_or(0));
    let reasoning = to_i64(varint_field(usage, 10).unwrap_or(0));
    if input == 0 && output == 0 && cache_read == 0 && reasoning == 0 {
        return None;
    }

    let dedup_key = string_field(usage, 11)
        .filter(|text| !text.trim().is_empty())
        .map(|text| text.to_string());
    if let Some(key) = &dedup_key {
        if !seen_response_ids.insert(key.clone()) {
            return None;
        }
    }

    let response_model = non_empty_string_field(chat_model, 19);
    let model_raw = response_model
        .filter(|m| !is_antigravity_routing_label(m))
        .or_else(|| session_models.recover(chat_model))
        .or_else(|| {
            // Routing labels carry no model identity of their own, but the
            // sibling `#21` display label names the tier. Recover the concrete
            // id from it; if the label is unknown, keep the routing label so
            // the submission-time exclusion still applies.
            non_empty_string_field(chat_model, 21).and_then(display_label_to_model_id)
        })
        .or(response_model)
        .unwrap_or("unknown");
    let model_id = pricing::aliases::resolve_alias(model_raw)
        .unwrap_or(model_raw)
        .to_string();
    let provider_id = provider_identity::inferred_provider_from_model(&model_id)
        .unwrap_or("antigravity")
        .to_string();

    Some(UnifiedMessage::new_with_dedup(
        "antigravity-cli",
        model_id,
        provider_id,
        session_id,
        timestamp,
        TokenBreakdown {
            input,
            output,
            cache_read,
            cache_write: 0,
            reasoning,
        },
        0.0,
        dedup_key,
    ))
}

/// Session-level facts read from the single `trajectory_metadata_blob` row.
///
/// `created_ms` and `fallback_ms` are deliberately kept apart. The decoded
/// created-at is the only value that dates the *start of the conversation*, and
/// so the only one allowed to anchor the inferred agy 1.1.18 reading;
/// `fallback_ms` merely has to produce some timestamp for a row that carries
/// none, and degrades to the file mtime to do it. Collapsing the two into one
/// number hands that mtime to the anchor as though it were a session start,
/// which it is not — it marks the last write to the file, is always positive,
/// and would therefore pass any "do we have an anchor?" test while corroborating
/// nothing.
struct TrajectoryMeta {
    /// `trajectory_metadata_blob.#2` as epoch ms, or `None` when the table/row
    /// is absent or the blob does not decode to a positive created-at.
    created_ms: Option<i64>,
    /// Per-row timestamp fallback: the created-at when it decoded, else the
    /// file mtime.
    fallback_ms: i64,
    workspace_key: Option<String>,
    workspace_label: Option<String>,
}

/// Read the session-level created-at timestamp and workspace from the single
/// `trajectory_metadata_blob` row. The created-at dates the conversation as a
/// whole; the per-row fallback for any `gen_metadata` row missing its own
/// per-generation `#9` wall-clock stamp drops to the file mtime when the blob
/// is absent or undecodable. See [`TrajectoryMeta`] for why the two are
/// reported separately.
fn read_trajectory_meta(conn: &Connection, path: &Path) -> TrajectoryMeta {
    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT data FROM trajectory_metadata_blob LIMIT 1",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .ok();

    let mut created_ms = None;
    let mut workspace_key = None;
    let mut workspace_label = None;

    if let Some(blob) = &blob {
        created_ms = session_created_ms(blob).filter(|&ms| ms > 0);

        if let Some(uri) = message_field(blob, 1).and_then(|folder| string_field(folder, 1)) {
            if let Some(path_str) = file_uri_to_path(uri) {
                workspace_key = normalize_workspace_key(&path_str);
                workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
            }
        }
    }

    TrajectoryMeta {
        created_ms,
        fallback_ms: created_ms.unwrap_or_else(|| file_modified_ms(path)),
        workspace_key,
        workspace_label,
    }
}

fn session_created_ms(blob: &[u8]) -> Option<i64> {
    proto_timestamp_ms(message_field(blob, 2)?)
}

/// Per-generation wall-clock time from the `chatModel.#9` sub-message.
///
/// agy ≤ 1.1.17 writes an explicit `{#1: seconds, #2: nanos}` Timestamp at
/// `#9.#4`. agy 1.1.18 dropped that field: a decode of a live 1.1.18
/// `gen_metadata` row (issue #1184) shows `#9` carrying `#2` = `u64::MAX` — an
/// `int64` -1, i.e. an "unset" sentinel and never a time — plus a new `#10`
/// holding 8 length-delimited bytes.
///
/// No agy 1.1.18 database was available to decode `#10` against, so its layout
/// is inferred from the byte count rather than observed. Three shapes are
/// plausible for 8 length-delimited bytes and all three are attempted,
/// most-structured first (see [`inferred_epoch_ms`]).
///
/// Every inferred reading is unit-detected, range-checked, *and* required to
/// land inside the containing session's own lifetime — `session_anchor` is the
/// created-at decoded from `trajectory_metadata_blob`, and nothing else. The
/// extra constraint is what makes the inference safe to act on: a believability
/// window spanning 2020 to five years out accepts a few percent of arbitrary
/// eight-byte payloads once both byte orders and all four time units are tried,
/// which is far too loose for bytes whose meaning is unconfirmed. A turn,
/// however, cannot predate the conversation that contains it and cannot happen
/// after we read the file, and that pair of bounds is usually hours or days
/// wide rather than a decade.
///
/// The direction of the trade matters: discarding a real stamp costs only the
/// conservative known-wrong behaviour of dating the turn at session start,
/// whereas accepting a wrong one silently corrupts day buckets and the
/// server-side monotonic ratchet, which has no correction path. When there is
/// no trustworthy anchor — `session_anchor` is `None` because the blob was
/// missing or would not decode, or the value it did decode to is not positive —
/// no inferred reading is accepted at all, and the row keeps whatever dating it
/// had before the 1.1.18 layout was handled, mtime fallback included.
fn generation_timestamp_ms(gen: &[u8], session_anchor: Option<i64>) -> Option<i64> {
    // agy <= 1.1.17. Tried first and kept on its original `ms > 0` filter, with
    // no session bound: existing databases and older installs still write it,
    // and it is an explicitly typed Timestamp read off real databases rather
    // than an inferred one, so it needs no corroboration to be trusted.
    if let Some(ms) = message_field(gen, 4)
        .and_then(proto_timestamp_ms)
        .filter(|&ms| ms > 0)
    {
        return Some(ms);
    }
    // agy 1.1.18. `#9.#2` is deliberately never consulted: the only value ever
    // observed there is the unset sentinel, and `epoch_scalar_to_ms` rejects
    // that value outright should it reach any other candidate path.
    message_field(gen, 10).and_then(|payload| inferred_epoch_ms(payload, session_anchor))
}

/// Decode the agy 1.1.18 `chatModel.#9.#10` payload as an epoch time.
///
/// Candidates, in order:
///
/// 1. a nested `{#1: seconds, #2: nanos}` Timestamp — the shape `#4` used, and
///    the one a schema change would most likely re-home;
/// 2. a nested message holding the epoch scalar in field 1, as a varint or as a
///    `fixed64`;
/// 3. the payload itself as 8 raw `fixed64`-style bytes, evaluating both
///    little-endian (protobuf's own byte order) and big-endian. When the
///    little-endian reading decodes as native milliseconds, a competing
///    big-endian reading that only clears the window as an artifact of byte
///    reversal in the nanoseconds branch is discarded in favor of the canonical
///    LE milliseconds reading. Any other conflicting in-window endianness
///    readings are rejected as ambiguous, falling back to the session stamp.
///
/// A raw IEEE-754 `f64` reading of the same 8 bytes is deliberately *not*
/// attempted. It is the one candidate whose false-positive rate against a
/// non-timestamp payload is non-trivial (any double in the 2^30-ish exponent
/// range decodes to a plausible epoch-second count), and nothing in the field
/// dump points at it.
///
/// The order is deliberate and must be preserved: eight arbitrary bytes are far
/// likelier to look like an integer than to parse as a well-formed nested
/// message, so the structurally validated readings are tried before the raw
/// ones and win whenever both would match.
///
/// Every candidate must clear both gates — the absolute
/// [`plausible_epoch_ms`] check *and* the session window from
/// [`session_window_ms`] — and each is judged independently, so a reading that
/// is a believable date but not a believable date *for this session* is
/// discarded rather than allowed to mask a later candidate. A payload with no
/// trustworthy session anchor is declined outright.
fn inferred_epoch_ms(payload: &[u8], session_anchor: Option<i64>) -> Option<i64> {
    // Sampled once so every candidate for this payload is judged against the
    // same window, and so a missing anchor short-circuits before any decode.
    let window = session_window_ms(session_anchor?)?;
    let accepted = |ms: i64| window.contains(&ms);

    if let Some(ms) =
        proto_timestamp_ms(payload).filter(|&ms| plausible_epoch_ms(ms) && accepted(ms))
    {
        return Some(ms);
    }
    if let Some(ms) = varint_field(payload, 1)
        .and_then(epoch_scalar_to_ms)
        .filter(|&ms| accepted(ms))
    {
        return Some(ms);
    }
    if let Some(ms) = fixed64_field(payload, 1)
        .and_then(epoch_scalar_to_ms)
        .filter(|&ms| accepted(ms))
    {
        return Some(ms);
    }
    let raw: [u8; 8] = payload.try_into().ok()?;
    let le = epoch_scalar_with_unit(u64::from_le_bytes(raw)).filter(|&(_, ms)| accepted(ms));
    let be = epoch_scalar_with_unit(u64::from_be_bytes(raw)).filter(|&(_, ms)| accepted(ms));
    match (le, be) {
        (Some((_, le_ms)), Some((_, be_ms))) if le_ms == be_ms => Some(le_ms),
        (Some((EpochUnit::Millis, le_ms)), Some((EpochUnit::Nanos, _))) => Some(le_ms),
        (Some(_), Some(_)) => None,
        (Some((_, le_ms)), None) => Some(le_ms),
        (None, Some((_, be_ms))) => Some(be_ms),
        (None, None) => None,
    }
}

/// agy's "unset" marker for the `#9.#2` int64: -1, which reaches this wire
/// reader as `u64::MAX`. It is a sentinel, never a time, so it is rejected
/// before any unit detection can promote it into a date.
const UNSET_TIME_SENTINEL: u64 = u64::MAX;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum EpochUnit {
    Seconds,
    Millis,
    Micros,
    Nanos,
}

fn epoch_scalar_with_unit(value: u64) -> Option<(EpochUnit, i64)> {
    if value == UNSET_TIME_SENTINEL {
        return None;
    }
    let value = i64::try_from(value).ok()?;
    [
        (EpochUnit::Seconds, value.checked_mul(1_000)),
        (EpochUnit::Millis, Some(value)),
        (EpochUnit::Micros, Some(value / 1_000)),
        (EpochUnit::Nanos, Some(value / 1_000_000)),
    ]
    .into_iter()
    .filter_map(|(unit, ms)| ms.map(|ms| (unit, ms)))
    .find(|&(_, ms)| plausible_epoch_ms(ms))
}

/// Interpret a bare integer as an epoch time, detecting its unit by magnitude.
///
/// Over the plausible window the four unit ranges are disjoint — 1.7e9 is a
/// believable second count but an absurd millisecond count, 1.7e12 the reverse,
/// and so on — so at most one unit can produce an in-window result and the
/// magnitude names the unit unambiguously. Returns `None` when no unit does,
/// which is what makes an unrelated 8-byte field (an id, a hash) fall through
/// to the session-created stamp instead of becoming a wrong date.
fn epoch_scalar_to_ms(value: u64) -> Option<i64> {
    epoch_scalar_with_unit(value).map(|(_, ms)| ms)
}

/// Whether an epoch-ms value is believable as an Antigravity CLI generation
/// time: no earlier than 2020-01-01 (the CLI did not exist) and no further than
/// five years ahead of now (that is clock skew or a misread field, not a turn).
fn plausible_epoch_ms(ms: i64) -> bool {
    /// 2020-01-01T00:00:00Z in epoch ms.
    const MIN_MS: i64 = 1_577_836_800_000;
    const FIVE_YEARS_MS: i64 = 5 * 365 * 24 * 60 * 60 * 1_000;

    let max_ms = chrono::Utc::now()
        .timestamp_millis()
        .saturating_add(FIVE_YEARS_MS);
    (MIN_MS..=max_ms).contains(&ms)
}

/// How far *before* the session-created stamp an inferred generation time may
/// still be accepted.
///
/// The session stamp and the generation stamp are written by the same process
/// on the same machine, so an honest gap in this direction is sub-second; an
/// hour absorbs a clock adjustment, or a session record flushed a beat after
/// the first turn was already in flight. It is kept deliberately tight because
/// the cost is asymmetric: rejecting a real stamp only restores the session-
/// start dating this module already falls back to, while accepting a wrong one
/// is uncorrectable downstream. Widening this to days would start handing back
/// the integer space the session window exists to take away.
const SESSION_START_TOLERANCE_MS: i64 = 60 * 60 * 1_000;

/// How far *after* the present moment an inferred generation time may still be
/// accepted.
///
/// A turn that has already been written to disk cannot be in the future, so the
/// only legitimate overshoot is clock skew — and since `now` is read from the
/// same clock that wrote the file, that too is normally zero. An hour covers a
/// database copied from a machine whose clock ran ahead, and nothing more:
/// anything further out is a misread field, not a turn.
const FUTURE_TOLERANCE_MS: i64 = 60 * 60 * 1_000;

/// The epoch-ms window an *inferred* generation time has to land in to be
/// believable for this particular session: no earlier than the session began
/// and no later than the moment the file is being read, each with a small
/// tolerance.
///
/// The only value that ever reaches here is a created-at decoded from
/// `trajectory_metadata_blob`. A database whose blob is missing or undecodable
/// is turned away by the caller instead, because the row fallback it would
/// otherwise offer is the file mtime — the *last* write to the file, not the
/// start of the conversation. Building a window on that would both admit an
/// opaque payload that happens to decode near the mtime and reject a genuine
/// older turn for sitting below it; those rows keep the dating they had before
/// 1.1.18 support existed, which is the correct outcome.
///
/// Returns `None` when `session_created` is not positive — a decoded stamp of
/// zero is no more of an anchor than a missing one. If the anchor is itself in
/// the future the range comes out empty, which rejects every candidate for the
/// same reason.
fn session_window_ms(session_created: i64) -> Option<RangeInclusive<i64>> {
    if session_created <= 0 {
        return None;
    }
    let earliest = session_created.saturating_sub(SESSION_START_TOLERANCE_MS);
    let latest = chrono::Utc::now()
        .timestamp_millis()
        .saturating_add(FUTURE_TOLERANCE_MS);
    Some(earliest..=latest)
}

/// Decode a protobuf `{#1: seconds, #2: nanos}` Timestamp message to epoch ms.
/// Shared by the session-created stamp, the per-generation `#9.#4` stamp, and
/// the nested-Timestamp reading of the agy 1.1.18 `#9.#10` payload.
///
/// `seconds` is an unbounded wire varint, so a malformed blob can carry a value
/// whose `* 1000` overflows `i64` and panics in debug builds. Use checked
/// arithmetic and return `None` on overflow to keep the module's
/// "malformed data degrades to `None`, never panics" contract.
///
/// `nanos` is range-validated against the protobuf Timestamp spec (must be
/// `0..=999_999_999`); an out-of-range or negative `nanos` marks the whole
/// stamp as malformed (`None`) so the caller's `ms > 0` filter and
/// session-timestamp fallback take over instead of producing a skewed time.
fn proto_timestamp_ms(ts: &[u8]) -> Option<i64> {
    let seconds = varint_field(ts, 1)? as i64;
    let nanos = i64::try_from(varint_field(ts, 2).unwrap_or(0)).ok()?;
    if !(0..=999_999_999).contains(&nanos) {
        return None;
    }
    seconds.checked_mul(1000)?.checked_add(nanos / 1_000_000)
}

fn file_modified_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .map(|time| chrono::DateTime::<chrono::Utc>::from(time).timestamp_millis())
        .unwrap_or(0)
}

/// Convert a `file://` URI to a filesystem path, percent-decoding UTF-8 escapes
/// (workspace paths on cloud drives can be percent-encoded CJK). After the
/// scheme the remainder is `authority + path`; the three shapes RFC 8089 (and
/// Antigravity) produces are handled:
/// - `file:///C:/x`        → `C:/x`            (empty authority, Windows drive: drop the leading slash)
/// - `file:///home/x`      → `/home/x`         (empty authority, POSIX absolute: keep as-is)
/// - `file://host/share/x` → `//host/share/x`  (non-empty authority → UNC: restore the leading `//`)
fn file_uri_to_path(uri: &str) -> Option<String> {
    let decoded = percent_decode(uri.strip_prefix("file://")?);
    let bytes = decoded.as_bytes();
    let path = if bytes.first() == Some(&b'/') {
        // Empty authority. Drop the slash before a Windows drive letter
        // (`/C:/...`); keep POSIX absolute paths untouched.
        if bytes.len() >= 3 && bytes[2] == b':' {
            decoded[1..].to_string()
        } else {
            decoded
        }
    } else {
        // Non-empty authority (`host/share/...`) is a UNC path; restore the
        // leading `//` so `normalize_workspace_key` preserves the UNC prefix
        // instead of collapsing it into the path body.
        format!("//{decoded}")
    };
    Some(path)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Minimal protobuf wire-format reader (no prost / schema dependency).
// ---------------------------------------------------------------------------

enum Wire<'a> {
    Varint(u64),
    Len(&'a [u8]),
    Fixed64(u64),
    Fixed32,
}

struct ProtoReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ProtoReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn read_varint(&mut self) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = *self.buf.get(self.pos)?;
            self.pos += 1;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }

    /// Yield the next `(field_number, value)` pair, or `None` at end-of-buffer
    /// or on a malformed/unsupported wire type. Group wire types (3/4) are
    /// deprecated and never appear here; we stop rather than risk desync.
    fn next_field(&mut self) -> Option<(u64, Wire<'a>)> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let tag = self.read_varint()?;
        let field = tag >> 3;
        let wire = match tag & 0x7 {
            0 => Wire::Varint(self.read_varint()?),
            1 => {
                let end = self.pos.checked_add(8).filter(|&p| p <= self.buf.len())?;
                let bytes: [u8; 8] = self.buf[self.pos..end].try_into().ok()?;
                self.pos = end;
                Wire::Fixed64(u64::from_le_bytes(bytes))
            }
            2 => {
                let len = self.read_varint()? as usize;
                let end = self.pos.checked_add(len).filter(|&p| p <= self.buf.len())?;
                let bytes = &self.buf[self.pos..end];
                self.pos = end;
                Wire::Len(bytes)
            }
            5 => {
                self.pos = self.pos.checked_add(4).filter(|&p| p <= self.buf.len())?;
                Wire::Fixed32
            }
            _ => return None,
        };
        Some((field, wire))
    }
}

/// First length-delimited (sub-message / string / bytes) value for `field`.
fn message_field(buf: &[u8], field: u64) -> Option<&[u8]> {
    let mut reader = ProtoReader::new(buf);
    while let Some((found, wire)) = reader.next_field() {
        if found == field {
            if let Wire::Len(bytes) = wire {
                return Some(bytes);
            }
        }
    }
    None
}

/// First varint value for `field`.
fn varint_field(buf: &[u8], field: u64) -> Option<u64> {
    let mut reader = ProtoReader::new(buf);
    while let Some((found, wire)) = reader.next_field() {
        if found == field {
            if let Wire::Varint(value) = wire {
                return Some(value);
            }
        }
    }
    None
}

/// First `fixed64` value for `field`, decoded little-endian as protobuf
/// specifies. Only the inferred agy 1.1.18 timestamp payload reads one.
fn fixed64_field(buf: &[u8], field: u64) -> Option<u64> {
    let mut reader = ProtoReader::new(buf);
    while let Some((found, wire)) = reader.next_field() {
        if found == field {
            if let Wire::Fixed64(value) = wire {
                return Some(value);
            }
        }
    }
    None
}

/// First UTF-8 string value for `field`.
fn string_field(buf: &[u8], field: u64) -> Option<&str> {
    message_field(buf, field).and_then(|bytes| std::str::from_utf8(bytes).ok())
}

/// [`string_field`], treating a blank value as absent. Antigravity writes the
/// model fields either fully or not at all, but a blank string must not be
/// mistaken for a usable model id or display label.
fn non_empty_string_field(buf: &[u8], field: u64) -> Option<&str> {
    string_field(buf, field).filter(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn enc_varint(field: u64, value: u64) -> Vec<u8> {
        let mut out = encode_varint(field << 3);
        out.extend(encode_varint(value));
        out
    }

    fn enc_len(field: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = encode_varint((field << 3) | 2);
        out.extend(encode_varint(payload.len() as u64));
        out.extend_from_slice(payload);
        out
    }

    fn encode_varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
        out
    }

    fn enc_fixed64(field: u64, value: u64) -> Vec<u8> {
        let mut out = encode_varint((field << 3) | 1);
        out.extend_from_slice(&value.to_le_bytes());
        out
    }

    /// A believable "just now" instant. Derived from the clock rather than
    /// pinned to a fixed date so the plausibility window these tests exercise
    /// cannot drift out from under them.
    fn recent_epoch_seconds() -> i64 {
        chrono::Utc::now().timestamp() - 3_600
    }

    /// One `gen_metadata` blob whose `chatModel.#9` sub-message is exactly
    /// `gen9`, so a test can drive the timestamp layout directly.
    fn build_row_with_gen9(gen9: &[u8], response_id: &str) -> Vec<u8> {
        let mut usage = Vec::new();
        usage.extend(enc_varint(2, 500)); // input
        usage.extend(enc_varint(9, 300)); // output
        usage.extend(enc_len(11, response_id.as_bytes())); // responseId

        let mut chat_model = Vec::new();
        chat_model.extend(enc_len(4, &usage));
        chat_model.extend(enc_len(9, gen9));
        chat_model.extend(enc_len(19, b"gemini-3-flash-a"));
        enc_len(1, &chat_model)
    }

    /// Timestamp parsed out of a row carrying `gen9`, with `session_fallback`
    /// standing in for a decoded session-created stamp — i.e. serving as both
    /// the row fallback and the inference anchor.
    fn gen9_timestamp(gen9: &[u8], session_fallback: i64) -> i64 {
        gen9_timestamp_anchored(gen9, session_fallback, Some(session_fallback))
    }

    /// The same, with the inference anchor supplied independently of the row
    /// fallback, so a test can model a database whose `trajectory_metadata_blob`
    /// never yielded a created-at while the row fallback is still positive.
    fn gen9_timestamp_anchored(gen9: &[u8], session_fallback: i64, anchor: Option<i64>) -> i64 {
        let mut seen = HashSet::new();
        parse_gen_metadata(
            &build_row_with_gen9(gen9, "resp"),
            "s",
            session_fallback,
            anchor,
            &SessionModels::default(),
            &mut seen,
        )
        .expect("row parses")
        .timestamp
    }

    /// Parse one row with no conversation-level attribution available, i.e. as
    /// if it were the file's only row. Rows that carry their own `#19` are
    /// unaffected by the session index, so most tests need nothing else.
    fn parse_isolated_row(
        blob: &[u8],
        session_id: &str,
        session_timestamp: i64,
        seen_response_ids: &mut HashSet<String>,
    ) -> Option<UnifiedMessage> {
        parse_gen_metadata(
            blob,
            session_id,
            session_timestamp,
            Some(session_timestamp),
            &SessionModels::default(),
            seen_response_ids,
        )
    }

    fn build_gen_metadata() -> Vec<u8> {
        build_gen_metadata_with_model("gemini-3-flash-a")
    }

    fn build_gen_metadata_with_model(model: &str) -> Vec<u8> {
        build_row(Some(model), None, "resp-1")
    }

    /// One `gen_metadata` blob with either model field independently present or
    /// absent, mirroring the real rows where `#19` is missing but `#21` is not.
    fn build_row(model: Option<&str>, display: Option<&str>, response_id: &str) -> Vec<u8> {
        // usage message (#4 of chatModel)
        let mut usage = Vec::new();
        usage.extend(enc_varint(1, 1132)); // fixed system prompt
        usage.extend(enc_varint(2, 500)); // new input
        usage.extend(enc_varint(5, 16000)); // cacheRead
        usage.extend(enc_varint(9, 300)); // output
        usage.extend(enc_varint(10, 40)); // thinking
        usage.extend(enc_len(11, response_id.as_bytes())); // responseId

        // chatModel message (#1 of gen_metadata)
        let mut chat_model = Vec::new();
        chat_model.extend(enc_len(4, &usage));
        if let Some(model) = model {
            chat_model.extend(enc_len(19, model.as_bytes()));
        }
        if let Some(display) = display {
            chat_model.extend(enc_len(21, display.as_bytes()));
        }

        enc_len(1, &chat_model)
    }

    /// Build a conversation database from `gen_metadata` blobs in row order.
    fn write_conversation(path: &Path, blobs: &[Vec<u8>]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("CREATE TABLE gen_metadata (idx integer, data blob, size integer);")
            .unwrap();
        for (idx, blob) in blobs.iter().enumerate() {
            conn.execute(
                "INSERT INTO gen_metadata (idx, data, size) VALUES (?1, ?2, 0)",
                params![idx as i64, blob],
            )
            .unwrap();
        }
    }

    fn build_trajectory_meta() -> Vec<u8> {
        let workspace = enc_len(1, b"file:///C:/Users/Frank/obsidian-vault");
        let created = {
            let mut created = Vec::new();
            created.extend(enc_varint(1, 1_781_502_653)); // seconds
            created.extend(enc_varint(2, 0)); // nanos
            created
        };
        let mut blob = Vec::new();
        blob.extend(enc_len(1, &workspace));
        blob.extend(enc_len(2, &created));
        blob
    }

    #[test]
    fn overlarge_varint_token_counts_are_clamped_not_wrapped() {
        // A corrupt/malicious blob encoding a varint > i64::MAX must clamp to a
        // non-negative i64 (saturating), never wrap `as i64` to a negative count.
        let mut usage = Vec::new();
        usage.extend(enc_varint(1, u64::MAX)); // huge fixed system prompt
        usage.extend(enc_varint(2, 10)); // + small input -> saturating_add
        usage.extend(enc_varint(9, u64::MAX)); // huge output
        usage.extend(enc_len(11, b"resp-overflow"));
        let mut chat_model = Vec::new();
        chat_model.extend(enc_len(4, &usage));
        chat_model.extend(enc_len(19, b"gemini-3-flash-a"));
        let blob = enc_len(1, &chat_model);

        let mut seen = HashSet::new();
        let msg = parse_isolated_row(&blob, "s", 1_000, &mut seen).expect("parses");
        assert_eq!(msg.tokens.output, i64::MAX);
        assert_eq!(msg.tokens.input, i64::MAX); // saturating_add, not negative
        assert!(msg.tokens.input >= 0 && msg.tokens.output >= 0);
    }

    #[test]
    fn parses_tokens_model_and_workspace_from_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-test.db");

        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE gen_metadata (idx integer, data blob, size integer);
                 CREATE TABLE trajectory_metadata_blob (id text, data blob);",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO gen_metadata (idx, data, size) VALUES (0, ?1, 0)",
                params![build_gen_metadata()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO trajectory_metadata_blob (id, data) VALUES ('main', ?1)",
                params![build_trajectory_meta()],
            )
            .unwrap();
        }

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "antigravity-cli");
        // `gemini-3-flash-a` (raw #19 responseModel) is alias-resolved to the
        // priced canonical model so cost lookups don't fall through to 0.
        // Per upstream (models.ts@603e3ea), `gemini-3-flash-a` is the legacy
        // responseModel for M132, the retired predecessor of M133 — i.e. the
        // High tier, not the unrelated gemini-3-flash-preview family.
        assert_eq!(message.model_id, "gemini-3.5-flash-high");
        assert_eq!(message.provider_id, "google");
        assert_eq!(message.session_id, "session-test");
        assert_eq!(message.tokens.input, 1632); // 1132 + 500
        assert_eq!(message.tokens.cache_read, 16000);
        assert_eq!(message.tokens.output, 300);
        assert_eq!(message.tokens.reasoning, 40);
        assert_eq!(message.dedup_key.as_deref(), Some("resp-1"));
        assert_eq!(message.timestamp, 1_781_502_653_000);
        assert_eq!(
            message.workspace_key.as_deref(),
            Some("C:/Users/Frank/obsidian-vault")
        );
        assert_eq!(message.workspace_label.as_deref(), Some("obsidian-vault"));
    }

    #[test]
    fn resolves_current_antigravity_cli_response_model() {
        let blob = build_gen_metadata_with_model("gemini-3-flash-agent");
        let mut seen = HashSet::new();

        let message = parse_isolated_row(&blob, "session", 1_000, &mut seen).unwrap();

        assert_eq!(message.model_id, "gemini-3.5-flash-high");
        assert_eq!(message.provider_id, "google");
    }

    // The generic routing label is preserved verbatim. It is not a concrete
    // billable model id, so submit can exclude it instead of inventing a cost.
    #[test]
    fn gemini_default_response_model_is_preserved() {
        let blob = build_gen_metadata_with_model("gemini-default");
        let mut seen = HashSet::new();

        let message = parse_isolated_row(&blob, "session", 1_000, &mut seen).unwrap();

        assert_eq!(message.model_id, "gemini-default");
        assert_eq!(message.provider_id, "google");
    }

    // Antigravity CLI omits `#19` on some continuation turns while still
    // writing `#21`. Observed in three real conversations: every such row sat
    // in a database whose other rows carried `#19` next to the identical `#21`
    // label, so the machine id is recoverable and the row must not degrade to
    // the unpriceable `antigravity/unknown`.
    #[test]
    fn missing_response_model_is_recovered_from_the_display_label() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("continuation.db");
        write_conversation(
            &path,
            &[
                build_row(
                    Some("gemini-3.6-flash"),
                    Some("Gemini 3.6 Flash (High)"),
                    "resp-0",
                ),
                build_row(None, Some("Gemini 3.6 Flash (High)"), "resp-1"),
            ],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 2);
        for message in &messages {
            assert_eq!(message.model_id, "gemini-3.6-flash");
            assert_eq!(message.provider_id, "google");
        }
    }

    #[test]
    fn recovery_reads_the_whole_conversation_not_just_earlier_rows() {
        // The index is built from every row before any row is parsed, so a
        // conversation whose first turn is the one missing `#19` recovers just
        // as well as one where the gap comes later.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gap-first.db");
        write_conversation(
            &path,
            &[
                build_row(None, Some("Gemini 3.6 Flash (High)"), "resp-0"),
                build_row(
                    Some("gemini-3.6-flash"),
                    Some("Gemini 3.6 Flash (High)"),
                    "resp-1",
                ),
            ],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].model_id, "gemini-3.6-flash");
    }

    #[test]
    fn recovered_model_still_resolves_through_the_alias_table() {
        // The recovered value is the raw `#19` wire string, so it must take the
        // same alias path as a directly-read one — otherwise recovery would
        // hand pricing an id it cannot match.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aliased.db");
        write_conversation(
            &path,
            &[
                build_row(
                    Some("gemini-3-flash-a"),
                    Some("Gemini 3.5 Flash (High)"),
                    "resp-0",
                ),
                build_row(None, Some("Gemini 3.5 Flash (High)"), "resp-1"),
            ],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].model_id, "gemini-3.5-flash-high");
        assert_eq!(messages[1].provider_id, "google");
    }

    #[test]
    fn a_display_label_no_row_identified_is_not_guessed_at() {
        // The conversation switched models: the row missing `#19` is labelled
        // Pro, and the only identified model is a Flash. Borrowing the Flash id
        // would bill the turn at the wrong tier, so the row stays `unknown` —
        // a label alone is not a model id.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("switched.db");
        write_conversation(
            &path,
            &[
                build_row(
                    Some("gemini-3.6-flash"),
                    Some("Gemini 3.6 Flash (High)"),
                    "resp-0",
                ),
                build_row(None, Some("Gemini 3.1 Pro"), "resp-1"),
            ],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].model_id, "gemini-3.6-flash");
        assert_eq!(messages[1].model_id, "unknown");
        assert_eq!(messages[1].provider_id, "antigravity");
    }

    #[test]
    fn a_display_label_used_by_two_models_is_not_used_for_recovery() {
        // Should a label ever be reused across machine ids (a rename landing
        // mid-conversation), it identifies nothing and must be discarded rather
        // than resolved to whichever row happened to be indexed last.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ambiguous.db");
        write_conversation(
            &path,
            &[
                build_row(Some("gemini-3-flash-a"), Some("Gemini Flash"), "resp-0"),
                build_row(Some("gemini-3.6-flash"), Some("Gemini Flash"), "resp-1"),
                build_row(None, Some("Gemini Flash"), "resp-2"),
            ],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].model_id, "unknown");
    }

    #[test]
    fn two_spellings_of_one_priced_model_are_not_an_ambiguous_label() {
        // Antigravity swaps machine ids mid-conversation without changing the
        // display label: `gemini-pro-default` and `gemini-pro-agent` are both
        // "Gemini 3.1 Pro (High)" and both price as `gemini-3.1-pro`. Comparing
        // the raw ids called that ambiguous and discarded the label, so the
        // continuation row below resolved to `unknown` — which has no pricing
        // and aborted `tokscale submit` outright (#1058).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aliased.db");
        write_conversation(
            &path,
            &[
                build_row(
                    Some("gemini-pro-default"),
                    Some("Gemini 3.1 Pro (High)"),
                    "resp-0",
                ),
                build_row(
                    Some("gemini-pro-agent"),
                    Some("Gemini 3.1 Pro (High)"),
                    "resp-1",
                ),
                build_row(None, Some("Gemini 3.1 Pro (High)"), "resp-2"),
            ],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].model_id, "gemini-3.1-pro");
        assert_ne!(messages[2].model_id, "unknown");
    }

    #[test]
    fn a_row_with_no_model_fields_falls_back_to_a_single_model_conversation() {
        // Nothing joins a row that carries neither `#19` nor `#21`. When the
        // whole conversation used one model there is only one answer it could
        // have; when it used several, there is no answer and the row stays
        // `unknown`.
        let dir = tempfile::tempdir().unwrap();

        let single = dir.path().join("single-model.db");
        write_conversation(
            &single,
            &[
                build_row(
                    Some("gemini-3.6-flash"),
                    Some("Gemini 3.6 Flash (High)"),
                    "resp-0",
                ),
                build_row(None, None, "resp-1"),
            ],
        );
        let messages = parse_antigravity_cli_file(&single);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].model_id, "gemini-3.6-flash");

        let mixed = dir.path().join("mixed-models.db");
        write_conversation(
            &mixed,
            &[
                build_row(Some("gemini-3.6-flash"), None, "resp-0"),
                build_row(Some("gemini-3.1-pro"), None, "resp-1"),
                build_row(None, None, "resp-2"),
            ],
        );
        let messages = parse_antigravity_cli_file(&mixed);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].model_id, "unknown");
    }

    #[test]
    fn a_label_no_row_identified_withholds_the_sole_model_fallback() {
        // Only one model is named here, but the Pro-labelled row proves a second
        // one ran. A row carrying no fields at all could be either, so counting
        // named ids alone would bill a model switch under the wrong model.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unnamed-second-model.db");
        write_conversation(
            &path,
            &[
                build_row(
                    Some("gemini-3.6-flash"),
                    Some("Gemini 3.6 Flash (High)"),
                    "resp-0",
                ),
                build_row(None, Some("Gemini 3.1 Pro"), "resp-1"),
                build_row(None, None, "resp-2"),
            ],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].model_id, "gemini-3.6-flash");
        assert_eq!(messages[1].model_id, "unknown");
        assert_eq!(messages[2].model_id, "unknown");
    }

    #[test]
    fn per_generation_timestamp_overrides_session_fallback() {
        // chatModel.#9.#4 = {#1: seconds, #2: nanos} is the per-turn wall-clock
        // stamp. When present it dates the row; when absent the row falls back
        // to the session-created timestamp passed in. (Verified against real
        // databases: every gen_metadata row carries a distinct, monotonic
        // #9.#4 stamp >= the session-created time.)
        let session_fallback = 111_000_i64;

        let mut usage = Vec::new();
        usage.extend(enc_varint(2, 500)); // input
        usage.extend(enc_varint(9, 300)); // output
        usage.extend(enc_len(11, b"with-time")); // responseId

        // #9 wraps a sub-message whose #4 is the {seconds, nanos} Timestamp.
        let mut gen_time = Vec::new();
        gen_time.extend(enc_varint(1, 1_781_000_000)); // seconds
        gen_time.extend(enc_varint(2, 250_000_000)); // nanos -> +250ms
        let gen9 = enc_len(4, &gen_time);

        let mut chat_model = Vec::new();
        chat_model.extend(enc_len(4, &usage));
        chat_model.extend(enc_len(9, &gen9));
        chat_model.extend(enc_len(19, b"gemini-3-flash-a"));
        let blob = enc_len(1, &chat_model);

        let mut seen = HashSet::new();
        let message = parse_isolated_row(&blob, "s", session_fallback, &mut seen).unwrap();
        assert_eq!(
            message.timestamp,
            1_781_000_000 * 1000 + 250,
            "per-generation #9.#4 timestamp must override the session fallback"
        );

        // The same row shape without #9 falls back to the session timestamp
        // (build_gen_metadata carries no #9.#4).
        let mut seen2 = HashSet::new();
        let fallback_msg =
            parse_isolated_row(&build_gen_metadata(), "s", session_fallback, &mut seen2).unwrap();
        assert_eq!(
            fallback_msg.timestamp, session_fallback,
            "a row without #9.#4 must use the session-created fallback"
        );
    }

    /// agy 1.1.18 replaced `chatModel.#9.#4` with `#2` = `u64::MAX` plus `#10`
    /// (8 length-delimited bytes). `#10`'s encoding is inferred rather than
    /// observed, so every shape the parser is willing to accept must yield the
    /// same per-generation stamp — and none of them may be disturbed by the
    /// sentinel sitting next to it.
    #[test]
    fn agy_1_1_18_gen9_field_10_dates_the_turn() {
        let seconds = recent_epoch_seconds();
        let expected_ms = seconds * 1_000;
        // Anchor the session shortly before the turn so the session window matches
        // real-world sessions and does not unnaturally widen to invite
        // endianness ambiguity across months.
        let session_fallback = expected_ms - 60_000;
        let sentinel = enc_varint(2, u64::MAX);

        // (1) nested {#1: seconds, #2: nanos} Timestamp.
        let mut nested_ts = Vec::new();
        nested_ts.extend(enc_varint(1, seconds as u64));
        nested_ts.extend(enc_varint(2, 250_000_000)); // -> +250ms
        let mut gen9 = sentinel.clone();
        gen9.extend(enc_len(10, &nested_ts));
        assert_eq!(
            gen9_timestamp(&gen9, session_fallback),
            expected_ms + 250,
            "a nested Timestamp in #9.#10 must date the turn"
        );

        for (unit, scalar) in [
            ("seconds", seconds as u64),
            ("millis", (seconds * 1_000) as u64),
            ("micros", (seconds * 1_000_000) as u64),
            ("nanos", (seconds * 1_000_000_000) as u64),
        ] {
            // (2) nested message holding the scalar in field 1, as a varint...
            let mut gen9 = sentinel.clone();
            gen9.extend(enc_len(10, &enc_varint(1, scalar)));
            assert_eq!(
                gen9_timestamp(&gen9, session_fallback),
                expected_ms,
                "nested varint {unit} must date the turn"
            );

            // ... and as a fixed64.
            let mut gen9 = sentinel.clone();
            gen9.extend(enc_len(10, &enc_fixed64(1, scalar)));
            assert_eq!(
                gen9_timestamp(&gen9, session_fallback),
                expected_ms,
                "nested fixed64 {unit} must date the turn"
            );

            // (3) the payload itself as 8 raw fixed64-style bytes.
            for (order, raw) in [
                ("little-endian", scalar.to_le_bytes()),
                ("big-endian", scalar.to_be_bytes()),
            ] {
                let mut gen9 = sentinel.clone();
                gen9.extend(enc_len(10, &raw));
                assert_eq!(
                    gen9_timestamp(&gen9, session_fallback),
                    expected_ms,
                    "raw {order} {unit} must date the turn"
                );
            }
        }
    }

    /// Regression test for #1256: an 8-byte payload whose little-endian reading
    /// decodes as nanoseconds and whose big-endian reading decodes as
    /// milliseconds can both clear an unnaturally wide session window while
    /// pointing to dates days or months apart. Rather than letting the
    /// little-endian reading arbitrarily outrank the big-endian reading and
    /// silently misdate the turn, the ambiguity must be rejected so the row
    /// falls back to the session stamp.
    #[test]
    fn ambiguous_competing_endianness_payload_falls_back_to_session_timestamp() {
        // Intended BE millis: 1_788_132_511_000 (2026-08-31)
        // Reversed LE reading as nanos: 1_787_084_994_892 (2026-08-19, off by ~12.1 days)
        let intended_ms = 1_788_132_511_000_i64;
        let raw_be = (intended_ms as u64).to_be_bytes();
        let wide_session_fallback = 1_781_502_653_000_i64; // 2026-06-16 (wide enough for both)

        let sentinel = enc_varint(2, u64::MAX);
        let mut gen9 = sentinel.clone();
        gen9.extend(enc_len(10, &raw_be));

        assert_eq!(
            gen9_timestamp(&gen9, wide_session_fallback),
            wide_session_fallback,
            "competing in-window endianness readings must fall back to session timestamp"
        );

        // Second observed reproduction case: 1_788_274_719_000 (2026-09-01)
        // Reversed LE reading as nanos: 1_781_589_670_136 (2026-06-17, off by ~77.4 days)
        let intended_ms_2 = 1_788_274_719_000_i64;
        let raw_be_2 = (intended_ms_2 as u64).to_be_bytes();
        let mut gen9_2 = sentinel;
        gen9_2.extend(enc_len(10, &raw_be_2));

        assert_eq!(
            gen9_timestamp(&gen9_2, wide_session_fallback),
            wide_session_fallback,
            "competing in-window endianness readings must fall back to session timestamp"
        );
    }

    /// Genuine little-endian millisecond payloads must still date the turn even
    /// when the session window is wide enough that the byte-reversed integer
    /// decodes as nanoseconds in-window (~1.2% of genuine LE millisecond values).
    #[test]
    fn genuine_le_millis_payload_dates_the_turn_with_competing_reversed_nanos() {
        // Construct a genuine LE millisecond timestamp whose low byte is 0x18
        // so that its byte-reversed big-endian integer starts with 0x18 and decodes
        // into a plausible nanoseconds date.
        let le_millis = (1_788_132_511_000_i64 & !0xff) | 0x18; // 1_788_132_510_744
        let raw_le = (le_millis as u64).to_le_bytes();
        let wide_session_fallback = 1_740_000_000_000_i64; // Early 2025 (accepts both)

        let sentinel = enc_varint(2, u64::MAX);
        let mut gen9 = sentinel;
        gen9.extend(enc_len(10, &raw_le));

        assert_eq!(
            gen9_timestamp(&gen9, wide_session_fallback),
            le_millis,
            "genuine LE milliseconds reading must date the turn over the reversed BE nanoseconds ghost"
        );
    }

    /// Nothing that is not a believable time may become one. Mis-dating a turn
    /// silently corrupts day buckets and the server-side monotonic ratchet,
    /// which is worse than the known-wrong session-start stamp this falls back
    /// to.
    #[test]
    fn unrecognised_gen9_payloads_fall_back_to_the_session_timestamp() {
        let session_fallback = 1_781_502_653_000_i64;
        let seconds = recent_epoch_seconds();

        // The unset sentinel alone, exactly as agy 1.1.18 writes `#9.#2`.
        assert_eq!(
            gen9_timestamp(&enc_varint(2, u64::MAX), session_fallback),
            session_fallback,
            "the #9.#2 unset sentinel must never be read as a time"
        );

        // The same value arriving through the `#10` payload instead.
        let mut sentinel_payload = enc_varint(2, u64::MAX);
        sentinel_payload.extend(enc_len(10, &u64::MAX.to_le_bytes()));
        assert_eq!(
            gen9_timestamp(&sentinel_payload, session_fallback),
            session_fallback,
            "u64::MAX in the #10 payload must never be read as a time"
        );
        assert_eq!(epoch_scalar_to_ms(u64::MAX), None);

        // An 8-byte `#10` that is not a timestamp at all (an id, a hash).
        let opaque = [0x9a, 0x3f, 0x00, 0x11, 0xc4, 0x7e, 0x5d, 0x02];
        assert_eq!(
            gen9_timestamp(&enc_len(10, &opaque), session_fallback),
            session_fallback,
            "an opaque 8-byte #10 must fall back rather than produce a date"
        );

        // Right shape, wrong window — in both directions.
        let stale = 631_152_000_u64; // 1990-01-01, before the CLI existed
        let far_future = (seconds + 10 * 365 * 24 * 60 * 60) as u64;
        for bogus in [stale, far_future] {
            assert_eq!(
                gen9_timestamp(&enc_len(10, &bogus.to_le_bytes()), session_fallback),
                session_fallback,
                "raw out-of-range {bogus} must fall back to the session stamp"
            );
            assert_eq!(
                gen9_timestamp(&enc_len(10, &enc_varint(1, bogus)), session_fallback),
                session_fallback,
                "nested out-of-range {bogus} must fall back to the session stamp"
            );
        }
    }

    /// Regression guard for every pre-1.1.18 database: the explicit `#9.#4`
    /// Timestamp still dates the row, and outranks the inferred `#9.#10`
    /// reading if a row ever carries both.
    #[test]
    fn explicit_field_4_timestamp_outranks_the_inferred_field_10_reading() {
        let session_fallback = 1_781_502_653_000_i64;
        let seconds = recent_epoch_seconds();

        let mut explicit = Vec::new();
        explicit.extend(enc_varint(1, seconds as u64));
        explicit.extend(enc_varint(2, 500_000_000)); // -> +500ms
        let mut gen9 = enc_len(4, &explicit);

        // A different but equally believable #10 value that must be ignored.
        let other = ((seconds - 7_200) * 1_000_000) as u64;
        gen9.extend(enc_len(10, &other.to_le_bytes()));

        assert_eq!(
            gen9_timestamp(&gen9, session_fallback),
            seconds * 1_000 + 500,
            "the explicit #9.#4 Timestamp must keep priority over #9.#10"
        );
    }

    /// The `#9.#4` path is a confirmed representation read off real
    /// pre-1.1.18 databases, so it is trusted on its own and is deliberately
    /// *not* bounded by the session window the inferred `#9.#10` readings must
    /// satisfy. A database whose session-created stamp is missing or wrong must
    /// not lose the stamp it actually recorded.
    #[test]
    fn explicit_field_4_timestamp_is_not_bounded_by_the_session_window() {
        let session_start = chrono::Utc::now().timestamp_millis() - 3 * 24 * 60 * 60 * 1_000;
        let long_before_session = 1_609_459_200_i64; // 2021-01-01, years before

        let mut explicit = Vec::new();
        explicit.extend(enc_varint(1, long_before_session as u64));
        explicit.extend(enc_varint(2, 0));
        let gen9 = enc_len(4, &explicit);

        assert_eq!(
            gen9_timestamp(&gen9, session_start),
            long_before_session * 1_000,
            "the explicit #9.#4 Timestamp must stay unbounded by the session window"
        );
    }

    /// The inferred `#9.#10` readings only mean anything relative to the
    /// session that contains them: a turn cannot predate its own conversation
    /// and cannot happen after the file was read. Both bounds carry a one-hour
    /// tolerance for clock skew, and both are exercised here from inside and
    /// outside.
    #[test]
    fn inferred_gen9_timestamps_must_land_inside_the_session_window() {
        const HOUR_MS: i64 = 60 * 60 * 1_000;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let session_start = now_ms - 3 * 24 * 60 * 60 * 1_000;

        // A raw millisecond payload, the shape the parser is likeliest to meet.
        let payload = |ms: i64| enc_len(10, &(ms as u64).to_le_bytes());

        // Inside: a turn an hour into a three-day-old session.
        let genuine = session_start + HOUR_MS;
        assert_eq!(
            gen9_timestamp(&payload(genuine), session_start),
            genuine,
            "a turn inside its own session must still date the row"
        );

        // Inside: the most recent turn of a session still being written.
        let latest = now_ms - HOUR_MS;
        assert_eq!(
            gen9_timestamp(&payload(latest), session_start),
            latest,
            "a turn from minutes ago must still date the row"
        );

        // Inside the lower tolerance: half an hour before the session stamp is
        // clock skew, not a different session.
        let slightly_early = session_start - HOUR_MS / 2;
        assert_eq!(
            gen9_timestamp(&payload(slightly_early), session_start),
            slightly_early,
            "the one-hour skew allowance below the session stamp must be honoured"
        );

        // Outside, below: two hours predates the session by more than skew.
        let before_tolerance = session_start - 2 * HOUR_MS;
        assert!(
            plausible_epoch_ms(before_tolerance),
            "the rejection must come from the session window, not the absolute one"
        );
        assert_eq!(
            gen9_timestamp(&payload(before_tolerance), session_start),
            session_start,
            "a stamp beyond the skew allowance below the session start must fall back"
        );

        // Outside, below: a full day before the session began.
        let before_session = session_start - 24 * HOUR_MS;
        assert!(plausible_epoch_ms(before_session));
        assert_eq!(
            gen9_timestamp(&payload(before_session), session_start),
            session_start,
            "a stamp predating the session must fall back to the session stamp"
        );

        // Outside, above: believable as a date, impossible as a recorded turn.
        let far_future = now_ms + 2 * 365 * 24 * HOUR_MS;
        assert!(
            plausible_epoch_ms(far_future),
            "the absolute window still accepts two years out, so only the \
             session window can reject this"
        );
        assert_eq!(
            gen9_timestamp(&payload(far_future), session_start),
            session_start,
            "a stamp in the future must fall back to the session stamp"
        );
    }

    /// The reason the session window exists. Eight opaque bytes read as a
    /// nanosecond count cover ~2% of the `u64` range within the absolute
    /// window alone, so an id or a hash has a few percent chance of passing as
    /// a date once both byte orders are tried. This payload is one of them: it
    /// clears the absolute check outright, and only the session window keeps it
    /// from silently re-dating a turn to 2023.
    #[test]
    fn an_opaque_payload_that_reads_as_a_plausible_date_is_still_rejected() {
        let session_start = chrono::Utc::now().timestamp_millis() - 3 * 24 * 60 * 60 * 1_000;
        let opaque = [0x37, 0xa9, 0x5c, 0xd3, 0x1e, 0x4b, 0x62, 0x17];

        // What the absolute gate on its own makes of it: a valid 2023 date.
        let absolute = epoch_scalar_to_ms(u64::from_le_bytes(opaque))
            .expect("these bytes do decode under the absolute plausibility window");
        assert_eq!(
            absolute, 1_684_991_806_357,
            "2023-05-25, read as nanoseconds"
        );
        assert!(plausible_epoch_ms(absolute));

        // What the session window makes of it: not a turn of this session.
        assert_eq!(
            gen9_timestamp(&enc_len(10, &opaque), session_start),
            session_start,
            "a payload that only looks like a date must not re-date the turn"
        );
    }

    /// No anchor, no inference. Without a session-created stamp there is
    /// nothing to corroborate an inferred reading against, so every candidate is
    /// declined and the row falls back exactly as it did before the 1.1.18
    /// layout was handled at all.
    ///
    /// The row fallback is held positive throughout, because that is the case
    /// that matters: `None` here is a database whose `trajectory_metadata_blob`
    /// did not decode, and its fallback is then the file mtime — positive, but
    /// no more of an anchor for that.
    #[test]
    fn a_missing_session_anchor_declines_every_inferred_reading() {
        let seconds = recent_epoch_seconds();
        let sentinel = enc_varint(2, u64::MAX);
        let fallback = 1_781_502_653_000_i64;

        for anchor in [None, Some(0_i64), Some(-1)] {
            // A nested Timestamp...
            let mut nested_ts = Vec::new();
            nested_ts.extend(enc_varint(1, seconds as u64));
            nested_ts.extend(enc_varint(2, 0));
            let mut gen9 = sentinel.clone();
            gen9.extend(enc_len(10, &nested_ts));
            assert_eq!(
                gen9_timestamp_anchored(&gen9, fallback, anchor),
                fallback,
                "a nested Timestamp must be declined without a session anchor"
            );

            // ... a nested scalar...
            let mut gen9 = sentinel.clone();
            gen9.extend(enc_len(10, &enc_varint(1, seconds as u64)));
            assert_eq!(
                gen9_timestamp_anchored(&gen9, fallback, anchor),
                fallback,
                "a nested scalar must be declined without a session anchor"
            );

            // ... and the raw eight bytes.
            let mut gen9 = sentinel.clone();
            gen9.extend(enc_len(10, &(seconds as u64).to_le_bytes()));
            assert_eq!(
                gen9_timestamp_anchored(&gen9, fallback, anchor),
                fallback,
                "a raw payload must be declined without a session anchor"
            );
        }

        assert!(session_window_ms(0).is_none());
        assert!(session_window_ms(-1).is_none());
        assert!(session_window_ms(1).is_some());
    }

    /// The anchor has to come from `trajectory_metadata_blob`, never from the
    /// file mtime the row fallback degrades to. An mtime dates the *last write*
    /// to the database, so a `#9.#10` payload landing anywhere near it sails
    /// through a window built on it while proving nothing about when the turn
    /// happened — and a genuinely older turn gets rejected for sitting below it.
    /// Driven end to end through the real SQLite path, because the mtime only
    /// enters the picture there. Both shapes that yield no created-at are
    /// covered: the table missing outright, and a blob present but carrying no
    /// `#2`.
    #[test]
    fn an_mtime_only_session_anchors_no_inference() {
        const HOUR_MS: i64 = 60 * 60 * 1_000;
        let dir = tempfile::tempdir().unwrap();

        // Half an hour before the file was written: comfortably inside the
        // window an mtime anchor would have produced, and inside the absolute
        // plausibility window too, so only the provenance of the anchor can
        // reject it.
        let would_pass_under_mtime = chrono::Utc::now().timestamp_millis() - HOUR_MS / 2;
        let mut gen9 = enc_varint(2, u64::MAX); // the 1.1.18 unset sentinel
        gen9.extend(enc_len(10, &(would_pass_under_mtime as u64).to_le_bytes()));
        let row = build_row_with_gen9(&gen9, "turn-1");

        // (a) `trajectory_metadata_blob` absent entirely.
        let no_table = dir.path().join("no-meta-table.db");
        write_conversation(&no_table, std::slice::from_ref(&row));

        // (b) the table and its row present, but the blob carries no created-at.
        // The workspace still decodes, so this is undecodable only in the one
        // respect that matters here.
        let no_created = dir.path().join("no-created-at.db");
        write_conversation(&no_created, std::slice::from_ref(&row));
        {
            let conn = Connection::open(&no_created).unwrap();
            conn.execute_batch("CREATE TABLE trajectory_metadata_blob (id text, data blob);")
                .unwrap();
            let workspace = enc_len(1, b"file:///home/frank/vault");
            conn.execute(
                "INSERT INTO trajectory_metadata_blob (id, data) VALUES ('main', ?1)",
                params![enc_len(1, &workspace)],
            )
            .unwrap();
        }

        for path in [&no_table, &no_created] {
            let messages = parse_antigravity_cli_file(path);
            assert_eq!(messages.len(), 1, "{}", path.display());

            let mtime = file_modified_ms(path);
            assert!(mtime > 0, "the fixture must have a positive mtime");
            assert!(
                session_window_ms(mtime)
                    .expect("a positive mtime does build a window")
                    .contains(&would_pass_under_mtime),
                "the payload has to be one an mtime-anchored window would have \
                 accepted, or this test proves nothing"
            );

            assert_ne!(
                messages[0].timestamp, would_pass_under_mtime,
                "an mtime is not a session start and must not anchor inference"
            );
            assert_eq!(
                messages[0].timestamp, mtime,
                "with no decoded created-at the row keeps its fallback dating"
            );
        }
    }

    /// Unit detection is by magnitude, which is only sound because the four
    /// unit windows do not overlap anywhere in the plausible range.
    #[test]
    fn epoch_scalar_unit_detection_is_unambiguous() {
        let seconds = recent_epoch_seconds();
        let expected = seconds * 1_000;

        assert_eq!(epoch_scalar_to_ms(seconds as u64), Some(expected));
        assert_eq!(epoch_scalar_to_ms((seconds * 1_000) as u64), Some(expected));
        assert_eq!(
            epoch_scalar_to_ms((seconds * 1_000_000) as u64),
            Some(expected)
        );
        assert_eq!(
            epoch_scalar_to_ms((seconds * 1_000_000_000) as u64),
            Some(expected)
        );

        assert_eq!(epoch_scalar_to_ms(0), None);
        assert_eq!(epoch_scalar_to_ms(u64::MAX), None);
    }

    /// The reported failure, end to end: a long-running session whose rows all
    /// use the 1.1.18 layout must date each row to its own turn instead of
    /// collapsing every turn onto the session-created date, which is what left
    /// `--today` empty. This is also the positive half of the anchor contract —
    /// `trajectory_metadata_blob` here does decode a created-at, so inference
    /// runs; see `an_mtime_only_session_anchors_no_inference` for the case where
    /// it does not.
    #[test]
    fn agy_1_1_18_rows_are_dated_per_turn_not_at_session_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-1118.db");

        let session_created_ms = 1_781_502_653_000_i64; // build_trajectory_meta
        let two_days_ago = recent_epoch_seconds() - 2 * 24 * 60 * 60;
        let now_ish = recent_epoch_seconds();

        let row = |seconds: i64, id: &str| {
            let mut gen9 = enc_varint(2, u64::MAX); // the unset sentinel
            gen9.extend(enc_len(10, &((seconds * 1_000_000) as u64).to_le_bytes()));
            build_row_with_gen9(&gen9, id)
        };

        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE gen_metadata (idx integer, data blob, size integer);
                 CREATE TABLE trajectory_metadata_blob (id text, data blob);",
            )
            .unwrap();
            for (idx, blob) in [row(two_days_ago, "turn-1"), row(now_ish, "turn-2")]
                .iter()
                .enumerate()
            {
                conn.execute(
                    "INSERT INTO gen_metadata (idx, data, size) VALUES (?1, ?2, 0)",
                    params![idx as i64, blob],
                )
                .unwrap();
            }
            conn.execute(
                "INSERT INTO trajectory_metadata_blob (id, data) VALUES ('main', ?1)",
                params![build_trajectory_meta()],
            )
            .unwrap();
        }

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].timestamp, two_days_ago * 1_000);
        assert_eq!(messages[1].timestamp, now_ish * 1_000);
        assert!(
            messages.iter().all(|m| m.timestamp != session_created_ms),
            "no row may keep the session-created stamp once #9.#10 decodes"
        );
    }

    #[test]
    fn dedupes_repeated_response_ids_and_skips_zero_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dupes.db");

        // Two rows share responseId "dup"; a third row has all-zero usage.
        let mut zero_usage = Vec::new();
        zero_usage.extend(enc_len(11, b"zero"));
        let mut zero_chat = Vec::new();
        zero_chat.extend(enc_len(4, &zero_usage));
        zero_chat.extend(enc_len(19, b"gemini-3-flash-a"));
        let zero_blob = enc_len(1, &zero_chat);

        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE gen_metadata (idx integer, data blob, size integer);")
                .unwrap();
            for (idx, blob) in [
                (0, build_gen_metadata()),
                (1, build_gen_metadata()),
                (2, zero_blob),
            ] {
                conn.execute(
                    "INSERT INTO gen_metadata (idx, data, size) VALUES (?1, ?2, 0)",
                    params![idx, blob],
                )
                .unwrap();
            }
        }

        let messages = parse_antigravity_cli_file(&path);
        // Only the first "resp-1" row survives; the duplicate and the
        // zero-usage row are dropped. Missing trajectory_metadata_blob table is
        // tolerated (timestamp falls back to file mtime).
        assert_eq!(messages.len(), 1);
        assert!(messages[0].timestamp > 0);
    }

    #[test]
    fn emitted_model_string_resolves_to_priced_alias() {
        // The parser emits the raw `#19` responseModel (`gemini-3-flash-a`) and
        // relies on the alias table to map it onto a priced model. Without the
        // alias the cost would resolve to 0, so lock the resolution here at the
        // unit level (an end-to-end calculate_cost path needs the live pricing
        // dataset, which is unavailable in unit tests).
        assert_eq!(
            pricing::aliases::resolve_alias("gemini-3-flash-a"),
            Some("gemini-3.5-flash-high")
        );
    }

    #[test]
    fn output_and_thinking_map_to_fields_9_and_10() {
        // Lock the field-mapping contract asserted by the module doc-comment:
        // `#9 + #10 == #3` (output + thinking == stored total output). Build a
        // synthetic blob where #9=output, #10=thinking, #3=output+thinking and
        // verify the parsed message keeps #9 as output and #10 as reasoning.
        let output = 300u64;
        let thinking = 40u64;
        let total_output = output + thinking; // #3

        let mut usage = Vec::new();
        usage.extend(enc_varint(1, 1132)); // fixed system prompt
        usage.extend(enc_varint(2, 500)); // new input
        usage.extend(enc_varint(3, total_output)); // stored total output (#3)
        usage.extend(enc_varint(9, output)); // output (#9)
        usage.extend(enc_varint(10, thinking)); // thinking (#10)
        usage.extend(enc_len(11, b"invariant-1"));

        let mut chat_model = Vec::new();
        chat_model.extend(enc_len(4, &usage));
        chat_model.extend(enc_len(19, b"gemini-3-flash-a"));
        let blob = enc_len(1, &chat_model);

        let mut seen = HashSet::new();
        let message = parse_isolated_row(&blob, "session", 0, &mut seen).unwrap();
        assert_eq!(message.tokens.output, output as i64);
        assert_eq!(message.tokens.reasoning, thinking as i64);
        // The contract: the two component fields sum to the stored total.
        assert_eq!(
            (message.tokens.output + message.tokens.reasoning) as u64,
            total_output
        );
    }

    #[test]
    fn malformed_blob_returns_none_without_panic() {
        let mut seen = HashSet::new();
        // Empty buffer: no chatModel sub-message.
        assert!(parse_isolated_row(&[], "s", 0, &mut seen).is_none());
        // Garbage bytes that do not form a valid wire-format message.
        assert!(parse_isolated_row(&[0xff, 0xff, 0xff, 0xff], "s", 0, &mut seen).is_none());
        // A length-delimited #1 whose declared length overruns the buffer:
        // exercises the ProtoReader bounds check (must stop, not index OOB).
        let truncated = [(1u8 << 3) | 2, 0x7f, 0x01, 0x02];
        assert!(parse_isolated_row(&truncated, "s", 0, &mut seen).is_none());
        // Valid outer #1 wrapping a #4 usage whose declared length overruns:
        // the inner reader must bail without panicking.
        let inner = [(4u8 << 3) | 2, 0x40, 0x00];
        let mut outer = vec![(1u8 << 3) | 2, inner.len() as u8];
        outer.extend_from_slice(&inner);
        assert!(parse_isolated_row(&outer, "s", 0, &mut seen).is_none());
    }

    #[test]
    fn proto_timestamp_ms_overflow_returns_none_without_panic() {
        // A malformed Timestamp can carry a `seconds` varint whose `* 1000`
        // overflows i64. Debug builds (overflow-checks = on) would panic on the
        // unchecked multiply; the decode must degrade to None instead, matching
        // the module's malformed-data contract.
        let mut overflow = Vec::new();
        overflow.extend(enc_varint(1, i64::MAX as u64)); // seconds -> *1000 overflows
        overflow.extend(enc_varint(2, 0)); // nanos
        assert_eq!(proto_timestamp_ms(&overflow), None);

        // The boundary case: largest `seconds` whose *1000 still fits i64 must
        // decode, proving the guard rejects only genuine overflow.
        let ok_seconds = i64::MAX / 1000;
        let mut ok = Vec::new();
        ok.extend(enc_varint(1, ok_seconds as u64));
        ok.extend(enc_varint(2, 0));
        assert_eq!(proto_timestamp_ms(&ok), Some(ok_seconds * 1000));

        // A normal, in-range stamp still decodes (seconds + nanos -> ms).
        let mut normal = Vec::new();
        normal.extend(enc_varint(1, 1_781_000_000));
        normal.extend(enc_varint(2, 250_000_000)); // +250ms
        assert_eq!(
            proto_timestamp_ms(&normal),
            Some(1_781_000_000 * 1000 + 250)
        );
    }

    #[test]
    fn proto_timestamp_ms_rejects_out_of_range_nanos() {
        // The protobuf Timestamp spec requires `nanos` in 0..=999_999_999.
        // An out-of-range `nanos` marks the stamp malformed (None) rather than
        // producing a skewed time. 1_000_000_000 (== one extra second) is the
        // first invalid value above the inclusive upper bound.
        let mut bad_nanos = Vec::new();
        bad_nanos.extend(enc_varint(1, 1_781_000_000)); // valid seconds
        bad_nanos.extend(enc_varint(2, 1_000_000_000)); // nanos out of range
        assert_eq!(proto_timestamp_ms(&bad_nanos), None);

        // A nanos varint large enough to be negative once cast to i64 is also
        // rejected (never wraps to a bogus negative offset).
        let mut huge_nanos = Vec::new();
        huge_nanos.extend(enc_varint(1, 1_781_000_000));
        huge_nanos.extend(enc_varint(2, u64::MAX));
        assert_eq!(proto_timestamp_ms(&huge_nanos), None);

        // The inclusive upper bound is accepted (999_999_999 ns -> +999 ms).
        let mut max_nanos = Vec::new();
        max_nanos.extend(enc_varint(1, 1_781_000_000));
        max_nanos.extend(enc_varint(2, 999_999_999));
        assert_eq!(
            proto_timestamp_ms(&max_nanos),
            Some(1_781_000_000 * 1000 + 999)
        );

        // End-to-end: a gen_metadata row whose #9.#4 carries out-of-range nanos
        // must fall back to the session-created timestamp (the caller's invalid
        // -> None -> session fallback path), not adopt a skewed per-turn stamp.
        let session_fallback = 222_000_i64;
        let mut usage = Vec::new();
        usage.extend(enc_varint(2, 500)); // input
        usage.extend(enc_varint(9, 300)); // output
        usage.extend(enc_len(11, b"bad-nanos")); // responseId

        let mut gen_time = Vec::new();
        gen_time.extend(enc_varint(1, 1_781_000_000)); // seconds
        gen_time.extend(enc_varint(2, 1_000_000_000)); // nanos out of range
        let gen9 = enc_len(4, &gen_time);

        let mut chat_model = Vec::new();
        chat_model.extend(enc_len(4, &usage));
        chat_model.extend(enc_len(9, &gen9));
        chat_model.extend(enc_len(19, b"gemini-3-flash-a"));
        let blob = enc_len(1, &chat_model);

        let mut seen = HashSet::new();
        let message = parse_isolated_row(&blob, "s", session_fallback, &mut seen).unwrap();
        assert_eq!(
            message.timestamp, session_fallback,
            "out-of-range per-generation nanos must fall back to the session timestamp"
        );
    }

    #[test]
    fn file_uri_to_path_handles_windows_posix_and_unc() {
        // Empty authority + Windows drive: drop the slash before the drive.
        assert_eq!(
            file_uri_to_path("file:///C:/Users/Frank/obsidian-vault").as_deref(),
            Some("C:/Users/Frank/obsidian-vault")
        );
        // Empty authority + POSIX absolute: keep as-is.
        assert_eq!(
            file_uri_to_path("file:///home/frank/project").as_deref(),
            Some("/home/frank/project")
        );
        // Non-empty authority is a UNC path; the host must survive as `//host`.
        assert_eq!(
            file_uri_to_path("file://server/share/code").as_deref(),
            Some("//server/share/code")
        );
        // Percent-encoded UTF-8 (CJK) decodes to valid characters.
        assert_eq!(
            file_uri_to_path("file:///D:/%E6%88%91%E7%9A%84").as_deref(),
            Some("D:/我的")
        );
        // Anything without the scheme prefix is rejected.
        assert_eq!(file_uri_to_path("not-a-file-uri"), None);
    }

    // A `gemini-default` routing-label row in a conversation whose other rows
    // carry the concrete `#19` for the same display label is resolved from that
    // sibling evidence, exactly like a row with no `#19` at all.
    #[test]
    fn routing_label_row_is_resolved_from_sibling_display_label() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routing-sibling.db");
        write_conversation(
            &path,
            &[
                build_row(
                    Some("gemini-3.5-flash-high"),
                    Some("Gemini 3.5 Flash (High)"),
                    "resp-0",
                ),
                build_row(
                    Some("gemini-default"),
                    Some("Gemini 3.5 Flash (High)"),
                    "resp-1",
                ),
            ],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].model_id, "gemini-3.5-flash-high");
        assert_eq!(messages[1].provider_id, "google");
    }

    // A routing-label row must not occupy the by_display slot for its label:
    // otherwise a sibling concrete id would read as ambiguous and the whole
    // mapping would be dropped.
    #[test]
    fn routing_label_does_not_poison_by_display_ambiguity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routing-no-poison.db");
        write_conversation(
            &path,
            &[
                build_row(
                    Some("gemini-3.6-flash"),
                    Some("Gemini 3.6 Flash (High)"),
                    "resp-0",
                ),
                build_row(
                    Some("gemini-default"),
                    Some("Gemini 3.6 Flash (High)"),
                    "resp-1",
                ),
                build_row(None, Some("Gemini 3.6 Flash (High)"), "resp-2"),
            ],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].model_id, "gemini-3.6-flash");
        assert_eq!(messages[2].model_id, "gemini-3.6-flash");
    }

    // When a routing-label row has no sibling evidence, the `#21` display label
    // itself names the tier. This is the case observed in real data: entire
    // Antigravity CLI conversations carry `gemini-default` with "Gemini 3.5
    // Flash (Low)" / "(Medium)" labels and no other `#19` at all (#1116).
    #[test]
    fn routing_label_resolved_from_display_label_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routing-display.db");
        write_conversation(
            &path,
            &[
                build_row(
                    Some("gemini-default"),
                    Some("Gemini 3.5 Flash (Low)"),
                    "resp-0",
                ),
                build_row(
                    Some("gemini-default"),
                    Some("Gemini 3.5 Flash (Medium)"),
                    "resp-1",
                ),
            ],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].model_id, "gemini-3.5-flash-extra-low");
        assert_eq!(messages[0].provider_id, "google");
        assert_eq!(messages[1].model_id, "gemini-3.5-flash-medium");
        assert_eq!(messages[1].provider_id, "google");
    }

    // A routing-label row whose display label is not in the verified map, and
    // whose file offers no sibling evidence, keeps the routing label verbatim so
    // the submission-time exclusion still applies rather than guessing.
    #[test]
    fn routing_label_without_any_evidence_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routing-unresolved.db");
        write_conversation(
            &path,
            &[build_row(
                Some("gemini-default"),
                Some("Gemini 9.9 Flash (Mystery)"),
                "resp-0",
            )],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-default");
        assert_eq!(messages[0].provider_id, "google");
    }

    // A recovered value from the display label is a concrete model id, so it
    // still flows through the alias table like any directly-read `#19`.
    #[test]
    fn routing_label_recover_still_resolves_through_the_alias_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routing-alias.db");
        write_conversation(
            &path,
            &[build_row(
                Some("gemini-default"),
                Some("Gemini 3.5 Flash (High)"),
                "resp-0",
            )],
        );

        let messages = parse_antigravity_cli_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gemini-3.5-flash-high");
        assert_eq!(messages[0].provider_id, "google");
    }
}
