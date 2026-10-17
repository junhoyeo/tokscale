//! Codex CLI session parser
//!
//! Parses JSONL files from `~/.codex/sessions/` and its sibling
//! `~/.codex/archived_sessions/` (where Codex CLI moves older sessions). Both
//! directories share an identical JSONL schema, so a single parser handles
//! both; the scan-root wiring that discovers `archived_sessions` lives in
//! `crate::scanner`. Session identity for dedup is derived from in-file
//! `session_meta` content (not the file path), so a session that happens to
//! be present in both directories at once is still counted only once — see
//! `codex_token_count_dedup_key`.
//! Note: This parser has stateful logic to track model and delta calculations.

use super::utils::{
    extract_i64, extract_string, file_modified_timestamp_ms, is_zst_path, parse_timestamp_value,
    session_id_from_path,
};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Mutex;

/// Agents-tab bucket for a regular interactive Codex thread. Codex stamps each
/// spawned thread with a random `agent_nickname` (a scientist/philosopher
/// name), which is a per-session alias rather than a role, so the parser
/// collapses them to a single name instead of letting every thread become its
/// own Agents row.
pub const CODEX_DEFAULT_AGENT: &str = "Codex";
/// Agents-tab bucket for threads Codex started itself: any `source.subagent`
/// variant, e.g. the `thread_spawn` workers of Ultra mode's orchestrated team.
/// Guardian reviews have their own bucket.
pub const CODEX_SUBAGENT_AGENT: &str = "Codex Subagent";
/// Agents-tab bucket for guardian review threads. Codex tags the same
/// `source.subagent.other: "guardian"` thread `thread_source: "guardian_review"`
/// in newer builds (seen from 0.150) but `"subagent"` in older ones; both land
/// here so the row does not split by Codex version.
pub const CODEX_GUARDIAN_AGENT: &str = "Codex Guardian";
/// Agents-tab bucket for `codex exec` headless runs.
pub const CODEX_HEADLESS_AGENT: &str = "Codex Headless";

/// Codex entry structure (from JSONL files)
#[derive(Debug, Deserialize)]
pub struct CodexEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub timestamp: Option<String>,
    pub payload: Option<CodexPayload>,
}

#[derive(Debug, Deserialize)]
pub struct CodexPayload {
    pub id: Option<String>,
    pub forked_from_id: Option<String>,
    /// `session_meta` only: the client that created the thread. Codex
    /// app-server stamps it from the `initialize` request's `clientInfo.name`,
    /// so a thread OpenClaw drove carries `"openclaw"` here.
    pub originator: Option<String>,
    #[serde(rename = "type")]
    pub payload_type: Option<String>,
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub model_info: Option<CodexModelInfo>,
    pub info: Option<CodexInfo>,
    /// Fast/Priority tier recorded directly on request-level rollout items.
    pub service_tier: Option<String>,
    /// `thread_settings_applied` snapshot. This is the only wire location Codex
    /// actually stamps the Fast/Priority service tier today (7,298 occurrences
    /// across ~5,900 local rollouts, all in this shape; upstream codex-rs emits
    /// it from `EventMsg::ThreadSettingsApplied` /
    /// `ThreadSettingsSnapshot.service_tier`). The snapshot is thread-scoped:
    /// emitted once near the head of a rollout and re-emitted only when the
    /// user changes settings mid-thread, so the tier applies to every turn
    /// until the next snapshot — unlike the request-scoped arms below, it must
    /// not be reset at `turn_context` boundaries.
    pub thread_settings: Option<CodexThreadSettings>,
    /// Usage of one model response on a `token_usage_record` item. The
    /// token_count stream normally reports the same response right after it,
    /// so the record only counts when no token_count confirms it; see
    /// `CodexParseState::pending_usage_record`. Also a service-tier
    /// forward-compat arm in case Codex stamps the tier here.
    pub usage: Option<CodexTokenUsage>,
    /// `token_usage_record` only: the response the usage belongs to.
    pub response_id: Option<String>,
    /// `token_usage_record` only: the thread whose response this is.
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    /// Unix timestamp (seconds) from `task_started` events. Legacy Codex turns
    /// may use UUID v4 ids, so this is their only causal ordering signal.
    /// Confirmed against codex-rs (`TurnStartedEvent::started_at`, serialized
    /// under `task_started`): documented as "Unix timestamp (in seconds)",
    /// `Option<i64>`. Deserialized leniently anyway: int/float values coerce
    /// to `i64`, and any other JSON type (string, object, ...) decodes as
    /// `None` rather than failing the whole `task_started` entry. A strict
    /// `Option<i64>` would make a wrong-typed value reject deserialization of
    /// the entire JSONL line, silently dropping the rest of that payload too.
    #[serde(default, deserialize_with = "deserialize_lenient_i64")]
    pub started_at: Option<i64>,
    pub source: Option<Value>,
    /// Thread origin from session_meta. `"user"` marks a human-initiated fork
    /// (e.g. a VS Code "fork conversation"), which replays parent history but
    /// never emits a `task_started` for the child's own turn.
    pub thread_source: Option<String>,
    /// Current working directory from session_meta.
    pub cwd: Option<String>,
    /// Provider identity from session_meta (e.g. "openai", "azure")
    pub model_provider: Option<String>,
    /// Free-text body of an `event_msg` `user_message` payload. Used to detect
    /// human turn boundaries: real human input is plain text, whereas
    /// system-injected context (`<environment_context>`, `<system-reminder>`,
    /// `<user_instructions>`, …) begins with `<`.
    pub message: Option<String>,
}

/// Lenient `Option<i64>` deserializer for `CodexPayload::started_at`. Coerces
/// JSON integers and floats to `i64`; any other type (string, bool, object,
/// array) or `null`/absent decodes as `None` instead of failing the entry.
fn deserialize_lenient_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_u64().map(|u| u as i64))
            .or_else(|| v.as_f64().map(|f| f as i64))
    }))
}

#[derive(Debug, Deserialize)]
pub struct CodexModelInfo {
    pub slug: Option<String>,
}

/// `thread_settings` snapshot carried by `event_msg` `thread_settings_applied`
/// payloads. Only the service tier is modeled; the remaining settings
/// (approval policy, permission profile, cwd, reasoning effort, …) do not
/// affect usage accounting.
#[derive(Debug, Deserialize)]
pub struct CodexThreadSettings {
    pub model_provider_id: Option<String>,
    pub service_tier: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodexInfo {
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub service_tier: Option<String>,
    pub last_token_usage: Option<CodexTokenUsage>,
    pub total_token_usage: Option<CodexTokenUsage>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CodexTokenUsage {
    pub service_tier: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
}

impl CodexTokenUsage {
    fn has_supported_counters(&self) -> bool {
        self.input_tokens.is_some()
            || self.output_tokens.is_some()
            || self.cached_input_tokens.is_some()
            || self.cache_read_input_tokens.is_some()
            || self.reasoning_output_tokens.is_some()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CodexTotals {
    input: i64,
    output: i64,
    cached: i64,
    reasoning: i64,
}

impl CodexTotals {
    fn from_usage(usage: &CodexTokenUsage) -> Self {
        Self {
            input: usage.input_tokens.unwrap_or(0).max(0),
            output: usage.output_tokens.unwrap_or(0).max(0),
            cached: usage
                .cached_input_tokens
                .unwrap_or(0)
                .max(usage.cache_read_input_tokens.unwrap_or(0))
                .max(0),
            reasoning: usage.reasoning_output_tokens.unwrap_or(0).max(0),
        }
    }

    fn delta_from(self, previous: Self) -> Option<Self> {
        if self.input < previous.input
            || self.output < previous.output
            || self.cached < previous.cached
            || self.reasoning < previous.reasoning
        {
            return None;
        }

        Some(Self {
            input: self.input - previous.input,
            output: self.output - previous.output,
            cached: self.cached - previous.cached,
            reasoning: self.reasoning - previous.reasoning,
        })
    }

    /// Whether two usage reports describe the same response: input and output
    /// must each match. The cached split of the input and the reasoning share
    /// of the output are not compared, since those are the parts a
    /// `token_usage_record` and a `token_count` may report differently.
    fn reports_same_response(self, other: Self) -> bool {
        self.input == other.input && self.output == other.output
    }

    fn saturating_add(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            output: self.output.saturating_add(other.output),
            cached: self.cached.saturating_add(other.cached),
            reasoning: self.reasoning.saturating_add(other.reasoning),
        }
    }

    fn total(self) -> i64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cached)
            .saturating_add(self.reasoning)
    }

    fn is_within(self, baseline: Self) -> bool {
        self.input <= baseline.input
            && self.output <= baseline.output
            && self.cached <= baseline.cached
            && self.reasoning <= baseline.reasoning
    }

    fn looks_like_stale_regression(self, previous: Self, last: Self) -> bool {
        let previous_total = previous.total();
        let current_total = self.total();
        let last_total = last.total();

        if previous_total <= 0 || current_total <= 0 || last_total <= 0 {
            return false;
        }

        // Some Codex token_count snapshots arrive slightly out of order: the cumulative
        // total regresses by roughly one recent increment, then resumes from the true
        // higher watermark on the next row. Treat those as stale snapshots rather than
        // hard resets so we do not count `last_token_usage` twice.
        current_total.saturating_mul(100) >= previous_total.saturating_mul(98)
            || current_total.saturating_add(last_total.saturating_mul(2)) >= previous_total
    }

    fn into_tokens(self) -> TokenBreakdown {
        // Clamp cached to not exceed input to prevent inflated totals when
        // malformed data reports more cached tokens than input tokens.
        let clamped_cached = self.cached.min(self.input).max(0);
        // `reasoning_output_tokens` is a SUBSET of `output_tokens`, not a
        // sibling of it: every Codex snapshot satisfies
        // `total_tokens == input_tokens + output_tokens`, with the reasoning
        // count never added on top. `TokenBreakdown` buckets are additive —
        // `total()` sums output and reasoning, and `compute_cost` prices their
        // sum at the output rate — so carrying the raw output through while
        // also filling `reasoning` counted every reasoning token twice and
        // billed it twice. Split it out instead, clamped so a malformed row
        // claiming more reasoning than output cannot drive the bucket negative.
        //
        // This is a Codex-specific correction. Providers that genuinely report
        // reasoning as a disjoint bucket (Gemini's `thoughtsTokenCount` beside
        // `candidatesTokenCount`) must keep feeding it unmodified.
        let clamped_reasoning = self.reasoning.max(0).min(self.output.max(0));
        TokenBreakdown {
            input: (self.input - clamped_cached).max(0),
            output: (self.output.max(0) - clamped_reasoning).max(0),
            cache_read: clamped_cached,
            cache_write: 0,
            cache_write_1h: 0,
            reasoning: clamped_reasoning,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct CodexParseState {
    /// Source completeness is independent of emitted messages and report dates.
    /// Replay-only history does not count as this file's own activity or usage.
    #[serde(default)]
    pub has_own_activity: bool,
    #[serde(default)]
    pub has_supported_usage: bool,
    #[serde(default)]
    pub has_own_synthetic_activity: bool,
    pub current_model: Option<String>,
    /// Tier from the latest thread-scoped `thread_settings_applied`
    /// snapshot. It applies to every later turn until another snapshot
    /// replaces or clears it, so turn boundaries never reset it.
    #[serde(default)]
    pub thread_service_tier: Option<String>,
    /// Tier from a request-scoped record (`token_usage_record`,
    /// `turn_context`, or a `token_count` info/usage field). It overrides the
    /// thread tier for its own turn and is cleared at the next
    /// `turn_context`, which restores the thread tier.
    #[serde(default)]
    pub request_service_tier: Option<String>,
    #[serde(default)]
    pub current_turn_start_ms: Option<i64>,
    #[serde(default)]
    pub last_accepted_token_timestamp_ms: Option<i64>,
    pub previous_totals: Option<CodexTotals>,
    pub session_is_headless: bool,
    /// Set when session_meta marks a thread Codex started itself (any
    /// `source.subagent` variant, or `thread_source: "subagent"`), as opposed
    /// to a human-initiated fork (`thread_source: "user"`). Guardian reviews
    /// set `session_is_guardian` instead.
    #[serde(default)]
    pub session_is_subagent: bool,
    /// Set when session_meta marks a guardian review thread. Never cleared, so
    /// a later session_meta in the same rollout cannot move its usage back
    /// into the interactive bucket.
    #[serde(default)]
    pub session_is_guardian: bool,
    pub session_id_from_meta: Option<String>,
    pub session_forked_from_id: Option<String>,
    pub forked_child_session_id: Option<String>,
    pub forked_child_replay_session_id: Option<String>,
    pub session_provider: Option<String>,
    pub session_agent: Option<String>,
    pub session_workspace_key: Option<String>,
    pub session_workspace_label: Option<String>,
    pub forked_child_waiting_for_turn_context: bool,
    pub forked_child_inherited_baseline: Option<CodexTotals>,
    pub forked_child_inherited_reported_total: Option<i64>,
    /// Set when a human `user_message` event is seen; consumed by the next
    /// token_count-derived message to mark it as a turn start. `#[serde(default)]`
    /// keeps a pending turn alive across incremental re-parses of appended chunks.
    #[serde(default)]
    pub pending_turn_start: bool,
    /// `turn_id`s announced by a `task_started` event while a forked child is
    /// still skipping its replayed parent history. The child's own turn is
    /// preceded by `task_started`; replayed parent turns are not. Used only to
    /// disambiguate a same-millisecond turn, where the UUID v7 timestamp ties
    /// and the random tail is meaningless — there, only a task-started turn_id
    /// ends the skip. Cleared when the skip ends or a new fork begins.
    /// `#[serde(default)]` keeps it across incremental re-parses.
    #[serde(default)]
    pub forked_child_task_started_turn_ids: std::collections::HashSet<String>,
    /// Set when the active forked child is a human-initiated (`thread_source:
    /// "user"`) fork. Such forks replay parent history but never emit a
    /// `task_started`, so the same-millisecond gate cannot lean on
    /// `task_started` to recognize the child's own turn — there the millisecond
    /// prefix tie is enough (a user fork's replayed parent turns carry the
    /// parent's millisecond prefix, not the child's). `#[serde(default)]` keeps
    /// it across incremental re-parses.
    #[serde(default)]
    pub forked_child_is_user_fork: bool,
    /// Set when `session_meta.originator` names OpenClaw: this rollout records
    /// turns OpenClaw drove through Codex app-server, so its usage belongs to
    /// the `openclaw` client. Messages are emitted with `client = "openclaw"`
    /// and the Codex thread id as their session id; the codex lane hands them
    /// to the openclaw lane, which replaces OpenClaw's own last-response-only
    /// mirror of those turns with them. `#[serde(default)]` keeps the decision
    /// across incremental re-parses of appended chunks.
    #[serde(default)]
    pub session_owned_by_openclaw: bool,
    /// `turn_id` of the turn being read, from its `task_started` /
    /// `turn_context`. Attributes each token_count to the turn that produced
    /// it, which is the granularity OpenClaw's transcript mirror uses when it
    /// refers back to a Codex thread. `#[serde(default)]` keeps the current
    /// turn across incremental re-parses.
    #[serde(default)]
    pub current_turn_id: Option<String>,
    /// True while `current_turn_id` came from a `task_started` that neither
    /// a `turn_context` nor any usage has followed yet. A `turn_context`
    /// without an id then keeps the announced one; once the turn has a
    /// `turn_context` or has produced usage, a `turn_context` without an id
    /// starts a turn that has none. `#[serde(default)]` keeps it across
    /// incremental re-parses.
    #[serde(default)]
    pub turn_id_announced_by_task_started: bool,
    /// Number of native token-count and headless usage events already visited.
    /// This is persisted so fallback-timestamp rows keep a stable ordinal when
    /// a live rollout is parsed incrementally from a cached offset.
    #[serde(default)]
    pub usage_event_ordinal: u64,
    /// The turns this rollout has recorded usage for so far; see
    /// [`CodexTurnCoverage`]. `#[serde(default)]` keeps it across incremental
    /// re-parses, since a mirror row for an earlier turn still has to yield.
    #[serde(default)]
    pub turn_coverage: CodexTurnCoverage,
    /// The latest `token_usage_record` that no token_count has confirmed yet.
    ///
    /// Codex writes one `token_usage_record` per model response and then a
    /// `token_count` whose `last_token_usage` is that same response, so the
    /// token_count stream stays the usage source and a confirmed record adds
    /// nothing. A context compaction request is the exception: its record is
    /// followed by a `compacted` item and a token_count that does not advance
    /// the totals, so the compaction's tokens appear in the record alone. A
    /// response whose turn ended before its token_count was written leaves
    /// the same trace. The record is therefore held here and settled against
    /// the usage the next token_count actually emits, whichever form that
    /// token_count takes (cumulative and last, total-only delta, last-only):
    /// - a token_count whose emitted usage reports this response (input and
    ///   output each equal; the cached and reasoning splits may differ)
    ///   confirms it, and the response is counted once, from the token_count;
    /// - a forked child's replayed parent snapshot drops it, since the record
    ///   is part of the same replay;
    /// - the next `token_usage_record` or the start of another turn arriving
    ///   first emits it as its own usage row, and so does, after a `compacted`
    ///   item, a token_count that emits nothing (Codex's non-advancing
    ///   estimate) or reports a different response.
    ///
    /// Outside compaction, a token_count that emits a different response still
    /// confirms the record by order: in every rollout measured the two are
    /// equal, and reading a mismatch as "unconfirmed" would count every
    /// response twice if Codex ever reported them differently. A token_count
    /// that reports a record already emitted on its own is recognized through
    /// [`CodexParseState::settled_usage_records`] instead.
    ///
    /// A record whose `thread_id` names another thread is never held: this
    /// rollout's token_counts do not report it, and its own thread counts it.
    ///
    /// A record still pending at the end of the file stays pending: the
    /// rollout may still be written, and an incremental re-parse resolves it
    /// once its token_count or the next turn appears. `#[serde(default)]`
    /// keeps it across incremental re-parses.
    #[serde(default)]
    pub pending_usage_record: Option<CodexPendingUsageRecord>,
    /// Usage of the latest records emitted on their own, at most
    /// [`SETTLED_USAGE_RECORD_LIMIT`], oldest first.
    ///
    /// A record is emitted when the next turn or record arrives before its
    /// token_count, so its token_count can still come later. Such a late
    /// report would otherwise count the response a second time, with a
    /// different key and possibly in another day. A token_count whose emitted
    /// usage matches an entry is taken as that late report: it advances the
    /// baseline, emits nothing, and consumes the entry. `#[serde(default)]`
    /// keeps the entries across incremental re-parses, so a late report in an
    /// appended chunk is recognized as well.
    #[serde(default)]
    pub settled_usage_records: std::collections::VecDeque<CodexTotals>,
}

/// A `token_usage_record` waiting for a token_count to confirm it; see
/// [`CodexParseState::pending_usage_record`].
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CodexPendingUsageRecord {
    usage: CodexTotals,
    timestamp_ms: Option<i64>,
    turn_id: Option<String>,
    response_id: Option<String>,
    service_tier: Option<String>,
    /// The record's own model, when Codex writes one; the turn's model
    /// otherwise.
    #[serde(default)]
    model: Option<String>,
    /// A `compacted` item followed the record; the next token_count settles
    /// it (see [`CodexParseState::pending_usage_record`]).
    #[serde(default)]
    after_compacted: bool,
    /// The record came after a human `user_message` that no usage row has
    /// marked yet, so if it is emitted it is the turn's first reply.
    #[serde(default)]
    turn_start: bool,
}

impl CodexPendingUsageRecord {
    /// Whether usage a token_count emits reports this record's response.
    fn reported_by(&self, emitted: CodexTotals) -> bool {
        self.usage.reports_same_response(emitted)
    }
}

/// How many records emitted on their own are remembered for a late
/// token_count (see [`CodexParseState::settled_usage_records`]).
const SETTLED_USAGE_RECORD_LIMIT: usize = 16;

/// Which of a thread's turns a rollout recorded usage for.
///
/// OpenClaw mirrors each Codex app-server turn into its own transcript as one
/// row with the last response's usage, keyed by `(thread id, turn id)`, and
/// the lanes replace such a row with the rollout's complete record of that
/// turn. The replacement has to be per turn, not per thread: a rollout that
/// is truncated, or that had turns appended after it was read, covers only
/// some of the turns its thread's mirror rows describe, and a mirror row for
/// a turn the rollout does not hold is the only record of that turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CodexTurnCoverage {
    /// Turn ids a usage message was emitted under.
    pub turn_ids: std::collections::HashSet<String>,
    /// Usage was emitted with no turn id in effect. Rollouts written before
    /// Codex stamped `turn_id` on `turn_context` look like this; such a
    /// rollout can only be matched to its mirror rows thread-wide.
    pub without_turn_id: bool,
}

impl CodexTurnCoverage {
    fn record(&mut self, turn_id: Option<&str>) {
        match turn_id {
            Some(turn_id) => {
                self.turn_ids.insert(turn_id.to_string());
            }
            None => self.without_turn_id = true,
        }
    }
}

/// Client id the `codex` parser tags rollouts OpenClaw drove with.
pub(crate) const OPENCLAW_CLIENT_ID: &str = "openclaw";

// Codex rollouts can include large tool output and image data. This bounds total
// decoded input separately from the per-record bound below.
pub(crate) const MAX_CODEX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_CODEX_RECORD_BYTES: usize = 16 * 1024 * 1024;

/// Codex thread id from a rollout filename, `rollout-<timestamp>-<uuid>.jsonl`.
///
/// Used where a rollout's ownership comes from its location rather than its
/// `session_meta.originator`, so the parser did not already put the thread id
/// on its messages. Returns `None` for any other spelling rather than guessing.
pub(crate) fn thread_id_from_rollout_path(path: &Path) -> Option<String> {
    let mut stem = path.file_stem()?.to_str()?;
    if is_zst_path(path) {
        stem = Path::new(stem).file_stem()?.to_str()?;
    }
    let stem = stem.strip_prefix("rollout-")?;
    // `2026-08-30T10-00-00-` is 20 chars; everything after is the thread id.
    let candidate = stem.get(20..)?;
    let is_uuid = candidate.len() == 36
        && candidate
            .bytes()
            .enumerate()
            .all(|(index, byte)| match index {
                8 | 13 | 18 | 23 => byte == b'-',
                _ => byte.is_ascii_hexdigit(),
            });
    is_uuid.then(|| candidate.to_string())
}

/// True when a `session_meta.originator` identifies OpenClaw as the client
/// that drove the thread. OpenClaw sends `clientInfo.name: "openclaw"` on the
/// app-server handshake; the comparison ignores case so a capitalized
/// product name (`"OpenClaw"`) matches too.
fn codex_originator_is_openclaw(originator: Option<&str>) -> bool {
    originator
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("openclaw"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CodexArchiveFailureReason {
    Decode,
    DecodedSizeLimit,
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CodexArchiveFailureCause {
    Decode {
        kind: std::io::ErrorKind,
        message: String,
    },
    DecodedSizeLimit {
        limit_bytes: u64,
    },
    Open {
        kind: std::io::ErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexArchiveFailure {
    pub path: PathBuf,
    pub cause: CodexArchiveFailureCause,
}

impl CodexArchiveFailure {
    pub(crate) fn open(path: &Path, error: std::io::Error) -> Self {
        Self {
            path: path.to_path_buf(),
            cause: CodexArchiveFailureCause::Open {
                kind: error.kind(),
                message: error.to_string(),
            },
        }
    }

    fn decode(path: &Path, error: std::io::Error) -> Self {
        Self::decode_parts(path, error.kind(), error.to_string())
    }

    fn decode_parts(path: &Path, kind: std::io::ErrorKind, message: String) -> Self {
        Self {
            path: path.to_path_buf(),
            cause: CodexArchiveFailureCause::Decode { kind, message },
        }
    }

    fn decoded_size_limit(path: &Path, limit_bytes: u64) -> Self {
        Self {
            path: path.to_path_buf(),
            cause: CodexArchiveFailureCause::DecodedSizeLimit { limit_bytes },
        }
    }

    pub(crate) fn reason(&self) -> CodexArchiveFailureReason {
        match &self.cause {
            CodexArchiveFailureCause::Decode { .. } => CodexArchiveFailureReason::Decode,
            CodexArchiveFailureCause::DecodedSizeLimit { .. } => {
                CodexArchiveFailureReason::DecodedSizeLimit
            }
            CodexArchiveFailureCause::Open { .. } => CodexArchiveFailureReason::Open,
        }
    }

    pub(crate) fn cause_summary(&self) -> String {
        match &self.cause {
            CodexArchiveFailureCause::Decode { kind, message } => {
                format!("decode failed: {message} [{kind:?}]")
            }
            CodexArchiveFailureCause::DecodedSizeLimit { limit_bytes } => {
                format!("decoded size limit exceeded: more than {limit_bytes} bytes")
            }
            CodexArchiveFailureCause::Open { kind, message } => {
                format!("open failed: {message} [{kind:?}]")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedCodexFile {
    pub messages: Vec<UnifiedMessage>,
    pub fallback_timestamp_indices: Vec<usize>,
    pub consumed_offset: u64,
    pub parse_succeeded: bool,
    /// Set only for an I/O or zstd-boundary failure while reading a compressed
    /// rollout. Malformed decoded JSON remains a parser failure, not an archive
    /// read failure.
    pub archive_failure: Option<CodexArchiveFailure>,
    reader_error: Option<(std::io::ErrorKind, String)>,
    /// Oversized records are consumed without materializing them so later usage
    /// rows can still be parsed.
    pub skipped_oversized_records: u64,
    /// True when model-less token_count rows were emitted without a later model.
    pub unresolved_model_events: bool,
    pub state: CodexParseState,
}

impl ParsedCodexFile {
    pub(crate) fn is_missing_usage(&self) -> bool {
        self.parse_succeeded && self.state.is_missing_usage()
    }
}

#[cfg(test)]
static ARCHIVE_LIMIT_OVERRIDES: Mutex<Option<HashMap<PathBuf, u64>>> = Mutex::new(None);
#[cfg(test)]
static RECORD_LIMIT_OVERRIDES: Mutex<Option<HashMap<PathBuf, usize>>> = Mutex::new(None);

/// Keeps one path-scoped decoded-size limit installed for a test. The exact
/// path lookup reaches Rayon workers without a process-wide tuning variable.
#[cfg(test)]
pub(crate) struct ArchiveLimitOverrideGuard {
    path: PathBuf,
}

#[cfg(test)]
impl Drop for ArchiveLimitOverrideGuard {
    fn drop(&mut self) {
        let Ok(mut overrides) = ARCHIVE_LIMIT_OVERRIDES.lock() else {
            return;
        };
        if let Some(overrides) = overrides.as_mut() {
            overrides.remove(&self.path);
        }
        if matches!(overrides.as_ref(), Some(overrides) if overrides.is_empty()) {
            *overrides = None;
        }
    }
}

#[cfg(test)]
pub(crate) fn register_archive_limit_override(
    path: &Path,
    max_bytes: u64,
) -> ArchiveLimitOverrideGuard {
    let path = path.to_path_buf();
    let inserted = {
        let mut overrides = ARCHIVE_LIMIT_OVERRIDES.lock().unwrap();
        let overrides = overrides.get_or_insert_with(HashMap::new);
        if overrides.contains_key(&path) {
            false
        } else {
            overrides.insert(path.clone(), max_bytes);
            true
        }
    };
    assert!(
        inserted,
        "a Codex archive limit override is already registered for this path"
    );
    ArchiveLimitOverrideGuard { path }
}

pub(crate) const CODEX_MAX_DECODED_BYTES_ENV: &str = "TOKSCALE_CODEX_MAX_DECODED_BYTES";

/// Whether a raw `TOKSCALE_CODEX_MAX_DECODED_BYTES` value is usable.
///
/// An unset variable is not a mistake, so it reports `None`. A value that is
/// set but cannot be used reports `Some(false)`: the limit then silently
/// reverts to the 1 GiB default, and the skipped-archive warning quotes that
/// default, so without this the user is told their archives are too large
/// while believing they already raised the ceiling.
///
/// Takes an `OsStr` because a non-UTF-8 value is set-but-unusable, not unset.
pub(crate) fn codex_archive_limit_override_is_usable(
    raw: Option<&std::ffi::OsStr>,
) -> Option<bool> {
    let raw = raw?;
    Some(parsed_codex_archive_limit(Some(raw)).is_some())
}

fn parsed_codex_archive_limit(raw: Option<&std::ffi::OsStr>) -> Option<u64> {
    raw.and_then(|v| v.to_str())
        .and_then(|v| v.trim().parse::<u64>().ok())
        // `archive_limit.saturating_add(1)` bounds the decoder, so u64::MAX
        // would saturate and stop bounding it at all.
        .filter(|&v| v > 0 && v < u64::MAX)
}

pub(crate) fn parse_max_codex_archive_bytes(raw: Option<&std::ffi::OsStr>) -> u64 {
    parsed_codex_archive_limit(raw).unwrap_or(MAX_CODEX_ARCHIVE_BYTES)
}

pub fn max_codex_archive_bytes() -> u64 {
    // `var` would report a set non-UTF-8 value as absent, so an undecodable
    // setting would take the silent default this warning exists to prevent.
    let raw = std::env::var_os(CODEX_MAX_DECODED_BYTES_ENV);
    // This is read per scanned archive and per cache-identity check, so the
    // diagnostic has to be emitted once rather than once per file.
    if codex_archive_limit_override_is_usable(raw.as_deref()) == Some(false) {
        warn_invalid_codex_archive_limit_once(raw.as_deref().unwrap_or_default());
    }
    parse_max_codex_archive_bytes(raw.as_deref())
}

fn warn_invalid_codex_archive_limit_once(raw: &std::ffi::OsStr) {
    static WARNED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    let mut emitted = false;
    WARNED.get_or_init(|| {
        emitted = true;
    });
    if !emitted {
        return;
    }
    crate::tui_signal::emit_or_defer_stderr(format!(
        "tokscale: warning: {CODEX_MAX_DECODED_BYTES_ENV}={raw:?} is not a whole number of bytes \
         between 1 and {max}; using the default {default}-byte decoded-size limit for compressed \
         Codex rollouts.",
        max = u64::MAX - 1,
        default = MAX_CODEX_ARCHIVE_BYTES,
    ));
}

fn archive_limit_for_path(_path: &Path) -> u64 {
    #[cfg(test)]
    if let Some(max_bytes) = ARCHIVE_LIMIT_OVERRIDES
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|overrides| overrides.get(_path).copied())
    {
        return max_bytes;
    }

    max_codex_archive_bytes()
}

#[cfg(test)]
pub(crate) struct RecordLimitOverrideGuard {
    path: PathBuf,
}

#[cfg(test)]
impl Drop for RecordLimitOverrideGuard {
    fn drop(&mut self) {
        let Ok(mut overrides) = RECORD_LIMIT_OVERRIDES.lock() else {
            return;
        };
        if let Some(overrides) = overrides.as_mut() {
            overrides.remove(&self.path);
        }
        if matches!(overrides.as_ref(), Some(overrides) if overrides.is_empty()) {
            *overrides = None;
        }
    }
}

#[cfg(test)]
pub(crate) fn register_record_limit_override(
    path: &Path,
    max_bytes: usize,
) -> RecordLimitOverrideGuard {
    let path = path.to_path_buf();
    let inserted = {
        let mut overrides = RECORD_LIMIT_OVERRIDES.lock().unwrap();
        let overrides = overrides.get_or_insert_with(HashMap::new);
        overrides.insert(path.clone(), max_bytes).is_none()
    };
    assert!(
        inserted,
        "a Codex record limit override is already registered for this path"
    );
    RecordLimitOverrideGuard { path }
}

fn record_limit_for_path(_path: &Path) -> usize {
    #[cfg(test)]
    if let Some(max_bytes) = RECORD_LIMIT_OVERRIDES
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|overrides| overrides.get(_path).copied())
    {
        return max_bytes;
    }

    MAX_CODEX_RECORD_BYTES
}

impl CodexParseState {
    /// Consume the settled record `emitted` reports, if there is one; see
    /// [`CodexParseState::settled_usage_records`].
    fn take_settled_usage_record(&mut self, emitted: CodexTotals) -> bool {
        let Some(index) = self
            .settled_usage_records
            .iter()
            .position(|usage| usage.reports_same_response(emitted))
        else {
            return false;
        };
        self.settled_usage_records.remove(index);
        true
    }

    fn remember_settled_usage_record(&mut self, usage: CodexTotals) {
        if self.settled_usage_records.len() == SETTLED_USAGE_RECORD_LIMIT {
            self.settled_usage_records.pop_front();
        }
        self.settled_usage_records.push_back(usage);
    }

    pub(crate) fn is_missing_usage(&self) -> bool {
        !self.session_owned_by_openclaw && self.has_own_activity && !self.has_supported_usage
    }

    fn remember_own_activity(&mut self, event_model: Option<&str>) {
        let model = event_model
            .or(self.current_model.as_deref())
            .unwrap_or("unknown");
        let provider = self
            .session_provider
            .as_deref()
            .or_else(|| inferred_provider_from_model(model))
            .unwrap_or("openai");
        self.has_own_synthetic_activity |= super::synthetic::is_synthetic_gateway(model, provider);
        self.has_own_activity = true;
    }

    fn next_usage_event_ordinal(&mut self) -> u64 {
        let ordinal = self.usage_event_ordinal;
        self.usage_event_ordinal = self.usage_event_ordinal.saturating_add(1);
        ordinal
    }
}

fn codex_workspace_from_cwd(cwd: &str) -> (Option<String>, Option<String>) {
    let workspace_key = normalize_codex_workspace_key(cwd);
    let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);

    if workspace_label.is_none() {
        return (None, None);
    }

    (workspace_key, workspace_label)
}

fn normalize_codex_workspace_key(raw: &str) -> Option<String> {
    let normalized = normalize_workspace_key(raw)?;
    if normalized.chars().any(char::is_control) {
        return None;
    }

    if looks_like_explicit_workspace_path(&normalized) {
        Some(normalized)
    } else {
        None
    }
}

fn looks_like_explicit_workspace_path(path: &str) -> bool {
    if path.starts_with("//") || path.starts_with('/') {
        return true;
    }

    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

#[derive(Debug)]
struct CodexFallbackDedupIdentity {
    rollout_id: String,
    usage_event_ordinal: u64,
}

#[derive(Debug)]
struct PendingCodexMessage {
    message: UnifiedMessage,
    used_fallback_timestamp: bool,
    fallback_dedup: Option<CodexFallbackDedupIdentity>,
}

enum BoundedJsonlRead {
    Eof,
    Line(u64),
    Oversized(u64),
}

/// Reads one JSONL record without allowing it to grow `line` beyond `max_bytes`.
/// An oversized record is consumed through its newline in fixed-size BufRead chunks.
fn read_bounded_jsonl_line<R: BufRead>(
    reader: &mut R,
    line: &mut String,
    max_bytes: usize,
) -> std::io::Result<BoundedJsonlRead> {
    line.clear();
    let mut consumed = 0_u64;
    let mut oversized = false;
    let mut utf8_tail = Vec::with_capacity(3);
    loop {
        let (take, has_newline) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                if !oversized && !utf8_tail.is_empty() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "incomplete UTF-8 at end of JSONL record",
                    ));
                }
                return Ok(if consumed == 0 {
                    BoundedJsonlRead::Eof
                } else if oversized {
                    BoundedJsonlRead::Oversized(consumed)
                } else {
                    BoundedJsonlRead::Line(consumed)
                });
            }
            let take = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            if !oversized {
                if line
                    .len()
                    .saturating_add(utf8_tail.len())
                    .saturating_add(take)
                    > max_bytes
                {
                    oversized = true;
                } else {
                    let mut utf8 = Vec::with_capacity(utf8_tail.len() + take);
                    utf8.extend_from_slice(&utf8_tail);
                    utf8.extend_from_slice(&available[..take]);
                    match std::str::from_utf8(&utf8) {
                        Ok(chunk) => {
                            line.push_str(chunk);
                            utf8_tail.clear();
                        }
                        Err(error) if error.error_len().is_none() => {
                            let valid_up_to = error.valid_up_to();
                            // A BufRead boundary may bisect a valid multi-byte character.
                            let chunk = std::str::from_utf8(&utf8[..valid_up_to]).unwrap();
                            line.push_str(chunk);
                            utf8_tail.clear();
                            utf8_tail.extend_from_slice(&utf8[valid_up_to..]);
                        }
                        Err(error) => {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                error,
                            ));
                        }
                    }
                }
            }
            (take, take < available.len() || available[take - 1] == b'\n')
        };
        reader.consume(take);
        consumed += take as u64;
        if has_newline {
            if !oversized && !utf8_tail.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "incomplete UTF-8 at end of JSONL record",
                ));
            }
            return Ok(if oversized {
                BoundedJsonlRead::Oversized(consumed)
            } else {
                BoundedJsonlRead::Line(consumed)
            });
        }
    }
}

