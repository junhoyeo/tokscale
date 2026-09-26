//! Command Code session parser
//!
//! Parses JSONL transcripts from `~/.commandcode/projects/<slug>/<session>.jsonl`.
//!
//! Command Code's on-disk transcript (version 3) is a stream of typed JSON
//! records, one per line:
//!
//! - `{"type":"session","version":3,"id":"<session>",...}` — the header, which
//!   carries the session id that every message line also repeats.
//! - `{"type":"message","id":...,"timestamp":...,"message":{"role":...,"content":[...]},"usage":{...},"model":"provider/model"}` —
//!   conversation entries. Assistant responses carry an authoritative `usage`
//!   block (`inputTokens`, `outputTokens`, `cacheReadTokens`,
//!   `cacheWriteTokens`, `costUsd`) plus the `model` in effect for that call.
//! - `{"type":"model_change","timestamp":...,"model":"provider/model"}` —
//!   mid-session model switches.
//!
//! Unlike earlier versions of this parser (which assumed Command Code never
//! persisted usage locally and estimated tokens at ~4 chars/token from message
//! text), the v3 transcript DOES persist per-request usage and cost. When a
//! message line carries a `usage` block, its token counts and `costUsd` are
//! used verbatim and marked provider-reported, so the cost is not overwritten
//! by tokscale's estimated pricing. The usage buckets are DISJOINT:
//! `inputTokens` is cache-exclusive and `cacheReadTokens`/`cacheWriteTokens`
//! are billed on top of it — the vendor's own cost arithmetic reproduces the
//! recorded `costUsd` exactly only when the full `inputTokens` is charged at
//! the input rate plus the cache buckets, so no subtraction is applied. Lines
//! without `usage` (user turns, tool results) contribute nothing themselves —
//! the assistant turn that follows them carries the full request accounting.
//!
//! The transcript is a TREE, not a log: `/rewind` moves the leaf pointer
//! backwards and `persistEntry` is append-only, so orphaned assistant entries
//! keep their `usage` on disk but are NOT on the active branch. The vendor
//! counts only `getBranch()` (the parent chain from the last entry), and this
//! parser matches that by dropping messages whose entry id is not on the
//! active branch. `/fork`/`/clone` copies entries into a new session file
//! leaving the original; the per-entry `id` + `timestamp` dedup key collapses
//! those copies across files.
//!
//! Legacy flat-schema transcripts (`{"role":..., "content":..., ...}` with no
//! `type`/`message` nesting) are still parsed via the ~4 chars/token estimation
//! fallback, using the configured agent model from `~/.commandcode/config.json`.
//!
//! The model id is read from the line's own `model` field, falling back to the
//! most recent `model_change` event, then to `~/.commandcode/config.json` (the
//! configured agent model), then to "unknown". Gateway ids such as
//! `MiniMaxAI/MiniMax-M3-Free` have their org prefix dropped for pricing
//! (the provider hint is recovered from the RAW id before stripping), but the
//! `-Free` suffix is preserved — `poolside/laguna-s-2.1-free` is a real
//! catalog id, so stripping it would invent a nonexistent model.

use super::utils::{
    estimate_tokens, file_modified_timestamp_ms, for_each_json_line, session_id_from_path,
    workspace_key_from_path,
};
use super::{workspace_label_from_key, UnifiedMessage};
use crate::TokenBreakdown;
use serde::Deserialize;
use std::path::Path;

const CLIENT_ID: &str = "commandcode";
const PROVIDER_ID: &str = "command-code";
const UNKNOWN_MODEL: &str = "unknown";

/// One JSONL record in a Command Code v3 transcript.
///
/// Only the fields this parser needs are modeled; everything else
/// (`parentId`, `meta`, `effort`, …) is ignored by serde. The top-level
/// `role`/`content` fields exist for legacy flat-schema transcripts (see
/// `parse_commandcode_file`).
#[derive(Debug, Deserialize)]
struct CommandCodeEntry {
    #[serde(rename = "type")]
    record_type: Option<String>,
    id: Option<String>,
    #[serde(rename = "parentId")]
    parent_id: Option<String>,
    timestamp: Option<String>,
    /// The conversation entry, present on `type == "message"` records.
    message: Option<CommandCodeMessage>,
    /// Authoritative usage block, present on assistant response lines.
    usage: Option<CommandCodeUsage>,
    /// The model in effect for this record (message or model_change).
    model: Option<String>,
    /// Legacy flat-schema fields: `{"role":..., "content":..., "sessionId":...}`.
    role: Option<String>,
    content: Option<serde_json::Value>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CommandCodeMessage {
    role: Option<String>,
    content: Option<serde_json::Value>,
}

/// The camelCase usage block Command Code persists on assistant responses.
#[derive(Debug, Deserialize)]
struct CommandCodeUsage {
    #[serde(rename = "inputTokens")]
    input_tokens: Option<i64>,
    #[serde(rename = "outputTokens")]
    output_tokens: Option<i64>,
    #[serde(rename = "cacheReadTokens")]
    cache_read_tokens: Option<i64>,
    #[serde(rename = "cacheWriteTokens")]
    cache_write_tokens: Option<i64>,
    #[serde(rename = "costUsd")]
    cost_usd: Option<f64>,
}

impl CommandCodeUsage {
    /// Token breakdown with every field clamped at zero.
    ///
    /// Command Code's buckets are DISJOINT: `inputTokens` is cache-exclusive
    /// and `cacheReadTokens`/`cacheWriteTokens` are billed on top of it. The
    /// vendor's own cost function charges the full `inputTokens` at the input
    /// rate plus the cache buckets, and that arithmetic reproduces the
    /// transcript's recorded `costUsd` exactly (verified: input 28534, output
    /// 205, cacheRead 7424 at deepseek-v4-flash rates yields 0.006464748, the
    /// exact recorded value; subtracting cache from input yields 0.004831468,
    /// which does not). So the buckets are passed through verbatim, and
    /// `total()` sums them without double-counting.
    ///
    /// Returns `None` ONLY when the block reports no counts at all (every
    /// bucket field absent) — the caller's signal that it may estimate. An
    /// all-zero block, where every bucket is present and `0`, is a reported
    /// zero and must NOT be confused with an absent one: Command Code seeds
    /// its accumulator with `{inputTokens:0, outputTokens:0,
    /// cacheReadTokens:0, cacheWriteTokens:0}` and its finish handler writes
    /// `?? 0` for every field, so all-zero is exactly what the transcript
    /// records when the provider reported nothing back. Conflating the two
    /// made the caller estimate tokens out of the message text while
    /// `reported_cost()` independently returned `Some(0.0)` on the same line —
    /// invented token counts stamped provider-reported at $0.00, immune to
    /// repricing and marking the day's cost complete. Reporting the zero
    /// verbatim lets the caller drop the message instead, which is what the
    /// vendor's own accounting does (adding 0 to each bucket is a no-op).
    fn to_breakdown(&self) -> Option<TokenBreakdown> {
        // "No counts reported" is the only case the caller may estimate from.
        // At least one bucket field present means the numbers below are the
        // provider's, zeros included.
        let any_bucket_reported = self.input_tokens.is_some()
            || self.output_tokens.is_some()
            || self.cache_read_tokens.is_some()
            || self.cache_write_tokens.is_some();
        if !any_bucket_reported {
            return None;
        }
        // Provider-reported values come from an untrusted transcript; clamp to
        // a plausible ceiling so one tampered line cannot poison aggregates
        // (and so downstream sums cannot overflow). ~1e12 tokens exceeds any
        // real session by orders of magnitude, and four clamped buckets still
        // sum well inside i64.
        const TOKEN_CEILING: i64 = 1_000_000_000_000;
        let input = self.input_tokens.unwrap_or(0).clamp(0, TOKEN_CEILING);
        let output = self.output_tokens.unwrap_or(0).clamp(0, TOKEN_CEILING);
        let cache_read = self.cache_read_tokens.unwrap_or(0).clamp(0, TOKEN_CEILING);
        let cache_write = self.cache_write_tokens.unwrap_or(0).clamp(0, TOKEN_CEILING);
        Some(TokenBreakdown {
            input,
            output,
            cache_read,
            cache_write,
            cache_write_1h: 0,
            reasoning: 0,
        })
    }