#[cfg(test)]
fn parse_codex_reader<R: BufRead>(
    reader: R,
    session_id: &str,
    fallback_timestamp: i64,
    start_offset: u64,
    state: CodexParseState,
) -> ParsedCodexFile {
    parse_codex_reader_with_record_limit(
        reader,
        session_id,
        fallback_timestamp,
        start_offset,
        state,
        MAX_CODEX_RECORD_BYTES,
    )
}

fn parse_codex_reader_with_record_limit<R: BufRead>(
    mut reader: R,
    session_id: &str,
    fallback_timestamp: i64,
    start_offset: u64,
    mut state: CodexParseState,
    max_record_bytes: usize,
) -> ParsedCodexFile {
    let mut messages = Vec::with_capacity(64);
    let mut fallback_timestamp_indices = Vec::new();
    let mut buffer = Vec::with_capacity(4096);
    let mut line = String::with_capacity(4096);
    let mut consumed_offset = start_offset;
    let mut parse_succeeded = true;
    let mut reader_error = None;
    let mut skipped_oversized_records = 0_u64;
    let mut pending_model_messages = Vec::new();
    let mut unresolved_model_events = false;

    loop {
        let bytes_read = match read_bounded_jsonl_line(&mut reader, &mut line, max_record_bytes) {
            Ok(BoundedJsonlRead::Eof) => break,
            Ok(BoundedJsonlRead::Line(bytes_read)) => bytes_read,
            Ok(BoundedJsonlRead::Oversized(bytes_read)) => {
                consumed_offset += bytes_read;
                skipped_oversized_records += 1;
                continue;
            }
            Err(error) => {
                parse_succeeded = false;
                reader_error = Some((error.kind(), error.to_string()));
                break;
            }
        };
        consumed_offset += bytes_read;

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut handled = false;
        buffer.clear();
        buffer.extend_from_slice(trimmed.as_bytes());
        if let Ok(entry) = simd_json::from_slice::<CodexEntry>(&mut buffer) {
            if let Some(payload) = entry.payload {
                // Request-level usage records carry actual per-response tier
                // metadata in newer Codex rollouts. The token_count snapshots
                // remain the canonical usage source; a record only counts when
                // no token_count confirms it (see `pending_usage_record`).
                if entry.entry_type == "token_usage_record" {
                    // Another thread's response (a subagent's, say) is not
                    // reported by this rollout's token_counts; its own
                    // rollout counts it.
                    let from_other_thread = payload
                        .thread_id
                        .as_deref()
                        .zip(state.session_id_from_meta.as_deref())
                        .is_some_and(|(thread, own)| thread != own);
                    if !state.forked_child_waiting_for_turn_context && !from_other_thread {
                        let service_tier = extract_request_service_tier(&payload);
                        // A record still pending here never got its
                        // token_count, so its usage is counted from it now,
                        // before this record's tier becomes the request tier.
                        emit_unconfirmed_usage_record(
                            &mut state,
                            session_id,
                            fallback_timestamp,
                            &mut messages,
                            &mut fallback_timestamp_indices,
                            &mut pending_model_messages,
                        );
                        if let Some(service_tier) = service_tier.clone() {
                            state.request_service_tier = Some(service_tier);
                        }
                        state.pending_usage_record = payload
                            .usage
                            .as_ref()
                            .filter(|usage| usage.has_supported_counters())
                            .map(|usage| CodexPendingUsageRecord {
                                usage: CodexTotals::from_usage(usage),
                                timestamp_ms: parse_codex_entry_timestamp(
                                    entry.timestamp.as_deref(),
                                ),
                                turn_id: payload
                                    .turn_id
                                    .clone()
                                    .or_else(|| state.current_turn_id.clone()),
                                response_id: payload.response_id.clone(),
                                service_tier,
                                model: extract_model(&payload),
                                after_compacted: false,
                                turn_start: state.pending_turn_start,
                            });
                        // A held record is a usage ledger even before it is
                        // settled, so the rollout is not reported as unmetered.
                        if state.pending_usage_record.is_some() {
                            state.has_supported_usage = true;
                        }
                    }
                    continue;
                }

                // A new turn ends any wait for the previous turn's token_count,
                // so the pending record is counted now. A `compacted` item only
                // marks it: the next token_count settles a compaction record.
                if !state.forked_child_waiting_for_turn_context {
                    if entry.entry_type == "compacted" {
                        if let Some(record) = state.pending_usage_record.as_mut() {
                            record.after_compacted = true;
                        }
                    }
                    let starts_other_turn = (entry.entry_type == "event_msg"
                        && payload.payload_type.as_deref() == Some("task_started"))
                        // A `turn_context` without an id starts a turn of its
                        // own unless a `task_started` just announced it, and
                        // that `task_started` has already settled the record.
                        || (entry.entry_type == "turn_context"
                            && state
                                .pending_usage_record
                                .as_ref()
                                .is_some_and(|record| record.turn_id != payload.turn_id));
                    // These terminal events mean this turn cannot later write
                    // its token_count. Settle the record now rather than leaving
                    // a completed rollout permanently undercounted. A stamped
                    // terminal event must name the pending record's turn; an
                    // unstamped one applies to the active pending turn.
                    let ends_pending_turn = entry.entry_type == "event_msg"
                        && matches!(
                            payload.payload_type.as_deref(),
                            Some("turn_aborted" | "task_complete")
                        )
                        && state.pending_usage_record.as_ref().is_some_and(|record| {
                            payload.turn_id.is_none() || record.turn_id == payload.turn_id
                        });
                    if starts_other_turn || ends_pending_turn {
                        emit_unconfirmed_usage_record(
                            &mut state,
                            session_id,
                            fallback_timestamp,
                            &mut messages,
                            &mut fallback_timestamp_indices,
                            &mut pending_model_messages,
                        );
                    }
                }

                // The service tier's only real wire location today: a
                // thread-scoped settings snapshot (upstream
                // `EventMsg::ThreadSettingsApplied`). It applies to every turn
                // until the next snapshot — including one whose tier is absent,
                // which clears it — so it must never be reset at a
                // `turn_context` boundary.
                if entry.entry_type == "event_msg"
                    && payload.payload_type.as_deref() == Some("thread_settings_applied")
                    && !state.forked_child_waiting_for_turn_context
                {
                    if let Some(settings) = payload.thread_settings.as_ref() {
                        if let Some(provider) = &settings.model_provider_id {
                            state.session_provider = Some(provider.clone());
                        }
                    }
                    if payload.thread_settings.is_some() {
                        // A snapshot without a tier (standard processing)
                        // clears the sticky tier from then on.
                        state.thread_service_tier = extract_thread_service_tier(&payload);
                    } else if let Some(tier) = extract_request_service_tier(&payload) {
                        // Forward-compat: a flat tier on the snapshot itself.
                        // An unrecognized shape never clears the sticky tier.
                        state.thread_service_tier = Some(tier);
                    }
                }

                let payload_model = extract_model(&payload);
                let is_token_count = entry.entry_type == "event_msg"
                    && payload.payload_type.as_deref() == Some("token_count");
                let info_model = if is_token_count {
                    payload.info.as_ref().and_then(extract_model_from_info)
                } else {
                    None
                };
                let event_model = payload_model.clone().or(info_model.clone());
                // Count every native token event, including ledger snapshots
                // that accounting later skips. Its position in the logical
                // rollout is stable across plain and compressed copies.
                let usage_event_ordinal = is_token_count.then(|| state.next_usage_event_ordinal());

                // The turn the following token_count rows belong to. Both
                // `task_started` and `turn_context` start a turn, so each
                // replaces the current id — with `None` when it carries no
                // `turn_id`, as older Codex wrote them, so that turn's usage
                // is not attributed to the previous turn. The one exception:
                // a `turn_context` without an id keeps the id its own
                // `task_started` announced just before it.
                if entry.entry_type == "event_msg"
                    && payload.payload_type.as_deref() == Some("task_started")
                {
                    state.current_turn_id = payload.turn_id.clone();
                    state.turn_id_announced_by_task_started = state.current_turn_id.is_some();
                } else if entry.entry_type == "turn_context" {
                    if payload.turn_id.is_some() {
                        state.current_turn_id = payload.turn_id.clone();
                    } else if !state.turn_id_announced_by_task_started {
                        state.current_turn_id = None;
                    }
                    state.turn_id_announced_by_task_started = false;
                }

                if state.forked_child_waiting_for_turn_context {
                    if entry.entry_type == "turn_context"
                        && forked_child_turn_starts_own_session(&state, payload.turn_id.as_deref())
                    {
                        state.forked_child_waiting_for_turn_context = false;
                        state.forked_child_replay_session_id = None;
                        state.forked_child_task_started_turn_ids.clear();
                        state.forked_child_is_user_fork = false;
                        if let Some(ref id) = state.forked_child_session_id {
                            state.session_id_from_meta = Some(id.clone());
                        }
                        state.current_model = payload_model.clone();
                        // No tier bookkeeping here: this `turn_context` falls
                        // through to the reset below, which clears any
                        // request-scoped tier left by the parent replay. The
                        // thread tier is untouched (replay snapshots were skipped).
                        handled = true;
                    } else {
                        if entry.entry_type == "event_msg"
                            && payload.payload_type.as_deref() == Some("task_started")
                        {
                            // The child's own turn is introduced by a
                            // `task_started`; remember it only when its id or
                            // timestamp places it at/after the child session.
                            // Nested child logs can replay ancestor task_started
                            // events before the child's live turn.
                            if forked_child_task_starts_own_session(
                                &state,
                                payload.turn_id.as_deref(),
                                payload.started_at,
                            ) {
                                // Safety of this branch is coupled to
                                // `forked_child_task_starts_own_session`
                                // returning false when `turn_id` is `None`.
                                if let Some(turn_id) = payload.turn_id.as_deref() {
                                    state
                                        .forked_child_task_started_turn_ids
                                        .insert(turn_id.to_string());
                                }
                            }
                        }
                        if entry.entry_type == "session_meta" {
                            if let Some(ref id) = payload.id {
                                if state
                                    .forked_child_session_id
                                    .as_deref()
                                    .is_some_and(|child_id| child_id != id)
                                {
                                    // Newer Codex fork logs can embed the
                                    // parent session metadata before replaying
                                    // parent token_count history. Keep
                                    // skipping while that copied upstream
                                    // transcript is active.
                                    state.forked_child_replay_session_id = Some(id.clone());
                                }
                            }
                        }
                        if is_token_count {
                            if let Some(info) = payload.info.as_ref() {
                                remember_forked_child_inherited_baseline(&mut state, info);
                            }
                        }
                        continue;
                    }
                }

                // A `turn_context` starts a new turn, which ends only the
                // request-scoped tier. The thread tier lives in its own slot,
                // so a request-scoped record can never end it early.
                if entry.entry_type == "turn_context" {
                    state.request_service_tier = None;
                }
                let has_own_activity = match entry.entry_type.as_str() {
                    "response_item" => matches!(
                        payload.payload_type.as_deref(),
                        Some(
                            "message"
                                | "agent_message"
                                | "reasoning"
                                | "function_call"
                                | "function_call_output"
                                | "local_shell_call"
                                | "custom_tool_call"
                                | "custom_tool_call_output"
                                | "tool_search_call"
                                | "tool_search_output"
                                | "web_search_call"
                                | "image_generation_call"
                        )
                    ),
                    "event_msg" => match payload.payload_type.as_deref() {
                        Some("user_message") => {
                            codex_message_is_human_turn(payload.message.as_deref())
                        }
                        Some("agent_message" | "agent_reasoning") => true,
                        _ => false,
                    },
                    _ => false,
                };
                if has_own_activity {
                    state.remember_own_activity(event_model.as_deref());
                }
                if let Some(service_tier) = extract_request_service_tier(&payload) {
                    state.request_service_tier = Some(service_tier);
                }

                if !pending_model_messages.is_empty()
                    && event_model.is_none()
                    && !is_token_count
                    && entry.entry_type != "session_meta"
                {
                    flush_pending_model_messages_as_unknown(
                        &mut pending_model_messages,
                        &mut messages,
                        &mut fallback_timestamp_indices,
                        &mut unresolved_model_events,
                    );
                }

                if entry.entry_type == "session_meta" {
                    // Only the file's own metadata decides ownership. A forked
                    // child replays its parent's `session_meta` too, and that
                    // copy is skipped by the replay gate above, so a child the
                    // user forked in Codex from an OpenClaw-driven thread stays
                    // Codex usage.
                    if codex_originator_is_openclaw(payload.originator.as_deref()) {
                        state.session_owned_by_openclaw = true;
                    }
                    if codex_source_is_exec(payload.source.as_ref()) {
                        state.session_is_headless = true;
                    }
                    if let Some(ref id) = payload.id {
                        state.session_id_from_meta = Some(id.clone());
                    }
                    let forked_from_id = payload
                        .forked_from_id
                        .as_deref()
                        .filter(|id| !id.is_empty())
                        .or_else(|| forked_from_id_from_source(payload.source.as_ref()));
                    if let Some(forked_from_id) = forked_from_id {
                        let repeated_active_child_meta = !state
                            .forked_child_waiting_for_turn_context
                            && payload.id.as_deref().is_some()
                            && state.forked_child_session_id.as_deref() == payload.id.as_deref();
                        state.session_forked_from_id = Some(forked_from_id.to_string());
                        state.forked_child_session_id = payload.id.clone();
                        if !repeated_active_child_meta {
                            state.forked_child_waiting_for_turn_context = true;
                            state.forked_child_replay_session_id = None;
                            state.forked_child_inherited_baseline = None;
                            state.forked_child_inherited_reported_total = None;
                            state.forked_child_task_started_turn_ids.clear();
                            state.forked_child_is_user_fork =
                                payload.thread_source.as_deref() == Some("user");
                        }
                    }
                    if let Some(ref provider) = payload.model_provider {
                        state.session_provider = Some(provider.clone());
                    }
                    if codex_thread_is_guardian(
                        payload.thread_source.as_deref(),
                        payload.source.as_ref(),
                    ) {
                        state.session_is_guardian = true;
                    } else if codex_thread_is_subagent(
                        payload.thread_source.as_deref(),
                        payload.source.as_ref(),
                    ) {
                        state.session_is_subagent = true;
                    }
                    // Codex's per-thread `agent_nickname` is a random alias
                    // (Popper, Dirac, …), not a role; bucket by thread kind
                    // instead so the Agents tab stays aggregateable.
                    state.session_agent = Some(
                        if state.session_is_guardian {
                            CODEX_GUARDIAN_AGENT
                        } else if state.session_is_subagent {
                            CODEX_SUBAGENT_AGENT
                        } else {
                            CODEX_DEFAULT_AGENT
                        }
                        .to_string(),
                    );
                    if let Some(ref cwd) = payload.cwd {
                        let (workspace_key, workspace_label) = codex_workspace_from_cwd(cwd);
                        state.session_workspace_key = workspace_key;
                        state.session_workspace_label = workspace_label;
                    }
                }
                // Extract model from turn_context
                if entry.entry_type == "turn_context" {
                    state.current_model = payload_model.clone();
                    let turn_start_ms = parse_codex_entry_timestamp(entry.timestamp.as_deref());
                    state.current_turn_start_ms = turn_start_ms;
                    state.last_accepted_token_timestamp_ms = turn_start_ms;
                    if let Some(model) = state.current_model.clone() {
                        flush_pending_model_messages(
                            &mut pending_model_messages,
                            &mut messages,
                            &mut fallback_timestamp_indices,
                            &model,
                        );
                    }
                    handled = true;
                }

                // Some resumed turns emit `task_started` without a following
                // `turn_context`. Reset the duration cursor at that boundary
                // so the next token_count cannot include the preceding idle
                // gap. When present, `started_at` is the turn's Unix-second
                // timestamp; otherwise use the event timestamp. A later
                // `turn_context` still replaces this anchor as usual.
                if entry.entry_type == "event_msg"
                    && payload.payload_type.as_deref() == Some("task_started")
                {
                    let turn_start_ms = payload
                        .started_at
                        .map(|timestamp| timestamp.saturating_mul(1_000))
                        .or_else(|| parse_codex_entry_timestamp(entry.timestamp.as_deref()));
                    state.current_turn_start_ms = turn_start_ms;
                    state.last_accepted_token_timestamp_ms = turn_start_ms;
                }

                // A human `user_message` event starts a new turn. The event
                // itself carries no tokens, so we defer the flag to the next
                // token_count-derived message (the assistant's reply). This
                // counts `codex exec` one-shots too: they are headless but still
                // carry a real human prompt, so each is one turn. Only
                // system-injected messages (leading `<`, e.g.
                // <environment_context>, <system-reminder>) are excluded as
                // non-human input. Forked-child replays of the parent prompt
                // arrive before turn_context and are skipped by the
                // `forked_child_waiting_for_turn_context` branch above, so they
                // never reach here.
                if entry.entry_type == "event_msg"
                    && payload.payload_type.as_deref() == Some("user_message")
                {
                    if codex_message_is_human_turn(payload.message.as_deref()) {
                        state.pending_turn_start = true;
                        // Defensively reset the start-anchor cursor here too.
                        // Normally `turn_context` resets it every turn (see
                        // above), but a resumed/compacted session can emit a
                        // `token_count` after this `user_message` with no
                        // intervening `turn_context`. Without this reset the
                        // cursor would still hold the previous turn's last
                        // token time and bridge backward across the idle gap.
                        state.last_accepted_token_timestamp_ms =
                            parse_codex_entry_timestamp(entry.timestamp.as_deref());
                    }
                    handled = true;
                }

                // Process token_count events
                if is_token_count {
                    let info = match payload.info {
                        Some(i) => i,
                        None => continue,
                    };

                    let model = payload_model
                        .or(info_model)
                        .or_else(|| state.current_model.clone());
                    if let Some(ref model) = model {
                        state.current_model = Some(model.clone());
                        flush_pending_model_messages(
                            &mut pending_model_messages,
                            &mut messages,
                            &mut fallback_timestamp_indices,
                            model,
                        );
                    }

                    // Use last_token_usage as the primary increment source.
                    // Upstream totals are mutable snapshots (compaction, context-window
                    // capping can rewrite them), so we only use total_token_usage for
                    // dedup and monotonicity checks — never as a direct delta source.
                    let total_usage = info.total_token_usage.as_ref().map(CodexTotals::from_usage);
                    let last_usage = info.last_token_usage.as_ref().map(CodexTotals::from_usage);
                    let has_supported_counters = info
                        .total_token_usage
                        .as_ref()
                        .is_some_and(CodexTokenUsage::has_supported_counters)
                        || info
                            .last_token_usage
                            .as_ref()
                            .is_some_and(CodexTokenUsage::has_supported_counters);

                    // Forked child logs can replay more than one parent
                    // token_count row after the first child turn_context,
                    // often with child-local timestamps. Keep the inherited
                    // baseline active until totals move beyond it.
                    if forked_child_should_skip_inherited_snapshot(
                        &state,
                        info.total_token_usage.as_ref(),
                        total_usage,
                    ) {
                        // An explicit reset to zero after the own-turn gate is
                        // a ledger too. Accounting can still suppress it to
                        // preserve the inherited baseline without calling an
                        // active child unmetered.
                        if total_usage == Some(CodexTotals::default()) {
                            state.has_supported_usage |= has_supported_counters;
                        }
                        // The record just before a replayed parent snapshot
                        // is the parent's replayed record of that response,
                        // which the parent rollout counts already.
                        state.pending_usage_record = None;
                        continue;
                    }
                    state.forked_child_inherited_baseline = None;
                    state.forked_child_inherited_reported_total = None;

                    // Zero and duplicate snapshots still prove that a supported
                    // ledger exists, even when accounting emits no message.
                    state.has_supported_usage |= has_supported_counters;

                    // The usage this token_count emits, in whichever form the
                    // snapshot reports it, and the baseline it leaves behind;
                    // `None` when it emits nothing.
                    let emitted = match (total_usage, last_usage, state.previous_totals) {
                        // Both present with previous baseline (standard path)
                        (Some(total), Some(last), Some(previous)) => {
                            if total == previous
                                || (total.delta_from(previous).is_none()
                                    && total.looks_like_stale_regression(previous, last))
                            {
                                None
                            } else {
                                Some((last, Some(total)))
                            }
                        }
                        // Both present, first event — use last (NOT full total) to
                        // avoid overcounting tokens carried from a resumed session.
                        (Some(total), Some(last), None) => Some((last, Some(total))),
                        // Only total, have previous (defensive — upstream schema
                        // requires both when info is present)
                        (Some(total), None, Some(previous)) => {
                            if total == previous {
                                None
                            } else if let Some(delta) = total.delta_from(previous) {
                                Some((delta, Some(total)))
                            } else {
                                state.previous_totals = Some(total);
                                None
                            }
                        }
                        // Only total, first event, no last — legacy/degraded path
                        (Some(total), None, None) => Some((total, Some(total))),
                        // Only last, have previous
                        (None, Some(last), Some(previous)) => {
                            Some((last, Some(previous.saturating_add(last))))
                        }
                        // Only last, no previous
                        (None, Some(last), None) => Some((last, None)),
                        // Neither
                        (None, None, _) => None,
                    }
                    // Skip zero-token snapshots without advancing the baseline so
                    // that post-compaction zero totals don't inflate later deltas.
                    .filter(|(usage, _)| {
                        let tokens = usage.into_tokens();
                        tokens.input != 0
                            || tokens.output != 0
                            || tokens.cache_read != 0
                            || tokens.reasoning != 0
                    });

                    let Some((emitted_usage, next_totals)) = emitted else {
                        // A token_count that emits nothing after `compacted`
                        // (Codex's non-advancing estimate) did not report the
                        // compaction, so its record is the only report of it.
                        if state
                            .pending_usage_record
                            .as_ref()
                            .is_some_and(|record| record.after_compacted)
                        {
                            emit_unconfirmed_usage_record(
                                &mut state,
                                session_id,
                                fallback_timestamp,
                                &mut messages,
                                &mut fallback_timestamp_indices,
                                &mut pending_model_messages,
                            );
                        }
                        continue;
                    };
                    let tokens = emitted_usage.into_tokens();
                    state.previous_totals = next_totals;

                    // Settle the pending record against the usage this
                    // token_count emits (see `pending_usage_record`).
                    let reports_pending = state
                        .pending_usage_record
                        .as_ref()
                        .is_some_and(|record| record.reported_by(emitted_usage));
                    if reports_pending {
                        // The same response: counted once, from here.
                        state.pending_usage_record = None;
                    } else if state.take_settled_usage_record(emitted_usage) {
                        // A late report of a response whose record was
                        // already counted on its own. The baseline moves on,
                        // and any pending record waits for its own report.
                        continue;
                    } else if state
                        .pending_usage_record
                        .as_ref()
                        .is_some_and(|record| record.after_compacted)
                    {
                        // This token_count reports another response, so the
                        // compaction before it is counted from its record.
                        emit_unconfirmed_usage_record(
                            &mut state,
                            session_id,
                            fallback_timestamp,
                            &mut messages,
                            &mut fallback_timestamp_indices,
                            &mut pending_model_messages,
                        );
                    } else {
                        // Outside compaction the next report confirms the
                        // record by order (see `pending_usage_record`).
                        state.pending_usage_record = None;
                    }

                    let parsed_timestamp = parse_codex_entry_timestamp(entry.timestamp.as_deref());
                    let timestamp = state
                        .last_accepted_token_timestamp_ms
                        .unwrap_or_else(|| parsed_timestamp.unwrap_or(fallback_timestamp));
                    let duration_ms = duration_between_ms(
                        state.last_accepted_token_timestamp_ms,
                        parsed_timestamp,
                    );

                    let mut message = new_codex_usage_message(
                        &state,
                        session_id,
                        model.as_deref(),
                        timestamp,
                        tokens,
                    );
                    message.service_tier = state
                        .request_service_tier
                        .clone()
                        .or_else(|| state.thread_service_tier.clone());
                    message.duration_ms = duration_ms;
                    state.turn_coverage.record(state.current_turn_id.as_deref());
                    // The announced turn has produced usage, so it is under
                    // way: a `turn_context` without an id that comes later
                    // starts another turn, not this one.
                    state.turn_id_announced_by_task_started = false;
                    // Apply a deferred human-turn marker from a preceding
                    // user_message to this assistant reply — the first
                    // token-bearing message after the human input.
                    if state.pending_turn_start {
                        message.is_turn_start = true;
                        state.pending_turn_start = false;
                    }
                    let fallback_dedup = if parsed_timestamp.is_some() || total_usage.is_some() {
                        // Fork/subagent children replay the same upstream
                        // token_count history into many sibling files. Those
                        // replays carry identical cumulative totals but a
                        // distinct per-file session id, so a session-scoped key
                        // never collapses them and the totals get counted once
                        // per sibling. Scope the key to the fork parent instead
                        // so sibling replays share one key. Unrelated sessions
                        // keep their own id and never merge.
                        let dedup_scope_id = state
                            .session_forked_from_id
                            .as_deref()
                            .or(state.session_id_from_meta.as_deref())
                            .unwrap_or(session_id);
                        set_codex_dedup_key(
                            &mut message,
                            model.as_deref().unwrap_or("unknown"),
                            dedup_scope_id,
                            total_usage,
                        );
                        None
                    } else {
                        // A timestamp-less, last-only row used to have no key,
                        // so a live rollout and its `.jsonl.zst` copy were both
                        // counted. The logical rollout, event ordinal, model,
                        // and emitted usage stay stable across identical copies,
                        // while the usage fingerprint keeps a rewritten live
                        // row distinct from an older archive at the same ordinal.
                        // This only stabilizes identity: after the plain file is
                        // removed, the archive can still move the row's day to
                        // its newer mtime because the original mtime is absent.
                        let identity = CodexFallbackDedupIdentity {
                            rollout_id: state
                                .session_id_from_meta
                                .as_deref()
                                .filter(|id| !id.is_empty())
                                .map(str::to_owned)
                                .unwrap_or_else(|| session_id.to_string()),
                            usage_event_ordinal: usage_event_ordinal
                                .expect("a token_count has an event ordinal"),
                        };
                        if let Some(model) = model.as_deref() {
                            set_codex_fallback_dedup_key(&mut message, model, &identity);
                        }
                        Some(identity)
                    };
                    message.set_workspace(
                        state.session_workspace_key.clone(),
                        state.session_workspace_label.clone(),
                    );
                    if let Some(timestamp_ms) = parsed_timestamp {
                        if state
                            .last_accepted_token_timestamp_ms
                            .is_none_or(|cursor_ms| timestamp_ms > cursor_ms)
                        {
                            state.last_accepted_token_timestamp_ms = Some(timestamp_ms);
                        }
                    }
                    if model.is_some() {
                        messages.push(message);
                        if parsed_timestamp.is_none() {
                            fallback_timestamp_indices.push(messages.len() - 1);
                        }
                    } else {
                        pending_model_messages.push(PendingCodexMessage {
                            message,
                            used_fallback_timestamp: parsed_timestamp.is_none(),
                            fallback_dedup,
                        });
                    }
                    handled = true;
                }
            }

            // Mark session_meta as handled (even if payload was processed above)
            if entry.entry_type == "session_meta" {
                handled = true;
            }
        }

        if handled {
            continue;
        }

        if state.forked_child_waiting_for_turn_context {
            let mut json_probe = trimmed.as_bytes().to_vec();
            if simd_json::from_slice::<Value>(&mut json_probe).is_ok() {
                continue;
            }
        }

        let headless_message =
            parse_codex_headless_line(trimmed, session_id, &mut state, fallback_timestamp);
        if !pending_model_messages.is_empty() {
            if let Some(model) = state.current_model.clone() {
                flush_pending_model_messages(
                    &mut pending_model_messages,
                    &mut messages,
                    &mut fallback_timestamp_indices,
                    &model,
                );
            } else {
                flush_pending_model_messages_as_unknown(
                    &mut pending_model_messages,
                    &mut messages,
                    &mut fallback_timestamp_indices,
                    &mut unresolved_model_events,
                );
            }
        }

        if let Some((mut msg, used_fallback_timestamp)) = headless_message {
            msg.set_workspace(
                state.session_workspace_key.clone(),
                state.session_workspace_label.clone(),
            );
            messages.push(msg);
            if used_fallback_timestamp {
                fallback_timestamp_indices.push(messages.len() - 1);
            }
            continue;
        }

        let mut json_probe = trimmed.as_bytes().to_vec();
        if simd_json::from_slice::<Value>(&mut json_probe).is_err() {
            parse_succeeded = false;
            continue;
        }
    }

    flush_pending_model_messages_as_unknown(
        &mut pending_model_messages,
        &mut messages,
        &mut fallback_timestamp_indices,
        &mut unresolved_model_events,
    );

    ParsedCodexFile {
        messages,
        fallback_timestamp_indices,
        consumed_offset,
        parse_succeeded,
        archive_failure: None,
        reader_error,
        skipped_oversized_records,
        unresolved_model_events,
        state,
    }
}