    /// Authoritative cost in USD, or `None` when absent/non-finite/beyond a
    /// plausible ceiling.
    ///
    /// An explicit `0.0` IS a reported cost (free-promo models such as
    /// `MiniMax-M3-Free` bill nothing) and must stay provider-reported so the
    /// pricing lane does not reprice it at the paid rate — the same convention
    /// junie/cursor/fx pin. Negative costs are rejected (`-1` is not a real
    /// bill); so is anything above a $1M ceiling, which no single request can
    /// reach and which would otherwise flow verbatim into aggregates.
    fn reported_cost(&self) -> Option<f64> {
        const COST_CEILING: f64 = 1_000_000.0;
        self.cost_usd
            .filter(|cost| cost.is_finite() && *cost >= 0.0 && *cost <= COST_CEILING)
    }
}

#[derive(Debug, Deserialize)]
struct CommandCodeConfig {
    model: Option<String>,
}

pub fn parse_commandcode_file(path: &Path) -> Vec<UnifiedMessage> {
    // The `*.jsonl` glob also matches the per-session checkpoint log
    // (`<session>.checkpoints.jsonl`), which is a snapshot stream, not a
    // transcript. Skip it explicitly rather than relying on schema mismatch.
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".checkpoints.jsonl"))
    {
        return Vec::new();
    }

    let fallback_timestamp = file_modified_timestamp_ms(path);
    let session_id_from_path = session_id_from_path(path);
    let workspace_key = workspace_key_from_path(path);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
    // Hoisted out of the loop: the legacy fallback path reads the configured
    // agent model once per file, not once per assistant line.
    let config_model = model_from_config(path);

    let mut messages = Vec::new();
    let mut session_id: Option<String> = None;
    let mut model_id: Option<String> = None;
    // Char count of the *new* context added since the previous assistant
    // response (the user prompt plus any tool results for this turn). Only used
    // for the estimation fallback on transcripts without a `usage` block.
    let mut turn_input_chars: usize = 0;
    // Tracks whether the most recent non-assistant entry started a user turn,
    // used to mark the first assistant response of each turn.
    let mut pending_turn_start = false;
    let mut assistant_index = 0usize;
    // The v3 transcript is a tree, not a log: `/rewind` moves the leaf pointer
    // backwards and `persistEntry` is append-only, so orphaned assistant
    // entries keep their `usage` on disk but are NOT on the active branch. The
    // vendor's own stats iterate `getBranch()` (the parent chain from the last
    // entry) and never count them, so the parser must too. Track every message
    // record's `id -> parentId` plus the last entry's id; after the scan,
    // retain only messages whose entry id sits on that branch. Parallel to
    // `messages`: each emitted message's entry id (None for legacy flat-schema
    // lines, which have no tree and are always kept).
    let mut message_entry_ids: Vec<Option<String>> = Vec::new();
    let mut parent_map: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let mut last_entry_id: Option<String> = None;

    for_each_json_line(path, &mut |_index, trimmed| {
        let entry = match serde_json::from_str::<CommandCodeEntry>(trimmed) {
            Ok(entry) => entry,
            Err(_) => return,
        };

        // The session id comes from the `session` header record. A message
        // record's `id` is a per-message UUID, not a session id — never use it
        // as a fallback when the header is missing/corrupt. Legacy flat-schema
        // lines carry no header but embed a real `sessionId` field, which is
        // used when no header has been seen.
        if entry.record_type.as_deref() == Some("session") {
            if let Some(id) = entry.id.as_deref().filter(|id| !id.is_empty()) {
                session_id = Some(id.to_string());
            }
        } else if session_id.is_none() {
            if let Some(id) = entry.session_id.as_deref().filter(|id| !id.is_empty()) {
                session_id = Some(id.to_string());
            }
        }

        // Track the most-recently-seen model: `model_change` records update it
        // without emitting anything, and message records carry the model in
        // effect for that call.
        if let Some(model) = entry.model.as_deref().filter(|model| !model.is_empty()) {
            model_id = Some(model.to_string());
        }

        // Record the tree edge so the active branch can be reconstructed after
        // the scan. Every entry with an id participates; the last entry seen
        // (by file order) is the current leaf.
        if let Some(id) = entry.id.as_deref().filter(|id| !id.is_empty()) {
            // An empty-string `parentId` means the same thing as JSON `null`:
            // this entry is a root. Normalizing it here is load-bearing — the
            // walk below distinguishes "reached a root" (stop, filter) from
            // "named ancestor missing" (broken chain, do not filter), and an
            // un-normalized `""` would take the broken-chain arm and disable
            // the abandoned-branch filter for the whole file.
            let parent = entry
                .parent_id
                .as_deref()
                .filter(|parent| !parent.is_empty())
                .map(str::to_string);
            parent_map.insert(id.to_string(), parent);
            last_entry_id = Some(id.to_string());
        }

        match entry.record_type.as_deref() {
            // A `model_change` record updates the model in effect; nothing to
            // emit (handled above by the shared model tracking).
            Some("model_change") => {}
            // Everything else is a conversation record: v3 `message` entries
            // nest `role`/`content` under `message`, while legacy flat-schema
            // lines (no `type` at all) put them at the top level. `session`
            // headers fall through here too and simply produce no message.
            _ => {
                let message = match entry.message.as_ref() {
                    Some(message) => message,
                    None if entry.role.is_some() || entry.content.is_some() => {
                        &legacy_message(&entry)
                    }
                    None => return,
                };
                let Some(role) = message.role.as_deref() else {
                    return;
                };

                let chars = message.content.as_ref().map(content_chars).unwrap_or(0);

                if role == "assistant" {
                    // Prefer authoritative usage; fall back to estimating from
                    // the per-turn delta of message content ONLY when no counts
                    // were reported at all (legacy transcripts, or a `usage`
                    // block carrying no bucket fields). A reported all-zero
                    // block is authoritative and must never be estimated over
                    // — see `to_breakdown`.
                    let breakdown = entry
                        .usage
                        .as_ref()
                        .and_then(CommandCodeUsage::to_breakdown)
                        .unwrap_or_else(|| {
                            let input = estimate_tokens(turn_input_chars);
                            let output = estimate_tokens(chars);
                            TokenBreakdown {
                                input,
                                output,
                                cache_read: 0,
                                cache_write: 0,
                                cache_write_1h: 0,
                                reasoning: 0,
                            }
                        });
                    turn_input_chars = 0;

                    // Read the authoritative cost BEFORE the all-zero drop
                    // below: whether a cost was billed is what decides
                    // whether the drop applies at all.
                    let cost = entry.usage.as_ref().and_then(|u| u.reported_cost());

                    if breakdown.total() == 0 && !cost.is_some_and(|usd| usd > 0.0) {
                        // Nothing was used and nothing was billed, so there is
                        // nothing to record. This drop is load-bearing for
                        // reported zeros: it keeps an all-zero turn from
                        // emitting a $0.00 provider-reported row, which would
                        // mark the day's cost complete on a message that
                        // records no usage. It matches the vendor's
                        // accounting, where adding a zero bucket and a zero
                        // cost are both no-ops.
                        //
                        // A *positive* `costUsd` on the same all-zero line is
                        // the exception, and is why the cost is read first.
                        // Some providers bill per request rather than per
                        // token, and a client that failed to parse a usage
                        // block still records what it was charged, so zero
                        // buckets alongside real money is a shape that
                        // occurs. Dropping it would discard recorded spend,
                        // so it is emitted as a zero-token provider-reported
                        // row instead.
                        //
                        // No message is emitted, so the next real assistant
                        // message in this turn must keep its is_turn_start
                        // marker — mirroring zcode's drop path.
                        return;
                    }

                    let resolved_session = session_id
                        .clone()
                        .unwrap_or_else(|| session_id_from_path.clone());
                    // Resolve the raw model: the line's own `model` (or the most
                    // recent `model_change`), else the configured agent model,
                    // else "unknown".
                    let raw_model = match model_id {
                        Some(ref model) => model.clone(),
                        None => config_model
                            .clone()
                            .unwrap_or_else(|| UNKNOWN_MODEL.to_string()),
                    };
                    let resolved_model = canonicalize_model(&raw_model);
                    // Recover the real provider from the RAW model id (e.g.
                    // `Meta/Meta-Muse-Spark-1.1` -> `meta`) so pricing resolves
                    // to that provider's catalog. The hint must come from the
                    // full gateway id BEFORE org-stripping: `canonicalize_model`
                    // drops the org segment, and several catalog ids carry their
                    // provider only in that segment (e.g. `meta/muse-spark-1.1`),
                    // so deriving the hint from the canonicalized model loses it.
                    // The client's own `command-code` provider is not a pricing
                    // provider; falls back to it when nothing is inferred.
                    let provider_id = provider_hint_for_model(&raw_model).unwrap_or(PROVIDER_ID);
                    // The message's anchor timestamp falls back to the file
                    // mtime when the entry has none.
                    let timestamp = entry
                        .timestamp
                        .as_deref()
                        .and_then(parse_rfc3339_ms)
                        .unwrap_or(fallback_timestamp);

                    // Dedup key. A `/fork`/`/clone` copies entries — with their
                    // per-entry `id` and `timestamp` preserved — into a new
                    // session file and leaves the original, so keying on the
                    // entry id + timestamp collapses the copies across files
                    // regardless of their position in the new file. The
                    // vendor's entry id is `randomUUID().slice(0,8)` (32 bits,
                    // unique only within one session), so a bare id is NOT a
                    // safe global key — combining it with the timestamp makes
                    // a collision across genuinely distinct entries effectively
                    // impossible while still matching a fork copy byte-for-byte.
                    //
                    // The key uses the entry's OWN parsed timestamp, not the
                    // message anchor: a missing/invalid timestamp falls back to
                    // the per-file mtime, which differs between the original and
                    // a forked copy and would break dedup. Absent/invalid
                    // timestamps use a stable "missing" sentinel instead, which
                    // a fork preserves verbatim.
                    //
                    // Legacy flat-schema lines carry no entry id and fall back
                    // to the session-scoped positional key.
                    let dedup_timestamp = entry
                        .timestamp
                        .as_deref()
                        .and_then(parse_rfc3339_ms)
                        .unwrap_or(-1);
                    let dedup_key = entry
                        .id
                        .as_deref()
                        .map(|id| format!("{}:{}", id, dedup_timestamp));
                    let mut message = UnifiedMessage::new_with_dedup(
                        CLIENT_ID,
                        resolved_model,
                        provider_id,
                        resolved_session.clone(),
                        timestamp,
                        breakdown,
                        cost.unwrap_or(0.0),
                        dedup_key.or_else(|| {
                            // Positional fallback for legacy flat-schema
                            // lines, which carry no entry id.
                            // `resolved_session` can itself fall back to the
                            // file stem, and two projects can hold files with
                            // the same stem, so the workspace is folded in to
                            // keep the key unique across directories. This
                            // key only has to be unique: a legacy `/fork`
                            // lands in a differently-named file, so unlike
                            // the id-based key it never collapses copies.
                            Some(format!(
                                "{}:{}:{}",
                                workspace_key.as_deref().unwrap_or("-"),
                                resolved_session,
                                assistant_index
                            ))
                        }),
                    );
                    message.message_count = 1;
                    message.is_turn_start = pending_turn_start;
                    message.set_workspace(workspace_key.clone(), workspace_label.clone());
                    if cost.is_some() {
                        message.mark_provider_reported_cost();
                    }
                    message_entry_ids.push(entry.id.clone());
                    messages.push(message);

                    assistant_index += 1;
                    pending_turn_start = false;
                } else if role == "user" {
                    // A tool result is delivered under `role: "user"` but is a
                    // continuation of the current turn, not a new prompt — it
                    // must not start a new turn. Only a genuine user text
                    // prompt does.
                    if !is_tool_result(&message.content) {
                        pending_turn_start = true;
                    }
                    turn_input_chars += chars;
                } else {
                    // Tool results (and any other roles) are part of the new
                    // context the model sees on the next turn.
                    turn_input_chars += chars;
                }
            }
        }
    });

    // Reconstruct the active branch from the last entry's parent chain and drop
    // messages whose entry id is not on it (abandoned `/rewind` branches). A
    // cycle guard bounds the walk; legacy flat-schema messages (None entry id)
    // are always kept.
    if let Some(branch) = active_branch_ids(&parent_map, last_entry_id.as_deref()) {
        let mut kept = Vec::with_capacity(messages.len());
        for (message, entry_id) in messages.into_iter().zip(message_entry_ids) {
            if entry_id.as_deref().is_none_or(|id| branch.contains(id)) {
                kept.push(message);
            }
        }
        kept
    } else {
        messages
    }
}