/// A usage message for this rollout, attributed the way every Codex usage
/// row is: client, session, provider and agent come from the session state.
fn new_codex_usage_message(
    state: &CodexParseState,
    session_id: &str,
    model: Option<&str>,
    timestamp: i64,
    tokens: TokenBreakdown,
) -> UnifiedMessage {
    let agent = if state.session_is_headless {
        Some(CODEX_HEADLESS_AGENT.to_string())
    } else {
        state.session_agent.clone()
    };

    let provider = state
        .session_provider
        .as_deref()
        .or_else(|| model.and_then(inferred_provider_from_model))
        .unwrap_or("openai");

    // A rollout OpenClaw drove is OpenClaw's usage: tag it so and key it by
    // the Codex thread id, which is what OpenClaw's transcript mirror names
    // when it refers back to this thread.
    let (client, message_session_id) = if state.session_owned_by_openclaw {
        (
            OPENCLAW_CLIENT_ID,
            state.session_id_from_meta.as_deref().unwrap_or(session_id),
        )
    } else {
        ("codex", session_id)
    };
    UnifiedMessage::new_with_agent(
        client,
        model.unwrap_or("unknown").to_string(),
        provider,
        message_session_id.to_string(),
        timestamp,
        tokens,
        0.0,
        agent,
    )
    .with_billing(
        state.session_provider.as_deref(),
        super::BillingIdentityOrigin::ExplicitSession,
    )
}

/// Count the pending `token_usage_record` as its own usage row: no token_count
/// confirmed it before another record or another turn arrived, or the
/// token_count after its `compacted` item did not report it, so the
/// token_count stream never counted its tokens. See
/// [`CodexParseState::pending_usage_record`] for the rule.
fn emit_unconfirmed_usage_record(
    state: &mut CodexParseState,
    session_id: &str,
    fallback_timestamp: i64,
    messages: &mut Vec<UnifiedMessage>,
    fallback_timestamp_indices: &mut Vec<usize>,
    pending_model_messages: &mut Vec<PendingCodexMessage>,
) {
    let Some(record) = state.pending_usage_record.take() else {
        return;
    };
    let tokens = record.usage.into_tokens();
    if tokens.input == 0 && tokens.output == 0 && tokens.cache_read == 0 && tokens.reasoning == 0 {
        return;
    }
    let model = record.model.clone().or_else(|| state.current_model.clone());
    let timestamp = record.timestamp_ms.unwrap_or(fallback_timestamp);
    let mut message =
        new_codex_usage_message(state, session_id, model.as_deref(), timestamp, tokens);
    // The first reply after human input marks the turn start, as on the
    // token_count path; a mark left for a later turn is not taken.
    if record.turn_start && state.pending_turn_start {
        message.is_turn_start = true;
        state.pending_turn_start = false;
    }
    message.service_tier = record
        .service_tier
        .clone()
        .or_else(|| state.request_service_tier.clone())
        .or_else(|| state.thread_service_tier.clone());
    // The record names its response, which is the identity sibling fork
    // replays share; without one, the record's time and counters stand in.
    let dedup_scope_id = state
        .session_forked_from_id
        .as_deref()
        .or(state.session_id_from_meta.as_deref())
        .unwrap_or(session_id);
    message.dedup_key = Some(match record.response_id.as_deref() {
        Some(response_id) => format!("codex:usage-record:{dedup_scope_id}:{response_id}"),
        None => format!(
            "codex:usage-record:{}:{}:{}:{}:{}:{}",
            dedup_scope_id,
            timestamp,
            record.usage.input,
            record.usage.output,
            record.usage.cached,
            record.usage.reasoning
        ),
    });
    message.set_workspace(
        state.session_workspace_key.clone(),
        state.session_workspace_label.clone(),
    );
    state.turn_coverage.record(record.turn_id.as_deref());
    state.has_supported_usage = true;
    state.remember_settled_usage_record(record.usage);
    if model.is_some() {
        messages.push(message);
        if record.timestamp_ms.is_none() {
            fallback_timestamp_indices.push(messages.len() - 1);
        }
    } else {
        // The row carries its own `codex:usage-record:` key, so the fallback
        // identity a token_count row would need is not used.
        pending_model_messages.push(PendingCodexMessage {
            message,
            used_fallback_timestamp: record.timestamp_ms.is_none(),
            fallback_dedup: None,
        });
    }
}

fn codex_source_is_exec(source: Option<&Value>) -> bool {
    source.and_then(Value::as_str) == Some("exec")
}

fn codex_thread_is_guardian(thread_source: Option<&str>, source: Option<&Value>) -> bool {
    thread_source == Some("guardian_review")
        || source
            .and_then(|source| source.get("subagent"))
            .and_then(|subagent| subagent.get("other"))
            .and_then(Value::as_str)
            == Some("guardian")
}

/// Any `source.subagent` variant counts, not only the `thread_spawn` shape
/// `forked_from_id_from_source` reads, so a subagent kind the parser does not
/// model still stays out of the interactive bucket.
fn codex_thread_is_subagent(thread_source: Option<&str>, source: Option<&Value>) -> bool {
    thread_source == Some("subagent") || source.and_then(|source| source.get("subagent")).is_some()
}

fn forked_from_id_from_source(source: Option<&Value>) -> Option<&str> {
    source?
        .get("subagent")?
        .get("thread_spawn")?
        .get("parent_thread_id")?
        .as_str()
        .filter(|id| !id.is_empty())
}

fn forked_child_turn_starts_own_session(state: &CodexParseState, turn_id: Option<&str>) -> bool {
    if state.forked_child_replay_session_id.is_none() {
        return true;
    }

    let Some(child_session_id) = state.forked_child_session_id.as_deref() else {
        return true;
    };

    match (turn_id, codex_uuid_v7_order_key(child_session_id)) {
        (Some(turn_id), Some(child_key)) => {
            let Some(turn_key) = codex_uuid_v7_order_key(turn_id) else {
                // Nested child logs can replay legacy UUID v4 turns from an
                // ancestor. Only a child-local task_started event may end the
                // replay gate for a non-v7 subagent turn. Human forks do not
                // emit task_started, so retain their existing fallback.
                return state.forked_child_is_user_fork
                    || state.forked_child_task_started_turn_ids.contains(turn_id);
            };
            // Compare only the UUID v7 48-bit millisecond timestamp (the first
            // 12 hex of the order key), not the full id. The child's own turn is
            // minted at or after its session_meta and the replayed parent turns
            // strictly earlier, so the millisecond prefix is the causal signal;
            // the version nibble + random tail of two independently-minted v7
            // UUIDs is a coin flip.
            match turn_key[..12].cmp(&child_key[..12]) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Less => false,
                // Same millisecond: the timestamp ties and the random tail
                // cannot order the child's own turn against a replayed parent
                // turn that happens to share the fork's millisecond. A subagent
                // fork announces its own turn with a `task_started` event while
                // replayed parent turns are not, so only a task-started turn_id
                // ends the skip there — otherwise an equal-prefix replayed parent
                // turn would be miscounted as child-local. A human (`thread_source:
                // "user"`) fork never emits `task_started`, but its replayed
                // parent turns carry the *parent's* millisecond prefix rather
                // than the child's, so reaching this equal-prefix branch already
                // means the turn shares the child's fork millisecond and is the
                // child's own turn — end the skip.
                //
                // Residual (accepted): for a user fork this resolves on
                // millisecond-prefix equality alone. If a replayed parent turn
                // were itself minted within the exact same 1ms as the child's
                // fork (so it shares the *child's* prefix, not the parent's),
                // this branch would end the skip one turn early. That requires a
                // sub-millisecond, human-paced fork coincidence and is accepted;
                // subagent forks are hardened separately via `task_started`,
                // which user forks do not emit.
                std::cmp::Ordering::Equal => {
                    state.forked_child_is_user_fork
                        || state.forked_child_task_started_turn_ids.contains(turn_id)
                }
            }
        }
        _ => true,
    }
}

fn forked_child_task_starts_own_session(
    state: &CodexParseState,
    turn_id: Option<&str>,
    started_at: Option<i64>,
) -> bool {
    let (Some(turn_id), Some(child_session_id)) =
        (turn_id, state.forked_child_session_id.as_deref())
    else {
        return false;
    };
    let Some(child_key) = codex_uuid_v7_order_key(child_session_id) else {
        return true;
    };

    if let Some(turn_key) = codex_uuid_v7_order_key(turn_id) {
        return turn_key[..12] >= child_key[..12];
    }

    let Some(started_at) = started_at else {
        return false;
    };
    let Ok(child_started_at_ms) = i64::from_str_radix(&child_key[..12], 16) else {
        return false;
    };

    // `child_started_at_ms / 1000` floors to the child's fork second, so a
    // legacy replay whose `started_at` lands in that same integer second
    // (but strictly before the child's sub-second fork instant) is admitted.
    started_at >= child_started_at_ms / 1000
}

fn codex_uuid_v7_order_key(id: &str) -> Option<String> {
    let mut parts = id.split('-');
    let first = parts.next()?;
    let second = parts.next()?;
    let third = parts.next()?;
    let fourth = parts.next()?;
    let fifth = parts.next()?;

    if parts.next().is_some()
        || first.len() != 8
        || second.len() != 4
        || third.len() != 4
        || fourth.len() != 4
        || fifth.len() != 12
        || !third.starts_with('7')
    {
        return None;
    }

    let mut key = String::with_capacity(32);
    for part in [first, second, third, fourth, fifth] {
        if !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        key.push_str(&part.to_ascii_lowercase());
    }
    Some(key)
}

fn parse_codex_entry_timestamp(timestamp: Option<&str>) -> Option<i64> {
    timestamp
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.timestamp_millis())
}

fn duration_between_ms(start_ms: Option<i64>, end_ms: Option<i64>) -> Option<i64> {
    let duration = end_ms?.saturating_sub(start_ms?);
    (duration > 0).then_some(duration)
}

fn codex_token_count_dedup_key(
    message: &UnifiedMessage,
    model: &str,
    upstream_session_id: &str,
    total_usage: Option<CodexTotals>,
) -> String {
    if let Some(total) = total_usage {
        // Codex fork/subagent logs can replay the same upstream token_count
        // history into many child files with child-local timestamps. The
        // cumulative total is the stable upstream identity; timestamp is only
        // a fallback when older rows do not carry totals.
        return format!(
            "codex:token_count-total:{}:{}:{}:{}:{}:{}:{}",
            upstream_session_id,
            message.provider_id,
            model,
            total.input,
            total.output,
            total.cached,
            total.reasoning
        );
    }

    format!(
        "codex:token_count:{}:{}:{}:{}:{}:{}:{}:{}",
        message.timestamp,
        message.provider_id,
        model,
        message.tokens.input,
        message.tokens.output,
        message.tokens.cache_read,
        message.tokens.cache_write,
        message.tokens.reasoning
    )
}

fn set_codex_dedup_key(
    message: &mut UnifiedMessage,
    model: &str,
    upstream_session_id: &str,
    total_usage: Option<CodexTotals>,
) {
    if message.dedup_key.is_none() {
        message.dedup_key = Some(codex_token_count_dedup_key(
            message,
            model,
            upstream_session_id,
            total_usage,
        ));
    }
}

fn set_codex_fallback_dedup_key(
    message: &mut UnifiedMessage,
    model: &str,
    identity: &CodexFallbackDedupIdentity,
) {
    if message.dedup_key.is_none() {
        message.dedup_key = Some(format!(
            "codex:usage-fallback:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            identity.rollout_id.len(),
            identity.rollout_id,
            identity.usage_event_ordinal,
            model.len(),
            model,
            message.tokens.input,
            message.tokens.output,
            message.tokens.cache_read,
            message.tokens.cache_write,
            message.tokens.cache_write_1h,
            message.tokens.reasoning
        ));
    }
}

fn flush_pending_model_messages(
    pending_model_messages: &mut Vec<PendingCodexMessage>,
    messages: &mut Vec<UnifiedMessage>,
    fallback_timestamp_indices: &mut Vec<usize>,
    model: &str,
) {
    for pending in pending_model_messages.drain(..) {
        let PendingCodexMessage {
            mut message,
            used_fallback_timestamp,
            fallback_dedup,
        } = pending;
        if let Some(identity) = fallback_dedup.as_ref() {
            set_codex_fallback_dedup_key(&mut message, model, identity);
        } else if !used_fallback_timestamp {
            let upstream_session_id = message.session_id.clone();
            set_codex_dedup_key(&mut message, model, &upstream_session_id, None);
        }
        message.model_id = model.to_string();
        messages.push(message);
        if used_fallback_timestamp {
            fallback_timestamp_indices.push(messages.len() - 1);
        }
    }
}

fn flush_pending_model_messages_as_unknown(
    pending_model_messages: &mut Vec<PendingCodexMessage>,
    messages: &mut Vec<UnifiedMessage>,
    fallback_timestamp_indices: &mut Vec<usize>,
    unresolved_model_events: &mut bool,
) {
    if pending_model_messages.is_empty() {
        return;
    }

    *unresolved_model_events = true;
    flush_pending_model_messages(
        pending_model_messages,
        messages,
        fallback_timestamp_indices,
        "unknown",
    );
}

/// Parse a Codex JSONL or .jsonl.zst file with stateful tracking
pub fn parse_codex_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_codex_file_incremental(path, 0, CodexParseState::default()).messages
}

fn reported_total_tokens(usage: &CodexTokenUsage) -> Option<i64> {
    usage.total_tokens.filter(|total| *total >= 0)
}

fn remember_forked_child_inherited_baseline(state: &mut CodexParseState, info: &CodexInfo) {
    let Some(total_usage) = info.total_token_usage.as_ref() else {
        return;
    };

    let totals = CodexTotals::from_usage(total_usage);
    state.previous_totals = Some(totals);
    state.forked_child_inherited_baseline = Some(totals);
    state.forked_child_inherited_reported_total = reported_total_tokens(total_usage);
}

fn forked_child_should_skip_inherited_snapshot(
    state: &CodexParseState,
    total_usage: Option<&CodexTokenUsage>,
    totals: Option<CodexTotals>,
) -> bool {
    if let (Some(usage), Some(baseline)) =
        (total_usage, state.forked_child_inherited_reported_total)
    {
        if reported_total_tokens(usage).is_some_and(|total| total <= baseline) {
            return true;
        }
    }

    if let (Some(totals), Some(baseline)) = (totals, state.forked_child_inherited_baseline) {
        return totals.is_within(baseline);
    }

    false
}

fn failed_codex_parse(
    state: CodexParseState,
    consumed_offset: u64,
    archive_failure: Option<CodexArchiveFailure>,
) -> ParsedCodexFile {
    ParsedCodexFile {
        messages: Vec::new(),
        fallback_timestamp_indices: Vec::new(),
        consumed_offset,
        parse_succeeded: false,
        archive_failure,
        reader_error: None,
        skipped_oversized_records: 0,
        unresolved_model_events: false,
        state,
    }
}

pub(crate) fn parse_codex_file_incremental(
    path: &Path,
    start_offset: u64,
    state: CodexParseState,
) -> ParsedCodexFile {
    let session_id = session_id_from_path(path);
    let fallback_timestamp = file_modified_timestamp_ms(path);

    if is_zst_path(path) {
        if start_offset != 0 {
            return failed_codex_parse(state, start_offset, None);
        }

        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) => {
                return failed_codex_parse(state, 0, Some(CodexArchiveFailure::open(path, error)));
            }
        };

        let mut decoder = match zstd::stream::read::Decoder::new(file) {
            Ok(decoder) => decoder,
            Err(error) => {
                return failed_codex_parse(
                    state,
                    0,
                    Some(CodexArchiveFailure::decode(path, error)),
                );
            }
        };
        if let Err(error) = decoder.window_log_max(26) {
            return failed_codex_parse(state, 0, Some(CodexArchiveFailure::decode(path, error)));
        }

        let archive_limit = archive_limit_for_path(path);
        let reader = BufReader::new(decoder.take(archive_limit.saturating_add(1)));
        let mut parsed = parse_codex_reader_with_record_limit(
            reader,
            &session_id,
            fallback_timestamp,
            0,
            state,
            record_limit_for_path(path),
        );
        parsed.archive_failure = if parsed.consumed_offset > archive_limit {
            Some(CodexArchiveFailure::decoded_size_limit(path, archive_limit))
        } else if let Some((kind, message)) = parsed.reader_error.take() {
            // Corrupt and truncated frames surface while the decoder is read.
            Some(CodexArchiveFailure::decode_parts(path, kind, message))
        } else {
            None
        };
        if parsed.skipped_oversized_records > 0 {
            tracing::warn!(
                path = %path.display(),
                max_bytes = record_limit_for_path(path),
                skipped_records = parsed.skipped_oversized_records,
                "Skipped oversized Codex JSONL records while retaining later usage"
            );
        }
        if parsed.archive_failure.is_some() || !parsed.parse_succeeded {
            parsed.messages.clear();
            parsed.fallback_timestamp_indices.clear();
            parsed.parse_succeeded = false;
        }
        return parsed;
    }

    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return failed_codex_parse(state, start_offset, None),
    };

    if file.seek(SeekFrom::Start(start_offset)).is_err() {
        return failed_codex_parse(state, start_offset, None);
    }

    let reader = BufReader::new(file);
    parse_codex_reader_with_record_limit(
        reader,
        &session_id,
        fallback_timestamp,
        start_offset,
        state,
        record_limit_for_path(path),
    )
}

fn extract_model(payload: &CodexPayload) -> Option<String> {
    payload
        .model_info
        .as_ref()
        .and_then(|mi| mi.slug.clone())
        .filter(|s| !s.is_empty())
        .or(payload.model.clone().filter(|s| !s.is_empty()))
        .or(payload.model_name.clone().filter(|s| !s.is_empty()))
        .or(payload.info.as_ref().and_then(extract_model_from_info))
}

fn extract_model_from_info(info: &CodexInfo) -> Option<String> {
    info.model
        .clone()
        .filter(|s| !s.is_empty())
        .or(info.model_name.clone().filter(|s| !s.is_empty()))
}

fn normalize_service_tier(tier: Option<&str>) -> Option<String> {
    tier.map(str::trim)
        .filter(|tier| !tier.is_empty())
        .map(str::to_ascii_lowercase)
}

/// The thread-scoped tier from a `thread_settings_applied` snapshot. An
/// absent tier returns `None`, which clears the sticky thread tier.
fn extract_thread_service_tier(payload: &CodexPayload) -> Option<String> {
    normalize_service_tier(
        payload
            .thread_settings
            .as_ref()
            .and_then(|settings| settings.service_tier.as_deref()),
    )
}

/// Request-scoped tier arms. None are emitted by current Codex builds; they
/// are kept for forward compatibility and never read `thread_settings`.
fn extract_request_service_tier(payload: &CodexPayload) -> Option<String> {
    normalize_service_tier(
        payload
            .service_tier
            .as_deref()
            .or_else(|| {
                payload.info.as_ref().and_then(|info| {
                    info.service_tier
                        .as_deref()
                        .or_else(|| {
                            info.last_token_usage
                                .as_ref()
                                .and_then(|usage| usage.service_tier.as_deref())
                        })
                        .or_else(|| {
                            info.total_token_usage
                                .as_ref()
                                .and_then(|usage| usage.service_tier.as_deref())
                        })
                })
            })
            .or_else(|| {
                payload
                    .usage
                    .as_ref()
                    .and_then(|usage| usage.service_tier.as_deref())
            }),
    )
}

struct CodexHeadlessUsage {
    has_supported_counters: bool,
    input: i64,
    output: i64,
    cached: i64,
    model: Option<String>,
    timestamp_ms: Option<i64>,
}