/// The set of entry ids on the active branch of a transcript tree: the parent
/// chain starting at `last_entry_id` walking `parent_id` links, including the
/// leaf itself.
///
/// Returns `None` when the branch cannot be reconstructed, in which case the
/// caller keeps every message rather than filtering against a partial chain:
///
/// * `last_entry_id` is absent — no tree at all (a legacy flat-schema
///   transcript).
/// * The chain names an ancestor this file does not contain. That happens
///   whenever a line between the leaf and the root was skipped: one
///   unparseable line (a lossy-UTF-8 byte, a partially-flushed append from a
///   crash, a field whose type drifted between vendor versions) removes its
///   `id` from the map, and the walk hits the hole. Filtering on the truncated
///   chain would silently discard every turn recorded BEFORE that line —
///   arbitrarily much real, already-billed usage — so the filter is refused
///   instead. This deliberately diverges from the vendor's `buildSessionPath`,
///   which truncates at the same hole: truncation is cosmetic when rendering a
///   conversation and is lost money when accounting for one. The cost of
///   failing open is re-counting an abandoned `/rewind` branch in that one
///   file, which is bounded; the cost of failing closed is not.
///
/// The walk is bounded by the number of entries in the map so a malformed
/// (cyclic) `parentId` chain cannot loop forever.
fn active_branch_ids(
    parent_map: &std::collections::HashMap<String, Option<String>>,
    last_entry_id: Option<&str>,
) -> Option<std::collections::HashSet<String>> {
    let mut branch = std::collections::HashSet::new();
    let mut current = last_entry_id.map(str::to_string)?;
    loop {
        if !branch.insert(current.clone()) {
            // Cycle: we have seen this id already.
            return Some(branch);
        }
        match parent_map.get(&current) {
            Some(Some(parent)) => current = parent.clone(),
            // A real root (`parentId` null or empty, normalized at insertion).
            // The chain is complete, so the branch is trustworthy.
            Some(None) => return Some(branch),
            // The named ancestor is not in this file: the chain is broken and
            // everything past the hole is unreachable from the leaf.
            None => return None,
        }
    }
}

/// Adapt a legacy flat-schema record (`{"role":..., "content":..., ...}` with
/// no `type`/`message` nesting) to the nested-message shape the parser shares
/// with v3 records.
fn legacy_message(entry: &CommandCodeEntry) -> CommandCodeMessage {
    CommandCodeMessage {
        role: entry.role.clone(),
        content: entry.content.clone(),
    }
}

/// Whether a user-role content block is a tool result (a continuation of the
/// current turn) rather than a genuine new user prompt.
///
/// Command Code delivers tool results under `role: "user"` with content blocks
/// of `type: "tool_result"`, so a plain role check would start a new turn for
/// every tool result. The content block's type is decisive: a real prompt
/// carries `type: "text"` (or a string/object form in older transcripts).
fn is_tool_result(content: &Option<serde_json::Value>) -> bool {
    match content {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(serde_json::Value::as_object)
            .any(|block| {
                block.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
            }),
        // A single bare `{"type":"tool_result",...}` object (not array-wrapped).
        Some(serde_json::Value::Object(map)) => {
            map.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
        }
        _ => false,
    }
}