fn parse_codex_headless_line(
    line: &str,
    session_id: &str,
    state: &mut CodexParseState,
    fallback_timestamp: i64,
) -> Option<(UnifiedMessage, bool)> {
    let mut bytes = line.as_bytes().to_vec();
    let value: Value = simd_json::from_slice(&mut bytes).ok()?;

    if let Some(model) = extract_model_from_value(&value) {
        state.current_model = Some(model);
    }

    if matches!(
        value.get("type").and_then(Value::as_str),
        Some("item.started" | "item.updated" | "item.completed")
    ) && value.get("item").is_some_and(Value::is_object)
    {
        state.remember_own_activity(None);
    }

    let usage = extract_headless_usage(&value)?;
    let usage_event_ordinal = state.next_usage_event_ordinal();
    state.has_supported_usage |= usage.has_supported_counters;
    let model = usage
        .model
        .or_else(|| state.current_model.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let timestamp = usage.timestamp_ms.unwrap_or(fallback_timestamp);

    if usage.input == 0 && usage.output == 0 && usage.cached == 0 {
        return None;
    }

    let provider = state
        .session_provider
        .as_deref()
        .or_else(|| inferred_provider_from_model(&model))
        .unwrap_or("openai");
    let agent = if state.session_is_headless {
        Some(CODEX_HEADLESS_AGENT.to_string())
    } else {
        state.session_agent.clone()
    };

    let used_fallback_timestamp = usage.timestamp_ms.is_none();
    let mut message = UnifiedMessage::new_with_agent(
        "codex",
        model.clone(),
        provider,
        session_id.to_string(),
        timestamp,
        TokenBreakdown {
            input: usage.input.max(0),
            output: usage.output.max(0),
            cache_read: usage.cached.max(0),
            cache_write: 0,
            cache_write_1h: 0,
            reasoning: 0,
        },
        0.0,
        agent,
    )
    .with_billing(
        state.session_provider.as_deref(),
        super::BillingIdentityOrigin::ExplicitSession,
    );
    if used_fallback_timestamp {
        let identity = CodexFallbackDedupIdentity {
            rollout_id: state
                .session_id_from_meta
                .as_deref()
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| session_id.to_string()),
            usage_event_ordinal,
        };
        set_codex_fallback_dedup_key(&mut message, &model, &identity);
    }

    Some((message, used_fallback_timestamp))
}

fn extract_headless_usage(value: &Value) -> Option<CodexHeadlessUsage> {
    let usage = value
        .get("usage")
        .or_else(|| value.get("data").and_then(|data| data.get("usage")))
        .or_else(|| value.get("result").and_then(|data| data.get("usage")))
        .or_else(|| value.get("response").and_then(|data| data.get("usage")))?;

    let input_tokens = extract_i64(usage.get("input_tokens"))
        .or_else(|| extract_i64(usage.get("prompt_tokens")))
        .or_else(|| extract_i64(usage.get("input")))
        .unwrap_or(0);
    let output_tokens = extract_i64(usage.get("output_tokens"))
        .or_else(|| extract_i64(usage.get("completion_tokens")))
        .or_else(|| extract_i64(usage.get("output")))
        .unwrap_or(0);
    let cached_tokens = extract_i64(usage.get("cached_input_tokens"))
        .or_else(|| extract_i64(usage.get("cache_read_input_tokens")))
        .or_else(|| extract_i64(usage.get("cached_tokens")))
        .unwrap_or(0);

    let model = extract_model_from_value(value)
        .or_else(|| value.get("data").and_then(extract_model_from_value));
    let timestamp_ms = extract_timestamp_from_value(value);

    Some(CodexHeadlessUsage {
        has_supported_counters: [
            "input_tokens",
            "prompt_tokens",
            "input",
            "output_tokens",
            "completion_tokens",
            "output",
            "cached_input_tokens",
            "cache_read_input_tokens",
            "cached_tokens",
        ]
        .iter()
        .any(|key| extract_i64(usage.get(*key)).is_some()),
        input: input_tokens.saturating_sub(cached_tokens),
        output: output_tokens,
        cached: cached_tokens,
        model,
        timestamp_ms,
    })
}

fn extract_model_from_value(value: &Value) -> Option<String> {
    extract_string(value.get("model"))
        .or_else(|| extract_string(value.get("model_name")))
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| extract_string(data.get("model")))
        })
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| extract_string(data.get("model_name")))
        })
        .or_else(|| {
            value
                .get("response")
                .and_then(|data| extract_string(data.get("model")))
        })
}

fn extract_timestamp_from_value(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .or_else(|| value.get("time"))
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("data").and_then(|data| data.get("timestamp")))
        .and_then(parse_timestamp_value)
}

/// Prefixes Codex prepends to context it injects as `user_message` events.
/// These are the bodies that must NOT be counted as human turns.
const CODEX_SYSTEM_INJECTED_PREFIXES: [&str; 3] = [
    "<environment_context>",
    "<system-reminder>",
    "<user_instructions>",
];

/// Returns true when a Codex `user_message` payload represents real human input
/// rather than system-injected context. Codex stores the body as a plain string
/// in `payload.message`; the harness injects context blocks that open with one of
/// the known tags in [`CODEX_SYSTEM_INJECTED_PREFIXES`] after trimming. Matching
/// those specific prefixes — rather than any leading `<` — avoids dropping
/// legitimate human prompts that happen to start with markup (asking about a
/// `<div>`, pasting an XML snippet, etc.). The `kind` field can't be used to
/// distinguish them: both human and injected bodies appear as `kind:"plain"` or
/// with no `kind` at all.
fn codex_message_is_human_turn(message: Option<&str>) -> bool {
    match message {
        Some(text) => {
            let trimmed = text.trim_start();
            !CODEX_SYSTEM_INJECTED_PREFIXES
                .iter()
                .any(|prefix| trimmed.starts_with(prefix))
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{aggregate_model_usage_entries, GroupBy};
    use std::ffi::OsStr;
    use std::io::{BufRead, Cursor, Error, ErrorKind, Seek, SeekFrom, Write};
    use tempfile::{NamedTempFile, TempDir};

    const CODEX_DURATION_FIXTURE: &str =
        include_str!("../../tests/fixtures/codex_duration_timing.jsonl");
    const CODEX_TASK_STARTED_DURATION_FIXTURE: &str =
        include_str!("../../tests/fixtures/codex_task_started_duration.jsonl");

    #[test]
    fn codex_human_turn_matches_only_known_system_tags() {
        // Real human prompts that happen to start with markup must still count.
        assert!(codex_message_is_human_turn(Some(
            "how do I center a <div>?"
        )));
        assert!(codex_message_is_human_turn(Some("<div>hi</div>")));
        assert!(codex_message_is_human_turn(Some("  plain question")));
        // Known system-injected context blocks are not human turns.
        assert!(!codex_message_is_human_turn(Some(
            "<environment_context>cwd=/tmp</environment_context>"
        )));
        assert!(!codex_message_is_human_turn(Some(
            "  <system-reminder>be concise</system-reminder>"
        )));
        assert!(!codex_message_is_human_turn(Some(
            "<user_instructions>do X</user_instructions>"
        )));
        // A missing body is never a human turn.
        assert!(!codex_message_is_human_turn(None));
    }

    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn bounded_jsonl_reader_preserves_utf8_split_across_chunks() {
        let input = "a한b\n";
        let mut reader = std::io::BufReader::with_capacity(2, Cursor::new(input.as_bytes()));
        let mut line = String::new();

        let result = read_bounded_jsonl_line(&mut reader, &mut line, input.len()).unwrap();

        assert!(matches!(result, BoundedJsonlRead::Line(bytes) if bytes == input.len() as u64));
        assert_eq!(line, input);
        assert!(matches!(
            read_bounded_jsonl_line(&mut reader, &mut line, input.len()).unwrap(),
            BoundedJsonlRead::Eof
        ));
    }

    const UNMETERED_ACTIVITY: &str = concat!(
        r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}"#,
        "\n"
    );

    #[test]
    fn missing_usage_distinguishes_activity_from_empty_or_unreadable_sources() {
        let cases = [
            ("", false),
            ("\n", false),
            (
                r#"{"type":"session_meta","payload":{"id":"empty","source":"cli"}}"#,
                false,
            ),
            (
                r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                false,
            ),
            (UNMETERED_ACTIVITY, true),
            (
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"hello"}}"#,
                true,
            ),
            (
                r#"{"type":"item.completed","item":{"type":"agent_message","text":"done"}}"#,
                true,
            ),
            ("not json\n", false),
        ];
        for (content, expected) in cases {
            let file = create_test_file(content);
            let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
            assert_eq!(parsed.is_missing_usage(), expected, "{content}");
            assert!(parsed.messages.is_empty());
        }

        let file = create_test_file(&format!("{UNMETERED_ACTIVITY}not json\n"));
        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(!parsed.parse_succeeded);
        assert!(!parsed.is_missing_usage());

        let reader = FailAfterFirstLine::new(UNMETERED_ACTIVITY);
        let parsed = parse_codex_reader(reader, "session", 0, 0, CodexParseState::default());
        assert!(parsed.state.has_own_activity);
        assert!(!parsed.parse_succeeded);
        assert!(!parsed.is_missing_usage());
    }

    #[test]
    fn missing_usage_recognizes_native_tool_items() {
        for kind in [
            "agent_message",
            "function_call",
            "function_call_output",
            "local_shell_call",
            "custom_tool_call",
            "custom_tool_call_output",
            "tool_search_call",
            "tool_search_output",
            "web_search_call",
            "image_generation_call",
        ] {
            let activity =
                format!("{{\"type\":\"response_item\",\"payload\":{{\"type\":\"{kind}\"}}}}\n");
            let file = create_test_file(&activity);
            let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
            assert!(parsed.is_missing_usage(), "{kind}");
            assert!(parsed.messages.is_empty());
        }
    }

    #[test]
    fn missing_usage_ignores_injected_user_context() {
        for (message, expected) in [
            (None, false),
            (
                Some("<environment_context>cwd=/tmp</environment_context>"),
                false,
            ),
            (
                Some("  <system-reminder>be concise</system-reminder>"),
                false,
            ),
            (Some("<user_instructions>do X</user_instructions>"), false),
            (Some("  plain question"), true),
            (Some("<div>hi</div>"), true),
        ] {
            let activity = serde_json::json!({
                "type": "event_msg",
                "payload": {"type": "user_message", "message": message}
            });
            let file = create_test_file(&format!("{activity}\n"));
            let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
            assert_eq!(parsed.is_missing_usage(), expected, "{message:?}");
            assert!(parsed.messages.is_empty());
        }
    }

    #[test]
    fn missing_usage_accepts_zero_native_and_headless_counters() {
        let ledgers = [
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":0,"output_tokens":0}}}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":0,"output_tokens":0}}}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":0,"output_tokens":0}}"#,
            r#"{"data":{"usage":{"prompt_tokens":0,"completion_tokens":0}}}"#,
            r#"{"result":{"usage":{"input":0,"output":0}}}"#,
            r#"{"response":{"usage":{"cached_tokens":0}}}"#,
        ];
        for ledger in ledgers {
            let file = create_test_file(&format!("{UNMETERED_ACTIVITY}{ledger}\n"));
            let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
            assert!(parsed.parse_succeeded);
            assert!(parsed.state.has_supported_usage, "{ledger}");
            assert!(!parsed.is_missing_usage());
            assert!(parsed.messages.is_empty());
        }
        for ledger in [
            r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{}}}}"#,
            r#"{"type":"turn.completed","usage":{}}"#,
        ] {
            let file = create_test_file(&format!("{UNMETERED_ACTIVITY}{ledger}\n"));
            let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
            assert!(parsed.is_missing_usage(), "{ledger}");
        }
    }

    #[test]
    fn missing_usage_excludes_openclaw_and_replayed_parent_activity() {
        let file = create_test_file(&format!(
            "{}\n{UNMETERED_ACTIVITY}",
            r#"{"type":"session_meta","payload":{"originator":"OpenClaw"}}"#
        ));
        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(!parsed.is_missing_usage());

        let inherited = format!(
            "{}\n{}\n{}\n{UNMETERED_ACTIVITY}",
            r#"{"type":"session_meta","payload":{"id":"019a0000-0002-7000-8000-000000000002","forked_from_id":"019a0000-0001-7000-8000-000000000001","thread_source":"user"}}"#,
            r#"{"type":"session_meta","payload":{"id":"019a0000-0001-7000-8000-000000000001","originator":"OpenClaw"}}"#,
            r#"{"type":"turn_context","payload":{"turn_id":"019a0000-0001-7000-8000-000000000003","model":"gpt-5.4"}}"#
        );
        let file = create_test_file(&inherited);
        let replay = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(!replay.state.has_own_activity);
        assert!(!replay.is_missing_usage());
        let own = format!(
            "{inherited}{}\n{UNMETERED_ACTIVITY}",
            r#"{"type":"turn_context","payload":{"turn_id":"019a0000-0002-7000-8000-000000000004","model":"gpt-5.4"}}"#
        );
        let file = create_test_file(&own);
        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(parsed.is_missing_usage());
        assert!(!parsed.state.session_owned_by_openclaw);
    }

    #[test]
    fn missing_usage_accepts_own_zero_ledger_after_inherited_nonzero_usage() {
        let content = format!(
            "{}\n{}\n{}\n{}\n{}\n{UNMETERED_ACTIVITY}{}\n",
            r#"{"type":"session_meta","payload":{"id":"019a0000-0002-7000-8000-000000000002","forked_from_id":"019a0000-0001-7000-8000-000000000001","thread_source":"user"}}"#,
            r#"{"type":"session_meta","payload":{"id":"019a0000-0001-7000-8000-000000000001"}}"#,
            r#"{"type":"turn_context","payload":{"turn_id":"019a0000-0001-7000-8000-000000000003","model":"gpt-5.4"}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":20},"last_token_usage":{"input_tokens":100,"output_tokens":20}}}}"#,
            r#"{"type":"turn_context","payload":{"turn_id":"019a0000-0002-7000-8000-000000000004","model":"gpt-5.4"}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":0,"output_tokens":0},"last_token_usage":{"input_tokens":0,"output_tokens":0}}}}"#
        );
        let file = create_test_file(&content);
        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(parsed.parse_succeeded);
        assert!(parsed.state.has_own_activity);
        assert!(parsed.state.has_supported_usage);
        assert!(!parsed.is_missing_usage());
        assert!(parsed.messages.is_empty());
    }

    struct FailAfterFirstLine {
        inner: Cursor<Vec<u8>>,
        fail_next_read: bool,
    }

    impl FailAfterFirstLine {
        fn new(contents: &str) -> Self {
            Self {
                inner: Cursor::new(contents.as_bytes().to_vec()),
                fail_next_read: false,
            }
        }
    }

    impl std::io::Read for FailAfterFirstLine {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl BufRead for FailAfterFirstLine {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            if self.fail_next_read {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "synthetic line decode failure",
                ));
            }
            self.inner.fill_buf()
        }

        fn consume(&mut self, amt: usize) {
            self.inner.consume(amt);
            self.fail_next_read |= amt > 0;
        }

        fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
            if self.fail_next_read {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "synthetic line decode failure",
                ));
            }
            let bytes_read = self.inner.read_line(buf)?;
            if bytes_read > 0 {
                self.fail_next_read = true;
            }
            Ok(bytes_read)
        }
    }

    #[test]
    fn test_headless_usage_line() {
        let content = r#"{"type":"turn.completed","model":"gpt-4o-mini","usage":{"input_tokens":120,"cached_input_tokens":20,"output_tokens":30}}"#;
        let file = create_test_file(content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o-mini");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
    }

    #[test]
    fn test_service_tier_is_carried_from_request_and_token_count_records() {
        let file = create_test_file(concat!(
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#,
            "\n",
            r#"{"type":"token_usage_record","payload":{"usage":{"input_tokens":10,"output_tokens":3,"service_tier":"priority"}}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"service_tier":"default","total_token_usage":{"input_tokens":20,"output_tokens":6,"total_tokens":26},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(
            messages.len(),
            2,
            "request usage records must not double-count tokens"
        );
        assert_eq!(messages[0].service_tier.as_deref(), Some("priority"));
        assert_eq!(messages[1].service_tier.as_deref(), Some("default"));
    }

    #[test]
    fn test_turn_context_resets_service_tier_without_dropping_new_turn_metadata() {
        let file = create_test_file(concat!(
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-terra","service_tier":"priority"}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-terra"}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"output_tokens":6,"total_tokens":26},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].service_tier.as_deref(), Some("priority"));
        assert_eq!(messages[1].service_tier, None);
    }

    #[test]
    fn test_thread_settings_applied_service_tier_is_sticky_across_turns_and_clears_on_none_snapshot(
    ) {
        // Fixture built from a real `thread_settings_applied` rollout line
        // (`~/.codex/sessions/2026/09/05`, thread settings payload reduced to
        // the fields Codex actually emits there). This is the ONLY wire
        // location that carries the tier today: ~7,300 `service_tier`
        // occurrences across the local rollout corpus, all in this shape.
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-09-05T07:00:39.728Z","ordinal":10,"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"01a0705e-8c8e-73d1-a6b5-c2d1c297342d","thread_settings":{"model":"gpt-5.6-luna","model_provider_id":"gateway","service_tier":"priority"}}}"#,
            "
",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}"#,
            "
",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "
",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}"#,
            "
",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"output_tokens":6,"total_tokens":26},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "
",
            // A later snapshot without a tier (e.g. the user switched back to
            // standard processing) clears the sticky tier from then on.
            r#"{"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"model":"gpt-5.6-luna","model_provider_id":"gateway"}}}"#,
            "
",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}"#,
            "