/// Char count of a message's `content` for token estimation, measured from its
/// canonical JSON serialization. Counting the serialized form keeps every
/// prompt-bearing byte the model receives — object keys (`command`, `path`, …),
/// tool-call arguments, tool-result payloads, and numeric/boolean values — and
/// avoids guessing which fields are structural versus content.
///
/// Genuinely empty content (null, `""`, `[]`, `{}`) counts as zero so that
/// contentless turns are not charged for their structural brackets.
fn content_chars(content: &serde_json::Value) -> usize {
    match content {
        serde_json::Value::Null => 0,
        serde_json::Value::String(s) if s.is_empty() => 0,
        serde_json::Value::Array(items) if items.is_empty() => 0,
        serde_json::Value::Object(map) if map.is_empty() => 0,
        _ => serde_json::to_string(content)
            .map(|serialized| serialized.chars().count())
            .unwrap_or(0),
    }
}

/// Canonicalize the model id for pricing. Command Code reports gateway ids such
/// as `MiniMaxAI/MiniMax-M3-Free`; the org prefix is not a key tokscale's
/// pricing resolver recognizes verbatim, so dropping the org segment yields the
/// model id (e.g. `MiniMax-M3-Free`) that pricing keys are matched against.
/// The provider hint that the org segment carried (e.g. `minimax`) is recovered
/// separately by [`provider_hint_for_model`] from the RAW id and applied to
/// `provider_id`.
///
/// No `-free` suffix stripping happens here: `poolside/laguna-s-2.1-free` is a
/// real catalog id, so stripping the suffix would invent a model that does not
/// exist. Whatever the id says is what pricing must match.
fn canonicalize_model(model: &str) -> String {
    model.rsplit('/').next().unwrap_or(model).to_string()
}

/// Recover the provider hint that the model id carries (e.g.
/// `MiniMaxAI/MiniMax-M3-Free` -> `minimax`) so pricing resolves to the real
/// provider's catalog. Command Code's own `command-code` provider id is not a
/// pricing provider, so without this hint a MiniMax model would never reach a
/// `minimax/...` pricing key. Returns `None` when no known provider can be
/// inferred, leaving the default `command-code` provider in place.
fn provider_hint_for_model(model: &str) -> Option<&'static str> {
    crate::provider_identity::inferred_provider_from_model(model)
}

fn parse_rfc3339_ms(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Read the configured agent model from `~/.commandcode/config.json`.
///
/// `session_path` is normally `<root>/.commandcode/projects/<slug>/<session>.jsonl`,
/// but the scanner walks `projects` recursively, so a transcript can sit at any
/// depth. Count-parents guessing would land on a *different* `config.json` for
/// nested paths (silently misattributing the model). Instead, walk up until the
/// parent directory is named `projects` — the Command Code root is one level
/// above it — and read `<root>/config.json` from there. Returns `None` when no
/// ancestor named `projects` is found (so the caller's "unknown" fallback
/// applies).
fn model_from_config(session_path: &Path) -> Option<String> {
    let mut dir = session_path.parent()?;
    // Find the `projects` directory that anchors the Command Code root.
    loop {
        if dir.file_name().and_then(|name| name.to_str()) == Some("projects") {
            break;
        }
        dir = dir.parent()?;
    }
    let commandcode_root = dir.parent()?;
    let config_path = commandcode_root.join("config.json");
    let bytes = std::fs::read(config_path).ok()?;
    let config: CommandCodeConfig = serde_json::from_slice(&bytes).ok()?;
    config.model.filter(|model| !model.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_session(dir: &TempDir, slug: &str, session: &str, jsonl: &str) -> std::path::PathBuf {
        let project_dir = dir.path().join("projects").join(slug);
        std::fs::create_dir_all(&project_dir).unwrap();
        let path = project_dir.join(format!("{session}.jsonl"));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(jsonl.as_bytes()).unwrap();
        path
    }

    fn write_config(dir: &TempDir, model: &str) {
        let path = dir.path().join("config.json");
        let mut file = std::fs::File::create(&path).unwrap();
        write!(file, r#"{{"provider":"command-code","model":"{model}"}}"#).unwrap();
    }

    /// Build a realistic v3 transcript line for an assistant response carrying
    /// authoritative usage and cost.
    // A fixture builder whose parameters mirror the transcript's own fields;
    // grouping them into a struct would only move the same list one level out.
    // Same treatment as `zed::tests::insert_thread`.
    #[allow(clippy::too_many_arguments)]
    fn assistant_line(
        id: &str,
        parent: &str,
        timestamp: &str,
        model: &str,
        text: &str,
        input: i64,
        output: i64,
        cache_read: i64,
        cost_usd: f64,
    ) -> String {
        json!({
            "type": "message",
            "id": id,
            "parentId": parent,
            "timestamp": timestamp,
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": text}]
            },
            "usage": {
                "inputTokens": input,
                "outputTokens": output,
                "cacheReadTokens": cache_read,
                "cacheWriteTokens": 0,
                "costUsd": cost_usd
            },
            "model": model
        })
        .to_string()
    }

    fn user_line(id: &str, parent: &str, timestamp: &str, text: &str) -> String {
        json!({
            "type": "message",
            "id": id,
            "parentId": parent,
            "timestamp": timestamp,
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": text}]
            }
        })
        .to_string()
    }

    /// A tool-result record, as Command Code writes it after a tool call.
    fn tool_result_line(
        id: &str,
        parent: &str,
        timestamp: &str,
        tool_use_id: &str,
        text: &str,
    ) -> String {
        json!({
            "type": "message",
            "id": id,
            "parentId": parent,
            "timestamp": timestamp,
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": [{"type": "text", "text": text}]
                }]
            }
        })
        .to_string()
    }

    /// Full realistic v3 session: header, model_change, user turn, tool call +
    /// tool result, then the assistant response carrying authoritative usage.
    /// Every entry chains its `parentId` to the previous one, as real
    /// transcripts do.
    #[test]
    fn test_parse_realistic_v3_session_with_tool_turns() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "deepseek/deepseek-v4-flash");
        let jsonl = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "sess-1", "timestamp": "2026-08-31T04:36:38.441Z", "cwd": "/home/al/learning"}),
            json!({"type": "model_change", "id": "m1", "parentId": null, "timestamp": "2026-08-31T04:37:00.000Z", "model": "deepseek/deepseek-v4-flash"}),
            user_line("u1", "m1", "2026-08-31T04:39:16.867Z", "list the repo"),
            json!({
                "type": "message",
                "id": "t1",
                "parentId": "u1",
                "timestamp": "2026-08-31T04:39:18.000Z",
                "message": {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "call_00_abc",
                        "name": "read_directory",
                        "input": {"path": "/home/al/learning"}
                    }],
                    "meta": {"source": "model"}
                },
                "usage": {
                    "inputTokens": 21000,
                    "outputTokens": 90,
                    "cacheReadTokens": 4000,
                    "cacheWriteTokens": 0,
                    "costUsd": 0.002
                },
                "model": "deepseek/deepseek-v4-flash"
            }),
            tool_result_line(
                "r1",
                "t1",
                "2026-08-31T04:39:19.000Z",
                "call_00_abc",
                "Found 28 items"
            ),
            assistant_line(
                "a1",
                "r1",
                "2026-08-31T04:39:20.351Z",
                "deepseek/deepseek-v4-flash",
                "There are 28 items.",
                28534,
                205,
                7424,
                0.006464748
            ),
        );
        let path = write_session(&dir, "home-al-learning", "sess-1", &jsonl);

        let messages = parse_commandcode_file(&path);

        // Two assistant records: the tool_use call (turn start) and the final
        // text response. Both carry authoritative usage, all on the active
        // branch (the chain ends at a1 -> r1 -> t1 -> u1 -> m1).
        assert_eq!(messages.len(), 2);
        let tool_call = &messages[0];
        assert_eq!(tool_call.model_id, "deepseek-v4-flash");
        assert_eq!(tool_call.provider_id, "deepseek");
        // Disjoint buckets: input is the full inputTokens, cache on top.
        assert_eq!(tool_call.tokens.input, 21000);
        assert_eq!(tool_call.tokens.output, 90);
        assert_eq!(tool_call.tokens.cache_read, 4000);
        assert!((tool_call.cost - 0.002).abs() < 1e-9);
        assert!(tool_call.has_authoritative_cost());
        assert!(tool_call.is_turn_start);
        assert_eq!(tool_call.dedup_key.as_deref(), Some("t1:1788151158000"));

        let msg = &messages[1];
        assert_eq!(msg.model_id, "deepseek-v4-flash");
        assert_eq!(msg.provider_id, "deepseek");
        assert_eq!(msg.session_id, "sess-1");
        // Disjoint buckets: full inputTokens (28534) + output (205), cache on
        // top, NOT subtracted.
        assert_eq!(msg.tokens.input, 28534);
        assert_eq!(msg.tokens.output, 205);
        assert_eq!(msg.tokens.cache_read, 7424);
        assert!((msg.cost - 0.006464748).abs() < 1e-9);
        assert!(msg.has_authoritative_cost());
        assert!(!msg.is_turn_start);
        assert_eq!(msg.dedup_key.as_deref(), Some("a1:1788151160351"));
    }

    /// A `/rewind` abandons a branch: orphaned assistant entries keep their
    /// `usage` on disk but are NOT on the active parent chain, and must not be
    /// counted — the vendor's `getBranch()` never sees them.
    #[test]
    fn test_abandoned_rewind_branch_entries_are_dropped() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        // The active branch is: u1 -> a2 (the last entry). a1 is an orphaned
        // assistant response from a `/rewind` — its usage must not count.
        // u1's `parentId` is `""`, the empty-string form of a root; the walk
        // must terminate there through the ROOT arm, not through the
        // broken-chain arm (which would refuse to filter and keep a1).
        let jsonl = format!(
            "{}\n{}\n{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", "original prompt"),
            assistant_line(
                "a1",
                "u1",
                "2026-08-31T00:00:05Z",
                "model-x",
                "first answer (abandoned by /rewind)",
                5000,
                200,
                1000,
                0.01
            ),
            user_line("u2", "u1", "2026-08-31T00:00:10Z", "actually, redo it"),
            assistant_line(
                "a2",
                "u2",
                "2026-08-31T00:00:15Z",
                "model-x",
                "the real final answer",
                6000,
                300,
                2000,
                0.02
            ),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1, "orphaned branch entries must be dropped");
        let msg = &messages[0];
        assert_eq!(msg.model_id, "model-x");
        assert_eq!(msg.tokens.input, 6000);
        assert_eq!(msg.tokens.output, 300);
        assert_eq!(msg.tokens.cache_read, 2000);
        assert!((msg.cost - 0.02).abs() < 1e-9);
        assert!(msg.is_turn_start);
    }

    /// One unparseable line must cost that line, not the file's history.
    ///
    /// Regression guard. A skipped line's `id` never enters the parent map, so
    /// the branch walk from the leaf hits a hole. The walk used to treat a
    /// missing ancestor exactly like a root — returning a chain truncated at
    /// the hole — and the retain filter then dropped every turn recorded
    /// BEFORE the bad line. On a long session that is almost the whole file:
    /// real, already-billed usage deleted because of one damaged byte.
    #[test]
    fn test_unparseable_midfile_line_does_not_discard_earlier_turns() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        // Line 4 is a truncated append (a crash mid-write, or one undecodable
        // byte that `String::from_utf8_lossy` turned into U+FFFD). It would
        // have been `u2`, which `a2` names as its parent.
        let jsonl = format!(
            "{}\n{}\n{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", "first prompt"),
            assistant_line(
                "a1",
                "u1",
                "2026-08-31T00:00:05Z",
                "model-x",
                "first answer",
                1000,
                100,
                0,
                0.01
            ),
            r#"{"type":"message","id":"u2","parentId":"a1","timestamp":"2026-08-3"#,
            assistant_line(
                "a2",
                "u2",
                "2026-08-31T00:00:15Z",
                "model-x",
                "second answer",
                2000,
                200,
                0,
                0.02
            ),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(
            messages.len(),
            2,
            "the turn before the damaged line must survive it"
        );
        assert_eq!(
            messages.iter().map(|m| m.tokens.input).sum::<i64>(),
            3000,
            "both turns' authoritative input tokens must be kept"
        );
        assert!(
            (messages.iter().map(|m| m.cost).sum::<f64>() - 0.03).abs() < 1e-9,
            "both turns' authoritative cost must be kept"
        );
    }

    /// Refusing to filter on a BROKEN chain must not become refusing to filter
    /// at all: when the chain reaches a genuine `parentId: null` root, the
    /// abandoned `/rewind` branch is still dropped.
    #[test]
    fn test_null_root_chain_still_drops_abandoned_branch() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let jsonl = format!(
            "{}\n{}\n{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            json!({
                "type": "message",
                "id": "u1",
                "parentId": null,
                "timestamp": "2026-08-31T00:00:00Z",
                "message": {"role": "user", "content": [{"type": "text", "text": "original"}]}
            }),
            assistant_line(
                "a1",
                "u1",
                "2026-08-31T00:00:05Z",
                "model-x",
                "abandoned by /rewind",
                5000,
                200,
                1000,
                0.01
            ),
            user_line("u2", "u1", "2026-08-31T00:00:10Z", "actually, redo it"),
            assistant_line(
                "a2",
                "u2",
                "2026-08-31T00:00:15Z",
                "model-x",
                "the real final answer",
                6000,
                300,
                2000,
                0.02
            ),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(
            messages.len(),
            1,
            "a complete chain to a null root must still drop the abandoned branch"
        );
        assert_eq!(messages[0].tokens.input, 6000);
        assert!((messages[0].cost - 0.02).abs() < 1e-9);
    }

    /// A `/fork` copies entries (same `id` + `timestamp`) into a new session
    /// file leaving the original. The dedup key must be identical for both so
    /// `should_keep_deduped_message` collapses them across files — regardless
    /// of position in the copied file.
    #[test]
    fn test_fork_copies_share_dedup_key() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        // Original session: u1 -> a1 (the last entry).
        let original = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "orig-sess"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", "prompt"),
            assistant_line(
                "a1",
                "u1",
                "2026-08-31T00:00:05Z",
                "model-x",
                "answer",
                100,
                10,
                0,
                0.001
            ),
        );
        // Fork: same entries, but a NEW session id and the assistant copied at
        // a DIFFERENT position (an extra user turn precedes the copy).
        let fork = format!(
            "{}\n{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "fork-sess"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", "prompt"),
            user_line("u2", "u1", "2026-08-31T00:00:02Z", "extra turn"),
            assistant_line(
                "a1",
                "u2",
                "2026-08-31T00:00:05Z",
                "model-x",
                "answer",
                100,
                10,
                0,
                0.001
            ),
        );

        let orig_path = write_session(&dir, "proj", "orig", &original);
        let fork_path = write_session(&dir, "proj", "fork", &fork);

        // Parse both files into one merged stream, collapsing duplicates by
        // dedup key the same way the lib.rs lane's should_keep_deduped_message
        // does.
        let mut seen = std::collections::HashSet::new();
        let mut merged: Vec<UnifiedMessage> = Vec::new();
        for path in [orig_path, fork_path] {
            for msg in parse_commandcode_file(&path) {
                let keep = msg
                    .dedup_key
                    .as_ref()
                    .is_none_or(|key| seen.insert(key.clone()));
                if keep {
                    merged.push(msg);
                }
            }
        }

        // The assistant entry appears in both files; it must be counted once.
        let assistant_msgs: Vec<_> = merged.iter().filter(|m| m.model_id == "model-x").collect();
        assert_eq!(
            assistant_msgs.len(),
            1,
            "fork copy of the same entry must collapse to one message"
        );
        assert_eq!(assistant_msgs[0].tokens.input, 100);
    }

    /// A fork copy whose entries have NO timestamp (or an unparseable one)
    /// must still dedup: the key uses a stable sentinel, not the per-file
    /// mtime, so the original and the copy agree.
    #[test]
    fn test_fork_with_missing_timestamp_still_dedups() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        // Both files carry the same assistant entry with NO timestamp field.
        // The files' mtimes differ, but the dedup key must not depend on them.
        let make_session = |session_id: &str| {
            format!(
                "{}\n{}\n{}",
                json!({"type": "session", "version": 3, "id": session_id}),
                json!({
                    "type": "message",
                    "id": "u1",
                    "parentId": null,
                    "message": {"role": "user", "content": [{"type": "text", "text": "prompt"}]}
                }),
                json!({
                    "type": "message",
                    "id": "a1",
                    "parentId": "u1",
                    "message": {"role": "assistant", "content": [{"type": "text", "text": "answer"}]},
                    "usage": {
                        "inputTokens": 100,
                        "outputTokens": 10,
                        "cacheReadTokens": 0,
                        "cacheWriteTokens": 0,
                        "costUsd": 0.001
                    },
                    "model": "model-x"
                }),
            )
        };
        let original = make_session("orig-sess");
        // Write the fork AFTER a small delay so its mtime definitely differs.
        let orig_path = write_session(&dir, "proj", "orig", &original);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let fork = make_session("fork-sess");
        let fork_path = write_session(&dir, "proj", "fork", &fork);

        let mut seen = std::collections::HashSet::new();
        let mut merged: Vec<UnifiedMessage> = Vec::new();
        for path in [orig_path, fork_path] {
            for msg in parse_commandcode_file(&path) {
                let keep = msg
                    .dedup_key
                    .as_ref()
                    .is_none_or(|key| seen.insert(key.clone()));
                if keep {
                    merged.push(msg);
                }
            }
        }

        let assistant_msgs: Vec<_> = merged.iter().filter(|m| m.model_id == "model-x").collect();
        assert_eq!(
            assistant_msgs.len(),
            1,
            "fork copy without timestamps must still collapse to one message"
        );
        assert_eq!(assistant_msgs[0].tokens.input, 100);
    }

    #[test]
    fn test_parse_v3_transcript_with_authoritative_usage() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "deepseek/deepseek-v4-flash");
        let jsonl = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "sess-1", "cwd": "/home/al/learning"}),
            user_line("u1", "", "2026-08-31T04:39:16.867Z", "hello"),
            assistant_line(
                "a1",
                "u1",
                "2026-08-31T04:39:20.351Z",
                "deepseek/deepseek-v4-flash",
                "hi there",
                28534,
                205,
                7424,
                0.006464748
            ),
        );
        let path = write_session(&dir, "home-al-learning", "sess-1", &jsonl);

        let messages = parse_commandcode_file(&path);

        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.client, "commandcode");
        assert_eq!(msg.provider_id, "deepseek");
        assert_eq!(msg.model_id, "deepseek-v4-flash");
        assert_eq!(msg.session_id, "sess-1");
        // Authoritative counts, not estimates. Buckets are DISJOINT: input is
        // the full inputTokens, cache on top (verified: charging full input +
        // cache at deepseek-v4-flash rates reproduces the recorded costUsd
        // exactly; subtracting cache does not).
        assert_eq!(msg.tokens.input, 28534);
        assert_eq!(msg.tokens.output, 205);
        assert_eq!(msg.tokens.cache_read, 7424);
        assert_eq!(msg.tokens.cache_write, 0);
        assert_eq!(msg.tokens.reasoning, 0);
        assert_eq!(msg.tokens.total(), 36163); // 28534 + 205 + 7424
                                               // Authoritative cost is embedded and marked provider-reported.
        assert!((msg.cost - 0.006464748).abs() < 1e-9);
        assert!(msg.has_authoritative_cost());
        assert!(msg.is_turn_start);
        assert_eq!(msg.message_count, 1);
        assert_eq!(msg.timestamp, 1788151160351); // 2026-08-31T04:39:20.351Z
        assert_eq!(msg.workspace_key.as_deref(), Some("home-al-learning"));
    }

    #[test]
    fn test_parse_v3_transcript_model_change_updates_model() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "deepseek/deepseek-v4-flash");
        let jsonl = format!(
            "{}\n{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "sess-1"}),
            json!({"type": "model_change", "id": "m1", "parentId": null, "timestamp": "2026-08-31T03:32:29.603Z", "model": "minimax/minimax-m3-free"}),
            user_line("u1", "m1", "2026-08-31T03:33:00.000Z", "first"),
            assistant_line(
                "a1",
                "u1",
                "2026-08-31T03:33:05.000Z",
                "minimax/minimax-m3-free",
                "resp one",
                100,
                10,
                0,
                0.001
            ),
        );
        let path = write_session(&dir, "proj", "sess-1", &jsonl);

        let messages = parse_commandcode_file(&path);

        assert_eq!(messages.len(), 1);
        // The model in effect at the assistant line is the model_change one.
        // `-free` is NOT stripped: `poolside/laguna-s-2.1-free` is a real
        // catalog id, so the suffix is preserved for pricing.
        assert_eq!(messages[0].model_id, "minimax-m3-free");
        assert_eq!(messages[0].provider_id, "minimax");
    }

    #[test]
    fn test_canonicalize_model_strips_org_prefix_only() {
        assert_eq!(
            canonicalize_model("MiniMaxAI/MiniMax-M3-Free"),
            "MiniMax-M3-Free"
        );
        assert_eq!(
            canonicalize_model("minimaxai/minimax-m3-free"),
            "minimax-m3-free"
        );
        assert_eq!(canonicalize_model("MiniMaxAI/MiniMax-M2.5"), "MiniMax-M2.5");
        assert_eq!(canonicalize_model("taste-1"), "taste-1");
        // `-free` is a real catalog suffix (poolside/laguna-s-2.1-free), so it
        // must NOT be stripped.
        assert_eq!(
            canonicalize_model("poolside/laguna-s-2.1-free"),
            "laguna-s-2.1-free"
        );
    }

    #[test]
    fn test_canonicalize_model_does_not_panic_on_non_ascii() {
        assert_eq!(canonicalize_model("vendor/modèle"), "modèle");
        assert_eq!(canonicalize_model("café-🚀"), "café-🚀");
        assert_eq!(canonicalize_model("café-free"), "café-free");
    }

    #[test]
    fn test_content_chars_counts_keys_numbers_and_nested_payloads() {
        assert!(content_chars(&json!([{"value": 12345}])) > 0);
        let small = content_chars(&json!([{"a": "x"}]));
        let large = content_chars(&json!([{"command": "run", "args": ["a", "b"], "n": 42}]));
        assert!(large > small);
    }

    #[test]
    fn test_checkpoint_files_are_skipped() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let project_dir = dir.path().join("projects").join("proj");
        std::fs::create_dir_all(&project_dir).unwrap();
        let path = project_dir.join("s.checkpoints.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(
            br#"{"type":"checkpoint","messageId":"m","snapshot":"snap","isSnapshotUpdate":false}"#,
        )
        .unwrap();

        let messages = parse_commandcode_file(&path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_missing_config_and_model_falls_back_to_unknown_model() {
        let dir = TempDir::new().unwrap();
        let jsonl = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", "hello"),
            assistant_line(
                "a1",
                "u1",
                "2026-08-31T00:00:05Z",
                "",
                "world",
                10,
                5,
                0,
                0.0
            ),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "unknown");
        assert_eq!(messages[0].provider_id, "command-code");
    }

    #[test]
    fn test_skips_malformed_lines_without_panicking() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let jsonl = concat!(
            "not valid json at all\n",
            r#"{"type":"session","version":3,"id":"s"}"#,
            "\n",
        );
        let path = write_session(&dir, "proj", "s", jsonl);

        let messages = parse_commandcode_file(&path);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_estimated_fallback_when_usage_absent() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let jsonl = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            json!({
                "type": "message",
                "id": "u1",
                "parentId": null,
                "timestamp": "2026-08-31T00:00:00Z",
                "message": {"role": "user", "content": [{"type": "text", "text": "12345678"}]}
            }),
            json!({
                "type": "message",
                "id": "a1",
                "parentId": "u1",
                "timestamp": "2026-08-31T00:00:05Z",
                "message": {"role": "assistant", "content": [{"type": "text", "text": "abcd"}]},
                "model": "model-x"
            }),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        // Estimated: input from the user turn's content, output from assistant.
        assert_eq!(
            msg.tokens.input,
            estimate_tokens(content_chars(
                &json!([{"type": "text", "text": "12345678"}])
            ))
        );
        assert_eq!(
            msg.tokens.output,
            estimate_tokens(content_chars(&json!([{"type": "text", "text": "abcd"}])))
        );
        assert_eq!(msg.cost, 0.0);
        assert!(!msg.has_authoritative_cost());
        assert!(msg.is_turn_start);
    }

    /// An explicit `costUsd: 0` is a real reported cost (free-promo models bill
    /// nothing) and must stay provider-reported — otherwise the pricing lane
    /// reprices the call at the paid rate. Mirrors the junie/cursor/fx
    /// convention.
    #[test]
    fn test_explicit_zero_cost_is_provider_reported() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "minimax/minimax-m3-free");
        let jsonl = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", "hi"),
            json!({
                "type": "message",
                "id": "a1",
                "parentId": "u1",
                "timestamp": "2026-08-31T00:00:05Z",
                "message": {"role": "assistant", "content": [{"type": "text", "text": "yo"}]},
                "usage": {
                    "inputTokens": 100,
                    "outputTokens": 10,
                    "cacheReadTokens": 0,
                    "cacheWriteTokens": 0,
                    "costUsd": 0
                },
                "model": "minimax/minimax-m3-free"
            }),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.cost, 0.0);
        assert!(
            msg.has_authoritative_cost(),
            "an explicit zero cost is provider-reported, not repriced"
        );
        // Org prefix stripped; `-free` suffix preserved (real catalog id).
        assert_eq!(msg.model_id, "minimax-m3-free");
    }

    /// The usage buckets are DISJOINT: `inputTokens` is cache-exclusive and the
    /// cache buckets are billed on top, so `total()` sums them without any
    /// subtraction (verified against the recorded `costUsd` arithmetic).
    #[test]
    fn test_usage_buckets_are_disjoint() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let jsonl = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", "hi"),
            json!({
                "type": "message",
                "id": "a1",
                "parentId": "u1",
                "timestamp": "2026-08-31T00:00:05Z",
                "message": {"role": "assistant", "content": [{"type": "text", "text": "yo"}]},
                "usage": {
                    "inputTokens": 1000,
                    "outputTokens": 50,
                    "cacheReadTokens": 900,
                    "cacheWriteTokens": 10,
                    "costUsd": 0.01
                },
                "model": "model-x"
            }),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        // Full inputTokens passed through verbatim; cache on top.
        assert_eq!(msg.tokens.input, 1000);
        assert_eq!(msg.tokens.cache_read, 900);
        assert_eq!(msg.tokens.cache_write, 10);
        // total() is the sum of all buckets, no double-count and no subtraction.
        assert_eq!(msg.tokens.total(), 1960);
    }

    /// An all-zero `usage` block is a REPORTED zero, not an absent one:
    /// Command Code seeds its accumulator with all four buckets at `0` and its
    /// finish handler writes `?? 0` for every field, so this is exactly what a
    /// transcript records when the provider reported nothing back — and the
    /// same line carries `costUsd: 0`.
    ///
    /// Regression guard. The parser used to read an all-zero block as "no
    /// usage", estimate ~1000 tokens out of the message text, and then stamp
    /// those invented tokens provider-reported at $0.00 (because
    /// `reported_cost()` accepts an explicit zero, independently of the
    /// buckets). Numbers nobody measured, published as authoritative, immune
    /// to repricing, and marking the day's cost complete.
    #[test]
    fn test_all_zero_usage_block_is_not_estimated_or_marked_provider_reported() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        // Long enough that the old estimation fallback produced a four-digit
        // token count — the fabricated number this guards against.
        let long_prompt = "x".repeat(4000);
        let long_answer = "y".repeat(4000);
        let zero_usage_line = |cost: serde_json::Value| {
            json!({
                "type": "message",
                "id": "a1",
                "parentId": "u1",
                "timestamp": "2026-08-31T00:00:05Z",
                "message": {"role": "assistant", "content": [{"type": "text", "text": long_answer}]},
                "usage": {
                    "inputTokens": 0,
                    "outputTokens": 0,
                    "cacheReadTokens": 0,
                    "cacheWriteTokens": 0,
                    "costUsd": cost
                },
                "model": "model-x"
            })
            .to_string()
        };

        // The dangerous shape: all four buckets zero AND an explicit zero cost.
        let with_zero_cost = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", &long_prompt),
            zero_usage_line(json!(0)),
        );
        let path = write_session(&dir, "proj", "zero-cost", &with_zero_cost);
        let messages = parse_commandcode_file(&path);
        assert!(
            messages.is_empty(),
            "a reported all-zero turn must emit nothing, not estimated tokens; got {:?}",
            messages
                .iter()
                .map(|m| (m.tokens.total(), m.cost, m.has_authoritative_cost()))
                .collect::<Vec<_>>()
        );

        // Same block with no `costUsd` at all: still authoritative zeros, so
        // the drop must not depend on a cost being present.
        let without_cost = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", &long_prompt),
            zero_usage_line(serde_json::Value::Null),
        );
        let path = write_session(&dir, "proj", "no-cost", &without_cost);
        let messages = parse_commandcode_file(&path);
        assert!(
            messages.is_empty(),
            "an all-zero block without costUsd is still authoritative; got {:?}",
            messages
                .iter()
                .map(|m| (m.tokens.total(), m.cost))
                .collect::<Vec<_>>()
        );
    }

    /// A `usage` block that reports only some buckets is still authoritative:
    /// the absent ones are zero, not an invitation to estimate. Pins that the
    /// "may estimate" rule keys on *no bucket field at all*, not on *every*
    /// bucket field being present.
    #[test]
    fn test_partial_usage_block_is_authoritative_not_estimated() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let jsonl = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", &"p".repeat(4000)),
            json!({
                "type": "message",
                "id": "a1",
                "parentId": "u1",
                "timestamp": "2026-08-31T00:00:05Z",
                "message": {"role": "assistant", "content": [{"type": "text", "text": "short"}]},
                "usage": {"outputTokens": 7, "costUsd": 0.25},
                "model": "model-x"
            }),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        // Absent buckets are reported zeros; the 4000-char prompt must NOT be
        // estimated into `input`.
        assert_eq!(msg.tokens.input, 0);
        assert_eq!(msg.tokens.output, 7);
        assert_eq!(msg.tokens.cache_read, 0);
        assert_eq!(msg.tokens.cache_write, 0);
        assert!((msg.cost - 0.25).abs() < 1e-9);
        assert!(msg.has_authoritative_cost());
    }

    /// A dropped all-zero *authoritative* line must not consume the turn-start
    /// marker either — the next real response in the same turn keeps it.
    #[test]
    fn test_turn_start_survives_dropped_all_zero_authoritative_line() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let jsonl = format!(
            "{}\n{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", "hi"),
            json!({
                "type": "message",
                "id": "a1",
                "parentId": "u1",
                "timestamp": "2026-08-31T00:00:05Z",
                "message": {"role": "assistant", "content": [{"type": "text", "text": "aborted"}]},
                "usage": {
                    "inputTokens": 0,
                    "outputTokens": 0,
                    "cacheReadTokens": 0,
                    "cacheWriteTokens": 0,
                    "costUsd": 0
                },
                "model": "model-x"
            }),
            assistant_line(
                "a2",
                "a1",
                "2026-08-31T00:00:09Z",
                "model-x",
                "the real response",
                10,
                5,
                0,
                0.001
            ),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1, "the all-zero line must not be emitted");
        let msg = &messages[0];
        assert_eq!(msg.tokens.input, 10);
        assert_eq!(msg.tokens.output, 5);
        assert!(
            msg.is_turn_start,
            "the turn started by u1 must be marked on the surviving response"
        );
    }

    /// A dropped zero-total assistant line must not consume the turn-start
    /// marker: the next real assistant response in the same turn keeps
    /// `is_turn_start`, mirroring zcode's drop path.
    #[test]
    fn test_turn_start_survives_dropped_zero_assistant_line() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        // The first assistant line has NO preceding context and empty content,
        // so estimation yields 0 tokens and it is dropped. Its turn-start
        // marker must survive for the next response in the same turn.
        let jsonl = format!(
            "{}\n{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            json!({
                "type": "message",
                "id": "a0",
                "parentId": null,
                "timestamp": "2026-08-31T00:00:01Z",
                "message": {"role": "assistant", "content": []}
            }),
            user_line("u1", "a0", "2026-08-31T00:00:02Z", "hi"),
            assistant_line(
                "a1",
                "u1",
                "2026-08-31T00:00:05Z",
                "model-x",
                "response",
                10,
                5,
                0,
                0.0
            ),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1);
        // The zero-total assistant line was dropped; the surviving response
        // belongs to the turn started by u1 and must be marked as its start.
        assert!(messages[0].is_turn_start);
    }

    /// Legacy flat-schema transcripts (`{"role":..., "content":...}` with no
    /// `type`/`message` nesting) still parse via estimation, and their embedded
    /// `sessionId` is honored even when it differs from the filename.
    #[test]
    fn test_legacy_flat_schema_still_parses_with_estimation() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "MiniMaxAI/MiniMax-M3-Free");
        let jsonl = concat!(
            r#"{"role":"user","sessionId":"legacy-session-42","content":[{"type":"text","text":"12345678"}]}"#,
            "\n",
            r#"{"role":"assistant","sessionId":"legacy-session-42","content":[{"type":"text","text":"abcd"}]}"#,
        );
        // The FILE is named `sess-1.jsonl`, but the transcript's sessionId is
        // `legacy-session-42` — the parser must use the embedded id.
        let path = write_session(&dir, "proj", "sess-1", jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(msg.client, "commandcode");
        // Org prefix stripped; `-free` preserved (real catalog id).
        assert_eq!(msg.model_id, "MiniMax-M3-Free");
        assert_eq!(msg.provider_id, "minimax");
        assert_eq!(msg.session_id, "legacy-session-42");
        assert_eq!(
            msg.tokens.input,
            estimate_tokens(content_chars(
                &json!([{"type": "text", "text": "12345678"}])
            ))
        );
        assert_eq!(
            msg.tokens.output,
            estimate_tokens(content_chars(&json!([{"type": "text", "text": "abcd"}])))
        );
        assert!(!msg.has_authoritative_cost());
    }

    /// An all-zero usage block carrying a *positive* `costUsd` is real spend
    /// and must survive the all-zero drop.
    ///
    /// Regression guard. The drop used to run before the cost was read, so a
    /// line like this vanished and took its recorded charge with it. Zero
    /// buckets next to a non-zero cost is a shape that occurs: providers that
    /// bill per request report no token counts, and a client that failed to
    /// parse a usage block still records what it was charged. Only the
    /// zero-cost case is a genuine no-op worth dropping.
    #[test]
    fn test_all_zero_usage_with_positive_cost_is_kept() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let jsonl = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", "hi"),
            json!({
                "type": "message",
                "id": "a1",
                "parentId": "u1",
                "timestamp": "2026-08-31T00:00:05Z",
                "message": {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "billed per request"}]
                },
                "usage": {
                    "inputTokens": 0,
                    "outputTokens": 0,
                    "cacheReadTokens": 0,
                    "cacheWriteTokens": 0,
                    "costUsd": 0.42
                },
                "model": "model-x"
            }),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1, "a billed turn must not be dropped");
        let msg = &messages[0];
        assert_eq!(
            msg.tokens.total(),
            0,
            "the provider's zero buckets are kept verbatim, not estimated"
        );
        assert_eq!(msg.cost, 0.42);
        assert!(
            msg.has_authoritative_cost(),
            "the recorded charge stays provider-reported"
        );
    }

    /// Legacy lines carry no entry id, so their dedup key is positional. Two
    /// projects can hold identically-named session files, and a legacy line
    /// with no embedded `sessionId` falls back to the file stem — so the
    /// positional key has to be qualified by the workspace, or entries from
    /// the two files collide and the deduping caller deletes real messages.
    #[test]
    fn test_legacy_positional_dedup_keys_differ_across_projects() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        // No `sessionId` field and no header, so `resolved_session` falls back
        // to the file stem — `chat` in both projects.
        let jsonl = concat!(
            r#"{"role":"user","content":[{"type":"text","text":"12345678"}]}"#,
            "\n",
            r#"{"role":"assistant","content":[{"type":"text","text":"abcd"}]}"#,
        );
        let a = write_session(&dir, "project-a", "chat", jsonl);
        let b = write_session(&dir, "project-b", "chat", jsonl);

        let keys = |path: &std::path::Path| {
            parse_commandcode_file(path)
                .into_iter()
                .map(|m| {
                    m.dedup_key
                        .expect("a legacy line still gets a positional dedup key")
                })
                .collect::<Vec<_>>()
        };
        let keys_a = keys(&a);
        let keys_b = keys(&b);
        assert_eq!(keys_a.len(), 1);
        assert_eq!(keys_b.len(), 1);
        assert_ne!(
            keys_a[0], keys_b[0],
            "identically-named legacy sessions in different projects must not share a dedup key"
        );
    }

    /// An empty `usage` block on an assistant line falls back to estimation.
    #[test]
    fn test_empty_usage_block_falls_back_to_estimation() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let jsonl = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", "12345678"),
            json!({
                "type": "message",
                "id": "a1",
                "parentId": "u1",
                "timestamp": "2026-08-31T00:00:05Z",
                "message": {"role": "assistant", "content": [{"type": "text", "text": "abcd"}]},
                "usage": {},
                "model": "model-x"
            }),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        assert_eq!(
            msg.tokens.input,
            estimate_tokens(content_chars(
                &json!([{"type": "text", "text": "12345678"}])
            ))
        );
        assert_eq!(
            msg.tokens.output,
            estimate_tokens(content_chars(&json!([{"type": "text", "text": "abcd"}])))
        );
        assert!(!msg.has_authoritative_cost());
    }

    /// Adversarial near-i64::MAX token fields and an absurd cost are clamped to
    /// sane ceilings instead of overflowing or poisoning aggregates.
    #[test]
    fn test_adversarial_usage_values_are_clamped() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "model-x");
        let jsonl = format!(
            "{}\n{}\n{}",
            json!({"type": "session", "version": 3, "id": "s"}),
            user_line("u1", "", "2026-08-31T00:00:00Z", "hi"),
            json!({
                "type": "message",
                "id": "a1",
                "parentId": "u1",
                "timestamp": "2026-08-31T00:00:05Z",
                "message": {"role": "assistant", "content": [{"type": "text", "text": "yo"}]},
                "usage": {
                    "inputTokens": 9223372036854775807i64,
                    "outputTokens": 9223372036854775807i64,
                    "cacheReadTokens": 9223372036854775807i64,
                    "cacheWriteTokens": 9223372036854775807i64,
                    "costUsd": 1.0e300
                },
                "model": "model-x"
            }),
        );
        let path = write_session(&dir, "proj", "s", &jsonl);

        let messages = parse_commandcode_file(&path);
        assert_eq!(messages.len(), 1);
        let msg = &messages[0];
        // Clamped to the ceiling; no overflow, no i64::MAX poisoning.
        assert_eq!(msg.tokens.input, 1_000_000_000_000);
        assert_eq!(msg.tokens.output, 1_000_000_000_000);
        assert_eq!(msg.tokens.cache_read, 1_000_000_000_000);
        assert_eq!(msg.tokens.total(), 4_000_000_000_000);
        // Absurd cost rejected: not provider-reported, so pricing applies.
        assert_eq!(msg.cost, 0.0);
        assert!(!msg.has_authoritative_cost());
    }
}