",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":30,"output_tokens":9,"total_tokens":39},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "
"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3);
        // The 2x OpenAI Fast/Priority premium engages on the first turn...
        assert_eq!(messages[0].service_tier.as_deref(), Some("priority"));
        // ...and persists across the next `turn_context` boundary because the
        // snapshot is thread-scoped, not request-scoped.
        assert_eq!(messages[1].service_tier.as_deref(), Some("priority"));
        // A later `thread_settings_applied` without a tier clears it.
        assert_eq!(messages[2].service_tier, None);
    }

    #[test]
    fn test_thread_snapshot_without_settings_object_keeps_or_sets_the_sticky_tier() {
        // Only a snapshot that carries `thread_settings` may clear the sticky
        // tier. A flat tier on the snapshot sets it (forward-compat), and an
        // unrecognized shape leaves it alone instead of wiping the premium.
        let file = create_test_file(concat!(
            r#"{"type":"event_msg","payload":{"type":"thread_settings_applied","service_tier":"priority"}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"thread_settings_applied"}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"output_tokens":6,"total_tokens":26},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].service_tier.as_deref(), Some("priority"));
        assert_eq!(messages[1].service_tier.as_deref(), Some("priority"));
    }

    #[test]
    fn test_request_scoped_tier_overrides_one_turn_without_ending_the_thread_tier() {
        // A request-scoped tier (forward-compat arm) applies to its own turn
        // only. The next `turn_context` must restore the sticky thread tier
        // rather than clearing every later turn back to standard pricing.
        let file = create_test_file(concat!(
            r#"{"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"model":"gpt-5.6-luna","service_tier":"priority"}}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}"#,
            "\n",
            r#"{"type":"token_usage_record","payload":{"usage":{"input_tokens":10,"output_tokens":3,"service_tier":"flex"}}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"output_tokens":6,"total_tokens":26},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":30,"output_tokens":9,"total_tokens":39},"last_token_usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].service_tier.as_deref(), Some("flex"));
        assert_eq!(messages[1].service_tier.as_deref(), Some("priority"));
        assert_eq!(messages[2].service_tier.as_deref(), Some("priority"));
    }

    #[test]
    fn test_token_count_durations_are_non_overlapping() {
        let file = create_test_file(CODEX_DURATION_FIXTURE);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.duration_ms)
                .collect::<Vec<_>>(),
            vec![Some(1_000), Some(4_000), Some(2_000)]
        );
    }

    #[test]
    fn test_token_count_durations_ignore_invalid_equal_and_backward_timestamps() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#,
            "\n",
            r#"{"timestamp":"not-a-timestamp","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":5,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":0}}}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"cached_input_tokens":4,"output_tokens":6,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":1,"reasoning_output_tokens":0}}}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:00.500Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":25,"cached_input_tokens":5,"output_tokens":8,"reasoning_output_tokens":2},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":1}}}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:04Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":35,"cached_input_tokens":7,"output_tokens":11,"reasoning_output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#,
            "\n"
        ));

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(parsed.parse_succeeded);
        assert_eq!(parsed.messages.len(), 5);
        assert_eq!(
            parsed
                .messages
                .iter()
                .map(|message| message.duration_ms)
                .collect::<Vec<_>>(),
            vec![Some(1_000), None, None, None, Some(3_000)]
        );
        assert_eq!(
            parsed.state.last_accepted_token_timestamp_ms,
            parse_codex_entry_timestamp(Some("2026-01-01T00:00:04Z"))
        );
        assert_eq!(
            parsed.consumed_offset,
            file.as_file().metadata().unwrap().len()
        );
    }

    #[test]
    fn test_duration_fixture_incremental_parse_matches_full_parse() {
        let lines = CODEX_DURATION_FIXTURE.lines().collect::<Vec<_>>();
        let initial_content = format!("{}\n", lines[..5].join("\n"));
        let appended_content = format!("{}\n", lines[5..].join("\n"));
        let file = create_test_file(&initial_content);
        let initial_size = file.as_file().metadata().unwrap().len();

        let initial = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert_eq!(initial.messages.len(), 1);
        assert_eq!(initial.messages[0].duration_ms, Some(1_000));
        assert_eq!(
            initial.state.last_accepted_token_timestamp_ms,
            parse_codex_entry_timestamp(Some("2040-01-01T00:00:01Z"))
        );

        let mut reopened = file.reopen().unwrap();
        reopened.seek(SeekFrom::End(0)).unwrap();
        reopened.write_all(appended_content.as_bytes()).unwrap();
        reopened.flush().unwrap();

        let incremental =
            parse_codex_file_incremental(file.path(), initial_size, initial.state.clone());
        let mut combined = initial.messages;
        combined.extend(incremental.messages);

        let full = parse_codex_file(file.path());
        assert_eq!(combined, full);
        assert_eq!(
            full.iter()
                .map(|message| message.duration_ms)
                .collect::<Vec<_>>(),
            vec![Some(1_000), Some(4_000), Some(2_000)]
        );
    }

    #[test]
    fn test_duration_fixture_aggregates_and_serializes_performance() {
        let file = create_test_file(CODEX_DURATION_FIXTURE);
        let messages = parse_codex_file(file.path());

        let entries = aggregate_model_usage_entries(messages, &GroupBy::ClientModel);

        assert_eq!(entries.len(), 1);
        let performance = &entries[0].performance;
        assert_eq!(performance.total_duration_ms, 7_000);
        assert_eq!(performance.timed_tokens, 160);
        assert_eq!(performance.sample_count, 3);
        assert_eq!(performance.token_coverage, 1.0);
        let expected_ms_per_1k = 7_000.0 * 1_000.0 / 160.0;
        assert!((performance.ms_per_1k_tokens.unwrap() - expected_ms_per_1k).abs() < f64::EPSILON);

        let json = serde_json::to_value(performance).unwrap();
        assert_eq!(json["totalDurationMs"], 7_000);
        assert_eq!(json["timedTokens"], 160);
        assert_eq!(json["sampleCount"], 3);
        assert_eq!(json["tokenCoverage"], 1.0);
        assert!(json["msPer1KTokens"].is_number());
        assert!(json.get("total_duration_ms").is_none());
    }

    #[test]
    fn test_headless_usage_nested_data() {
        let content = r#"{"type":"result","data":{"model_name":"gpt-4o","usage":{"input_tokens":50,"cached_input_tokens":5,"output_tokens":12}}}"#;
        let file = create_test_file(content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o");
        assert_eq!(messages[0].tokens.input, 45);
        assert_eq!(messages[0].tokens.output, 12);
        assert_eq!(messages[0].tokens.cache_read, 5);
    }

    #[test]
    fn test_incremental_parse_matches_full_parse_for_appended_lines() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"chat","model_provider":"openai","agent_nickname":"builder","cwd":"/Users/alice/codex-demo"}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            "\n"
        ));

        let initial_size = file.as_file().metadata().unwrap().len();
        let initial = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert_eq!(initial.messages.len(), 1);
        assert_eq!(initial.consumed_offset, initial_size);
        assert_eq!(
            initial.messages[0].workspace_key.as_deref(),
            Some("/Users/alice/codex-demo")
        );
        assert_eq!(
            initial.messages[0].workspace_label.as_deref(),
            Some("codex-demo")
        );

        let appended = concat!(
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":5},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2}}}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":22,"cached_input_tokens":4,"output_tokens":7},"last_token_usage":{"input_tokens":7,"cached_input_tokens":1,"output_tokens":2}}}}"#,
            "\n"
        );

        let mut reopened = file.reopen().unwrap();
        reopened.seek(SeekFrom::End(0)).unwrap();
        reopened.write_all(appended.as_bytes()).unwrap();
        reopened.flush().unwrap();

        let incremental =
            parse_codex_file_incremental(file.path(), initial_size, initial.state.clone());
        let mut combined = initial.messages.clone();
        combined.extend(incremental.messages);
        assert_eq!(
            incremental.consumed_offset,
            file.as_file().metadata().unwrap().len()
        );

        let full = parse_codex_file(file.path());
        assert_eq!(combined, full);
        assert_eq!(
            combined
                .iter()
                .map(|msg| msg.workspace_key.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("/Users/alice/codex-demo"),
                Some("/Users/alice/codex-demo"),
                Some("/Users/alice/codex-demo")
            ]
        );
    }

    #[test]
    fn test_token_count_before_turn_context_uses_later_model() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"interactive","model_provider":"openai","agent_nickname":"builder","cwd":"/Users/alice/codex-demo"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":5,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":0}}}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:04Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":22,"cached_input_tokens":4,"output_tokens":7,"reasoning_output_tokens":2},"last_token_usage":{"input_tokens":7,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":1}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["gpt-5.5", "gpt-5.5", "gpt-5.5"]
        );
        assert_eq!(
            messages
                .iter()
                .map(|message| message.workspace_key.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("/Users/alice/codex-demo"),
                Some("/Users/alice/codex-demo"),
                Some("/Users/alice/codex-demo")
            ]
        );
        assert_eq!(messages[0].tokens.input, 8);
        assert_eq!(messages[0].tokens.output, 2);
        assert_eq!(messages[0].tokens.cache_read, 2);
        assert_eq!(messages[0].tokens.reasoning, 1);
        assert_eq!(messages[1].tokens.input, 4);
        assert_eq!(messages[1].tokens.output, 2);
        assert_eq!(messages[1].tokens.cache_read, 1);
        assert_eq!(messages[1].tokens.reasoning, 0);
        assert_eq!(messages[2].tokens.input, 6);
        assert_eq!(messages[1].tokens.output, 2);
        assert_eq!(messages[2].tokens.cache_read, 1);
        assert_eq!(messages[2].tokens.reasoning, 1);

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(!parsed.unresolved_model_events);
    }

    #[test]
    fn test_token_count_without_model_stays_unknown_but_is_not_cacheable() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"interactive","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#,
            "\n"
        ));

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(parsed.parse_succeeded);
        assert!(parsed.unresolved_model_events);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].model_id, "unknown");
    }

    #[test]
    fn test_model_only_headless_line_flushes_pending_token_counts() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"interactive","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#,
            "\n",
            r#"{"model":"gpt-5.5","type":"metadata"}"#,
            "\n"
        ));

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(parsed.parse_succeeded);
        assert!(!parsed.unresolved_model_events);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].model_id, "gpt-5.5");
    }

    #[test]
    fn test_parse_reader_marks_failure_on_line_read_error() {
        let reader = FailAfterFirstLine::new(concat!(
            r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            "\n"
        ));

        let parsed = parse_codex_reader(reader, "session", 0, 0, CodexParseState::default());

        assert!(!parsed.parse_succeeded);
        assert!(parsed.messages.is_empty());
    }

    #[test]
    fn test_parse_file_returns_empty_on_invalid_utf8_line_error() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            concat!(
                r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                "\n"
            )
            .as_bytes(),
        )
        .unwrap();
        file.write_all(&[0xff, b'\n']).unwrap();
        file.flush().unwrap();

        let messages = parse_codex_file(file.path());
        assert!(messages.is_empty());

        let incremental = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(!incremental.parse_succeeded);
    }

    #[test]
    fn test_parse_file_preserves_valid_messages_after_late_invalid_utf8_line_error() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            concat!(
                r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
                "\n"
            )
            .as_bytes(),
        )
        .unwrap();
        file.write_all(&[0xff, b'\n']).unwrap();
        file.flush().unwrap();

        let messages = parse_codex_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.4");
        assert_eq!(messages[0].tokens.input, 8);
        assert_eq!(messages[0].tokens.output, 3);
        assert_eq!(messages[0].tokens.cache_read, 2);

        let incremental = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(!incremental.parse_succeeded);
        assert_eq!(incremental.messages.len(), 1);
    }

    #[test]
    fn test_session_meta_exec_marks_headless() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"originator":"codex_exec","source":"exec"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#;
        let content = format!("{}\n{}", line1, line2);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent.as_deref(), Some(CODEX_HEADLESS_AGENT));
    }

    #[test]
    fn test_session_meta_openclaw_originator_retags_the_rollout_as_openclaw() {
        // OpenClaw drives Codex app-server with `clientInfo.name: "openclaw"`,
        // which Codex records as the rollout's originator. The turns are
        // OpenClaw's usage, so they leave the parser tagged `openclaw` and
        // keyed by the Codex thread id instead of the file stem.
        let line1 = r#"{"timestamp":"2026-08-30T10:00:00Z","type":"session_meta","payload":{"id":"thread-1","originator":"openclaw","cli_version":"0.120.0","source":"cli","model_provider":"openai","cwd":"/Users/alice/.openclaw/workspace"}}"#;
        let line2 = r#"{"timestamp":"2026-08-30T10:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2-codex"}}"#;
        let line3 = r#"{"timestamp":"2026-08-30T10:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30}}}}"#;
        let content = format!("{}\n{}\n{}\n", line1, line2, line3);
        let file = create_test_file(&content);

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(parsed.parse_succeeded);
        assert!(parsed.state.session_owned_by_openclaw);
        assert_eq!(parsed.consumed_offset, content.len() as u64);
        assert_eq!(parsed.messages.len(), 1);
        let message = &parsed.messages[0];
        assert_eq!(message.client, "openclaw");
        assert_eq!(message.session_id, "thread-1");
        assert_eq!(message.model_id, "gpt-5.2-codex");
        assert_eq!(message.provider_id, "openai");
        assert_eq!(message.tokens.input, 80);
        assert_eq!(message.tokens.cache_read, 20);
        assert_eq!(message.tokens.output, 30);

        // Case only differs by product spelling; still OpenClaw.
        let capitalized =
            content.replace(r#""originator":"openclaw""#, r#""originator":"OpenClaw""#);
        let file = create_test_file(&capitalized);
        let messages = parse_codex_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "openclaw");
    }

    #[test]
    fn test_openclaw_ownership_survives_incremental_resume() {
        let head = concat!(
            r#"{"timestamp":"2026-08-30T10:00:00Z","type":"session_meta","payload":{"id":"thread-1","originator":"openclaw","source":"cli","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-30T10:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2-codex"}}"#,
            "\n"
        );
        let file = create_test_file(head);
        let first = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(first.messages.is_empty());
        assert!(first.state.session_owned_by_openclaw);
        assert_eq!(first.consumed_offset, head.len() as u64);

        // A later turn appends token counts; the resumed parse never re-reads
        // the session_meta line, so ownership has to come from the state.
        let tail = concat!(
            r#"{"timestamp":"2026-08-30T10:05:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30}}}}"#,
            "\n"
        );
        std::fs::OpenOptions::new()
            .append(true)
            .open(file.path())
            .unwrap()
            .write_all(tail.as_bytes())
            .unwrap();

        let resumed = parse_codex_file_incremental(file.path(), first.consumed_offset, first.state);
        assert_eq!(resumed.messages.len(), 1);
        assert_eq!(resumed.messages[0].client, "openclaw");
        assert_eq!(resumed.messages[0].session_id, "thread-1");
        assert_eq!(resumed.consumed_offset, (head.len() + tail.len()) as u64);
    }

    #[test]
    fn test_other_originators_stay_codex() {
        for originator in ["codex_cli_rs", "codex-tui", "Codex Desktop", "codex_exec"] {
            let line1 = format!(
                r#"{{"timestamp":"2026-08-30T10:00:00Z","type":"session_meta","payload":{{"id":"thread-1","originator":"{originator}","source":"cli","model_provider":"openai"}}}}"#
            );
            let line2 = r#"{"timestamp":"2026-08-30T10:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
            let line3 = r#"{"timestamp":"2026-08-30T10:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#;
            let file = create_test_file(&format!("{}\n{}\n{}", line1, line2, line3));
            let messages = parse_codex_file(file.path());
            assert_eq!(
                messages.len(),
                1,
                "originator {originator:?} must keep parsing"
            );
            assert_eq!(messages[0].client, "codex");
            assert_ne!(messages[0].session_id, "thread-1");
        }
    }

    #[test]
    fn test_replayed_parent_originator_does_not_claim_a_forked_child() {
        // A child the user forked in Codex from an OpenClaw-driven thread
        // replays the parent's session_meta (originator openclaw) before its
        // own turns. Ownership comes from the child's own metadata only.
        let child_by_user = concat!(
            r#"{"timestamp":"2026-08-30T10:01:00Z","type":"session_meta","payload":{"id":"child","forked_from_id":"parent","originator":"Codex Desktop","source":"vscode","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-30T10:00:00Z","type":"session_meta","payload":{"id":"parent","originator":"openclaw","source":"cli","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-30T10:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30}}}}"#,
            "\n",
            r#"{"timestamp":"2026-08-30T10:01:02Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-30T10:01:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":30,"output_tokens":45},"last_token_usage":{"input_tokens":50,"cached_input_tokens":10,"output_tokens":15}}}}"#,
            "\n"
        );
        let file = create_test_file(child_by_user);
        let messages = parse_codex_file(file.path());
        assert!(!messages.is_empty());
        assert!(messages.iter().all(|m| m.client == "codex"), "{messages:?}");

        // The mirror image: OpenClaw forked the thread itself.
        let child_by_openclaw = child_by_user.replace(
            r#""id":"child","forked_from_id":"parent","originator":"Codex Desktop""#,
            r#""id":"child","forked_from_id":"parent","originator":"openclaw""#,
        );
        let file = create_test_file(&child_by_openclaw);
        let messages = parse_codex_file(file.path());
        assert!(!messages.is_empty());
        assert!(
            messages.iter().all(|m| m.client == "openclaw"),
            "{messages:?}"
        );
        assert!(
            messages.iter().all(|m| m.session_id == "child"),
            "{messages:?}"
        );
    }

    #[test]
    fn test_turn_coverage_records_the_turns_usage_was_emitted_under() {
        // OpenClaw's transcript mirrors a Codex turn by (thread, turn id), so
        // a rollout has to say which turns it holds usage for. Either of
        // `task_started` and `turn_context` announces a turn; the coverage
        // survives an incremental resume; a rollout written before Codex
        // stamped turn ids can only be matched thread-wide and says so.
        let turn_a = concat!(
            r#"{"timestamp":"2026-08-30T10:00:00Z","type":"session_meta","payload":{"id":"thread-1","originator":"openclaw","source":"cli","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-30T10:00:01Z","type":"event_msg","payload":{"type":"task_started","turn_id":"turn-a","started_at":1756548001}}"#,
            "\n",
            r#"{"timestamp":"2026-08-30T10:00:01Z","type":"turn_context","payload":{"turn_id":"turn-a","model":"gpt-5.2-codex"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-30T10:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30}}}}"#,
            "\n",
        );
        let later_turns = concat!(
            // Announced by `turn_context` alone.
            r#"{"timestamp":"2026-08-30T10:01:01Z","type":"turn_context","payload":{"turn_id":"turn-b","model":"gpt-5.2-codex"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-30T10:01:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":250,"cached_input_tokens":40,"output_tokens":75},"last_token_usage":{"input_tokens":150,"cached_input_tokens":20,"output_tokens":45}}}}"#,
            "\n",
            // Announced by `task_started` alone.
            r#"{"timestamp":"2026-08-30T10:02:01Z","type":"event_msg","payload":{"type":"task_started","turn_id":"turn-c","started_at":1756548121}}"#,
            "\n",
            r#"{"timestamp":"2026-08-30T10:02:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":400,"cached_input_tokens":60,"output_tokens":100},"last_token_usage":{"input_tokens":150,"cached_input_tokens":20,"output_tokens":25}}}}"#,
            "\n",
        );
        let expected_turns = |turns: &[&str]| CodexTurnCoverage {
            turn_ids: turns.iter().map(|turn| turn.to_string()).collect(),
            without_turn_id: false,
        };

        let file = create_test_file(&format!("{turn_a}{later_turns}"));
        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert_eq!(parsed.messages.len(), 3);
        assert_eq!(
            parsed.state.turn_coverage,
            expected_turns(&["turn-a", "turn-b", "turn-c"])
        );

        // Read in two chunks, the way the cache resumes an appended rollout.
        let file = create_test_file(turn_a);
        let first = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert_eq!(first.state.turn_coverage, expected_turns(&["turn-a"]));
        std::fs::OpenOptions::new()
            .append(true)
            .open(file.path())
            .unwrap()
            .write_all(later_turns.as_bytes())
            .unwrap();
        let resumed = parse_codex_file_incremental(file.path(), first.consumed_offset, first.state);
        assert_eq!(resumed.messages.len(), 2);
        assert_eq!(
            resumed.state.turn_coverage,
            expected_turns(&["turn-a", "turn-b", "turn-c"])
        );

        // No turn ids anywhere: usage was emitted, but under no turn.
        let unstamped = turn_a
            .replace(r#""turn_id":"turn-a","#, "")
            .replace(r#"{"turn_id":"turn-a","model""#, r#"{"model""#);
        assert!(!unstamped.contains("turn_id"), "{unstamped}");
        let file = create_test_file(&unstamped);
        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(
            parsed.state.turn_coverage,
            CodexTurnCoverage {
                turn_ids: Default::default(),
                without_turn_id: true,
            }
        );

        // A turn that arrives without an id after turns that had one (a
        // resume under an older Codex) must not be attributed to the last
        // identified turn: it is usage without a turn id.
        let unstamped_later = format!(
            "{turn_a}{}",
            concat!(
                r#"{"timestamp":"2026-08-30T10:03:01Z","type":"turn_context","payload":{"model":"gpt-5.2-codex"}}"#,
                "\n",
                r#"{"timestamp":"2026-08-30T10:03:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":500,"cached_input_tokens":70,"output_tokens":120},"last_token_usage":{"input_tokens":100,"cached_input_tokens":10,"output_tokens":20}}}}"#,
                "\n",
            )
        );
        let file = create_test_file(&unstamped_later);
        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(
            parsed.state.turn_coverage,
            CodexTurnCoverage {
                turn_ids: ["turn-a".to_string()].into_iter().collect(),
                without_turn_id: true,
            }
        );

        // A `turn_context` without an id right after its own `task_started`
        // keeps the announced id.
        let announced_only = format!(
            "{turn_a}{}",
            concat!(
                r#"{"timestamp":"2026-08-30T10:03:01Z","type":"event_msg","payload":{"type":"task_started","turn_id":"turn-d","started_at":1756548181}}"#,
                "\n",
                r#"{"timestamp":"2026-08-30T10:03:01Z","type":"turn_context","payload":{"model":"gpt-5.2-codex"}}"#,
                "\n",
                r#"{"timestamp":"2026-08-30T10:03:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":500,"cached_input_tokens":70,"output_tokens":120},"last_token_usage":{"input_tokens":100,"cached_input_tokens":10,"output_tokens":20}}}}"#,
                "\n",
            )
        );
        let file = create_test_file(&announced_only);
        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(
            parsed.state.turn_coverage,
            expected_turns(&["turn-a", "turn-d"])
        );

        // Once the announced turn has produced usage, a later `turn_context`
        // without an id is another turn, not a late confirmation of that
        // one: its usage is recorded without a turn id.
        let announced_then_unstamped = format!(
            "{turn_a}{}",
            concat!(
                r#"{"timestamp":"2026-08-30T10:03:01Z","type":"event_msg","payload":{"type":"task_started","turn_id":"turn-d","started_at":1756548181}}"#,
                "\n",
                r#"{"timestamp":"2026-08-30T10:03:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":500,"cached_input_tokens":70,"output_tokens":120},"last_token_usage":{"input_tokens":100,"cached_input_tokens":10,"output_tokens":20}}}}"#,
                "\n",
                r#"{"timestamp":"2026-08-30T10:04:01Z","type":"turn_context","payload":{"model":"gpt-5.2-codex"}}"#,
                "\n",
                r#"{"timestamp":"2026-08-30T10:04:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":700,"cached_input_tokens":90,"output_tokens":150},"last_token_usage":{"input_tokens":200,"cached_input_tokens":20,"output_tokens":30}}}}"#,
                "\n",
            )
        );
        let file = create_test_file(&announced_then_unstamped);
        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert_eq!(parsed.messages.len(), 3);
        assert_eq!(
            parsed.state.turn_coverage,
            CodexTurnCoverage {
                turn_ids: ["turn-a", "turn-d"]
                    .map(str::to_string)
                    .into_iter()
                    .collect(),
                without_turn_id: true,
            }
        );

        // A rollout with no usage covers nothing.
        let file = create_test_file(
            r#"{"timestamp":"2026-08-30T10:00:00Z","type":"session_meta","payload":{"id":"thread-1","originator":"openclaw"}}
{"timestamp":"2026-08-30T10:00:01Z","type":"turn_context","payload":{"turn_id":"turn-a","model":"gpt-5.2-codex"}}
"#,
        );
        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(parsed.messages.is_empty());
        assert_eq!(parsed.state.turn_coverage, CodexTurnCoverage::default());
    }

    #[test]
    fn test_thread_id_from_rollout_path() {
        assert_eq!(
            thread_id_from_rollout_path(Path::new(
                "/x/codex-home/sessions/2026/08/30/rollout-2026-08-30T10-00-00-0192f3a4-5b6c-7d8e-9f01-23456789abcd.jsonl"
            ))
            .as_deref(),
            Some("0192f3a4-5b6c-7d8e-9f01-23456789abcd")
        );
        assert_eq!(
            thread_id_from_rollout_path(Path::new("/x/session.jsonl")),
            None
        );
        assert_eq!(
            thread_id_from_rollout_path(Path::new(
                "/x/rollout-2026-08-30T10-00-00-not-a-uuid.jsonl"
            )),
            None
        );
    }

    #[test]
    fn test_token_count_uses_total_deltas_when_totals_repeat() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 25);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
    }

    #[test]
    fn test_token_count_falls_back_to_last_usage_when_totals_reset() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 25);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 2);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);
    }

    #[test]
    fn test_token_count_advances_baseline_after_missing_total_fallback() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":110,"cached_input_tokens":22,"output_tokens":33,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}\n{}", line1, line2, line3, line4);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 25);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 2);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);
    }

    #[test]
    fn test_token_count_skips_regressed_totals_without_last_usage() {
        // When totals regress and last_usage is absent, the row should be
        // skipped entirely to avoid double-counting the full cumulative total.
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        // Totals regress (lower values) and no last_token_usage — should skip
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"cached_input_tokens":10,"output_tokens":15,"reasoning_output_tokens":2}}}}"#;
        // Normal continuation after reset
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":80,"cached_input_tokens":15,"output_tokens":25,"reasoning_output_tokens":4}}}}"#;
        let content = format!("{}\n{}\n{}\n{}", line1, line2, line3, line4);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        // Should produce 2 messages: first from line2 (full total),
        // then delta from line4 relative to line3 (baseline reset).
        assert_eq!(messages.len(), 2);
        // First message: full total
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 25);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        // Second message: delta from 50→80
        assert_eq!(messages[1].tokens.input, 25);
        assert_eq!(messages[1].tokens.output, 8);
        assert_eq!(messages[1].tokens.cache_read, 5);
        assert_eq!(messages[1].tokens.reasoning, 2);
    }

    #[test]
    fn test_into_tokens_splits_reasoning_out_of_output() {
        // A real `token_count` snapshot from ~/.codex/sessions. Codex reports
        // `total_tokens == input_tokens + output_tokens`, with
        // `reasoning_output_tokens` counted inside `output_tokens` rather than
        // beside it.
        let totals = CodexTotals {
            input: 16_845_360,
            output: 63_820,
            cached: 16_358_912,
            reasoning: 24_882,
        };
        let reported_total = totals.input + totals.output; // 16_909_180

        let tokens = totals.into_tokens();

        // Conservation: the additive buckets must land back on Codex's own
        // total. This only holds when reasoning was split out of output; if the
        // split regresses, the sum overshoots by exactly the reasoning count.
        assert_eq!(tokens.total(), reported_total);
        assert_eq!(tokens.reasoning, 24_882);
        assert_eq!(tokens.output, 63_820 - 24_882);
        assert_eq!(tokens.cache_read, 16_358_912);
        assert_eq!(tokens.input, 16_845_360 - 16_358_912);
    }

    #[test]
    fn test_into_tokens_clamps_reasoning_to_output() {
        // A malformed row claiming more reasoning than output must not drive
        // the output bucket negative, and must not inflate the total.
        let totals = CodexTotals {
            input: 100,
            output: 10,
            cached: 0,
            reasoning: 999,
        };

        let tokens = totals.into_tokens();

        assert_eq!(tokens.output, 0);
        assert_eq!(tokens.reasoning, 10);
        assert_eq!(tokens.total(), 110);
    }

    #[test]
    fn test_into_tokens_clamps_cached_to_input() {
        // When cached > input (malformed data), cached should be clamped to input
        // so that input + cache_read never exceeds the raw input value.
        let totals = CodexTotals {
            input: 50,
            output: 30,
            cached: 100, // More than input — malformed
            reasoning: 5,
        };
        let tokens = totals.into_tokens();
        assert_eq!(tokens.cache_read, 50); // Clamped to input
        assert_eq!(tokens.input, 0); // input - clamped_cached = 0
                                     // Reasoning is a subset of output, so the output bucket carries only
                                     // the non-reasoning remainder.
        assert_eq!(tokens.output, 25);
        assert_eq!(tokens.reasoning, 5);
        assert_eq!(tokens.total(), 80); // 0 + 25 + 50 + 0 + 5
    }

    #[test]
    fn test_token_count_ignores_negative_fallback_usage_in_baseline() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":-10,"cached_input_tokens":-2,"output_tokens":-3,"reasoning_output_tokens":-1}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":110,"cached_input_tokens":22,"output_tokens":33,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}\n{}", line1, line2, line3, line4);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 25);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 2);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);
    }

    #[test]
    fn test_token_count_avoids_double_counting_stale_cumulative_regressions() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":110,"cached_input_tokens":22,"output_tokens":33,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":109,"cached_input_tokens":21,"output_tokens":32,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":9,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":0}}}}"#;
        let line5 = r#"{"timestamp":"2026-01-01T00:00:04Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":119,"cached_input_tokens":23,"output_tokens":35,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":0}}}}"#;
        let content = format!("{}\n{}\n{}\n{}\n{}", line1, line2, line3, line4, line5);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[0].tokens.output, 25);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);

        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 2);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);

        // Stale snapshot (line4) is now skipped entirely; messages[2]
        // comes from line5's last_token_usage instead.
        assert_eq!(messages[2].tokens.input, 8);
        assert_eq!(messages[2].tokens.output, 3);
        assert_eq!(messages[2].tokens.cache_read, 2);
        assert_eq!(messages[2].tokens.reasoning, 0);
    }

    #[test]
    fn test_token_count_handles_multiple_stale_regressions_before_recovery() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5},"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":30,"reasoning_output_tokens":5}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":110,"cached_input_tokens":22,"output_tokens":33,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":109,"cached_input_tokens":21,"output_tokens":32,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":9,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":0}}}}"#;
        let line5 = r#"{"timestamp":"2026-01-01T00:00:04Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":118,"cached_input_tokens":22,"output_tokens":34,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":9,"cached_input_tokens":1,"output_tokens":2,"reasoning_output_tokens":0}}}}"#;
        let line6 = r#"{"timestamp":"2026-01-01T00:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":128,"cached_input_tokens":24,"output_tokens":37,"reasoning_output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":0}}}}"#;
        let content = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            line1, line2, line3, line4, line5, line6
        );
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        // Stale line4 is skipped; messages come from lines 2, 3, 5, 6.
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].tokens.input, 80);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[2].tokens.input, 8);
        assert_eq!(messages[2].tokens.output, 2);
        assert_eq!(messages[2].tokens.cache_read, 1);
        assert_eq!(messages[2].tokens.reasoning, 0);
        assert_eq!(messages[3].tokens.input, 8);
        assert_eq!(messages[3].tokens.output, 3);
        assert_eq!(messages[3].tokens.cache_read, 2);
        assert_eq!(messages[3].tokens.reasoning, 0);
    }

    #[test]
    fn test_token_count_treats_large_regressions_as_real_resets() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10000,"cached_input_tokens":1000,"output_tokens":400,"reasoning_output_tokens":50},"last_token_usage":{"input_tokens":10000,"cached_input_tokens":1000,"output_tokens":400,"reasoning_output_tokens":50}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":7600,"cached_input_tokens":800,"output_tokens":280,"reasoning_output_tokens":35},"last_token_usage":{"input_tokens":25,"cached_input_tokens":5,"output_tokens":4,"reasoning_output_tokens":1}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":7625,"cached_input_tokens":805,"output_tokens":284,"reasoning_output_tokens":36},"last_token_usage":{"input_tokens":25,"cached_input_tokens":5,"output_tokens":4,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}\n{}", line1, line2, line3, line4);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].tokens.input, 9000);
        assert_eq!(messages[0].tokens.output, 350);
        assert_eq!(messages[0].tokens.cache_read, 1000);
        assert_eq!(messages[0].tokens.reasoning, 50);

        assert_eq!(messages[1].tokens.input, 20);
        assert_eq!(messages[1].tokens.output, 3);
        assert_eq!(messages[1].tokens.cache_read, 5);
        assert_eq!(messages[1].tokens.reasoning, 1);

        assert_eq!(messages[2].tokens.input, 20);
        assert_eq!(messages[2].tokens.output, 3);
        assert_eq!(messages[2].tokens.cache_read, 5);
        assert_eq!(messages[2].tokens.reasoning, 1);
    }

    #[test]
    fn test_first_event_uses_last_not_total_for_resumed_sessions() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5000,"cached_input_tokens":500,"output_tokens":800,"reasoning_output_tokens":100},"last_token_usage":{"input_tokens":12,"cached_input_tokens":2,"output_tokens":5,"reasoning_output_tokens":1}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":5012,"cached_input_tokens":502,"output_tokens":805,"reasoning_output_tokens":101},"last_token_usage":{"input_tokens":12,"cached_input_tokens":2,"output_tokens":5,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 4);
        assert_eq!(messages[0].tokens.cache_read, 2);
        assert_eq!(messages[0].tokens.reasoning, 1);
        assert_eq!(messages[1].tokens.input, 10);
        assert_eq!(messages[1].tokens.output, 4);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);
    }

    #[test]
    fn test_zero_token_snapshot_does_not_inflate_later_deltas() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":500,"cached_input_tokens":50,"output_tokens":80,"reasoning_output_tokens":10},"last_token_usage":{"input_tokens":500,"cached_input_tokens":50,"output_tokens":80,"reasoning_output_tokens":10}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0},"last_token_usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0}}}}"#;
        let line4 = r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":510,"cached_input_tokens":52,"output_tokens":83,"reasoning_output_tokens":11},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}\n{}", line1, line2, line3, line4);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 450);
        assert_eq!(messages[0].tokens.output, 70);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.reasoning, 10);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 2);
        assert_eq!(messages[1].tokens.cache_read, 2);
        assert_eq!(messages[1].tokens.reasoning, 1);
    }

    #[test]
    fn test_model_info_slug_from_turn_context() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model_info":{"slug":"o3-pro"}}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}", line1, line2);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "o3-pro");
        assert_eq!(messages[0].duration_ms, Some(1000));
    }

    #[test]
    fn test_session_meta_provider_and_agent() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","model_provider":"azure","agent_nickname":"my-agent"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "azure");
        assert_eq!(messages[0].agent.as_deref(), Some(CODEX_DEFAULT_AGENT));
    }

    #[test]
    fn test_session_meta_object_source_keeps_provider_agent_and_workspace() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"fork-session","forked_from_id":"parent-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/Users/alice/codex-fork"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].agent.as_deref(), Some(CODEX_SUBAGENT_AGENT));
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/alice/codex-fork")
        );
        assert!(messages[0].dedup_key.is_some());
    }

    fn single_turn_agent(session_meta_lines: &str) -> Option<String> {
        let file = create_test_file(&format!(
            "{session_meta_lines}\n{}\n{}\n",
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        messages[0].agent.clone()
    }

    #[test]
    fn test_guardian_review_thread_gets_guardian_agent() {
        let agent = single_turn_agent(
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"guardian-thread","parent_thread_id":"parent-thread","source":{"subagent":{"other":"guardian"}},"thread_source":"guardian_review","model_provider":"openai","cwd":"/repo"}}"#,
        );

        assert_eq!(agent.as_deref(), Some(CODEX_GUARDIAN_AGENT));
    }

    #[test]
    fn test_guardian_thread_tagged_subagent_gets_guardian_agent() {
        // Older Codex builds tagged the same guardian thread
        // `thread_source: "subagent"`; it must not land in the subagent row.
        let agent = single_turn_agent(
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"guardian-thread","parent_thread_id":"parent-thread","source":{"subagent":{"other":"guardian"}},"thread_source":"subagent","model_provider":"openai","cwd":"/repo"}}"#,
        );

        assert_eq!(agent.as_deref(), Some(CODEX_GUARDIAN_AGENT));
    }

    #[test]
    fn test_non_spawn_subagent_source_gets_subagent_agent() {
        let agent = single_turn_agent(
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"review-thread","source":{"subagent":"review"},"model_provider":"openai","cwd":"/repo"}}"#,
        );

        assert_eq!(agent.as_deref(), Some(CODEX_SUBAGENT_AGENT));
    }

    #[test]
    fn test_later_session_meta_keeps_guardian_agent() {
        let agent = single_turn_agent(concat!(
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"guardian-thread","source":{"subagent":{"other":"guardian"}},"thread_source":"guardian_review","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:00.500Z","type":"session_meta","payload":{"id":"parent-thread","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
        ));

        assert_eq!(agent.as_deref(), Some(CODEX_GUARDIAN_AGENT));
    }

    #[test]
    fn test_forked_child_ignores_inherited_records_before_turn_context() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","forked_from_id":"parent-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.992Z","type":"session_meta","payload":{"id":"parent-session","source":"interactive","model_provider":"azure","agent_nickname":"parent","cwd":"/repo-parent"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.993Z","type":"event_msg","payload":{"type":"user_message","message":"parent prompt copied into child log"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":116000,"cached_input_tokens":114000,"output_tokens":1000,"total_tokens":117000},"last_token_usage":{"input_tokens":73000,"cached_input_tokens":72000,"output_tokens":500,"total_tokens":73500}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":116000,"cached_input_tokens":114000,"output_tokens":1000,"total_tokens":117000},"last_token_usage":{"input_tokens":73000,"cached_input_tokens":72000,"output_tokens":500,"total_tokens":73500}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":117500,"cached_input_tokens":115000,"output_tokens":1200,"reasoning_output_tokens":50,"total_tokens":118700},"last_token_usage":{"input_tokens":1500,"cached_input_tokens":1000,"output_tokens":200,"reasoning_output_tokens":50,"total_tokens":1700}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.5");
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].agent.as_deref(), Some(CODEX_SUBAGENT_AGENT));
        assert_eq!(messages[0].workspace_key.as_deref(), Some("/repo-child"));
        assert_eq!(messages[0].tokens.input, 500);
        assert_eq!(messages[0].tokens.cache_read, 1000);
        assert_eq!(messages[0].tokens.output, 150);
        assert_eq!(messages[0].tokens.reasoning, 50);
    }

    #[test]
    fn test_forked_child_ignores_replayed_parent_rows_after_turn_context() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","forked_from_id":"parent-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55},"last_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.949Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":250,"output_tokens":25,"total_tokens":275}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":310,"output_tokens":32,"total_tokens":342},"last_token_usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.5");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 2);
    }

    #[test]
    fn test_forked_child_submit_cap_regression_skips_large_inherited_cache_replays() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","forked_from_id":"parent-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1,"agent_role":"architect"}}},"model_provider":"openai","agent_nickname":"architect","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1200000000,"cached_input_tokens":1180000000,"output_tokens":1000000,"reasoning_output_tokens":100000,"total_tokens":1201100000},"last_token_usage":{"input_tokens":750000000,"cached_input_tokens":740000000,"output_tokens":500000,"reasoning_output_tokens":50000,"total_tokens":750550000}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1180000000,"cached_input_tokens":1160000000,"output_tokens":900000,"reasoning_output_tokens":90000,"total_tokens":1180990000},"last_token_usage":{"input_tokens":20000000,"cached_input_tokens":20000000,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":20000000}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.949Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1200000000,"cached_input_tokens":1180000000,"output_tokens":1000000,"reasoning_output_tokens":100000,"total_tokens":1201100000},"last_token_usage":{"input_tokens":20000000,"cached_input_tokens":20000000,"output_tokens":100000,"reasoning_output_tokens":10000,"total_tokens":20110000}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1200001500,"cached_input_tokens":1180001000,"output_tokens":1000200,"reasoning_output_tokens":100050,"total_tokens":1201101750},"last_token_usage":{"input_tokens":1500,"cached_input_tokens":1000,"output_tokens":200,"reasoning_output_tokens":50,"total_tokens":1750}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.5");
        assert_eq!(messages[0].tokens.input, 500);
        assert_eq!(messages[0].tokens.cache_read, 1000);
        assert_eq!(messages[0].tokens.output, 150);
        assert_eq!(messages[0].tokens.reasoning, 50);
    }

    #[test]
    fn test_forked_child_detects_thread_spawn_source_without_top_level_fork_id() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55},"last_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":310,"output_tokens":32,"total_tokens":342},"last_token_usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.5");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 2);
    }

    #[test]
    fn test_user_forked_child_counts_own_turn_after_parent_replay() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-01-02T03:10:00.000Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.001Z","type":"session_meta","payload":{"id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.100Z","type":"turn_context","payload":{"turn_id":"11111111-3333-7333-8333-333333333333","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100}}}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:30.100Z","type":"turn_context","payload":{"turn_id":"22222222-4444-7444-8444-444444444444","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:30.200Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:31.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1250,"cached_input_tokens":450,"output_tokens":120,"total_tokens":1370},"last_token_usage":{"input_tokens":250,"cached_input_tokens":50,"output_tokens":20,"total_tokens":270}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 200);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.output, 20);
    }

    #[test]
    fn test_user_forked_child_same_millisecond_own_turn_counts_without_task_started() {
        // Human (`thread_source:"user"`) fork where the child session_meta and
        // the child's own first turn_context are minted in the SAME millisecond,
        // so both UUID v7 ids share the 48-bit prefix (`22222222-2222`). A user
        // fork never emits a `task_started`, so a same-millisecond gate that
        // requires `task_started` would keep skipping forever and drop the
        // child's own turn (0 messages). The replayed parent turn carries the
        // *parent's* millisecond prefix (`11111111`), so it sorts strictly
        // earlier and is still skipped; only the child's own turn — the one that
        // shares the child's fork millisecond — must end the skip and be counted
        // (200/20 delta from the inherited 1000/100 baseline).
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-01-02T03:10:00.000Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.001Z","type":"session_meta","payload":{"id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.100Z","type":"turn_context","payload":{"turn_id":"11111111-3333-7333-8333-333333333333","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100}}}}"#,
            "\n",
            // child's own turn: turn_id shares the child session's millisecond
            // prefix (`22222222-2222`) — same-millisecond tie with the fork.
            r#"{"timestamp":"2026-01-02T03:10:30.100Z","type":"turn_context","payload":{"turn_id":"22222222-2222-7444-8444-444444444444","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:30.200Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:31.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1250,"cached_input_tokens":450,"output_tokens":120,"total_tokens":1370},"last_token_usage":{"input_tokens":250,"cached_input_tokens":50,"output_tokens":20,"total_tokens":270}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 200);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.output, 20);
    }

    #[test]
    fn test_user_fork_replayed_parent_shares_child_ms_ends_skip_early() {
        // Documents (locks) the accepted residual called out at the Equal branch:
        // a human (`thread_source:"user"`) fork resolves a same-millisecond tie on
        // the millisecond prefix alone, because user forks never emit a
        // `task_started` to harden the gate. Here the *replayed parent* turn is
        // (pathologically) minted within the exact same 1ms as the child's fork
        // session_meta, so it shares the *child's* prefix (`22222222-2222`) rather
        // than the parent's. Because the gate cannot distinguish it from the
        // child's own turn, it ends the skip one turn early and counts that
        // replayed parent row (500/50 delta off the 1000/100 baseline) as the
        // child's first turn. This is a sub-millisecond, human-paced coincidence;
        // the test pins the CURRENT behavior so any future change is intentional.
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-01-02T03:10:00.000Z","type":"session_meta","payload":{"id":"22222222-2222-7222-8222-222222222222","forked_from_id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.001Z","type":"session_meta","payload":{"id":"11111111-1111-7111-8111-111111111111","source":"vscode","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100},"last_token_usage":{"input_tokens":1000,"cached_input_tokens":400,"output_tokens":100,"total_tokens":1100}}}}"#,
            "\n",
            // replayed parent turn whose turn_id coincidentally shares the child's
            // fork millisecond prefix (`22222222-2222`) — equal-prefix tie. With a
            // user fork (no task_started) this ends the skip here, one turn early.
            r#"{"timestamp":"2026-01-02T03:10:00.200Z","type":"turn_context","payload":{"turn_id":"22222222-2222-7333-8333-333333333333","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:00.300Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":450,"output_tokens":150,"total_tokens":1650},"last_token_usage":{"input_tokens":500,"cached_input_tokens":50,"output_tokens":50,"total_tokens":550}}}}"#,
            "\n",
            // the child's actual own turn (also shares the child's prefix).
            r#"{"timestamp":"2026-01-02T03:10:30.100Z","type":"turn_context","payload":{"turn_id":"22222222-2222-7444-8444-444444444444","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-02T03:10:31.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1750,"cached_input_tokens":500,"output_tokens":170,"total_tokens":1920},"last_token_usage":{"input_tokens":250,"cached_input_tokens":50,"output_tokens":20,"total_tokens":270}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        // CURRENT behavior: the skip ends one turn early at the equal-prefix
        // replayed parent turn, so BOTH that row and the child's own turn are
        // counted (two messages) rather than only the child's own turn. The
        // first message is the replayed parent delta (total 1500-1000=500, of
        // which 50 is cache_read, leaving 450 non-cached input + 50 output); the
        // second is the child's own delta (250-50=200 input + 20 output). A
        // future change that hardened this tie would instead yield a single
        // message with the child's 200/20.
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.input, 450);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[1].tokens.input, 200);
        assert_eq!(messages[1].tokens.cache_read, 50);
        assert_eq!(messages[1].tokens.output, 20);
    }

    #[test]
    fn test_forked_child_skips_nested_parent_replay_until_own_turn() {
        let parent = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"019e5b00-0000-7000-8000-000000000001","source":"vscode","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.000Z","type":"turn_context","payload":{"turn_id":"019e5b00-0001-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n"
        ));
        let child = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5c03-1e99-7000-8000-000000000001","forked_from_id":"019e5b00-0000-7000-8000-000000000001","source":{"subagent":{"thread_spawn":{"parent_thread_id":"019e5b00-0000-7000-8000-000000000001","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5b00-0000-7000-8000-000000000001","source":"vscode","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.100Z","type":"turn_context","payload":{"turn_id":"019e5b00-0001-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.000Z","type":"event_msg","payload":{"type":"task_started","turn_id":"019e5c03-6425-7000-8000-000000000001"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.100Z","type":"turn_context","payload":{"turn_id":"019e5c03-6425-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":320,"output_tokens":32,"total_tokens":352},"last_token_usage":{"input_tokens":20,"output_tokens":2,"total_tokens":22}}}}"#,
            "\n"
        ));

        let parent_messages = parse_codex_file(parent.path());
        let child_messages = parse_codex_file(child.path());

        assert_eq!(parent_messages.len(), 1);
        assert_eq!(child_messages.len(), 1);
        assert_ne!(parent_messages[0].dedup_key, child_messages[0].dedup_key);
        assert_eq!(child_messages[0].tokens.input, 20);
        assert_eq!(child_messages[0].tokens.output, 2);
    }

    #[test]
    fn test_forked_child_same_millisecond_turn_starts_own_session() {
        // Regression: the child's own first turn starts in the SAME millisecond
        // as its fork session_meta, so both UUID v7 ids share the 48-bit ms
        // prefix (`019e5c03-1e99`) and differ only in the random tail
        // (`…0001` vs `…00ff`). Comparing the full id makes the gate fall through
        // to the coin-flip tail (here `0001 < 00ff`), so the replay-skip never
        // ends and the child's own turn is dropped. Comparing only the ms prefix
        // keeps the child's own turn.
        let child = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5c03-1e99-7000-8000-0000000000ff","forked_from_id":"019e5b00-0000-7000-8000-000000000001","source":{"subagent":{"thread_spawn":{"parent_thread_id":"019e5b00-0000-7000-8000-000000000001","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5b00-0000-7000-8000-000000000001","source":"vscode","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.100Z","type":"turn_context","payload":{"turn_id":"019e5b00-0001-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.000Z","type":"event_msg","payload":{"type":"task_started","turn_id":"019e5c03-1e99-7000-8000-000000000001"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.100Z","type":"turn_context","payload":{"turn_id":"019e5c03-1e99-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":320,"output_tokens":32,"total_tokens":352},"last_token_usage":{"input_tokens":20,"output_tokens":2,"total_tokens":22}}}}"#,
            "\n"
        ));

        let child_messages = parse_codex_file(child.path());

        assert_eq!(child_messages.len(), 1);
        assert_eq!(child_messages[0].tokens.input, 20);
        assert_eq!(child_messages[0].tokens.output, 2);
    }

    #[test]
    fn test_forked_child_same_millisecond_replayed_parent_turn_keeps_skipping() {
        // A replayed parent `turn_context` can coincidentally share the child's
        // fork millisecond (here both `019e5c03-1e99`) while NOT being preceded
        // by a `task_started`. A millisecond-prefix-only gate would treat that
        // equal-prefix turn as child-local, end the skip early, and count the
        // inherited replayed row (500/50) as the child's own usage. The child's
        // own turn is the later one announced by `task_started`; only it should
        // end the skip, so only its 20/2 delta is counted.
        let child = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5c03-1e99-7000-8000-0000000000ff","forked_from_id":"019e5b00-0000-7000-8000-000000000001","source":{"subagent":{"thread_spawn":{"parent_thread_id":"019e5b00-0000-7000-8000-000000000001","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5b00-0000-7000-8000-000000000001","source":"vscode","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            // replayed parent turn that shares the child's fork millisecond, with
            // NO task_started — must NOT end the skip.
            r#"{"timestamp":"2026-05-05T21:52:10.100Z","type":"turn_context","payload":{"turn_id":"019e5c03-1e99-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":500,"output_tokens":50,"total_tokens":550},"last_token_usage":{"input_tokens":500,"output_tokens":50,"total_tokens":550}}}}"#,
            "\n",
            // the child's real own turn, announced by task_started.
            r#"{"timestamp":"2026-05-05T21:52:20.000Z","type":"event_msg","payload":{"type":"task_started","turn_id":"019e5c03-1e99-7000-8000-000000000002"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.100Z","type":"turn_context","payload":{"turn_id":"019e5c03-1e99-7000-8000-000000000002","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":520,"output_tokens":52,"total_tokens":572},"last_token_usage":{"input_tokens":20,"output_tokens":2,"total_tokens":22}}}}"#,
            "\n"
        ));

        let child_messages = parse_codex_file(child.path());

        assert_eq!(child_messages.len(), 1);
        assert_eq!(child_messages[0].tokens.input, 20);
        assert_eq!(child_messages[0].tokens.output, 2);
    }

    #[test]
    fn test_nested_child_skips_replayed_legacy_uuid_v4_turn() {
        // Nested Codex child logs can replay an ancestor turn whose legacy UUID
        // v4 id cannot be ordered against the child's UUID v7 session id. Its
        // task_started timestamp still predates the child, so it must not open
        // the gate or count the inherited token snapshot.
        let child = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:52:10.197Z","type":"session_meta","payload":{"id":"019e5c03-1f5d-7000-8000-000000000001","forked_from_id":"019e5c03-0000-7000-8000-000000000001","source":{"subagent":{"thread_spawn":{"parent_thread_id":"019e5c03-0000-7000-8000-000000000001","depth":2}}},"thread_source":"subagent","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.197Z","type":"session_meta","payload":{"id":"019e5c03-0000-7000-8000-000000000001","forked_from_id":"019e5b00-0000-7000-8000-000000000001","source":{"subagent":{"thread_spawn":{"parent_thread_id":"019e5b00-0000-7000-8000-000000000001","depth":1}}},"thread_source":"subagent","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.197Z","type":"session_meta","payload":{"id":"019e5b00-0000-7000-8000-000000000001","source":"cli","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.197Z","type":"event_msg","payload":{"type":"task_started","turn_id":"81d2f55b-894b-4d67-b75b-436ead477f65","started_at":1778017800}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.197Z","type":"turn_context","payload":{"turn_id":"81d2f55b-894b-4d67-b75b-436ead477f65","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.198Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.610Z","type":"event_msg","payload":{"type":"task_started","turn_id":"019e5c03-2100-7000-8000-000000000001","started_at":1779660169}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.611Z","type":"turn_context","payload":{"turn_id":"019e5c03-2100-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.612Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":320,"output_tokens":32,"total_tokens":352},"last_token_usage":{"input_tokens":20,"output_tokens":2,"total_tokens":22}}}}"#,
            "\n"
        ));

        let child_messages = parse_codex_file(child.path());

        assert_eq!(child_messages.len(), 1);
        assert_eq!(child_messages[0].tokens.input, 20);
        assert_eq!(child_messages[0].tokens.output, 2);
    }

    #[test]
    fn test_forked_child_legacy_turn_pins_seconds_unit_contract() {
        // Pins the `started_at` unit contract used by
        // `forked_child_task_starts_own_session`: it compares against the
        // child's fork second (`started_at >= child_started_at_ms / 1000`),
        // so a legacy replayed turn timestamped exactly one second before
        // the child's fork second must stay rejected, while one landing on
        // that same second must be admitted. The child's UUID v7 id here
        // (`018bcfe5-6800-...`) encodes ms=1700000000000, i.e.
        // floor(ms/1000) == 1700000000, a round multiple of 1000 so there is
        // no ambiguity from the ms->s truncation.
        let child = create_test_file(concat!(
            r#"{"timestamp":"2023-11-14T22:13:20.000Z","type":"session_meta","payload":{"id":"018bcfe5-6800-7000-8000-000000000001","forked_from_id":"018bcfe5-0000-7000-8000-000000000001","source":{"subagent":{"thread_spawn":{"parent_thread_id":"018bcfe5-0000-7000-8000-000000000001","depth":2}}},"thread_source":"subagent","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2023-11-14T22:13:20.000Z","type":"session_meta","payload":{"id":"018bcfe5-0000-7000-8000-000000000001","forked_from_id":"018bcfe4-0000-7000-8000-000000000001","source":{"subagent":{"thread_spawn":{"parent_thread_id":"018bcfe4-0000-7000-8000-000000000001","depth":1}}},"thread_source":"subagent","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2023-11-14T22:13:20.000Z","type":"session_meta","payload":{"id":"018bcfe4-0000-7000-8000-000000000001","source":"cli","thread_source":"user","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            // legacy turn one second BEFORE the child's fork second
            // (1700000000) -- must NOT open the gate or count its token
            // snapshot.
            r#"{"timestamp":"2023-11-14T22:13:19.000Z","type":"event_msg","payload":{"type":"task_started","turn_id":"71d2f55b-894b-4d67-b75b-436ead477f65","started_at":1699999999}}"#,
            "\n",
            r#"{"timestamp":"2023-11-14T22:13:19.000Z","type":"turn_context","payload":{"turn_id":"71d2f55b-894b-4d67-b75b-436ead477f65","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2023-11-14T22:13:19.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            // legacy turn exactly AT the child's fork second -- must be
            // admitted.
            r#"{"timestamp":"2023-11-14T22:13:20.100Z","type":"event_msg","payload":{"type":"task_started","turn_id":"82d2f55b-894b-4d67-b75b-436ead477f66","started_at":1700000000}}"#,
            "\n",
            r#"{"timestamp":"2023-11-14T22:13:20.200Z","type":"turn_context","payload":{"turn_id":"82d2f55b-894b-4d67-b75b-436ead477f66","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2023-11-14T22:13:20.300Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":320,"output_tokens":32,"total_tokens":352},"last_token_usage":{"input_tokens":20,"output_tokens":2,"total_tokens":22}}}}"#,
            "\n"
        ));

        let child_messages = parse_codex_file(child.path());

        assert_eq!(child_messages.len(), 1);
        assert_eq!(child_messages[0].tokens.input, 20);
        assert_eq!(child_messages[0].tokens.output, 2);
    }

    #[test]
    fn test_forked_child_task_started_non_numeric_started_at_does_not_fail_parsing() {
        // A `task_started` event with a non-integer `started_at` (e.g. a
        // string, from a malformed or unexpected log) must not fail
        // deserialization of the whole JSONL line -- it should decode with
        // `started_at: None`, which keeps the replay gate closed (same as a
        // missing timestamp) and still allows the rest of the file,
        // including a valid subsequent `task_started`, to parse normally.
        let child = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5c03-1e99-7000-8000-000000000001","forked_from_id":"019e5b00-0000-7000-8000-000000000001","source":{"subagent":{"thread_spawn":{"parent_thread_id":"019e5b00-0000-7000-8000-000000000001","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.000Z","type":"session_meta","payload":{"id":"019e5b00-0000-7000-8000-000000000001","source":"vscode","model_provider":"openai","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.100Z","type":"turn_context","payload":{"turn_id":"019e5b00-0001-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:10.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            // legacy task_started with a malformed (non-numeric) started_at
            // -- must decode with started_at: None (not fail the whole
            // entry), so the gate stays closed rather than opening on a
            // wrong-typed value.
            r#"{"timestamp":"2026-05-05T21:52:15.000Z","type":"event_msg","payload":{"type":"task_started","turn_id":"71d2f55b-894b-4d67-b75b-436ead477f65","started_at":"not-a-number"}}"#,
            "\n",
            // the child's real own turn, announced by a well-formed
            // task_started -- proves the malformed line above didn't corrupt
            // parser state or halt parsing of the rest of the file.
            r#"{"timestamp":"2026-05-05T21:52:20.000Z","type":"event_msg","payload":{"type":"task_started","turn_id":"019e5c03-6425-7000-8000-000000000001"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.100Z","type":"turn_context","payload":{"turn_id":"019e5c03-6425-7000-8000-000000000001","model":"gpt-5.5","cwd":"/repo"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:52:20.200Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":320,"output_tokens":32,"total_tokens":352},"last_token_usage":{"input_tokens":20,"output_tokens":2,"total_tokens":22}}}}"#,
            "\n"
        ));

        let parsed = parse_codex_file_incremental(child.path(), 0, CodexParseState::default());

        // The malformed line did not abort file-level parsing.
        assert!(parsed.parse_succeeded);
        // The malformed task_started did not open the gate; only the
        // well-formed one that follows (admitted via the UUID v7 ordering
        // path) did.
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].tokens.input, 20);
        assert_eq!(parsed.messages[0].tokens.output, 2);
    }

    #[test]
    fn test_forked_child_incremental_state_skips_inherited_prefix() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","forked_from_id":"parent-session","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-session","depth":1}}},"model_provider":"openai","agent_nickname":"worker","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.992Z","type":"session_meta","payload":{"id":"parent-session","source":"interactive","model_provider":"azure","agent_nickname":"parent","cwd":"/repo-parent"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":116000,"cached_input_tokens":114000,"output_tokens":1000,"total_tokens":117000},"last_token_usage":{"input_tokens":73000,"cached_input_tokens":72000,"output_tokens":500,"total_tokens":73500}}}}"#,
            "\n"
        ));
        let prefix_size = file.as_file().metadata().unwrap().len();
        let prefix = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(prefix.parse_succeeded);
        assert!(!prefix.unresolved_model_events);
        assert!(prefix.messages.is_empty());

        let appended = concat!(
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5","cwd":"/repo-child"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":116000,"cached_input_tokens":114000,"output_tokens":1000,"total_tokens":117000},"last_token_usage":{"input_tokens":73000,"cached_input_tokens":72000,"output_tokens":500,"total_tokens":73500}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":117500,"cached_input_tokens":115000,"output_tokens":1200,"reasoning_output_tokens":50,"total_tokens":118700},"last_token_usage":{"input_tokens":1500,"cached_input_tokens":1000,"output_tokens":200,"reasoning_output_tokens":50,"total_tokens":1700}}}}"#,
            "\n"
        );
        let mut reopened = file.reopen().unwrap();
        reopened.seek(SeekFrom::End(0)).unwrap();
        reopened.write_all(appended.as_bytes()).unwrap();
        reopened.flush().unwrap();

        let incremental =
            parse_codex_file_incremental(file.path(), prefix_size, prefix.state.clone());
        let full = parse_codex_file(file.path());

        assert_eq!(incremental.messages, full);
        assert_eq!(incremental.messages.len(), 1);
        assert_eq!(incremental.messages[0].tokens.input, 500);
        assert_eq!(incremental.messages[0].tokens.cache_read, 1000);
        assert_eq!(incremental.messages[0].tokens.output, 150);
        assert_eq!(incremental.messages[0].tokens.reasoning, 50);
    }

    #[test]
    fn test_session_meta_cwd_sets_workspace_metadata() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","cwd":"/Users/alice/demo-repo"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/Users/alice/demo-repo")
        );
        assert_eq!(messages[0].workspace_label.as_deref(), Some("demo-repo"));
    }

    #[test]
    fn test_inaccessible_cwd_still_parses_token_usage() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","cwd":"/path/that/does/not/exist/demo-repo"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 8);
        assert_eq!(messages[0].tokens.output, 2);
        assert_eq!(messages[0].tokens.cache_read, 2);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("/path/that/does/not/exist/demo-repo")
        );
        assert_eq!(messages[0].workspace_label.as_deref(), Some("demo-repo"));
    }

    #[test]
    fn test_session_meta_empty_cwd_clears_workspace_metadata() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","cwd":"   "}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key, None);
        assert_eq!(messages[0].workspace_label, None);
        assert_eq!(messages[0].tokens.input, 8);
    }

    #[test]
    fn test_session_meta_malformed_cwd_clears_workspace_metadata() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","cwd":"file:///Users/alice/demo-repo"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].workspace_key, None);
        assert_eq!(messages[0].workspace_label, None);
        assert_eq!(messages[0].tokens.input, 8);
    }

    #[test]
    fn test_session_meta_path_like_noncanonical_cwd_normalizes_consistently() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"interactive","cwd":"//server//share///demo-repo/"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3,"reasoning_output_tokens":1}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].workspace_key.as_deref(),
            Some("//server/share/demo-repo")
        );
        assert_eq!(messages[0].workspace_label.as_deref(), Some("demo-repo"));
        assert_eq!(messages[0].tokens.input, 8);
    }

    #[test]
    fn test_cached_tokens_takes_max_of_both_fields() {
        let usage = CodexTokenUsage {
            service_tier: None,
            input_tokens: Some(100),
            output_tokens: Some(30),
            cached_input_tokens: Some(10),
            cache_read_input_tokens: Some(20),
            reasoning_output_tokens: Some(5),
            total_tokens: None,
        };
        let totals = CodexTotals::from_usage(&usage);
        assert_eq!(totals.cached, 20);
    }

    #[test]
    fn test_compaction_total_drop_uses_last_as_increment() {
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150000,"cached_input_tokens":10000,"output_tokens":20000,"reasoning_output_tokens":5000},"last_token_usage":{"input_tokens":150000,"cached_input_tokens":10000,"output_tokens":20000,"reasoning_output_tokens":5000}}}}"#;
        let line3 = r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":200000,"cached_input_tokens":15000,"output_tokens":25000,"reasoning_output_tokens":6000},"last_token_usage":{"input_tokens":50,"cached_input_tokens":5,"output_tokens":10,"reasoning_output_tokens":2}}}}"#;
        let content = format!("{}\n{}\n{}", line1, line2, line3);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].tokens.input, 45);
        assert_eq!(messages[1].tokens.output, 8);
        assert_eq!(messages[1].tokens.cache_read, 5);
        assert_eq!(messages[1].tokens.reasoning, 2);
    }

    #[test]
    fn test_headless_fallback_uses_session_provider_and_agent() {
        // session_meta sets provider to "azure" and the default agent bucket,
        // then a line falls through to headless parsing (no structured entry_type)
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"model_provider":"azure","agent_nickname":"my-bot"}}"#;
        let line2 = r#"{"type":"turn.completed","model":"gpt-4o","usage":{"input_tokens":100,"output_tokens":50}}"#;
        let content = format!("{}\n{}", line1, line2);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "azure");
        assert_eq!(messages[0].billing_identity.id(), Some("azure"));
        assert!(messages[0]
            .dedup_key
            .as_deref()
            .unwrap()
            .starts_with("codex:usage-fallback:"));
        assert_eq!(messages[0].agent.as_deref(), Some(CODEX_DEFAULT_AGENT));
    }

    #[test]
    fn test_headless_fallback_defaults_to_openai_without_session_meta() {
        // No session_meta — headless fallback should default to "openai"
        let content = r#"{"type":"turn.completed","model":"gpt-4o-mini","usage":{"input_tokens":120,"cached_input_tokens":20,"output_tokens":30}}"#;
        let file = create_test_file(content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openai");
        assert!(messages[0].agent.is_none());
    }

    #[test]
    fn test_extract_model_skips_empty_slug_falls_through_to_model() {
        // model_info.slug is empty string, but payload.model has a valid value.
        // extract_model should skip the empty slug and return payload.model.
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"turn_context","payload":{"model_info":{"slug":""},"model":"gpt-4o"}}"#;
        let line2 = r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":5}}}}"#;
        let content = format!("{}\n{}", line1, line2);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o");
    }

    #[test]
    fn test_pending_model_messages_do_not_bind_across_unrelated_turns() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"interactive","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:02Z","type":"assistant_message"}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:04Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":5},"last_token_usage":{"input_tokens":5,"cached_input_tokens":1,"output_tokens":2}}}}"#,
            "\n"
        ));

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(parsed.parse_succeeded);
        assert!(parsed.unresolved_model_events);
        assert_eq!(parsed.messages.len(), 2);
        assert_eq!(parsed.messages[0].model_id, "unknown");
        assert_eq!(parsed.messages[1].model_id, "gpt-5.5");
    }

    #[test]
    fn test_token_count_ignores_empty_info_model_until_later_valid_model() {
        let file = create_test_file(concat!(
            r#"{"type":"session_meta","payload":{"source":"interactive","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"model":"","model_name":"","total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            "\n",
            r#"{"timestamp":"2026-04-27T10:00:04Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
            "\n"
        ));

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(parsed.parse_succeeded);
        assert!(!parsed.unresolved_model_events);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].model_id, "gpt-5.5");
    }

    #[test]
    fn test_user_message_marks_next_token_count_as_turn_start() {
        let content = [
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"continue please"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
            r#"{"timestamp":"2026-01-01T00:00:04Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"cached_input_tokens":4,"output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
        ]
        .join("\n");
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert!(
            messages[0].is_turn_start,
            "first reply after a human user_message is a turn start"
        );
        assert!(
            !messages[1].is_turn_start,
            "a later reply with no new user_message is not a turn start"
        );
    }

    #[test]
    fn test_xml_user_message_does_not_mark_turn_start() {
        let content = [
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"\n<environment_context>\n  <cwd>/tmp</cwd>\n</environment_context>"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
        ]
        .join("\n");
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert!(
            !messages[0].is_turn_start,
            "a system-injected <...> message is not a human turn"
        );
    }

    #[test]
    fn test_exec_user_message_still_marks_turn_start() {
        // A `codex exec` one-shot is headless but still carries a real human
        // prompt, so it counts as exactly one turn (verified against a real
        // `codex exec` session: 1 user_message -> turn_count 1).
        let content = [
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"source":"exec"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"hello"}}"#,
            // A real `codex exec` interleaves an agent_message between the user
            // prompt and the token_count; the deferred turn flag must survive it.
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"agent_message","message":"hi"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#,
        ]
        .join("\n");
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].is_turn_start,
            "an exec one-shot with a human prompt counts as one turn"
        );
        assert_eq!(messages[0].agent.as_deref(), Some(CODEX_HEADLESS_AGENT));
    }

    #[test]
    fn test_incremental_parse_preserves_pending_turn_start() {
        let content = [
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#,
            r#"{"timestamp":"2026-01-01T00:00:02Z","type":"event_msg","payload":{"type":"user_message","message":"hello"}}"#,
            "",
        ]
        .join("\n");
        let file = create_test_file(&content);
        let initial_size = file.as_file().metadata().unwrap().len();

        let initial = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(
            initial.messages.is_empty(),
            "no token_count yet, so no message"
        );
        assert!(
            initial.state.pending_turn_start,
            "a pending turn survives a chunk that ends before the token_count"
        );

        let appended = format!(
            "{}\n",
            r#"{"timestamp":"2026-01-01T00:00:03Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#
        );
        let mut reopened = file.reopen().unwrap();
        reopened.seek(SeekFrom::End(0)).unwrap();
        reopened.write_all(appended.as_bytes()).unwrap();
        reopened.flush().unwrap();

        let incremental =
            parse_codex_file_incremental(file.path(), initial_size, initial.state.clone());

        assert_eq!(incremental.messages.len(), 1);
        assert!(
            incremental.messages[0].is_turn_start,
            "the deferred turn applies to the message parsed in the next chunk"
        );
        assert!(
            !incremental.state.pending_turn_start,
            "the pending flag is consumed once applied"
        );
    }

    #[test]
    fn test_token_count_timestamp_is_start_anchored() {
        let line1 = r#"{"timestamp":"1970-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"1970-01-01T00:00:01.005Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#;
        let content = format!("{}\n{}", line1, line2);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].timestamp, 1_000,
            "timestamp must be the turn_context start (1000ms epoch)"
        );
        assert_eq!(
            messages[0].duration_ms,
            Some(5),
            "duration_ms must span from turn start to token_count event (5ms)"
        );
    }

    #[test]
    fn test_user_message_without_turn_context_anchors_at_user_message() {
        // Regression: a resumed/compacted session can emit a human
        // `user_message` followed directly by a `token_count` with no
        // intervening `turn_context` (which normally resets the start-anchor
        // cursor every turn). Before this fix, the token_count would anchor
        // at the previous turn's last accepted token timestamp instead of
        // this user message, bridging backward across the idle gap between
        // turns.
        let line1 = r#"{"timestamp":"1970-01-01T00:00:01Z","type":"turn_context","payload":{"model":"gpt-5.2"}}"#;
        let line2 = r#"{"timestamp":"1970-01-01T00:00:01.100Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#;
        // A long idle gap follows: the session resumes with a human
        // user_message but no fresh turn_context before the next token_count.
        let line3 = r#"{"timestamp":"1970-01-01T01:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"still there?"}}"#;
        let line4 = r#"{"timestamp":"1970-01-01T01:00:00.500Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":20,"cached_input_tokens":4,"output_tokens":6},"last_token_usage":{"input_tokens":10,"cached_input_tokens":2,"output_tokens":3}}}}"#;
        let content = [line1, line2, line3, line4].join("\n");
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[1].timestamp,
            parse_codex_entry_timestamp(Some("1970-01-01T01:00:00Z")).unwrap(),
            "the second token_count must anchor at the user_message, not the \
             previous turn's last accepted token timestamp"
        );
        assert_eq!(
            messages[1].duration_ms,
            Some(500),
            "duration_ms must span from the user_message to its token_count \
             (500ms), not bridge backward across the idle gap"
        );
        assert!(
            messages[1].is_turn_start,
            "the deferred turn-start marker must still apply"
        );
    }

    #[test]
    fn test_task_started_duration_boundaries_from_fixture() {
        let file = create_test_file(CODEX_TASK_STARTED_DURATION_FIXTURE);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 4);
        assert_eq!(
            messages
                .iter()
                .map(|message| message.duration_ms)
                .collect::<Vec<_>>(),
            vec![Some(1_000), Some(1_000), Some(750), Some(500)]
        );
        assert_eq!(messages[1].timestamp, 2_209_075_200_000);
        assert_eq!(
            messages[2].timestamp,
            parse_codex_entry_timestamp(Some("2040-01-03T00:00:00.250Z")).unwrap()
        );
    }

    #[test]
    fn test_thread_id_from_rollout_path_handles_plain_and_compressed_zst() {
        let plain =
            Path::new("rollout-2026-08-30T10-00-00-550e8400-e29b-41d4-a716-446655440000.jsonl");
        let compressed =
            Path::new("rollout-2026-08-30T10-00-00-550e8400-e29b-41d4-a716-446655440000.jsonl.zst");
        assert_eq!(
            thread_id_from_rollout_path(plain),
            Some("550e8400-e29b-41d4-a716-446655440000".to_string())
        );
        assert_eq!(
            thread_id_from_rollout_path(compressed),
            Some("550e8400-e29b-41d4-a716-446655440000".to_string())
        );
    }

    #[test]
    fn fallback_usage_dedup_keys_are_copy_stable_and_event_specific() {
        let dir = TempDir::new().unwrap();
        let content = concat!(
            r#"{"type":"session_meta","payload":{"id":"logical-rollout"}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":3,"output_tokens":5,"reasoning_output_tokens":2}}}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":3,"output_tokens":5,"reasoning_output_tokens":2}}}}"#,
            "\n",
            r#"{"type":"metadata","model":"gpt-5"}"#,
            "\n"
        );
        let plain_path = dir.path().join("fallback.jsonl");
        let zst_path = dir.path().join("fallback.jsonl.zst");
        std::fs::write(&plain_path, content).unwrap();
        std::fs::write(&zst_path, zstd::encode_all(content.as_bytes(), 0).unwrap()).unwrap();

        let plain = parse_codex_file(&plain_path);
        let compressed = parse_codex_file(&zst_path);
        let plain_keys = plain
            .iter()
            .map(|message| message.dedup_key.as_deref().unwrap())
            .collect::<Vec<_>>();
        let compressed_keys = compressed
            .iter()
            .map(|message| message.dedup_key.as_deref().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(plain.len(), 2);
        assert_eq!(compressed_keys, plain_keys);
        assert_eq!(
            plain_keys,
            vec![
                "codex:usage-fallback:15:logical-rollout:0:5:gpt-5:7:3:3:0:0:2",
                "codex:usage-fallback:15:logical-rollout:1:5:gpt-5:7:3:3:0:0:2",
            ]
        );
    }

    #[test]
    fn fallback_usage_dedup_key_covers_every_emitted_token_bucket() {
        let identity = CodexFallbackDedupIdentity {
            rollout_id: "all-buckets".to_string(),
            usage_event_ordinal: 0,
        };
        let key_for = |tokens: TokenBreakdown| {
            let mut message =
                UnifiedMessage::new("codex", "gpt-5", "openai", "all-buckets", 0, tokens, 0.0);
            set_codex_fallback_dedup_key(&mut message, "gpt-5", &identity);
            message.dedup_key.unwrap()
        };
        let mut keys = std::collections::HashSet::new();
        keys.insert(key_for(TokenBreakdown::default()));
        for tokens in [
            TokenBreakdown {
                input: 1,
                ..Default::default()
            },
            TokenBreakdown {
                output: 1,
                ..Default::default()
            },
            TokenBreakdown {
                cache_read: 1,
                ..Default::default()
            },
            TokenBreakdown {
                cache_write: 1,
                ..Default::default()
            },
            TokenBreakdown {
                cache_write_1h: 1,
                ..Default::default()
            },
            TokenBreakdown {
                reasoning: 1,
                ..Default::default()
            },
        ] {
            keys.insert(key_for(tokens));
        }
        assert_eq!(
            keys.len(),
            7,
            "every emitted token bucket must affect identity"
        );
    }

    #[test]
    fn fallback_usage_ordinal_survives_incremental_resume() {
        let first_event = concat!(
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":2}}}}"#,
            "\n"
        );
        let second_event = concat!(
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":11,"output_tokens":3}}}}"#,
            "\n"
        );
        let mut file = create_test_file(&format!(
            "{}\n{}\n{first_event}",
            r#"{"type":"session_meta","payload":{"id":"incremental-rollout"}}"#,
            r#"{"type":"turn_context","payload":{"model":"gpt-5"}}"#,
        ));
        let initial = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert_eq!(initial.messages.len(), 1);
        assert_eq!(initial.state.usage_event_ordinal, 1);
        assert_eq!(
            initial.messages[0].dedup_key.as_deref(),
            Some("codex:usage-fallback:19:incremental-rollout:0:5:gpt-5:10:2:0:0:0:0")
        );
        let initial_key = initial.messages[0].dedup_key.clone().unwrap();

        file.as_file_mut()
            .write_all(second_event.as_bytes())
            .unwrap();
        file.as_file_mut().flush().unwrap();
        let appended =
            parse_codex_file_incremental(file.path(), initial.consumed_offset, initial.state);

        assert_eq!(appended.messages.len(), 1);
        assert_eq!(appended.state.usage_event_ordinal, 2);
        assert_eq!(
            appended.messages[0].dedup_key.as_deref(),
            Some("codex:usage-fallback:19:incremental-rollout:1:5:gpt-5:11:3:0:0:0:0")
        );
        let resumed_keys = [initial_key, appended.messages[0].dedup_key.clone().unwrap()];
        let full_keys = parse_codex_file(file.path())
            .into_iter()
            .map(|message| message.dedup_key.unwrap())
            .collect::<Vec<_>>();
        assert_eq!(full_keys, resumed_keys);
    }

    #[test]
    fn headless_fallback_usage_identity_is_copy_stable_and_content_sensitive() {
        let dir = TempDir::new().unwrap();
        let content = concat!(
            r#"{"type":"session_meta","payload":{"id":"headless-rollout"}}"#,
            "\n",
            r#"{"type":"turn.completed","model":"gpt-4o","usage":{"input_tokens":12,"output_tokens":3}}"#,
            "\n"
        );
        let diverged = concat!(
            r#"{"type":"session_meta","payload":{"id":"headless-rollout"}}"#,
            "\n",
            r#"{"type":"turn.completed","model":"gpt-4o","usage":{"input_tokens":13,"output_tokens":4}}"#,
            "\n"
        );
        let plain_path = dir.path().join("headless.jsonl");
        let zst_path = dir.path().join("headless.jsonl.zst");
        std::fs::write(&plain_path, content).unwrap();
        std::fs::write(&zst_path, zstd::encode_all(content.as_bytes(), 0).unwrap()).unwrap();

        let plain = parse_codex_file(&plain_path);
        let compressed = parse_codex_file(&zst_path);

        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].dedup_key, compressed[0].dedup_key);
        assert_eq!(
            plain[0].dedup_key.as_deref(),
            Some("codex:usage-fallback:16:headless-rollout:0:6:gpt-4o:12:3:0:0:0:0")
        );

        std::fs::write(&plain_path, diverged).unwrap();
        let rewritten = parse_codex_file(&plain_path);
        assert_eq!(rewritten.len(), 1);
        assert_eq!(
            rewritten[0].dedup_key.as_deref(),
            Some("codex:usage-fallback:16:headless-rollout:0:6:gpt-4o:13:4:0:0:0:0")
        );
        assert_ne!(rewritten[0].dedup_key, compressed[0].dedup_key);
    }

    #[test]
    fn compressed_archives_preserve_codex_usage_and_session_identity() {
        let dir = TempDir::new().unwrap();
        let content = concat!(
            r#"{"timestamp":"2026-10-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5"}}"#,
            "\n",
            r#"{"timestamp":"2026-10-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":140,"cached_input_tokens":30,"output_tokens":20,"reasoning_output_tokens":10,"total_tokens":160},"last_token_usage":{"input_tokens":140,"cached_input_tokens":30,"output_tokens":20,"reasoning_output_tokens":10,"total_tokens":160}}}}"#,
            "\n"
        );
        let plain_path = dir.path().join("fixture.jsonl");
        std::fs::write(&plain_path, content).unwrap();
        let expected = parse_codex_file(&plain_path);
        assert_eq!(expected.len(), 1);
        assert_eq!(expected[0].model_id, "gpt-5");
        assert_eq!(expected[0].session_id, "fixture");
        assert_eq!(expected[0].tokens.input, 110);
        assert_eq!(expected[0].tokens.cache_read, 30);
        assert_eq!(expected[0].tokens.output, 10);
        assert_eq!(expected[0].tokens.reasoning, 10);
        assert_eq!(expected[0].tokens.total(), 160);
        assert_eq!(
            expected[0].dedup_key.as_deref(),
            Some("codex:token_count-total:fixture:openai:gpt-5:140:20:30:10"),
            "the normal timestamped fixture's existing key must not change"
        );

        let zst_path = dir.path().join("fixture.jsonl.zst");
        std::fs::write(&zst_path, zstd::encode_all(content.as_bytes(), 0).unwrap()).unwrap();
        let compressed_parsed = parse_codex_file(&zst_path);
        assert_eq!(compressed_parsed, expected);

        let incremental = parse_codex_file_incremental(&zst_path, 0, CodexParseState::default());
        assert!(incremental.parse_succeeded);
        assert!(incremental.archive_failure.is_none());
        assert_eq!(incremental.messages, expected);
    }

    const COMPRESSED_TURN_CONTEXT: &str =
        r#"{"timestamp":"2026-10-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5"}}"#;
    const COMPRESSED_USAGE: &str = concat!(
        r#"{"timestamp":"2026-10-01T00:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":140,"#,
        r#""cached_input_tokens":30,"output_tokens":20,"reasoning_output_tokens":10,"total_tokens":160},"last_token_usage":{"input_tokens":140,"#,
        r#""cached_input_tokens":30,"output_tokens":20,"reasoning_output_tokens":10,"total_tokens":160}}}}"#,
    );

    #[test]
    fn compressed_archive_accepts_aggregate_input_above_simulated_legacy_limit() {
        const LEGACY_ARCHIVE_LIMIT: u64 = 256 * 1024 * 1024;
        const SIMULATED_LEGACY_LIMIT: u64 = 512;
        const LIMIT_SCALE: u64 = LEGACY_ARCHIVE_LIMIT / SIMULATED_LEGACY_LIMIT;
        let simulated_current_limit = MAX_CODEX_ARCHIVE_BYTES / LIMIT_SCALE;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("above-legacy-limit.jsonl.zst");
        let padding_record = format!(
            r#"{{"timestamp":"2026-10-01T00:00:01Z","type":"response_item","payload":{{"type":"function_call_output","output":"{}"}}}}"#,
            "x".repeat(128)
        );
        let content = format!(
            "{COMPRESSED_TURN_CONTEXT}\n{padding_record}\n{padding_record}\n{COMPRESSED_USAGE}\n"
        );
        assert!(content.len() as u64 > SIMULATED_LEGACY_LIMIT);
        assert!(content.len() as u64 <= simulated_current_limit);
        std::fs::write(&path, zstd::encode_all(content.as_bytes(), 0).unwrap()).unwrap();

        {
            let _limit = register_archive_limit_override(&path, SIMULATED_LEGACY_LIMIT);
            let legacy = parse_codex_file_incremental(&path, 0, CodexParseState::default());
            assert!(!legacy.parse_succeeded);
            assert_eq!(
                legacy.archive_failure.unwrap().cause,
                CodexArchiveFailureCause::DecodedSizeLimit {
                    limit_bytes: SIMULATED_LEGACY_LIMIT,
                }
            );
        }

        let _limit = register_archive_limit_override(&path, simulated_current_limit);
        let parsed = parse_codex_file_incremental(&path, 0, CodexParseState::default());

        assert!(parsed.parse_succeeded);
        assert!(parsed.archive_failure.is_none());
        assert!(parsed.consumed_offset > SIMULATED_LEGACY_LIMIT);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].tokens.total(), 160);
    }

    #[test]
    fn codex_archive_limit_override_usability_distinguishes_unset_from_invalid() {
        let usable = |raw: &str| codex_archive_limit_override_is_usable(Some(OsStr::new(raw)));
        assert_eq!(codex_archive_limit_override_is_usable(None), None);
        assert_eq!(usable("65536"), Some(true));
        assert_eq!(usable(" 2147483648 "), Some(true));
        // Each of these silently reverts to the default, so each has to be
        // reported rather than treated as an accepted ceiling.
        assert_eq!(usable("0"), Some(false));
        assert_eq!(usable("-1"), Some(false));
        assert_eq!(usable("512MiB"), Some(false));
        assert_eq!(usable(""), Some(false));
        assert_eq!(usable(&u64::MAX.to_string()), Some(false));

        // A set but undecodable value is unusable, not unset: `env::var` would
        // report it as absent and take the silent default.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            let undecodable = OsStr::from_bytes(b"512\xff");
            assert_eq!(
                codex_archive_limit_override_is_usable(Some(undecodable)),
                Some(false)
            );
            assert_eq!(
                parse_max_codex_archive_bytes(Some(undecodable)),
                MAX_CODEX_ARCHIVE_BYTES
            );
        }
    }

    #[test]
    fn parse_max_codex_archive_bytes_validates_bounds_and_falls_back() {
        let parse = |raw: &str| parse_max_codex_archive_bytes(Some(OsStr::new(raw)));
        assert_eq!(parse_max_codex_archive_bytes(None), MAX_CODEX_ARCHIVE_BYTES);
        assert_eq!(parse("65536"), 65_536);
        assert_eq!(parse(" 2147483648 "), 2_147_483_648);
        assert_eq!(parse(&(u64::MAX - 1).to_string()), u64::MAX - 1);
        assert_eq!(parse("0"), MAX_CODEX_ARCHIVE_BYTES);
        assert_eq!(parse("-1"), MAX_CODEX_ARCHIVE_BYTES);
        assert_eq!(parse("invalid"), MAX_CODEX_ARCHIVE_BYTES);
        assert_eq!(parse(&u64::MAX.to_string()), MAX_CODEX_ARCHIVE_BYTES);
    }

    #[test]
    fn compressed_archive_skips_oversized_record_and_retains_later_usage() {
        use std::io::Write as _;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("oversized-record.jsonl.zst");
        let oversized_record = format!(
            r#"{{"timestamp":"2026-10-01T00:00:01Z","type":"response_item","payload":{{"type":"function_call_output","output":"{}"}}}}"#,
            "x".repeat(1024)
        );
        let mut encoder =
            zstd::stream::write::Encoder::new(std::fs::File::create(&path).unwrap(), 0).unwrap();
        writeln!(encoder, "{COMPRESSED_TURN_CONTEXT}").unwrap();
        writeln!(encoder, "{oversized_record}").unwrap();
        writeln!(encoder, "{COMPRESSED_USAGE}").unwrap();
        encoder.finish().unwrap();

        let _limit = register_record_limit_override(&path, 512);
        let parsed = parse_codex_file_incremental(&path, 0, CodexParseState::default());

        assert!(parsed.parse_succeeded);
        assert!(parsed.archive_failure.is_none());
        assert_eq!(parsed.skipped_oversized_records, 1);
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].tokens.total(), 160);
    }

    #[test]
    fn compressed_archive_open_failure_retains_io_cause() {
        let dir = TempDir::new().unwrap();
        let missing_path = dir.path().join("missing.jsonl.zst");
        let parsed = parse_codex_file_incremental(&missing_path, 0, CodexParseState::default());

        assert!(!parsed.parse_succeeded);
        assert!(parsed.messages.is_empty());
        let failure = parsed.archive_failure.expect("missing archive failure");
        assert_eq!(failure.path, missing_path);
        match failure.cause {
            CodexArchiveFailureCause::Open { kind, message } => {
                assert_eq!(kind, ErrorKind::NotFound);
                assert!(!message.is_empty(), "the OS error text must be retained");
            }
            cause => panic!("expected an open failure, got {cause:?}"),
        }
    }

    #[test]
    fn unreadable_compressed_codex_archives_report_typed_failures_without_partial_usage() {
        let dir = TempDir::new().unwrap();
        let content = br#"{"timestamp":"2026-10-01T00:00:00Z","type":"turn_context","payload":{"model":"gpt-5"}}
{"timestamp":"2026-10-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":50,"total_tokens":150},"last_token_usage":{"input_tokens":100,"output_tokens":50,"total_tokens":150}}}}"#;

        let assert_decode_failure = |failure: Option<CodexArchiveFailure>, path: &Path| {
            let failure = failure.expect("decode failure");
            assert_eq!(failure.path, path);
            match failure.cause {
                CodexArchiveFailureCause::Decode { message, .. } => {
                    assert!(!message.is_empty(), "the zstd error text must be retained");
                }
                cause => panic!("expected a decode failure, got {cause:?}"),
            }
        };

        let truncated_path = dir.path().join("truncated.jsonl.zst");
        let mut bytes = zstd::encode_all(&content[..], 0).unwrap();
        bytes.pop();
        std::fs::write(&truncated_path, bytes).unwrap();
        assert!(parse_codex_file(&truncated_path).is_empty());
        let truncated =
            parse_codex_file_incremental(&truncated_path, 0, CodexParseState::default());
        assert!(!truncated.parse_succeeded);
        assert!(truncated.messages.is_empty());
        assert_decode_failure(truncated.archive_failure, &truncated_path);

        let corrupt_path = dir.path().join("corrupt.jsonl.zst");
        std::fs::write(&corrupt_path, b"not a zstd stream").unwrap();
        assert!(parse_codex_file(&corrupt_path).is_empty());
        let corrupt = parse_codex_file_incremental(&corrupt_path, 0, CodexParseState::default());
        assert!(!corrupt.parse_succeeded);
        assert!(corrupt.messages.is_empty());
        assert_decode_failure(corrupt.archive_failure, &corrupt_path);

        let capped_path = dir.path().join("capped.jsonl.zst");
        std::fs::write(&capped_path, zstd::encode_all(&content[..], 0).unwrap()).unwrap();
        let _limit = register_archive_limit_override(&capped_path, 32);
        let capped = parse_codex_file_incremental(&capped_path, 0, CodexParseState::default());
        assert!(!capped.parse_succeeded);
        assert!(capped.messages.is_empty());
        let capped_failure = capped.archive_failure.expect("decoded-size failure");
        assert_eq!(capped_failure.path, capped_path);
        assert_eq!(
            capped_failure.cause,
            CodexArchiveFailureCause::DecodedSizeLimit { limit_bytes: 32 }
        );

        let malformed_path = dir.path().join("malformed.jsonl.zst");
        std::fs::write(
            &malformed_path,
            zstd::encode_all(&b"not json\n"[..], 0).unwrap(),
        )
        .unwrap();
        let malformed =
            parse_codex_file_incremental(&malformed_path, 0, CodexParseState::default());
        assert!(!malformed.parse_succeeded);
        assert!(malformed.messages.is_empty());
        assert!(
            malformed.archive_failure.is_none(),
            "decoded malformed JSON is not an archive read failure"
        );
    }

    // Rollout lines shaped like Codex 0.153+ app-server rollouts: one
    // `token_usage_record` per model response, followed by the token_count
    // that reports the same response. A context compaction request writes its
    // record, then `compacted`, then a token_count that does not advance.
    const COMPACTION_THREAD: &str = "01a0f000-0000-7000-8000-000000000001";

    fn usage_json(input: i64, cached: i64, output: i64) -> String {
        format!(
            r#"{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output},"reasoning_output_tokens":0,"total_tokens":{}}}"#,
            input + output
        )
    }

    fn record_line(ts: &str, turn: &str, response: &str, usage: (i64, i64, i64)) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"token_usage_record","payload":{{"thread_id":"{COMPACTION_THREAD}","turn_id":"{turn}","response_id":"{response}","usage":{}}}}}"#,
            usage_json(usage.0, usage.1, usage.2)
        ) + "\n"
    }

    fn token_count_line(ts: &str, total: (i64, i64, i64), last: (i64, i64, i64)) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{},"last_token_usage":{}}}}}}}"#,
            usage_json(total.0, total.1, total.2),
            usage_json(last.0, last.1, last.2)
        ) + "\n"
    }

    fn turn_lines(ts: &str, turn: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"task_started","turn_id":"{turn}"}}}}"#
        ) + "\n"
            + &format!(
                r#"{{"timestamp":"{ts}","type":"turn_context","payload":{{"turn_id":"{turn}","model":"gpt-5.5"}}}}"#
            )
            + "\n"
    }

    fn session_meta_line(originator: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-10-01T00:00:00Z","type":"session_meta","payload":{{"id":"{COMPACTION_THREAD}","originator":"{originator}","cli_version":"0.153.0","source":"vscode","model_provider":"openai","cwd":"/workspace/project"}}}}"#
        ) + "\n"
    }

    /// Turn 1 answers once, turn 2 compacts the context, turn 3 answers once.
    fn compaction_rollout(originator: &str) -> String {
        let compacted = r#"{"timestamp":"2026-10-01T00:02:00.010Z","type":"compacted","payload":{"message":"","replacement_history":[]}}"#.to_string() + "\n";
        session_meta_line(originator)
            + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
            + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
            + &token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10))
            + &turn_lines("2026-10-01T00:02:00Z", "turn-2")
            + &record_line("2026-10-01T00:02:00.005Z", "turn-2", "resp-2", (500, 200, 40))
            + &compacted
            // Totals stay at turn 1's; the compaction never reaches them.
            + &token_count_line("2026-10-01T00:02:00.020Z", (100, 80, 10), (0, 0, 0))
            + &turn_lines("2026-10-01T00:03:00Z", "turn-3")
            + &record_line("2026-10-01T00:03:05Z", "turn-3", "resp-3", (50, 0, 5))
            + &token_count_line("2026-10-01T00:03:05.005Z", (150, 80, 15), (50, 0, 5))
    }

    fn total_tokens(messages: &[UnifiedMessage]) -> i64 {
        messages.iter().map(|m| m.tokens.total()).sum()
    }

    #[test]
    fn native_usage_paths_preserve_explicit_billing_without_inference() {
        for originator in ["codex_vscode", "openclaw"] {
            for provider in [Some("azure"), None] {
                let original = compaction_rollout(originator);
                let content = match provider {
                    Some(provider) => original.replace(
                        r#""model_provider":"openai""#,
                        &format!(r#""model_provider":"{provider}""#),
                    ),
                    None => original.replace(r#","model_provider":"openai""#, ""),
                };
                let file = create_test_file(&content);
                let messages = parse_codex_file(file.path());
                assert_eq!(messages.len(), 3, "regular, compaction, regular");
                assert_eq!(total_tokens(&messages), 705);
                for message in &messages {
                    assert_eq!(message.billing_identity.id(), provider);
                    assert_eq!(message.provider_id, provider.unwrap_or("openai"));
                }
            }
        }
    }

    #[test]
    fn terminal_usage_records_preserve_explicit_billing_without_inference() {
        for terminal in ["task_complete", "turn_aborted"] {
            for provider in [Some("gateway"), None] {
                let meta = match provider {
                    Some(provider) => session_meta_line("codex_vscode").replace(
                        r#""model_provider":"openai""#,
                        &format!(r#""model_provider":"{provider}""#),
                    ),
                    None => session_meta_line("codex_vscode")
                        .replace(r#","model_provider":"openai""#, ""),
                };
                let content = meta
                    + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                    + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                    + &format!(
                        r#"{{"timestamp":"2026-10-01T00:01:06Z","type":"event_msg","payload":{{"type":"{terminal}","turn_id":"turn-1"}}}}"#
                    )
                    + "\n";
                let file = create_test_file(&content);
                let messages = parse_codex_file(file.path());
                assert_eq!(messages.len(), 1, "{terminal}");
                assert_eq!(total_tokens(&messages), 110);
                assert_eq!(messages[0].billing_identity.id(), provider);
            }
        }
    }

    #[test]
    fn test_compaction_usage_record_is_counted() {
        let file = create_test_file(&compaction_rollout("codex_vscode"));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3, "turn 1, the compaction, turn 3");
        let compaction = &messages[1];
        assert_eq!(compaction.client, "codex");
        assert_eq!(compaction.model_id, "gpt-5.5");
        assert_eq!(compaction.tokens.input, 300);
        assert_eq!(compaction.tokens.cache_read, 200);
        assert_eq!(compaction.tokens.output, 40);
        assert_eq!(
            compaction.timestamp,
            parse_codex_entry_timestamp(Some("2026-10-01T00:02:00.005Z")).unwrap()
        );
        assert_eq!(
            compaction.dedup_key.as_deref(),
            Some(format!("codex:usage-record:{COMPACTION_THREAD}:resp-2").as_str())
        );
        // Every response counted once: 110 + 540 + 55.
        assert_eq!(total_tokens(&messages), 705);
    }

    #[test]
    fn test_usage_records_confirmed_by_token_count_are_not_counted_again() {
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:01Z", "turn-1", "resp-1", (100, 0, 10))
                + &token_count_line("2026-10-01T00:01:01.005Z", (100, 0, 10), (100, 0, 10))
                + &record_line("2026-10-01T00:01:02Z", "turn-1", "resp-2", (120, 100, 20))
                + &token_count_line("2026-10-01T00:01:02.005Z", (220, 100, 30), (120, 100, 20))
                + &turn_lines("2026-10-01T00:02:00Z", "turn-2")
                + &record_line("2026-10-01T00:02:01Z", "turn-2", "resp-3", (130, 120, 5))
                + &token_count_line("2026-10-01T00:02:01.005Z", (350, 220, 35), (130, 120, 5))),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3);
        assert!(messages.iter().all(|m| m
            .dedup_key
            .as_deref()
            .unwrap()
            .starts_with("codex:token_count")));
        assert_eq!(total_tokens(&messages), 385);
    }

    #[test]
    fn test_rollout_without_usage_records_is_unchanged() {
        // Older Codex wrote no `token_usage_record`, and some reset the
        // totals to zero after compacting. Nothing about that stream changes.
        let compacted = r#"{"timestamp":"2026-10-01T00:02:00.010Z","type":"compacted","payload":{"message":"","replacement_history":[]}}"#.to_string() + "\n";
        let file = create_test_file(
            &(session_meta_line("codex_cli_rs")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &token_count_line("2026-10-01T00:01:05Z", (100, 80, 10), (100, 80, 10))
                + &turn_lines("2026-10-01T00:02:00Z", "turn-2")
                + &compacted
                + &token_count_line("2026-10-01T00:02:00.020Z", (0, 0, 0), (0, 0, 0))
                + &turn_lines("2026-10-01T00:03:00Z", "turn-3")
                + &token_count_line("2026-10-01T00:03:05Z", (50, 0, 5), (50, 0, 5))),
        );

        let messages = parse_codex_file(file.path());

        let summary: Vec<(i64, i64, i64)> = messages
            .iter()
            .map(|m| (m.tokens.input, m.tokens.cache_read, m.tokens.output))
            .collect();
        assert_eq!(summary, vec![(20, 80, 10), (50, 0, 5)]);
        assert!(messages.iter().all(|m| m
            .dedup_key
            .as_deref()
            .unwrap()
            .starts_with("codex:token_count")));
    }

    #[test]
    fn test_usage_record_left_without_token_count_by_its_turn_is_counted() {
        // The response finished but its turn ended before the token_count
        // was written; the next turn's start settles it.
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + &token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10))
                + &record_line("2026-10-01T00:01:09Z", "turn-1", "resp-2", (110, 100, 4))
                + &turn_lines("2026-10-01T00:05:00Z", "turn-2")
                + &record_line("2026-10-01T00:05:05Z", "turn-2", "resp-3", (120, 110, 6))
                + &token_count_line("2026-10-01T00:05:05.005Z", (220, 190, 16), (120, 110, 6))),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[1].dedup_key.as_deref(),
            Some(format!("codex:usage-record:{COMPACTION_THREAD}:resp-2").as_str())
        );
        assert_eq!(total_tokens(&messages), 110 + 114 + 126);
    }

    #[test]
    fn test_usage_record_pending_at_end_of_chunk_waits_for_its_token_count() {
        let mut file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))),
        );

        let first = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        assert!(first.messages.is_empty(), "the token_count may still come");
        assert!(first.state.pending_usage_record.is_some());

        // The cached state round-trips through serde between scans.
        let state: CodexParseState =
            serde_json::from_str(&serde_json::to_string(&first.state).unwrap()).unwrap();
        file.write_all(
            token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10)).as_bytes(),
        )
        .unwrap();
        file.flush().unwrap();
        let second = parse_codex_file_incremental(file.path(), first.consumed_offset, state);

        assert_eq!(second.messages.len(), 1);
        assert_eq!(total_tokens(&second.messages), 110);
        assert!(second.state.pending_usage_record.is_none());
    }

    #[test]
    fn test_openclaw_rollout_compaction_is_openclaw_usage_of_its_turn() {
        let file = create_test_file(&compaction_rollout("openclaw"));

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert_eq!(parsed.messages.len(), 3);
        assert!(parsed
            .messages
            .iter()
            .all(|m| m.client == OPENCLAW_CLIENT_ID && m.session_id == COMPACTION_THREAD));
        assert_eq!(total_tokens(&parsed.messages), 705);
        // The compaction turn is recorded, so OpenClaw's mirror row for it
        // yields to the rollout like every other turn.
        assert!(parsed.state.turn_coverage.turn_ids.contains("turn-2"));
    }

    #[test]
    fn test_forked_child_replayed_parent_compaction_is_not_counted() {
        let child = "01a0f000-0000-7000-8000-0000000000c1";
        let compacted = r#"{"timestamp":"2026-10-01T00:02:00.010Z","type":"compacted","payload":{"message":"","replacement_history":[]}}"#.to_string() + "\n";
        let file = create_test_file(
            &(format!(
                r#"{{"timestamp":"2026-10-01T01:00:00Z","type":"session_meta","payload":{{"id":"{child}","forked_from_id":"{COMPACTION_THREAD}","originator":"codex_vscode","source":"vscode","model_provider":"openai"}}}}"#
            ) + "\n"
                // Replayed parent history, compaction included.
                + &record_line("2026-10-01T00:02:00.005Z", "turn-2", "resp-2", (500, 200, 40))
                + &compacted
                + &token_count_line("2026-10-01T00:02:00.020Z", (100, 80, 10), (0, 0, 0))
                // The child's own turn.
                + r#"{"timestamp":"2026-10-01T01:00:01Z","type":"turn_context","payload":{"turn_id":"01a0f000-0001-7000-8000-000000000001","model":"gpt-5.5"}}"#
                + "\n"
                + &record_line("2026-10-01T01:00:05Z", "child-turn", "resp-9", (40, 0, 4))
                + &token_count_line("2026-10-01T01:00:05.005Z", (140, 80, 14), (40, 0, 4))),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1, "only the child's own response");
        assert_eq!(total_tokens(&messages), 44);
    }

    #[test]
    fn test_forked_child_parent_record_replayed_after_gate_is_not_counted() {
        // A forked child can keep replaying parent history after its first
        // turn_context, and the replay can carry the parent's records too.
        // The replayed snapshot is skipped, and its record goes with it.
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","forked_from_id":"parent-session","source":"vscode","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.9475Z","type":"token_usage_record","payload":{"turn_id":"p","response_id":"parent-resp","usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55},"last_token_usage":{"input_tokens":50,"output_tokens":5,"total_tokens":55}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.2525Z","type":"token_usage_record","payload":{"turn_id":"c","response_id":"child-resp","usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":310,"output_tokens":32,"total_tokens":342},"last_token_usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}}}"#,
            "\n"
        ));

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1, "only the child's own response");
        assert_eq!(total_tokens(&messages), 12);
        assert!(messages[0]
            .dedup_key
            .as_deref()
            .unwrap()
            .starts_with("codex:token_count"));
    }

    #[test]
    fn test_unconfirmed_record_row_keeps_reasoning_tier_and_falls_back_without_response_id() {
        let compacted = r#"{"timestamp":"2026-10-01T00:02:00.010Z","type":"compacted","payload":{"message":"","replacement_history":[]}}"#.to_string() + "\n";
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:02:00Z", "turn-2")
                + r#"{"timestamp":"2026-10-01T00:02:00.005Z","type":"token_usage_record","payload":{"turn_id":"turn-2","usage":{"input_tokens":500,"cached_input_tokens":200,"output_tokens":40,"reasoning_output_tokens":10,"total_tokens":540,"service_tier":"priority"}}}"#
                + "\n"
                + &compacted
                + &estimate_token_count_line("2026-10-01T00:02:00.020Z", (0, 0, 0), 15823)),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        let row = &messages[0];
        assert_eq!(row.tokens.input, 300);
        assert_eq!(row.tokens.cache_read, 200);
        assert_eq!(row.tokens.output, 30, "reasoning is a subset of output");
        assert_eq!(row.tokens.reasoning, 10);
        assert_eq!(row.service_tier.as_deref(), Some("priority"));
        let timestamp = parse_codex_entry_timestamp(Some("2026-10-01T00:02:00.005Z")).unwrap();
        assert_eq!(
            row.dedup_key.as_deref(),
            Some(
                format!("codex:usage-record:{COMPACTION_THREAD}:{timestamp}:500:40:200:10")
                    .as_str()
            )
        );
    }

    #[test]
    fn test_compaction_in_the_shape_codex_writes_it() {
        // Seen in real rollouts: after `compacted` the same turn's
        // turn_context is re-emitted, and the token_count that follows keeps
        // the totals but repeats a nonzero `last_token_usage`. The next turn
        // also starts with a duplicate snapshot before its own response.
        let compacted = r#"{"timestamp":"2026-10-01T00:02:00.010Z","type":"compacted","payload":{"message":"","replacement_history":[]}}"#.to_string() + "\n";
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + &token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10))
                + &turn_lines("2026-10-01T00:02:00Z", "turn-2")
                + &record_line(
                    "2026-10-01T00:02:00.005Z",
                    "turn-2",
                    "resp-2",
                    (500, 200, 40),
                )
                + &compacted
                + r#"{"timestamp":"2026-10-01T00:02:00.015Z","type":"turn_context","payload":{"turn_id":"turn-2","model":"gpt-5.5"}}"#
                + "\n"
                + &token_count_line("2026-10-01T00:02:00.020Z", (100, 80, 10), (100, 80, 10))
                + &turn_lines("2026-10-01T00:03:00Z", "turn-3")
                + &token_count_line("2026-10-01T00:03:00.005Z", (100, 80, 10), (100, 80, 10))
                + &record_line("2026-10-01T00:03:05Z", "turn-3", "resp-3", (30, 0, 3))
                + &token_count_line("2026-10-01T00:03:05.005Z", (130, 80, 13), (30, 0, 3))),
        );

        let messages = parse_codex_file(file.path());

        let totals: Vec<i64> = messages.iter().map(|m| m.tokens.total()).collect();
        assert_eq!(totals, vec![110, 540, 33]);
    }

    fn compacted_line(ts: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"compacted","payload":{{"message":"","replacement_history":[]}}}}"#
        ) + "\n"
    }

    /// The token_count Codex writes after a remote compaction: the totals do
    /// not move, and `last_token_usage` carries only an estimated
    /// `total_tokens` with zero counters.
    fn estimate_token_count_line(ts: &str, total: (i64, i64, i64), estimate: i64) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{},"last_token_usage":{{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":{estimate}}}}}}}}}"#,
            usage_json(total.0, total.1, total.2)
        ) + "\n"
    }

    fn turn_totals(messages: &[UnifiedMessage]) -> Vec<i64> {
        messages.iter().map(|m| m.tokens.total()).collect()
    }

    #[test]
    fn test_compaction_followed_by_estimate_token_count_is_counted() {
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + &token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10))
                + &record_line("2026-10-01T00:01:06Z", "turn-1", "resp-c", (500, 200, 40))
                + &compacted_line("2026-10-01T00:01:06.010Z")
                + r#"{"timestamp":"2026-10-01T00:01:06.015Z","type":"turn_context","payload":{"turn_id":"turn-1","model":"gpt-5.5"}}"#
                + "\n"
                + &estimate_token_count_line("2026-10-01T00:01:06.020Z", (100, 80, 10), 15823)
                + &record_line("2026-10-01T00:01:07Z", "turn-1", "resp-2", (30, 0, 3))
                + &token_count_line("2026-10-01T00:01:07.005Z", (130, 80, 13), (30, 0, 3))),
        );

        assert_eq!(
            turn_totals(&parse_codex_file(file.path())),
            vec![110, 540, 33]
        );
    }

    #[test]
    fn test_inline_compaction_reported_by_token_count_is_counted_once() {
        // An inline compaction reports its response through a token_count
        // before `compacted`, then Codex writes the estimate snapshot.
        let file = create_test_file(
            &(session_meta_line("codex_cli_rs")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + &token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10))
                + &record_line("2026-10-01T00:01:06Z", "turn-1", "resp-c", (500, 200, 40))
                + &token_count_line("2026-10-01T00:01:06.005Z", (600, 280, 50), (500, 200, 40))
                + &compacted_line("2026-10-01T00:01:06.010Z")
                + &estimate_token_count_line("2026-10-01T00:01:06.020Z", (600, 280, 50), 15823)
                + &record_line("2026-10-01T00:01:07Z", "turn-1", "resp-2", (30, 0, 3))
                + &token_count_line("2026-10-01T00:01:07.005Z", (630, 280, 53), (30, 0, 3))),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(turn_totals(&messages), vec![110, 540, 33]);
        assert!(messages.iter().all(|m| m
            .dedup_key
            .as_deref()
            .unwrap()
            .starts_with("codex:token_count")));
    }

    #[test]
    fn test_compaction_reported_by_the_token_count_after_compacted_is_counted_once() {
        // Should Codex start reporting the compaction in the token_count
        // after `compacted`, that token_count counts it and the record is
        // confirmed rather than counted a second time.
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + &token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10))
                + &record_line(
                    "2026-10-01T00:02:00.005Z",
                    "turn-1",
                    "resp-2",
                    (500, 200, 40),
                )
                + &compacted_line("2026-10-01T00:02:00.010Z")
                + &token_count_line("2026-10-01T00:02:00.020Z", (600, 280, 50), (500, 200, 40))),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(turn_totals(&messages), vec![110, 540]);
        assert!(messages.iter().all(|m| m
            .dedup_key
            .as_deref()
            .unwrap()
            .starts_with("codex:token_count")));
    }

    #[test]
    fn test_forked_child_parent_compaction_replayed_after_gate_is_not_counted() {
        let file = create_test_file(concat!(
            r#"{"timestamp":"2026-05-05T21:51:57.991Z","type":"session_meta","payload":{"id":"child-session","forked_from_id":"parent-session","source":"vscode","model_provider":"openai"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:57.994Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.947Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.9475Z","type":"token_usage_record","payload":{"turn_id":"p","response_id":"parent-compact","usage":{"input_tokens":500,"output_tokens":40,"total_tokens":540}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.9476Z","type":"compacted","payload":{"message":"","replacement_history":[]}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:58.948Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":300,"output_tokens":30,"total_tokens":330},"last_token_usage":{"input_tokens":0,"output_tokens":0,"total_tokens":15823}}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.2525Z","type":"token_usage_record","payload":{"turn_id":"c","response_id":"child-resp","usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}}"#,
            "\n",
            r#"{"timestamp":"2026-05-05T21:51:59.253Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":310,"output_tokens":32,"total_tokens":342},"last_token_usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}}}"#,
            "\n"
        ));

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert_eq!(
            turn_totals(&parsed.messages),
            vec![12],
            "only the child's own response"
        );
        assert!(!parsed.state.turn_coverage.turn_ids.contains("p"));
    }

    #[test]
    fn test_usage_record_from_another_thread_is_not_counted() {
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &format!(
                    r#"{{"timestamp":"2026-10-01T00:01:02Z","type":"token_usage_record","payload":{{"thread_id":"01a0f000-0000-7000-8000-0000000000aa","turn_id":"turn-x","response_id":"resp-x","usage":{}}}}}"#,
                    usage_json(900, 0, 90)
                )
                + "\n"
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + &token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10))
                + &format!(
                    r#"{{"timestamp":"2026-10-01T00:01:06Z","type":"token_usage_record","payload":{{"thread_id":"01a0f000-0000-7000-8000-0000000000aa","turn_id":"turn-x","response_id":"resp-y","usage":{}}}}}"#,
                    usage_json(800, 0, 80)
                )
                + "\n"
                + &turn_lines("2026-10-01T00:02:00Z", "turn-2")),
        );

        assert_eq!(turn_totals(&parse_codex_file(file.path())), vec![110]);
    }

    #[test]
    fn test_unconfirmed_record_uses_its_own_model_when_it_names_one() {
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:02:00Z", "turn-2")
                + &format!(
                    r#"{{"timestamp":"2026-10-01T00:02:00.005Z","type":"token_usage_record","payload":{{"thread_id":"{COMPACTION_THREAD}","turn_id":"turn-2","response_id":"resp-2","model":"gpt-5.4-mini","usage":{}}}}}"#,
                    usage_json(500, 200, 40)
                )
                + "\n"
                + &compacted_line("2026-10-01T00:02:00.010Z")
                + &estimate_token_count_line("2026-10-01T00:02:00.020Z", (0, 0, 0), 15823)),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.4-mini");
    }

    #[test]
    fn test_incremental_split_between_record_and_compacted_matches_full_parse() {
        let full = compaction_rollout("codex_vscode");
        let cut = full.find(r#""type":"compacted""#).unwrap();
        let cut = full[..cut].rfind('\n').unwrap() + 1;
        let mut file = create_test_file(&full[..cut]);
        let first = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
        let state: CodexParseState =
            serde_json::from_str(&serde_json::to_string(&first.state).unwrap()).unwrap();
        file.write_all(&full.as_bytes()[cut..]).unwrap();
        file.flush().unwrap();
        let second = parse_codex_file_incremental(file.path(), first.consumed_offset, state);

        let incremental: Vec<(i64, Option<String>)> = first
            .messages
            .iter()
            .chain(second.messages.iter())
            .map(|m| (m.tokens.total(), m.dedup_key.clone()))
            .collect();
        let whole: Vec<(i64, Option<String>)> = parse_codex_file(file.path())
            .iter()
            .map(|m| (m.tokens.total(), m.dedup_key.clone()))
            .collect();
        assert_eq!(incremental, whole);
        assert_eq!(total_tokens(&parse_codex_file(file.path())), 705);
        assert!(second.state.turn_coverage.turn_ids.contains("turn-2"));
    }

    #[test]
    fn test_cut_off_response_is_emitted_when_its_turn_is_aborted() {
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + r#"{"timestamp":"2026-10-01T00:01:06Z","type":"event_msg","payload":{"type":"turn_aborted","turn_id":"turn-1","reason":"interrupted"}}"#
                + "\n"),
        );

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert_eq!(turn_totals(&parsed.messages), vec![110]);
        assert!(parsed.state.pending_usage_record.is_none());
    }

    #[test]
    fn test_cut_off_response_is_emitted_when_its_task_completes() {
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + r#"{"timestamp":"2026-10-01T00:01:06Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}}"#
                + "\n"),
        );

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert_eq!(turn_totals(&parsed.messages), vec![110]);
        assert!(parsed.state.pending_usage_record.is_none());
    }

    #[test]
    fn test_compaction_only_turn_with_cut_off_response_at_end_of_file() {
        // Known limitation, pinned: the compaction row records turn 2, so
        // OpenClaw's mirror row for turn 2 yields, while the cut-off response
        // after it stays pending at the end of a finished file.
        let file = create_test_file(
            &(session_meta_line("openclaw")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + &token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10))
                + &turn_lines("2026-10-01T00:02:00Z", "turn-2")
                + &record_line(
                    "2026-10-01T00:02:00.005Z",
                    "turn-2",
                    "resp-c",
                    (500, 200, 40),
                )
                + &compacted_line("2026-10-01T00:02:00.010Z")
                + &estimate_token_count_line("2026-10-01T00:02:00.020Z", (100, 80, 10), 15823)
                + &record_line("2026-10-01T00:02:05Z", "turn-2", "resp-2", (70, 0, 7))),
        );

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert_eq!(turn_totals(&parsed.messages), vec![110, 540]);
        assert!(parsed.state.turn_coverage.turn_ids.contains("turn-2"));
        assert!(parsed.state.pending_usage_record.is_some());
    }

    #[test]
    fn test_unconfirmed_record_keeps_its_own_tier_when_the_next_record_names_one() {
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + &format!(
                    r#"{{"timestamp":"2026-10-01T00:01:06Z","type":"token_usage_record","payload":{{"thread_id":"{COMPACTION_THREAD}","turn_id":"turn-1","response_id":"resp-2","usage":{{"input_tokens":30,"cached_input_tokens":0,"output_tokens":3,"reasoning_output_tokens":0,"total_tokens":33,"service_tier":"priority"}}}}}}"#
                )
                + "\n"
                + &token_count_line("2026-10-01T00:01:06.005Z", (30, 0, 3), (30, 0, 3))),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(turn_totals(&messages), vec![110, 33]);
        assert_eq!(messages[0].service_tier, None, "resp-1 named no tier");
        assert_eq!(messages[1].service_tier.as_deref(), Some("priority"));
    }

    #[test]
    fn test_record_pending_at_end_of_file_still_counts_as_a_usage_ledger() {
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + UNMETERED_ACTIVITY
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))),
        );

        let parsed = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());

        assert!(parsed.messages.is_empty());
        assert!(parsed.state.has_supported_usage);
        assert!(!parsed.is_missing_usage());
    }

    #[test]
    fn test_turn_context_without_turn_id_ends_the_previous_turns_wait() {
        // A `turn_context` without an id, and no `task_started` before it,
        // starts another turn; the previous turn's record is settled there
        // instead of being taken as confirmed by the new turn's token_count.
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + r#"{"timestamp":"2026-10-01T00:02:00Z","type":"turn_context","payload":{"model":"gpt-5.5"}}"#
                + "\n"
                + &token_count_line("2026-10-01T00:02:05.005Z", (30, 0, 3), (30, 0, 3))),
        );

        assert_eq!(turn_totals(&parse_codex_file(file.path())), vec![110, 33]);
    }

    #[test]
    fn test_unconfirmed_record_after_human_message_is_the_turn_start() {
        let user_message = r#"{"timestamp":"2026-10-01T00:02:00.001Z","type":"event_msg","payload":{"type":"user_message","message":"summarize the log"}}"#.to_string() + "\n";
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:02:00Z", "turn-2")
                + &user_message
                + &record_line(
                    "2026-10-01T00:02:00.005Z",
                    "turn-2",
                    "resp-c",
                    (500, 200, 40),
                )
                + &compacted_line("2026-10-01T00:02:00.010Z")
                + &estimate_token_count_line("2026-10-01T00:02:00.020Z", (0, 0, 0), 15823)
                + &record_line("2026-10-01T00:02:05Z", "turn-2", "resp-2", (30, 0, 3))
                + &token_count_line("2026-10-01T00:02:05.005Z", (30, 0, 3), (30, 0, 3))),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(turn_totals(&messages), vec![540, 33]);
        assert!(
            messages[0].is_turn_start,
            "the compaction is the first reply"
        );
        assert!(!messages[1].is_turn_start);
    }

    fn total_only_token_count_line(ts: &str, total: (i64, i64, i64)) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{}}}}}}}"#,
            usage_json(total.0, total.1, total.2)
        ) + "\n"
    }

    fn last_only_token_count_line(ts: &str, last: (i64, i64, i64)) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{}}}}}}}"#,
            usage_json(last.0, last.1, last.2)
        ) + "\n"
    }

    fn utc_day(timestamp_ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(timestamp_ms)
            .unwrap()
            .format("%Y-%m-%d")
            .to_string()
    }

    #[test]
    fn test_compaction_confirmed_by_a_last_only_token_count_is_counted_once() {
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:02:00Z", "turn-2")
                + &record_line("2026-10-01T00:02:00.005Z", "turn-2", "resp-2", (500, 0, 40))
                + &compacted_line("2026-10-01T00:02:00.010Z")
                + &last_only_token_count_line("2026-10-01T00:02:00.020Z", (500, 0, 40))),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(turn_totals(&messages), vec![540]);
        assert!(messages[0]
            .dedup_key
            .as_deref()
            .unwrap()
            .starts_with("codex:token_count"));
    }

    #[test]
    fn test_compaction_confirmed_by_a_total_only_token_count_is_counted_once() {
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + &token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10))
                + &record_line(
                    "2026-10-01T00:02:00.005Z",
                    "turn-1",
                    "resp-2",
                    (500, 200, 40),
                )
                + &compacted_line("2026-10-01T00:02:00.010Z")
                + &total_only_token_count_line("2026-10-01T00:02:00.020Z", (600, 280, 50))),
        );

        assert_eq!(turn_totals(&parse_codex_file(file.path())), vec![110, 540]);
    }

    #[test]
    fn test_compaction_confirmed_by_a_last_only_token_count_after_a_baseline_is_counted_once() {
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + &token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10))
                + &record_line(
                    "2026-10-01T00:02:00.005Z",
                    "turn-1",
                    "resp-2",
                    (500, 200, 40),
                )
                + &compacted_line("2026-10-01T00:02:00.010Z")
                + &last_only_token_count_line("2026-10-01T00:02:00.020Z", (500, 200, 40))),
        );

        assert_eq!(turn_totals(&parse_codex_file(file.path())), vec![110, 540]);
    }

    #[test]
    fn test_compaction_is_not_confirmed_by_a_response_with_the_same_token_sum() {
        // (500 in, 40 out) and (400 in, 140 out) both sum to 540; they are
        // different responses, so the compaction is counted from its record.
        let file = create_test_file(
            &(session_meta_line("codex_vscode")
                + &turn_lines("2026-10-01T00:01:00Z", "turn-1")
                + &record_line("2026-10-01T00:01:05Z", "turn-1", "resp-1", (100, 80, 10))
                + &token_count_line("2026-10-01T00:01:05.005Z", (100, 80, 10), (100, 80, 10))
                + &record_line(
                    "2026-10-01T00:02:00.005Z",
                    "turn-1",
                    "resp-c",
                    (500, 200, 40),
                )
                + &compacted_line("2026-10-01T00:02:00.010Z")
                + &token_count_line("2026-10-01T00:02:00.020Z", (500, 80, 150), (400, 0, 140))),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(turn_totals(&messages), vec![110, 540, 540]);
        assert_eq!(
            messages[1].dedup_key.as_deref(),
            Some(format!("codex:usage-record:{COMPACTION_THREAD}:resp-c").as_str())
        );
        assert_eq!(
            (messages[2].tokens.input, messages[2].tokens.output),
            (400, 140)
        );
    }

    /// Turn 1 answers; turn 2's response is recorded just before UTC
    /// midnight; turn 3 starts before turn 2's token_count is written, and
    /// that token_count arrives late, then turn 3 answers.
    fn late_confirmation_rollout() -> (String, String) {
        let head = session_meta_line("codex_vscode")
            + &turn_lines("2026-10-01T23:58:00Z", "turn-1")
            + &record_line("2026-10-01T23:58:05Z", "turn-1", "resp-1", (100, 80, 10))
            + &token_count_line("2026-10-01T23:58:05.005Z", (100, 80, 10), (100, 80, 10))
            + &turn_lines("2026-10-01T23:59:00Z", "turn-2")
            + &record_line(
                "2026-10-01T23:59:59.500Z",
                "turn-2",
                "resp-2",
                (500, 200, 40),
            )
            + &turn_lines("2026-10-02T00:00:01Z", "turn-3");
        let tail = token_count_line("2026-10-02T00:00:01.500Z", (600, 280, 50), (500, 200, 40))
            + &record_line("2026-10-02T00:00:05Z", "turn-3", "resp-3", (30, 0, 3))
            + &token_count_line("2026-10-02T00:00:05.005Z", (630, 280, 53), (30, 0, 3));
        (head, tail)
    }

    #[test]
    fn test_late_token_count_for_a_record_already_counted_adds_nothing() {
        let (head, tail) = late_confirmation_rollout();
        let file = create_test_file(&(head + &tail));

        let messages = parse_codex_file(file.path());

        assert_eq!(turn_totals(&messages), vec![110, 540, 33]);
        assert_eq!(
            messages[1].dedup_key.as_deref(),
            Some(format!("codex:usage-record:{COMPACTION_THREAD}:resp-2").as_str())
        );
        assert_eq!(utc_day(messages[1].timestamp), "2026-10-01");
    }

    #[test]
    fn test_late_token_count_does_not_take_the_next_turns_pending_record() {
        // Turn 3's record is already pending when turn 2's late token_count
        // arrives; the late report settles nothing for turn 3.
        let (head, _) = late_confirmation_rollout();
        let file = create_test_file(
            &(head
                + &record_line("2026-10-02T00:00:01.200Z", "turn-3", "resp-3", (30, 0, 3))
                + &token_count_line("2026-10-02T00:00:01.500Z", (600, 280, 50), (500, 200, 40))
                + &token_count_line("2026-10-02T00:00:05.005Z", (630, 280, 53), (30, 0, 3))),
        );

        let messages = parse_codex_file(file.path());

        assert_eq!(turn_totals(&messages), vec![110, 540, 33]);
        assert!(messages[2]
            .dedup_key
            .as_deref()
            .unwrap()
            .starts_with("codex:token_count"));
    }

    #[test]
    fn test_late_token_count_across_incremental_chunks_matches_full_parse() {
        let (head, tail) = late_confirmation_rollout();
        let full = head.clone() + &tail;
        let whole = parse_codex_file(create_test_file(&full).path());
        let whole_rows: Vec<(i64, String)> = whole
            .iter()
            .map(|m| (m.tokens.total(), utc_day(m.timestamp)))
            .collect();
        assert_eq!(
            whole_rows,
            vec![
                (110, "2026-10-01".to_string()),
                (540, "2026-10-01".to_string()),
                (33, "2026-10-02".to_string())
            ]
        );

        // Split after the record (still pending) and after the next turn's
        // start (already emitted and remembered as settled).
        let after_record = head
            .rfind(r#""type":"event_msg","payload":{"type":"task_started","turn_id":"turn-3""#)
            .unwrap();
        let after_record = head[..after_record].rfind('\n').unwrap() + 1;
        for cut in [after_record, head.len()] {
            let mut file = create_test_file(&full[..cut]);
            let first = parse_codex_file_incremental(file.path(), 0, CodexParseState::default());
            let state: CodexParseState =
                serde_json::from_str(&serde_json::to_string(&first.state).unwrap()).unwrap();
            file.write_all(&full.as_bytes()[cut..]).unwrap();
            file.flush().unwrap();
            let second = parse_codex_file_incremental(file.path(), first.consumed_offset, state);
            let rows: Vec<(i64, String)> = first
                .messages
                .iter()
                .chain(second.messages.iter())
                .map(|m| (m.tokens.total(), utc_day(m.timestamp)))
                .collect();
            assert_eq!(rows, whole_rows, "cut at byte {cut}");
        }
    }

    #[test]
    fn test_settled_usage_records_are_bounded() {
        let mut state = CodexParseState::default();
        for input in 0..(SETTLED_USAGE_RECORD_LIMIT as i64 + 4) {
            state.remember_settled_usage_record(CodexTotals {
                input,
                output: 1,
                cached: 0,
                reasoning: 0,
            });
        }

        assert_eq!(
            state.settled_usage_records.len(),
            SETTLED_USAGE_RECORD_LIMIT
        );
        assert_eq!(state.settled_usage_records.front().unwrap().input, 4);
    }
}
