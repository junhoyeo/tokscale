//! Pi (badlogic/pi-mono) session parser
//!
//! Parses JSONL files from `~/.pi/agent/sessions/<encoded-cwd>/*.jsonl`. Current
//! OMP builds write a `title` metadata record before the `session` header in
//! newly-created session files; see `PRE_SESSION_METADATA_TYPES`.
//!
//! Human-readable session titles resolve in precedence order: the `session`
//! header's `title` field (v3 headers; OMP rewrites the header in place on
//! rename, so it holds the latest title), then the most recent `title` or
//! `title_change` record seen so far (`title` records are read before the
//! header; afterwards renames arrive as `title_change`). Resolution is
//! point-in-time: a mid-session rename retitles only the messages that
//! follow it, so on pre-v3 transcripts (no header title) the early messages
//! carry the first recorded title. The resolved title is copied onto every
//! message's `session_title`; the TUI adopts the most recent one, so the
//! Sessions tab shows the latest rename instead of the UUID.
//!
//! Pi descendants reuse this record layout verbatim, so `parse_pi_format_file`
//! is shared: see `sessions::kimchi` for Kimchi, `sessions::senpi` for Senpi (OmO Native),
//! `sessions::omp` for Oh My Pi, which owns the `~/.omp/agent/sessions` root, and
//! `sessions::prime_agent` for Prime Agent, which enters the same parser through
//! `parse_pi_format_rlm_file_with_observer`.
//!
//! [`PI_FORMAT_PARSER_BASE_VERSION`] defines the shared base parser version for these
//! clients, ensuring format-level changes in `pi.rs` invalidate cached messages
//! consistently across all five (#1195).

use super::utils::{
    back_anchor_timestamp, file_modified_timestamp_ms, for_each_json_line_with_bytes,
    parse_json_line,
};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::clients::ClientId;
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::path::Path;

/// Base parser version for clients sharing the Pi transcript format.
///
/// Any parsing change in `sessions/pi.rs` that affects how transcripts are parsed
/// changes what byte-identical files parse to across all delegating clients:
/// Pi, Kimchi, Oh My Pi (Omp), Senpi, and Prime Agent.
///
/// Bumping this base version moves all delegating clients together in
/// `message_cache::parser_version()`, while allowing each client to keep its
/// own client-specific version offset for independent historical invalidations
/// (e.g. dedup key changes, session metadata).
///
/// v3 retained assistant `durationMs`; v4 prefers the assistant message's
/// authoritative request-start `timestamp` over the later persistence time.
/// v5 includes standalone `usage` in Prime Agent, tolerates unrelated root
/// metadata, and separates operation dedup identities from assistant messages.
/// Other billed operations retain their member offsets in `message_cache.rs`.
pub const PI_FORMAT_PARSER_BASE_VERSION: u32 = 5;

/// Pi session header (first line of JSONL)
#[derive(Debug, Deserialize)]
pub struct PiSessionHeader {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub id: String,
    #[allow(dead_code)]
    pub timestamp: Option<String>,
    #[allow(dead_code)]
    pub cwd: Option<String>,
    /// v3 OMP headers carry the current session title (rewritten in place on
    /// auto-rename). Absent in older Pi/OMP files.
    pub title: Option<String>,
    #[serde(rename = "parentSession")]
    pub parent_session: Option<String>,
    #[serde(rename = "rlmDepth")]
    pub rlm_depth: Option<u32>,
}

impl PiSessionHeader {
    pub(crate) fn has_invalid_lineage(&self) -> bool {
        let is_rlm_child = self.rlm_depth.unwrap_or(0) > 0;
        is_rlm_child
            && self
                .parent_session
                .as_deref()
                .is_none_or(|parent| parent.trim().is_empty() || has_replacement_character(parent))
    }
}

fn damaged_key_may_name(key: &str, expected: &str) -> bool {
    if !has_replacement_character(key) {
        return false;
    }

    let pattern: Vec<char> = key.chars().collect();
    let expected: Vec<char> = expected.chars().collect();
    let mut matches = vec![vec![false; expected.len() + 1]; pattern.len() + 1];
    matches[0][0] = true;
    for pattern_index in 0..pattern.len() {
        for expected_index in 0..=expected.len() {
            if !matches[pattern_index][expected_index] {
                continue;
            }
            if pattern[pattern_index] == char::REPLACEMENT_CHARACTER {
                // A replacement in the middle may represent either damaged
                // bytes that replaced key characters or an undecodable byte
                // inserted between otherwise intact characters. At an edge it
                // must consume at least one expected character, so complete
                // known keys with a damaged extension prefix/suffix are not
                // mistaken for the known key itself.
                let minimum = if pattern_index > 0 && pattern_index + 1 < pattern.len() {
                    expected_index
                } else {
                    expected_index + 1
                };
                for matched in matches[pattern_index + 1].iter_mut().skip(minimum) {
                    *matched = true;
                }
            } else if expected_index < expected.len()
                && pattern[pattern_index] == expected[expected_index]
            {
                matches[pattern_index + 1][expected_index + 1] = true;
            }
        }
    }
    matches[pattern.len()][expected.len()]
}

const PRIME_LINEAGE_HEADER_KEYS: &[&str] = &["parentSession", "rlmDepth"];

/// Inspect every top-level JSON key after JSON string decoding and reject a
/// replacement-bearing spelling that may be a damaged Prime lineage key.
///
/// Valid U+FFFD characters are not inherently damage: unrelated extension
/// keys, including `rlmDepth\u{FFFD}`, remain valid. Invalid UTF-8 is tracked
/// separately because a replacement immediately beside a complete structural
/// key may have replaced rather than extended that key.
pub(crate) fn raw_json_has_damaged_lineage_header_key(raw: &[u8]) -> bool {
    let mut index = 0usize;
    let mut depth = 0usize;
    while index < raw.len() {
        match raw[index] {
            b'{' | b'[' => {
                depth += 1;
                index += 1;
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            b'"' => {
                let quoted_start = index;
                index += 1;
                let mut escaped = false;
                while index < raw.len() {
                    let byte = raw[index];
                    if escaped {
                        escaped = false;
                    } else if byte == b'\\' {
                        escaped = true;
                    } else if byte == b'"' {
                        break;
                    }
                    index += 1;
                }
                if index >= raw.len() {
                    return false;
                }

                let quoted = &raw[quoted_start..=index];
                let key_bytes = &raw[quoted_start + 1..index];
                let mut after = index + 1;
                while after < raw.len() && raw[after].is_ascii_whitespace() {
                    after += 1;
                }
                if depth == 1 && raw.get(after) == Some(&b':') {
                    let key_had_invalid_utf8 = std::str::from_utf8(key_bytes).is_err();
                    let lossy_quoted = String::from_utf8_lossy(quoted);
                    if let Ok(decoded) = serde_json::from_str::<String>(&lossy_quoted) {
                        let without_replacements: String = decoded
                            .chars()
                            .filter(|character| *character != char::REPLACEMENT_CHARACTER)
                            .collect();
                        if PRIME_LINEAGE_HEADER_KEYS.iter().any(|expected| {
                            damaged_key_may_name(&decoded, expected)
                                || (key_had_invalid_utf8 && without_replacements == *expected)
                        }) {
                            return true;
                        }
                    }
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    false
}

/// Pre-session `title` metadata record written by OMP. Only the title string
/// is consumed; `v`/`source`/`updatedAt`/`pad` are ignored.
#[derive(Debug, Deserialize)]
struct PiTitleRecord {
    title: Option<String>,
}

/// Loose type-only probe for a JSONL line, used to identify pre-session
/// metadata records without requiring their full schema.
#[derive(Debug, Deserialize)]
struct PiEntryTypeProbe {
    #[serde(rename = "type")]
    entry_type: String,
}

/// Record types OMP may write before the `session` header (e.g. an
/// auto-generated-title record). The parser skips these while looking for
/// `session` rather than discarding the whole file. Any other unrecognized
/// type before `session` is still treated as a malformed file.
pub(crate) const PRE_SESSION_METADATA_TYPES: &[&str] = &["title"];

/// A lossy pre-header line is skippable only when it could not be parsed for
/// its record type. A replacement-bearing line that parses as a real type is
/// still treated as a foreign/malformed file, keeping both Prime scans aligned.
pub(crate) fn has_replacement_character(value: &str) -> bool {
    value.contains(char::REPLACEMENT_CHARACTER)
}

pub(crate) fn pre_header_line_is_skippable(trimmed: &str, parsed_type: Option<&str>) -> bool {
    parsed_type.is_none() && has_replacement_character(trimmed)
}

/// Pi session entry (subsequent lines of JSONL)
#[derive(Debug, Deserialize)]
pub struct PiSessionEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    #[allow(dead_code)]
    pub id: Option<String>,
    #[serde(rename = "parentId")]
    #[allow(dead_code)]
    pub parent_id: Option<String>,
    pub timestamp: Option<String>,
    pub message: Option<PiMessage>,
    /// Present on billed operation entries (`compaction`, `branch_summary`,
    /// `usage`): the API call behind them has no `message` record of its own.
    #[serde(default, deserialize_with = "deserialize_standalone_field")]
    pub usage: Option<PiUsage>,
    /// Standalone `usage` records identify their billed provider and model.
    /// `model_change` records use `provider` plus `modelId` instead.
    #[serde(default, deserialize_with = "deserialize_standalone_field")]
    pub provider: Option<String>,
    #[serde(default, deserialize_with = "deserialize_standalone_field")]
    pub model: Option<String>,
    #[serde(rename = "modelId")]
    pub model_id: Option<String>,
    pub name: Option<String>,
    /// Present on `title_change` entries.
    pub title: Option<String>,
    #[serde(rename = "targetId")]
    pub target_id: Option<String>,
    #[serde(rename = "childUsage")]
    pub child_usage: Option<PiUsage>,
    #[serde(rename = "aggregateUsage")]
    pub aggregate_usage: Option<PiUsage>,
    #[serde(flatten)]
    #[allow(dead_code)]
    extra_fields: BTreeMap<String, serde_json::Value>,
}

// Other record types can use these top-level names for unrelated metadata.
fn deserialize_standalone_field<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

impl PiSessionEntry {
    fn has_damaged_timestamp(&self) -> bool {
        self.timestamp
            .as_deref()
            .is_some_and(has_replacement_character)
            || self
                .extra_fields
                .keys()
                .any(|key| damaged_key_may_name(key, "timestamp"))
    }
}

#[derive(Debug, Deserialize)]
pub struct PiMessage {
    pub role: Option<String>,
    pub usage: Option<PiUsage>,
    pub model: Option<String>,
    pub provider: Option<String>,
    #[serde(rename = "responseId")]
    pub response_id: Option<String>,
    pub timestamp: Option<serde_json::Value>,
    #[serde(rename = "durationMs")]
    pub duration_ms: Option<serde_json::Value>,
}

/// The camelCase usage block of a Pi record: `utils::CamelUsage`'s
/// `{input, output, cacheRead, cacheWrite, totalTokens}` plus `reasoning` and
/// a flattened map of every remaining key. See the note on `CamelUsage` for
/// why the two shapes are not merged.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiUsage {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    /// Subset of `cache_write` billed at the 1-hour retention rate.
    pub cache_write_1h: Option<i64>,
    #[allow(dead_code)]
    pub total_tokens: Option<i64>,
    /// Pi's authoritative cost breakdown for this call. The operation rows
    /// newly emitted by this parser use `total` directly so custom or routed
    /// models do not become unpriced merely because Tokscale lacks a rate.
    pub cost: Option<PiUsageCost>,
    /// Parsed so the omission in `PiUsage::to_breakdown` is a real decision
    /// rather than an accident of the schema, but never summed.
    #[allow(dead_code)]
    pub reasoning: Option<i64>,
    #[serde(flatten)]
    extra_fields: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PiUsageCost {
    pub total: Option<f64>,
}

impl PiUsage {
    fn reported_cost(&self) -> Option<f64> {
        self.cost
            .as_ref()
            .and_then(|cost| cost.total)
            .filter(|total| total.is_finite() && *total >= 0.0)
    }

    /// Token breakdown with every field clamped at zero, in the spelling
    /// `utils::CamelUsage` uses for the same wire shape.
    ///
    /// `reasoning` is read but deliberately not mapped onto
    /// `TokenBreakdown::reasoning`. In the Pi format reasoning tokens are a
    /// subset of `output` (Pi's own `totalTokens` excludes them), whereas
    /// tokscale totals `reasoning` as its own additive bucket. Mapping it
    /// through would double count.
    pub(crate) fn to_breakdown(&self) -> TokenBreakdown {
        self.to_breakdown_inner(false)
    }

    fn to_operation_breakdown(&self) -> TokenBreakdown {
        self.to_breakdown_inner(true)
    }

    fn to_breakdown_inner(&self, include_cache_write_1h: bool) -> TokenBreakdown {
        let cache_write = self.cache_write.unwrap_or(0).max(0);
        TokenBreakdown {
            input: self.input.unwrap_or(0).max(0),
            output: self.output.unwrap_or(0).max(0),
            cache_read: self.cache_read.unwrap_or(0).max(0),
            cache_write,
            cache_write_1h: if include_cache_write_1h {
                self.cache_write_1h.unwrap_or(0).max(0).min(cache_write)
            } else {
                0
            },
            reasoning: 0,
        }
    }

    pub(crate) fn has_damaged_key(&self) -> bool {
        const TOKEN_COUNTER_KEYS: &[&str] = &[
            "input",
            "output",
            "cacheRead",
            "cacheWrite",
            "totalTokens",
            "reasoning",
        ];

        self.extra_fields.keys().any(|key| {
            TOKEN_COUNTER_KEYS
                .iter()
                .any(|expected| damaged_key_may_name(key, expected))
        })
    }
}

fn is_generated_id(value: &str) -> bool {
    (value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || (value.len() == 36
            && value.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            }))
}

fn strip_generated_id(value: &str) -> Option<&str> {
    for id_len in [36, 8] {
        if value.len() <= id_len || value.as_bytes()[value.len() - id_len - 1] != b'-' {
            continue;
        }
        let id = &value[value.len() - id_len..];
        if is_generated_id(id) {
            return Some(&value[..value.len() - id_len - 1]);
        }
    }
    None
}

/// OMP nests subagent transcripts one level below the workspace dir inside a
/// directory named after the PARENT session's file stem
/// (`<timestamp>_<parent-uuid>/<AgentName>.jsonl`, with the parent transcript
/// at `<timestamp>_<parent-uuid>.jsonl` beside it). Recover the parent
/// session id from that layout; top-level sessions sit directly in the
/// workspace dir and return `None`.
fn parent_session_id_from_path(path: &Path) -> Option<String> {
    let dir_name = path.parent()?.file_name()?.to_str()?;
    let (timestamp, uuid) = dir_name.rsplit_once('_')?;
    // Timestamp stems look like `2026-07-28T20-45-44-298Z` — require the
    // shape loosely so ordinary workspace dirs never match.
    if !(timestamp.starts_with(|c: char| c.is_ascii_digit())
        && timestamp.contains('T')
        && timestamp.ends_with('Z'))
    {
        return None;
    }
    is_generated_id(uuid).then(|| uuid.to_string())
}

/// Trim a raw title, dropping blanks so an empty `title` field never
/// shadows a real one from another record.
fn normalized_title(title: Option<&str>) -> Option<String> {
    title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

fn pi_subagent_name(session_name: &str) -> Option<String> {
    let name = session_name.strip_prefix("subagent-")?;
    let without_id = strip_generated_id(name).or_else(|| {
        let (without_index, index) = name.rsplit_once('-')?;
        if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        strip_generated_id(without_index)
    })?;

    (!without_id.is_empty()).then(|| without_id.to_string())
}

/// Parse a Pi JSONL session file
pub fn parse_pi_file(path: &Path) -> Vec<UnifiedMessage> {
    parse_pi_format_file(path, "pi", "pi")
}

/// Parse a JSONL session file written in the Pi record format.
///
/// `client` is the tokscale client id stamped on every emitted message, and
/// `fallback_provider` is used only when the message carries no provider and
/// the model name is not recognizable.
///
/// Deduplication is cross-session (`PiParseOptions::standard_deduped`):
/// Pi branch/fork creates a new session file and copies prior assistant
/// records into it verbatim (#1306), so a per-session key would count every
/// copied record once per file. The parser emits a session-independent key —
/// `message.responseId` preferred, the entry id plus immutable event fields
/// as fallback — and the Pi lane drops repeats during aggregation.
pub(crate) fn parse_pi_format_file(
    path: &Path,
    client: &str,
    fallback_provider: &'static str,
) -> Vec<UnifiedMessage> {
    let mut observer = NoopPiFormatObserver;
    parse_pi_format_file_inner(
        path,
        client,
        fallback_provider,
        Some(client),
        PiParseOptions::standard_deduped(),
        &mut observer,
    )
}

/// Parse a Pi-format session and retain message ids in namespaced,
/// session-scoped dedup keys (`{client}:{session_id}:{message_id}`).
///
/// Used by Kimchi, which predates the cross-session dedup the standard lane
/// adopted for #1306 and is versioned separately; do not "unify" this with
/// `parse_pi_format_file` without a Kimchi cache-version bump.
pub(crate) fn parse_pi_format_file_with_dedup(
    path: &Path,
    client: &str,
    fallback_provider: &'static str,
) -> Vec<UnifiedMessage> {
    let mut observer = NoopPiFormatObserver;
    parse_pi_format_file_inner(
        path,
        client,
        fallback_provider,
        Some(client),
        PiParseOptions::standard(),
        &mut observer,
    )
}

/// Receives already-decoded Pi records while the shared parser walks a file.
///
/// Prime Agent uses this hook to derive its fork/child accounting metadata in
/// the same pass that emits messages. The emitted message is supplied only for
/// an assistant or standalone usage record that passed the shared validation.
pub(crate) trait PiFormatObserver {
    fn observe_header(&mut self, _header: &PiSessionHeader) {}

    fn observe_entry(&mut self, _entry: &PiSessionEntry, _emitted: Option<&UnifiedMessage>) {}
}

struct NoopPiFormatObserver;

impl PiFormatObserver for NoopPiFormatObserver {}

/// Parse the Prime Agent Pi-compatible format whose `session_info.name`
/// identifies an RLM subagent when the session header has `rlmDepth > 0`.
///
/// Deduplication is intentionally cross-session: Prime Agent forks copy prior
/// message entries into a file with a new session id. Provider response ids are
/// preferred; the message id plus immutable event fields is the fallback.
///
/// Prime Agent's append-only JSONL may contain a UTF-8 BOM or undecodable
/// records. Lossy line handling keeps malformed records local to their own
/// line without changing the historical behavior of other Pi clients.
pub(crate) fn parse_pi_format_rlm_file_with_observer(
    path: &Path,
    client: &str,
    fallback_provider: &'static str,
    observer: &mut impl PiFormatObserver,
) -> Vec<UnifiedMessage> {
    parse_pi_format_file_inner(
        path,
        client,
        fallback_provider,
        Some(client),
        PiParseOptions::prime_agent(),
        observer,
    )
}

#[derive(Clone, Copy)]
struct PiParseOptions {
    rlm_session_name_as_agent: bool,
    cross_session_dedup: bool,
    lossy_line_reader: bool,
    /// Whether the transcript bills work outside assistant replies
    /// (`compaction`, `branch_summary`, `toolResult`). Standalone `usage`
    /// is counted for every client. Prime Agent keeps other operations off
    /// while its accounting reconciles only assistant completions.
    count_operations: bool,
}

impl PiParseOptions {
    /// Keep the historical byte-strict behavior for Pi, Senpi, and Kimchi.
    /// Their cache namespaces are intentionally not invalidated by this
    /// Prime-Agent-only migration; revisit this when those clients opt into
    /// lossy decoding and receive their own parser-version bumps.
    const fn standard() -> Self {
        Self {
            rlm_session_name_as_agent: false,
            cross_session_dedup: false,
            lossy_line_reader: false,
            count_operations: true,
        }
    }

    /// Standard byte-strict parsing with cross-session deduplication: the
    /// Pi/Senpi/Omp lanes, whose clients all fork sessions by copying prior
    /// assistant records into a new file (#1306).
    const fn standard_deduped() -> Self {
        Self {
            rlm_session_name_as_agent: false,
            cross_session_dedup: true,
            lossy_line_reader: false,
            count_operations: true,
        }
    }

    const fn prime_agent() -> Self {
        Self {
            rlm_session_name_as_agent: true,
            cross_session_dedup: true,
            lossy_line_reader: true,
            count_operations: false,
        }
    }
}

fn accepts_replacement_field(value: &str, lossy_line_reader: bool) -> bool {
    !lossy_line_reader || !has_replacement_character(value)
}

fn damaged_cross_session_dedup_key(namespace: &str, raw_line: &[u8]) -> String {
    let mut hasher = Sha256::new();
    // Exact source bytes distinguish invalid UTF-8 sequences that lossy decode
    // maps to the same U+FFFD while keeping copied fork records stable.
    hasher.update(raw_line);
    format!("{namespace}:damaged:{:x}", hasher.finalize())
}

fn damaged_session_placeholder(path: &Path) -> String {
    let source_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(source_path.as_os_str().as_encoded_bytes());
    format!("unknown:path:{:x}", hasher.finalize())
}

/// One record, one decision: does this entry become a [`UnifiedMessage`]?
///
/// Two walks read the same transcript. [`parse_pi_format_file_inner`] streams it
/// and builds messages; Prime Agent's accounting walk replays already-parsed
/// messages positionally against the same records
/// (`sessions::prime_agent::analyze_prime_agent_accounting`). A record that only
/// one of them counts shifts every later index in the other, which silently
/// re-targets Prime's usage reconciliation. Both therefore ask this function
/// rather than restating the rules, so neither can drift from the other.
struct PiEmittedRecord<'a> {
    /// The message behind the record. `None` for a billed operation entry,
    /// which has usage but no message.
    message: Option<&'a PiMessage>,
    usage: &'a PiUsage,
    /// Model/provider attribution carried by this record. Standalone `usage`
    /// entries carry both at their root; compactions and branch summaries carry
    /// neither and stay in Pi's separate tools/summary bucket.
    recorded_model: Option<&'a str>,
    recorded_provider: Option<&'a str>,
    /// Of emitted records, only assistant replies advance the session model;
    /// `model_change` records do so separately. Cache-warm usage or tool
    /// summaries can use other attribution without changing the selection.
    updates_current_model: bool,
    /// Official Pi intentionally separates un-attributed tool and summary
    /// usage from model-attributed calls. Do not charge it to the preceding
    /// conversation model merely because the record follows that reply.
    uses_tools_summary_bucket: bool,
    /// These rows did not exist in the parser before this change. Preserve
    /// their authoritative Pi `usage.cost.total` instead of trying to infer a
    /// possibly custom/routed model's price from Tokscale's catalog.
    uses_reported_cost: bool,
}

const PI_TOOLS_SUMMARIES_MODEL: &str = "tools/summaries";

/// Trimmed field value, or `None` when it is absent or only whitespace.
/// Applied to the record types this parser newly counts, whose identifiers
/// have no stored spelling to stay compatible with.
fn non_blank_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// The entry types that bill an API call without being an assistant reply.
/// Pi's `/compact` writes `compaction`, a branch summarization writes
/// `branch_summary`, and standalone accounting (e.g. cache warming) writes
/// `usage`. All three carry the call's usage at the record root.
const PI_OPERATION_ENTRY_TYPES: &[&str] = &["compaction", "branch_summary", "usage"];

fn pi_emitted_record<'a>(
    entry: &'a PiSessionEntry,
    options: PiParseOptions,
    is_rlm_subagent: bool,
) -> Option<PiEmittedRecord<'a>> {
    let message = entry.message.as_ref();
    let role = message.and_then(|message| message.role.as_deref());
    let is_message = entry.entry_type == "message";
    let is_operation = PI_OPERATION_ENTRY_TYPES.contains(&entry.entry_type.as_str());
    let (
        usage,
        recorded_model,
        recorded_provider,
        updates_current_model,
        uses_tools_summary_bucket,
        uses_reported_cost,
    ) = if is_message && role == Some("assistant") {
        // A reply names the model it was billed against. Pi always records
        // one; a record without a model has no price, so it stays out as
        // before. Existing assistant output remains unchanged.
        let message = message?;
        (
            message.usage.as_ref()?,
            Some(message.model.as_deref()?),
            // Preserve recorded spellings, but infer a missing/blank provider.
            message.provider.as_deref().filter(|p| !p.trim().is_empty()),
            true,
            false,
            false,
        )
    } else if is_message && role == Some("toolResult") && options.count_operations {
        // A tool result can report the usage of the call that produced it.
        // These records are new to this parser, so surrounding whitespace is
        // normalized away rather than stored as part of an identifier.
        let message = message?;
        let recorded_model = non_blank_trimmed(message.model.as_deref());
        (
            message.usage.as_ref()?,
            recorded_model,
            non_blank_trimmed(message.provider.as_deref()),
            false,
            recorded_model.is_none(),
            true,
        )
    } else if is_operation && (options.count_operations || entry.entry_type == "usage") {
        // Current Pi UsageEntry records carry concrete root-level model and
        // provider fields. Older compaction/branch-summary records do not.
        (
            entry.usage.as_ref()?,
            non_blank_trimmed(entry.model.as_deref()),
            non_blank_trimmed(entry.provider.as_deref()),
            false,
            entry.entry_type != "usage",
            true,
        )
    } else {
        return None;
    };

    // An RLM child's completion timestamp participates in matching its
    // usage back to the parent attribution. Recovering a replacement-
    // damaged value as "missing" would make that match impossible while
    // still emitting the child, so the parent's aggregate and child would
    // both be counted. Other Pi messages do not use this timestamp as a
    // reconciliation join and remain recoverable.
    if options.lossy_line_reader
        && is_rlm_subagent
        && (entry.has_damaged_timestamp()
            || entry
                .timestamp
                .as_deref()
                .is_some_and(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).is_err()))
    {
        return None;
    }

    if options.lossy_line_reader && usage.has_damaged_key() {
        return None;
    }

    // The caller resolves explicit attribution, the official tools/summary
    // bucket, or the legacy current-model fallback for this emitted record.
    Some(PiEmittedRecord {
        message,
        usage,
        recorded_model,
        recorded_provider,
        updates_current_model,
        uses_tools_summary_bucket,
        uses_reported_cost,
    })
}

/// [`pi_emitted_record`] under the preset RLM transcripts are parsed with, as a
/// yes/no answer for a walk that already holds the emitted messages.
pub(crate) fn rlm_entry_emits_message(entry: &PiSessionEntry, is_rlm_subagent: bool) -> bool {
    pi_emitted_record(entry, PiParseOptions::prime_agent(), is_rlm_subagent).is_some()
}

fn parse_pi_format_file_inner(
    path: &Path,
    client: &str,
    fallback_provider: &'static str,
    dedup_namespace: Option<&str>,
    options: PiParseOptions,
    observer: &mut impl PiFormatObserver,
) -> Vec<UnifiedMessage> {
    let fallback_timestamp = file_modified_timestamp_ms(path);
    // The nested `<timestamp>_<uuid>/<Agent>.jsonl` layout is OMP's; Pi,
    // Kimchi, Senpi, and Prime Agent keep flat transcripts, so only OMP may
    // read a parent session out of the path.
    let parent_session_id = (client == ClientId::Omp.as_str())
        .then(|| parent_session_id_from_path(path))
        .flatten();

    let mut messages: Vec<UnifiedMessage> = Vec::with_capacity(64);
    let mut buffer = Vec::with_capacity(4096);

    let mut session_id: Option<String> = None;
    let mut workspace_key: Option<String> = None;
    let mut workspace_label: Option<String> = None;
    let mut agent: Option<String> = None;
    // The session's current model/provider selection. Official tool and
    // summary operations use their own bucket; these values remain the fallback
    // for legacy model-less standalone usage records.
    let mut last_model: Option<String> = None;
    let mut last_provider: Option<String> = None;
    let mut is_rlm_subagent = false;
    // A header this parser rejects discards the whole transcript, not one
    // record, so the sink stops the scan and nothing is returned.
    let mut malformed_transcript = false;
    // Title from the v3 `session` header — authoritative latest, since OMP
    // rewrites the header in place when the title changes.
    let mut header_title: Option<String> = None;
    // Latest title seen in a `title`/`title_change` record; used only when
    // the header carries no title.
    let mut rolling_title: Option<String> = None;

    for_each_json_line_with_bytes(path, &mut |line| {
        // Pi, Senpi and Kimchi keep the byte-strict record skipping of
        // `BufRead::lines()`: a record whose bytes are not valid UTF-8 is
        // dropped rather than read through its replacement characters.
        // Reading it lossily would make them emit messages they do not emit
        // today, which is the opt-in `PiParseOptions::standard` defers.
        if !options.lossy_line_reader && !line.valid_utf8 {
            return ControlFlow::Continue(());
        }
        let trimmed = line.trimmed;

        if session_id.is_none() {
            let entry_type = match parse_json_line::<PiEntryTypeProbe>(trimmed, &mut buffer) {
                Some(probe) => probe.entry_type,
                None if options.lossy_line_reader
                    && pre_header_line_is_skippable(trimmed, None) =>
                {
                    return ControlFlow::Continue(());
                }
                None => {
                    malformed_transcript = true;
                    return ControlFlow::Break(());
                }
            };

            if entry_type != "session" {
                if entry_type == "title" {
                    if let Some(record) = parse_json_line::<PiTitleRecord>(trimmed, &mut buffer) {
                        if let Some(title) = normalized_title(record.title.as_deref()) {
                            rolling_title = Some(title);
                        }
                    }
                    return ControlFlow::Continue(());
                }
                if PRE_SESSION_METADATA_TYPES.contains(&entry_type.as_str()) {
                    return ControlFlow::Continue(());
                }
                malformed_transcript = true;
                return ControlFlow::Break(());
            }

            let Some(header) = parse_json_line::<PiSessionHeader>(trimmed, &mut buffer) else {
                malformed_transcript = true;
                return ControlFlow::Break(());
            };
            if options.lossy_line_reader
                && (header.has_invalid_lineage()
                    || raw_json_has_damaged_lineage_header_key(line.bytes))
            {
                malformed_transcript = true;
                return ControlFlow::Break(());
            }

            observer.observe_header(&header);
            let clean_cwd = header
                .cwd
                .as_deref()
                .filter(|cwd| accepts_replacement_field(cwd, options.lossy_line_reader));
            session_id = Some(
                if !accepts_replacement_field(&header.id, options.lossy_line_reader) {
                    damaged_session_placeholder(path)
                } else {
                    header.id.clone()
                },
            );
            workspace_key = clean_cwd.and_then(normalize_workspace_key);
            workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
            is_rlm_subagent = header.rlm_depth.unwrap_or(0) > 0;
            header_title = normalized_title(header.title.as_deref());
            return ControlFlow::Continue(());
        }

        let Some(entry) = parse_json_line::<PiSessionEntry>(trimmed, &mut buffer) else {
            return ControlFlow::Continue(());
        };

        if entry.entry_type == "model_change" {
            // Pi persists the selected model immediately. Keep the selection
            // accurate for backward-compatible attribution of a legacy
            // model-less usage record before the next assistant reply.
            let clean_model = entry.model_id.as_deref().filter(|model| {
                !model.trim().is_empty()
                    && accepts_replacement_field(model, options.lossy_line_reader)
            });
            let clean_provider = entry.provider.as_deref().filter(|provider| {
                !provider.trim().is_empty()
                    && accepts_replacement_field(provider, options.lossy_line_reader)
            });
            if let (Some(model), Some(provider)) = (clean_model, clean_provider) {
                last_model = Some(model.to_string());
                last_provider = Some(provider.to_string());
            }
            observer.observe_entry(&entry, None);
            return ControlFlow::Continue(());
        }

        if entry.entry_type == "session_info" {
            agent = if options.rlm_session_name_as_agent && is_rlm_subagent {
                entry
                    .name
                    .as_ref()
                    .filter(|name| {
                        !name.trim().is_empty()
                            && accepts_replacement_field(name, options.lossy_line_reader)
                    })
                    .cloned()
            } else {
                entry
                    .name
                    .as_deref()
                    .filter(|name| accepts_replacement_field(name, options.lossy_line_reader))
                    .and_then(pi_subagent_name)
            };
            observer.observe_entry(&entry, None);
            return ControlFlow::Continue(());
        }

        if entry.entry_type == "title_change" {
            if let Some(title) = normalized_title(entry.title.as_deref()) {
                rolling_title = Some(title);
            }
            return ControlFlow::Continue(());
        }

        let Some(PiEmittedRecord {
            message,
            usage,
            recorded_model,
            recorded_provider,
            updates_current_model,
            uses_tools_summary_bucket,
            uses_reported_cost,
        }) = pi_emitted_record(&entry, options, is_rlm_subagent)
        else {
            observer.observe_entry(&entry, None);
            return ControlFlow::Continue(());
        };

        // Standalone `usage` rows and non-standard tool results can name their
        // own billed model. Official model-less tool/summary work stays in its
        // own bucket. Only legacy model-less `usage` rows inherit the current
        // session selection updated by `model_change` and assistant replies.
        let clean_recorded_provider = recorded_provider
            .filter(|provider| accepts_replacement_field(provider, options.lossy_line_reader));
        // Pi itself groups model-less tool results, compactions, and branch
        // summaries as Tools/summaries. Keep that billed work separate rather
        // than falsely charging the preceding conversation model. Standalone
        // usage rows name their actual model; legacy model-less usage inherits
        // the current selection as an atomic model/provider pair.
        let (model, provider) = if uses_tools_summary_bucket {
            (
                PI_TOOLS_SUMMARIES_MODEL.to_string(),
                fallback_provider.to_string(),
            )
        } else {
            match recorded_model {
                Some(recorded_model) => {
                    let model =
                        if accepts_replacement_field(recorded_model, options.lossy_line_reader) {
                            recorded_model.to_string()
                        } else {
                            "unknown".to_string()
                        };
                    let provider =
                        clean_recorded_provider
                            .map(str::to_string)
                            .unwrap_or_else(|| {
                                inferred_provider_from_model(&model)
                                    .unwrap_or(fallback_provider)
                                    .to_string()
                            });
                    (model, provider)
                }
                None => {
                    let model = last_model.clone().unwrap_or_else(|| "unknown".to_string());
                    let provider = last_provider.clone().unwrap_or_else(|| {
                        inferred_provider_from_model(&model)
                            .unwrap_or(fallback_provider)
                            .to_string()
                    });
                    (model, provider)
                }
            }
        };

        if updates_current_model {
            last_model = Some(model.clone());
            last_provider = Some(provider.clone());
        }

        let recorded_timestamp = entry
            .timestamp
            .as_deref()
            .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
            .map(|timestamp| timestamp.timestamp_millis());
        let duration_ms = message
            .and_then(|message| message.duration_ms.as_ref())
            .and_then(serde_json::Value::as_i64)
            .filter(|duration| *duration > 0);
        let message_start_timestamp = message
            .and_then(|message| message.timestamp.as_ref())
            .and_then(serde_json::Value::as_i64)
            .filter(|timestamp| *timestamp > 0);
        // Pi's assistant timestamp is the authoritative request start. Older
        // messages do not carry it, so derive the start from the session entry's
        // completion timestamp and elapsed duration as a compatibility fallback.
        let timestamp = match duration_ms {
            Some(duration) => message_start_timestamp
                .or_else(|| recorded_timestamp.map(|end| back_anchor_timestamp(end, duration)))
                .unwrap_or(fallback_timestamp),
            None => recorded_timestamp.unwrap_or(fallback_timestamp),
        };
        // A billed operation has no response of its own, so its dedup key falls
        // back to the entry id below.
        let response_id = message.and_then(|message| message.response_id.as_deref());

        let reported_cost = uses_reported_cost.then(|| usage.reported_cost()).flatten();
        let mut unified = UnifiedMessage::new_with_agent(
            client,
            model.as_str(),
            provider.as_str(),
            session_id.clone().unwrap_or_else(|| "unknown".to_string()),
            timestamp,
            if uses_reported_cost {
                usage.to_operation_breakdown()
            } else {
                usage.to_breakdown()
            },
            reported_cost.unwrap_or(0.0),
            agent.clone(),
        )
        .with_billing(
            // Record-level evidence only: a reply or tool result names its
            // provider on the message, a standalone operation at the root.
            // Legacy model-less usage rows inherit the session selection for
            // attribution but supply no billing evidence.
            clean_recorded_provider,
            super::BillingIdentityOrigin::ExplicitRecord,
        );
        if reported_cost.is_some() {
            unified.mark_provider_reported_cost();
        }
        unified.duration_ms = duration_ms;
        if let Some(namespace) = dedup_namespace {
            if options.cross_session_dedup {
                let clean_response_key = response_id
                    .filter(|id| {
                        !id.trim().is_empty()
                            && accepts_replacement_field(id, options.lossy_line_reader)
                    })
                    .map(|id| format!("{namespace}:response:{id}"));
                let clean_message_key = entry.id.as_deref().filter(|id| {
                    !id.trim().is_empty()
                        && accepts_replacement_field(id, options.lossy_line_reader)
                });
                unified.dedup_key = clean_response_key.or_else(|| {
                    clean_message_key.map(|id| {
                        let stable_timestamp = recorded_timestamp
                            .map(|timestamp| timestamp.to_string())
                            .unwrap_or_else(|| "missing".to_string());
                        format!(
                            "{namespace}:{}:{id}:{stable_timestamp}:{provider}:{model}:{}:{}:{}:{}",
                            entry.entry_type,
                            unified.tokens.input,
                            unified.tokens.output,
                            unified.tokens.cache_read,
                            unified.tokens.cache_write,
                        )
                    })
                });
                if unified.dedup_key.is_none() && options.lossy_line_reader {
                    let has_damaged_id = entry.id.as_deref().is_some_and(|id| {
                        !accepts_replacement_field(id, options.lossy_line_reader)
                    }) || response_id.is_some_and(|id| {
                        !accepts_replacement_field(id, options.lossy_line_reader)
                    });
                    if has_damaged_id {
                        unified.dedup_key =
                            Some(damaged_cross_session_dedup_key(namespace, line.bytes));
                    }
                }
            } else if let Some(message_id) = entry.id.as_deref().filter(|id| !id.trim().is_empty())
            {
                let session_id = session_id.as_deref().unwrap_or("unknown");
                unified.dedup_key = Some(if entry.entry_type == "usage" {
                    format!("{namespace}:{session_id}:usage:{message_id}")
                } else {
                    format!("{namespace}:{session_id}:{message_id}")
                });
            }
        }
        unified.session_title = header_title.clone().or_else(|| rolling_title.clone());
        unified.parent_session_id = parent_session_id.clone();
        unified.set_workspace(workspace_key.clone(), workspace_label.clone());
        observer.observe_entry(&entry, Some(&unified));
        messages.push(unified);
        ControlFlow::Continue(())
    });

    if malformed_transcript {
        return Vec::new();
    }

    // A `title_change` after the last message has no later message to retitle,
    // and a pre-v3 transcript has no header title to fall back on, so the
    // rename would never surface. The Sessions bucket adopts the latest title
    // from the latest message, so stamp the final title there.
    if header_title.is_none() {
        if let (Some(title), Some(last)) = (rolling_title, messages.last_mut()) {
            last.session_title = Some(title);
        }
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    fn parse_prime_test_file(path: &Path) -> Vec<UnifiedMessage> {
        let mut observer = NoopPiFormatObserver;
        parse_pi_format_rlm_file_with_observer(path, "prime-agent", "prime-agent", &mut observer)
    }

    #[test]
    fn standalone_usage_counts_arbitrary_kinds_across_pi_format_clients() {
        let file = create_test_file(
            r#"{"type":"session","id":"usage-session","cwd":"/tmp/project","title":"Background usage"}
{"type":"session_info","name":"subagent-reviewer-abcd1234-1"}
{"type":"usage","id":"warm-1","timestamp":"2026-10-16T12:00:00Z","kind":"cache_warm","provider":"anthropic","model":"claude-sonnet-4-5","usage":{"input":0,"output":0,"cacheRead":100,"cacheWrite":20,"totalTokens":120}}
{"type":"usage","id":"other-1","timestamp":"2026-10-16T12:01:00Z","kind":"future_operation","provider":"example-provider","model":"example-model","usage":{"input":12,"output":3,"reasoning":2,"cacheRead":4,"cacheWrite":5,"totalTokens":24,"cost":{"total":0.01}}}
{"type":"message","id":"reply-1","timestamp":"2026-10-16T12:02:00Z","message":{"role":"assistant","provider":"openai","model":"gpt-5","usage":{"input":10,"output":5}}}"#,
        );

        for messages in [
            parse_pi_file(file.path()),
            crate::sessions::senpi::parse_senpi_file(file.path()),
            crate::sessions::kimchi::parse_kimchi_file(file.path()),
            crate::sessions::omp::parse_omp_file(file.path()),
            parse_prime_test_file(file.path()),
        ] {
            assert_eq!(messages.len(), 3);
            assert_eq!(messages[0].tokens.cache_read, 100);
            assert_eq!(messages[0].tokens.cache_write, 20);
            assert_eq!(messages[1].model_id, "example-model");
            assert_eq!(messages[1].provider_id, "example-provider");
            assert_eq!(messages[1].tokens.total(), 24);
            assert_eq!(messages[1].tokens.reasoning, 0);
            assert_eq!(messages[1].cost, 0.01);
            assert_eq!(
                messages[1].cost_source,
                super::super::CostSource::ProviderReported
            );
            assert_eq!(messages[1].timestamp, 1792152060000);
            assert_eq!(messages[1].duration_ms, None);
            assert_eq!(messages[1].session_id, "usage-session");
            assert_eq!(
                messages[1].session_title.as_deref(),
                Some("Background usage")
            );
            assert_eq!(messages[1].agent.as_deref(), Some("reviewer"));
            assert_eq!(messages[1].workspace_key.as_deref(), Some("/tmp/project"));
            assert_eq!(messages[2].tokens.total(), 15);
        }
    }

    #[test]
    fn standalone_usage_fields_do_not_reject_unrelated_records() {
        for field in [
            r#""usage":"extension metadata""#,
            r#""usage":{"input":"extension metadata"}"#,
            r#""model":{"name":"extension metadata"}"#,
            r#""provider":["extension metadata"]"#,
        ] {
            let file = create_test_file(&format!(
                "{{\"type\":\"session\",\"id\":\"metadata\"}}\n\
                 {{\"type\":\"session_info\",\"name\":\"subagent-reviewer-abcd1234-1\",{field}}}\n\
                 {{\"type\":\"message\",\"id\":\"reply\",{field},\"message\":{{\"role\":\"assistant\",\"model\":\"gpt-5\",\"usage\":{{\"input\":10}}}}}}"
            ));
            for client in ["pi", "omp", "senpi", "kimchi", "prime-agent"] {
                let messages = parse_pi_format_file(file.path(), client, "pi");
                assert_eq!(messages.len(), 1, "{client}: {field}");
                assert_eq!(messages[0].tokens.input, 10, "{client}: {field}");
                assert_eq!(
                    messages[0].agent.as_deref(),
                    Some("reviewer"),
                    "{client}: {field}"
                );
            }
        }
    }

    #[test]
    fn standalone_usage_and_assistant_whitespace_providers_use_fallback() {
        let file = create_test_file(
            r#"{"type":"session","id":"providers"}
{"type":"usage","id":"standalone","model":"gpt-5","provider":" \t ","usage":{"input":10}}
{"type":"message","id":"reply","message":{"role":"assistant","model":"gpt-5","provider":" \t ","usage":{"input":20}}}
{"type":"usage","id":"explicit","model":"gpt-5","provider":"custom-gateway","usage":{"input":30}}"#,
        );
        for client in ["pi", "omp", "senpi", "kimchi", "prime-agent"] {
            let messages = parse_pi_format_file(file.path(), client, "pi");
            assert_eq!(messages.len(), 3, "{client}");
            assert_eq!(messages[0].provider_id, "openai", "{client}");
            assert_eq!(messages[1].provider_id, "openai", "{client}");
            assert_eq!(messages[2].provider_id, "custom-gateway", "{client}");
        }
    }

    #[test]
    fn standalone_usage_skips_invalid_records_and_preserves_fallbacks() {
        let file = create_test_file(
            r#"{"type":"session","id":"fallbacks"}
{"type":"usage","model":"gpt-5"}
{"type":"usage","usage":{"input":999}}
{"type":"usage","model":" ","usage":{"input":999}}
{"type":"usage","model":"gpt-5","usage":"invalid"}
{"type":"usage","model":"gpt-5","usage":{"input":"invalid"}}
{"type":"custom","model":"gpt-5","usage":{"input":999}}
{"type":"usage","model":"gpt-5","usage":{"input":-10,"output":5,"cacheRead":-4,"cacheWrite":3,"totalTokens":999}}
{"type":"usage","id":"zero","model":"other-model","provider":"other-provider","usage":{"input":0,"output":0}}"#,
        );
        let messages = parse_pi_file(file.path());
        assert_eq!(messages.len(), 4);
        for message in &messages[..2] {
            assert_eq!(message.model_id, "unknown");
            assert_eq!(message.tokens.input, 999);
        }
        assert_eq!(messages[2].provider_id, "openai");
        assert_eq!(messages[2].tokens.input, 0);
        assert_eq!(messages[2].tokens.cache_read, 0);
        assert_eq!(messages[2].tokens.total(), 8);
        assert_eq!(
            messages[2].timestamp,
            file_modified_timestamp_ms(file.path())
        );
        assert_eq!(messages[3].tokens.total(), 0);
    }

    #[test]
    fn standalone_usage_keeps_kimchi_session_scoped_identity() {
        let file = create_test_file(
            r#"{"type":"session","id":"kimchi-session"}
{"type":"usage","id":"same-id","model":"gpt-5","usage":{"input":10}}
{"type":"message","id":"same-id","message":{"role":"assistant","model":"gpt-5","usage":{"input":10}}}"#,
        );
        let messages = crate::sessions::kimchi::parse_kimchi_file(file.path());
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("kimchi:kimchi-session:usage:same-id")
        );
        assert_eq!(
            messages[1].dedup_key.as_deref(),
            Some("kimchi:kimchi-session:same-id")
        );
    }

    #[test]
    fn standalone_usage_fork_copies_share_keys_without_colliding_with_messages() {
        let record = r#"{"type":"usage","id":"same-id","timestamp":"2026-10-16T12:00:00Z","kind":"cache_warm","provider":"openai","model":"gpt-5","usage":{"input":10,"output":5}}"#;
        let make = |session| {
            create_test_file(&format!(
            "{{\"type\":\"session\",\"id\":\"{session}\"}}\n{record}\n\
             {{\"type\":\"message\",\"id\":\"same-id\",\"timestamp\":\"2026-10-16T12:00:00Z\",\"message\":{{\"role\":\"assistant\",\"provider\":\"openai\",\"model\":\"gpt-5\",\"usage\":{{\"input\":10,\"output\":5}}}}}}"
        ))
        };
        let parent = make("parent");
        let fork = make("fork");
        for client in ["pi", "omp", "senpi", "prime-agent"] {
            let first = parse_pi_format_file(parent.path(), client, "pi");
            let copied = parse_pi_format_file(fork.path(), client, "pi");
            assert_eq!(first.len(), 2);
            assert_eq!(first[0].dedup_key, copied[0].dedup_key);
            assert_ne!(first[0].dedup_key, first[1].dedup_key);
            assert!(first[0]
                .dedup_key
                .as_deref()
                .unwrap()
                .starts_with(&format!("{client}:usage:")));
        }
    }

    #[test]
    fn assistant_duration_populates_performance_across_pi_format_clients() {
        let file = create_test_file(
            r#"{"type":"session","id":"session-timed","cwd":"/tmp/project"}
{"type":"message","id":"assistant-1","timestamp":"2026-10-10T06:44:18.931Z","message":{"role":"assistant","model":"gpt-5","provider":"openai","timestamp":1791614655931,"durationMs":2500,"usage":{"input":100,"output":50,"cacheRead":200,"reasoning":20}}}"#,
        );
        let request_start = 1791614655931;
        for messages in [
            parse_pi_file(file.path()),
            crate::sessions::senpi::parse_senpi_file(file.path()),
            crate::sessions::kimchi::parse_kimchi_file(file.path()),
            crate::sessions::omp::parse_omp_file(file.path()),
            parse_prime_test_file(file.path()),
        ] {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].duration_ms, Some(2500));
            assert_eq!(messages[0].timestamp, request_start);
            assert_eq!(messages[0].tokens.total(), 350);
            assert_eq!(messages[0].tokens.reasoning, 0);
            let entries =
                crate::aggregate_model_usage_entries(messages, &crate::GroupBy::ClientModel);
            assert_eq!(entries[0].performance.total_duration_ms, 2500);
            assert_eq!(entries[0].performance.timed_tokens, 350);
            assert_eq!(entries[0].performance.sample_count, 1);
        }
    }

    #[test]
    fn absent_or_invalid_duration_keeps_usage_and_completion_timestamp() {
        for field in [
            "",
            r#", "durationMs":null"#,
            r#", "durationMs":0"#,
            r#", "durationMs":-1"#,
            r#", "durationMs":"2500""#,
            r#", "durationMs":true"#,
            r#", "durationMs":{}"#,
            r#", "durationMs":2.5"#,
            r#", "durationMs":9223372036854775808"#,
        ] {
            let file = create_test_file(&format!(
                "{{\"type\":\"session\",\"id\":\"session-untimed\"}}\n\
                 {{\"type\":\"message\",\"timestamp\":\"2026-10-10T06:44:18.431Z\",\"message\":\
                 {{\"role\":\"assistant\",\"model\":\"gpt-5\",\"timestamp\":1791614655931,\"usage\":{{\"input\":100,\"output\":50}}{field}}}}}"
            ));
            let messages = parse_pi_file(file.path());
            assert_eq!(messages.len(), 1, "{field}");
            assert_eq!(messages[0].duration_ms, None, "{field}");
            assert_eq!(messages[0].tokens.total(), 150, "{field}");
            assert_eq!(messages[0].timestamp, 1791614658431, "{field}");
        }
    }

    #[test]
    fn invalid_message_start_falls_back_to_completion_minus_duration() {
        let completion = chrono::DateTime::parse_from_rfc3339("2026-10-10T06:44:18.431Z")
            .unwrap()
            .timestamp_millis();
        for timestamp in [
            "null",
            "0",
            "-1",
            r#""1791614655931""#,
            "true",
            "{}",
            "2.5",
            "9223372036854775808",
        ] {
            let file = create_test_file(&format!(
                "{{\"type\":\"session\",\"id\":\"session-timed\"}}\n                 {{\"type\":\"message\",\"timestamp\":\"2026-10-10T06:44:18.431Z\",\"message\":                 {{\"role\":\"assistant\",\"model\":\"gpt-5\",\"timestamp\":{timestamp},\"durationMs\":2500,\"usage\":{{\"output\":50}}}}}}"
            ));
            let messages = parse_pi_file(file.path());
            assert_eq!(messages.len(), 1, "{timestamp}");
            assert_eq!(messages[0].timestamp, completion - 2500, "{timestamp}");
            assert_eq!(messages[0].duration_ms, Some(2500), "{timestamp}");
        }
    }

    #[test]
    fn valid_message_start_does_not_require_a_completion_timestamp() {
        let file = create_test_file(
            r#"{"type":"session","id":"session-timed"}
{"type":"message","message":{"role":"assistant","model":"gpt-5","timestamp":1791614655931,"durationMs":2500,"usage":{"output":50}}}"#,
        );

        let messages = parse_pi_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].timestamp, 1791614655931);
        assert_eq!(messages[0].duration_ms, Some(2500));
    }

    #[test]
    fn duration_preserves_fallback_timestamp_and_cross_session_dedup_key() {
        let record = r#"{"type":"message","id":"assistant-1","timestamp":"2026-10-10T06:44:18.431Z","message":{"role":"assistant","model":"gpt-5","durationMs":2500,"usage":{"input":100,"output":50}}}"#;
        let timed = create_test_file(&format!(
            "{{\"type\":\"session\",\"id\":\"session-timed\"}}\n{record}"
        ));
        let untimed = create_test_file(&format!(
            "{{\"type\":\"session\",\"id\":\"session-copy\"}}\n{}",
            record.replace("\"durationMs\":2500,", "")
        ));
        assert_eq!(
            parse_pi_file(timed.path())[0].dedup_key,
            parse_pi_file(untimed.path())[0].dedup_key
        );
        let missing_timestamp = create_test_file(&format!(
            "{{\"type\":\"session\",\"id\":\"session-fallback\"}}\n{}",
            record.replace("\"timestamp\":\"2026-10-10T06:44:18.431Z\",", "")
        ));
        let rows = parse_pi_file(missing_timestamp.path());
        assert_eq!(rows[0].duration_ms, Some(2500));
        assert_eq!(
            rows[0].timestamp,
            file_modified_timestamp_ms(missing_timestamp.path())
        );
    }

    #[test]
    fn prime_rejects_replacement_mangled_usage_keys() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"{\"type\":\"session\",\"id\":\"root\",\"cwd\":\"/tmp/project\"}\n")
            .unwrap();
        file.write_all(
            b"{\"type\":\"message\",\"id\":\"damaged-byte\",\"timestamp\":\"2026-08-08T00:00:01Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"in\xffput\":100,\"output\":5}}}\n",
        )
        .unwrap();
        file.write_all(
            "{\"type\":\"message\",\"id\":\"damaged-unicode\",\"timestamp\":\"2026-08-08T00:00:02Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"out�put\":7,\"input\":10}}}\n"
                .as_bytes(),
        )
        .unwrap();
        file.write_all(
            b"{\"type\":\"message\",\"id\":\"clean\",\"timestamp\":\"2026-08-08T00:00:03Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":20,\"output\":8}}}\n",
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_prime_test_file(file.path());

        assert_eq!(messages.len(), 1);
        assert!(messages[0]
            .dedup_key
            .as_deref()
            .is_some_and(|key| key.starts_with("prime-agent:message:clean:")));
        assert_eq!(messages[0].tokens.total(), 28);
    }

    #[test]
    fn damaged_usage_extension_keys_do_not_hide_known_counters() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"{\"type\":\"session\",\"id\":\"root\"}\n")
            .unwrap();
        file.write_all(
            b"{\"type\":\"message\",\"id\":\"kept\",\"timestamp\":\"2026-08-08T00:00:01Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":20,\"output\":8,\"future-\xff-field\":999}}}\n",
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_prime_test_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.total(), 28);
    }

    #[test]
    fn standard_pi_skips_invalid_utf8_line_and_reads_later_records() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"{\"type\":\"session\",\"id\":\"root\"}\n")
            .unwrap();
        file.write_all(b"invalid \xff record\n").unwrap();
        file.write_all(
            b"{\"type\":\"message\",\"id\":\"later\",\"timestamp\":\"2026-08-08T00:00:01Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":20,\"output\":8}}}\n",
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_pi_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.total(), 28);
    }

    #[test]
    fn utf8_bom_before_the_header_keeps_the_transcript() {
        // A BOM decodes cleanly, so `str::trim` leaves U+FEFF glued to the
        // front of the header (it is not White_Space) and the header fails to
        // parse. That failure drops the entire transcript, not one record, so
        // every Pi-format client has to strip it the way the lossy reader
        // already does for Prime Agent.
        let file = create_test_file(concat!(
            "\u{feff}",
            r#"{"type":"session","id":"session-with-bom","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project"}"#,
            "\n",
            r#"{"type":"message","id":"assistant-1","timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","usage":{"input":20,"output":8}}}"#,
            "\n",
        ));

        for messages in [
            parse_pi_file(file.path()),
            crate::sessions::senpi::parse_senpi_file(file.path()),
            crate::sessions::kimchi::parse_kimchi_file(file.path()),
            parse_prime_test_file(file.path()),
        ] {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].session_id, "session-with-bom");
            assert_eq!(messages[0].tokens.total(), 28);
        }
    }

    #[test]
    fn a_marker_after_the_first_record_still_costs_only_its_own_record() {
        // The strip is scoped to the file's first line, where a byte-order mark
        // can actually appear. A U+FEFF anywhere else stays an ordinary
        // malformed record, which the reader already loses alone.
        let file = create_test_file(concat!(
            r#"{"type":"session","id":"session-clean","timestamp":"2026-08-08T00:00:00.000Z","cwd":"/tmp/project"}"#,
            "\n\u{feff}",
            r#"{"type":"message","id":"assistant-1","timestamp":"2026-08-08T00:00:01.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","usage":{"input":1,"output":1}}}"#,
            "\n",
            r#"{"type":"message","id":"assistant-2","timestamp":"2026-08-08T00:00:02.000Z","message":{"role":"assistant","provider":"anthropic","model":"claude-opus-5","usage":{"input":20,"output":8}}}"#,
            "\n",
        ));

        let messages = parse_pi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "session-clean");
        assert_eq!(messages[0].tokens.total(), 28);
    }

    #[test]
    fn prime_rejects_only_matching_critical_child_timestamps() {
        for (depth, expected_ids) in [(1, vec!["clean"]), (0, vec!["damaged", "clean"])] {
            let mut file = NamedTempFile::new().unwrap();
            writeln!(
                file,
                "{{\"type\":\"session\",\"id\":\"root\",\"parentSession\":\"/tmp/parent.jsonl\",\"rlmDepth\":{depth}}}"
            )
            .unwrap();
            file.write_all(
                b"{\"type\":\"message\",\"id\":\"damaged\",\"timestamp\":\"2026-08-08T00:00:0\xffZ\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":10}}}\n",
            ).unwrap();
            file.write_all(b"{\"type\":\"message\",\"id\":\"clean\",\"timestamp\":\"2026-08-08T00:00:02Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":20}}}\n").unwrap();
            file.flush().unwrap();

            let messages = parse_prime_test_file(file.path());
            let ids = messages
                .iter()
                .map(|message| {
                    message
                        .dedup_key
                        .as_deref()
                        .unwrap()
                        .split(':')
                        .nth(2)
                        .unwrap()
                })
                .collect::<Vec<_>>();
            assert_eq!(ids, expected_ids, "rlmDepth={depth}");
        }
    }

    #[test]
    fn prime_rejects_child_message_with_damaged_timestamp_key() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"{\"type\":\"session\",\"id\":\"child\",\"parentSession\":\"/tmp/parent.jsonl\",\"rlmDepth\":1}\n")
            .unwrap();
        file.write_all(b"{\"type\":\"message\",\"id\":\"damaged\",\"time\xffstamp\":\"2026-08-08T00:00:01Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":10}}}\n").unwrap();
        file.write_all(b"{\"type\":\"message\",\"id\":\"clean\",\"timestamp\":\"2026-08-08T00:00:02Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":20}}}\n").unwrap();
        file.flush().unwrap();

        let messages = parse_prime_test_file(file.path());
        assert_eq!(messages.len(), 1);
        assert!(messages[0]
            .dedup_key
            .as_deref()
            .unwrap()
            .contains(":clean:"));
    }

    #[test]
    fn damaged_prime_session_placeholders_are_unique_by_source_path() {
        let temp = tempfile::tempdir().unwrap();
        let first_dir = temp.path().join("first");
        let second_dir = temp.path().join("second");
        std::fs::create_dir_all(&first_dir).unwrap();
        std::fs::create_dir_all(&second_dir).unwrap();
        let first = first_dir.join("session.jsonl");
        let second = second_dir.join("session.jsonl");
        let contents = b"{\"type\":\"session\",\"id\":\"root-\xff\",\"cwd\":\"/tmp/project\"}\n{\"type\":\"message\",\"id\":\"clean\",\"timestamp\":\"2026-08-08T00:00:03Z\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":20,\"output\":8}}}\n";
        std::fs::write(&first, contents).unwrap();
        std::fs::write(&second, contents).unwrap();

        let first_messages = parse_prime_test_file(&first);
        let second_messages = parse_prime_test_file(&second);

        assert_eq!(first_messages.len(), 1);
        assert_eq!(second_messages.len(), 1);
        assert!(first_messages[0].session_id.starts_with("unknown:path:"));
        assert_ne!(first_messages[0].session_id, second_messages[0].session_id);
    }

    #[test]
    fn test_parse_pi_jsonl_valid_assistant_message() {
        // given
        let content = r#"{"type":"session","id":"pi_ses_001","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","id":"msg_001","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"claude-3-5-sonnet","provider":"anthropic","usage":{"input":100,"output":50,"cacheRead":10,"cacheWrite":5,"totalTokens":165}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "pi");
        assert_eq!(messages[0].session_id, "pi_ses_001");
        assert_eq!(messages[0].model_id, "claude-3-5-sonnet");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.cache_read, 10);
        assert_eq!(messages[0].tokens.cache_write, 5);
        assert_eq!(messages[0].workspace_key, Some("/tmp".to_string()));
        assert_eq!(messages[0].workspace_label, Some("tmp".to_string()));
    }

    #[test]
    fn test_parse_pi_infers_provider_from_model_when_absent() {
        // given: no "provider" key at all — a missing provider must be
        // inferred from the model name (gpt-5 -> openai), not hardcoded
        // to "pi".
        let content = r#"{"type":"session","id":"pi_ses_005","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","id":"msg_001","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"gpt-5","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5");
        assert_eq!(messages[0].provider_id, "openai");
    }

    #[test]
    fn test_parse_pi_infers_provider_from_model_when_blank() {
        // given: "provider" present but blank — same inference path as
        // fully absent.
        let content = r#"{"type":"session","id":"pi_ses_006","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","id":"msg_001","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"gpt-5","provider":"","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "openai");
    }

    #[test]
    fn test_parse_pi_falls_back_to_pi_when_provider_unrecoverable() {
        // given: no provider and a model name inference can't identify —
        // falls back to "pi" rather than dropping the message.
        let content = r#"{"type":"session","id":"pi_ses_007","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","id":"msg_001","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"totally-unrecognized-model-xyz","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "pi");
    }

    #[test]
    fn test_parse_pi_subagent_session_name_as_agent() {
        let content = r#"{"type":"session","id":"pi_subagent_001","timestamp":"2026-07-10T00:00:00.000Z","cwd":"/tmp"}
{"type":"session_info","id":"info_001","parentId":null,"timestamp":"2026-07-10T00:00:00.100Z","name":"subagent-go-reviewer-e2e7405c-cb84-4f0a-a6da-9d987494d130-1"}
{"type":"message","id":"msg_001","parentId":"info_001","timestamp":"2026-07-10T00:00:01.000Z","message":{"role":"assistant","model":"gpt-5","provider":"openai","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        let messages = parse_pi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].agent.as_deref(), Some("go-reviewer"));
        assert_eq!(
            pi_subagent_name("subagent-context-builder-208242ce-1").as_deref(),
            Some("context-builder")
        );
        assert_eq!(pi_subagent_name("Refactor auth module"), None);
    }

    #[test]
    fn test_parse_pi_counts_billed_operations_with_official_attribution() {
        // given: Pi bills work outside assistant replies on their own records —
        // `/compact` writes `compaction`, branch summarization `branch_summary`,
        // standalone accounting (cache warming) `usage`. These legacy-shaped
        // records name no model, and a summary without usage bills nothing.
        let content = r#"{"type":"session","id":"pi_ses_ops","timestamp":"2026-10-12T02:27:31.079Z","cwd":"/tmp"}
{"type":"message","id":"m1","timestamp":"2026-10-12T02:28:43.068Z","message":{"role":"assistant","model":"deepseek-flash","provider":"deepseek","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}
{"type":"compaction","id":"c1","timestamp":"2026-10-12T03:38:44.507Z","summary":"summarized","firstKeptEntryId":"m1","tokensBefore":150,"usage":{"input":184302,"output":3627,"cacheRead":0,"cacheWrite":0,"reasoning":108,"totalTokens":187929,"cost":{"total":0.059643}}}
{"type":"branch_summary","id":"b1","timestamp":"2026-10-12T04:00:00.000Z","summary":"branch","usage":{"input":29964,"output":882,"cacheRead":0,"cacheWrite":0,"totalTokens":30846}}
{"type":"usage","id":"w1","timestamp":"2026-10-12T04:30:00.000Z","kind":"cache_warm","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":512}}
{"type":"compaction","id":"c2","timestamp":"2026-10-12T05:00:00.000Z","summary":"no usage here"}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[1].model_id, PI_TOOLS_SUMMARIES_MODEL);
        assert_eq!(messages[1].provider_id, "pi");
        assert_eq!(messages[1].tokens.input, 184302);
        assert_eq!(messages[1].tokens.output, 3627);
        // reasoning is a subset of output and is never added to the buckets.
        assert_eq!(messages[1].tokens.reasoning, 0);
        assert_eq!(messages[2].tokens.input, 29964);
        assert_eq!(messages[2].model_id, PI_TOOLS_SUMMARIES_MODEL);
        assert_eq!(messages[3].tokens.cache_write, 512);
        // The old model-less UsageEntry shape keeps its compatibility fallback.
        assert_eq!(messages[3].model_id, "deepseek-flash");
        assert_eq!(messages[3].provider_id, "deepseek");
        // The usage-free summary is not emitted at all, so nothing else follows.
        assert_eq!(messages[3].session_id, "pi_ses_ops");
    }

    #[test]
    fn test_parse_pi_uses_standalone_usage_model_provider_and_cost() {
        // Pi's current UsageEntry schema records the concrete billed model and
        // provider at the root. Cache warming may intentionally use a model
        // other than the active conversation model.
        let content = r#"{"type":"session","id":"pi_ses_usage","timestamp":"2026-10-12T02:27:31.079Z","cwd":"/tmp"}
{"type":"message","id":"m1","timestamp":"2026-10-12T02:28:43.068Z","message":{"role":"assistant","model":"claude-sonnet-4-5","provider":"anthropic","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}
{"type":"usage","id":"w1","timestamp":"2026-10-12T04:30:00.000Z","kind":"cache_warm","provider":"openrouter","model":"vendor/custom-warm-model","usage":{"input":7,"output":0,"cacheRead":900,"cacheWrite":100,"cacheWrite1h":80,"totalTokens":1007,"cost":{"input":0.01,"output":0,"cacheRead":0.02,"cacheWrite":0,"total":0.03}}}"#;
        let file = create_test_file(content);

        let messages = parse_pi_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].model_id, "vendor/custom-warm-model");
        assert_eq!(messages[1].provider_id, "openrouter");
        assert_eq!(messages[1].tokens.input, 7);
        assert_eq!(messages[1].tokens.cache_read, 900);
        assert_eq!(messages[1].tokens.cache_write, 100);
        assert_eq!(messages[1].tokens.cache_write_1h, 80);
        assert_eq!(messages[1].cost, 0.03);
        assert_eq!(
            messages[1].cost_source,
            super::super::CostSource::ProviderReported
        );
    }

    #[test]
    fn test_parse_pi_operation_blank_attribution_falls_back() {
        // A whitespace-only model/provider is not an identifier, so the row
        // must reach the normal inference and fallback paths.
        let content = r#"{"type":"session","id":"pi_ses_blank","timestamp":"2026-10-12T02:27:31.079Z","cwd":"/tmp"}
{"type":"usage","id":"u1","timestamp":"2026-10-12T03:00:00.000Z","kind":"cache_warm","provider":"  ","model":"  gpt-5  ","usage":{"input":1,"output":1}}
{"type":"usage","id":"u2","timestamp":"2026-10-12T03:05:00.000Z","kind":"cache_warm","provider":"   ","model":"   ","usage":{"input":2,"output":2}}"#;
        let file = create_test_file(content);

        let messages = parse_pi_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].model_id, "gpt-5");
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[1].model_id, "unknown");
        assert_eq!(messages[1].provider_id, "pi");
    }

    #[test]
    fn test_parse_pi_preserves_operation_cost_and_1h_cache_split() {
        let content = r#"{"type":"session","id":"pi_ses_cost_split","timestamp":"2026-10-12T02:27:31.079Z","cwd":"/tmp"}
{"type":"compaction","id":"c1","timestamp":"2026-10-12T03:38:44.507Z","summary":"summarized","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":100,"cacheWrite1h":80,"totalTokens":115,"cost":{"total":0.059643}}}"#;
        let file = create_test_file(content);

        let messages = parse_pi_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.cache_write, 100);
        assert_eq!(messages[0].tokens.cache_write_1h, 80);
        assert_eq!(messages[0].cost, 0.059643);
        assert_eq!(
            messages[0].cost_source,
            super::super::CostSource::ProviderReported
        );
    }

    #[test]
    fn test_parse_pi_model_change_attributes_legacy_model_less_usage() {
        // A model change updates Pi's selected model immediately. This keeps
        // compatibility for old model-less UsageEntry records before the next
        // assistant reply, while summaries use their separate official bucket.
        let content = r#"{"type":"session","id":"pi_ses_model_change","timestamp":"2026-10-12T02:27:31.079Z","cwd":"/tmp"}
{"type":"message","id":"m1","timestamp":"2026-10-12T02:28:43.068Z","message":{"role":"assistant","model":"old-model","provider":"old-provider","usage":{"input":100,"output":50}}}
{"type":"model_change","id":"mc1","timestamp":"2026-10-12T03:00:00.000Z","provider":"new-provider","modelId":"new-model"}
{"type":"usage","id":"u1","timestamp":"2026-10-12T03:01:00.000Z","kind":"legacy","usage":{"input":10,"output":5,"cost":{"total":0.25}}}"#;
        let file = create_test_file(content);

        let messages = parse_pi_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].model_id, "new-model");
        assert_eq!(messages[1].provider_id, "new-provider");
        assert_eq!(messages[1].cost, 0.25);
        assert_eq!(
            messages[1].cost_source,
            super::super::CostSource::ProviderReported
        );
    }

    #[test]
    fn test_parse_pi_keeps_summary_before_any_reply_in_tools_bucket() {
        // given: a transcript whose first billed record is a summary operation.
        let content = r#"{"type":"session","id":"pi_ses_ops2","timestamp":"2026-10-12T02:27:31.079Z","cwd":"/tmp"}
{"type":"compaction","id":"c1","timestamp":"2026-10-12T03:38:44.507Z","summary":"early","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, PI_TOOLS_SUMMARIES_MODEL);
        assert_eq!(messages[0].provider_id, "pi");
        assert_eq!(messages[0].tokens.input, 10);
    }

    #[test]
    fn test_parse_pi_counts_a_tool_result_that_bills_usage() {
        // given: a tool result reporting the usage of the call behind it, after
        // a reply that named the model.
        let content = r#"{"type":"session","id":"pi_ses_tool","timestamp":"2026-10-12T02:27:31.079Z","cwd":"/tmp"}
{"type":"message","id":"m1","timestamp":"2026-10-12T02:28:43.068Z","message":{"role":"assistant","model":"gpt-5","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}
{"type":"message","id":"t1","parentId":"m1","timestamp":"2026-10-12T02:28:44.000Z","message":{"role":"toolResult","toolCallId":"call_1","content":"output","usage":{"input":7,"output":3,"cacheRead":0,"cacheWrite":0,"totalTokens":10}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].tokens.input, 7);
        assert_eq!(messages[1].model_id, PI_TOOLS_SUMMARIES_MODEL);
        assert_eq!(messages[1].provider_id, "pi");
    }

    #[test]
    fn test_parse_prime_agent_leaves_billed_operations_out() {
        // given: Prime Agent rides on the shared parser but reconciles emitted
        // messages against its own `child_usage_attributed` records, so its lane
        // keeps the operation records out.
        let content = r#"{"type":"session","id":"prime_ses_ops","timestamp":"2026-10-12T02:27:31.079Z","cwd":"/tmp"}
{"type":"message","id":"m1","timestamp":"2026-10-12T02:28:43.068Z","message":{"role":"assistant","model":"gpt-5","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}
{"type":"compaction","id":"c1","timestamp":"2026-10-12T03:38:44.507Z","summary":"summarized","usage":{"input":184302,"output":3627,"cacheRead":0,"cacheWrite":0,"totalTokens":187929}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_prime_test_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 10);
    }

    #[test]
    fn test_parse_pi_skips_non_assistant_messages() {
        // given
        let content = r#"{"type":"session","id":"pi_ses_002","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"user","model":"claude-3-5-sonnet","provider":"anthropic","usage":{"input":100,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":150}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_pi_skips_missing_usage() {
        // given
        let content = r#"{"type":"session","id":"pi_ses_003","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"claude-3-5-sonnet","provider":"anthropic"}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert!(messages.is_empty());
    }

    #[test]
    fn test_parse_pi_skips_malformed_json_lines() {
        // given
        let content = r#"{"type":"session","id":"pi_ses_004","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}
not valid json
{"type":"message","timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"assistant","model":"gpt-4o-mini","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-4o-mini");
        assert_eq!(messages[0].provider_id, "openai");
    }

    #[test]
    fn test_parse_pi_skips_leading_title_record() {
        // given: current OMP builds write a `title` metadata record before
        // `session` (tokscale#802) — the parser must skip it, not discard
        // the whole file.
        let content = r#"{"type":"title","v":1,"title":"Comment on GitHub issue","source":"auto","updatedAt":"2026-07-02T18:08:49.723Z"}
{"type":"session","id":"pi_ses_005","timestamp":"2026-07-02T18:07:14.690Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-02T18:08:53.229Z","message":{"role":"assistant","model":"claude-sonnet-5","provider":"anthropic","usage":{"input":2,"output":180,"cacheRead":0,"cacheWrite":70844,"totalTokens":71026}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "pi_ses_005");
        assert_eq!(messages[0].model_id, "claude-sonnet-5");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].tokens.input, 2);
        assert_eq!(messages[0].tokens.output, 180);
        assert_eq!(messages[0].tokens.cache_write, 70844);
        assert_eq!(
            messages[0].session_title.as_deref(),
            Some("Comment on GitHub issue")
        );
    }

    #[test]
    fn test_parse_pi_uses_last_leading_title_record() {
        // given: multiple pre-session title records (a title rewritten by a
        // later auto-rename) — the last one wins.
        let content = r#"{"type":"title","v":1,"title":"first"}
{"type":"title","v":1,"title":"renamed"}
{"type":"session","id":"pi_ses_t1","timestamp":"2026-07-02T18:07:14.690Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-02T18:08:53.229Z","message":{"role":"assistant","model":"gpt-4o-mini","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_title.as_deref(), Some("renamed"));
    }

    #[test]
    fn test_parse_pi_header_title_is_authoritative() {
        // given: a v3 OMP header carries the current title — it wins over
        // both the leading title record and historical title_change entries.
        let content = r#"{"type":"title","v":1,"title":"Stale record title","source":"auto","updatedAt":"2026-07-28T18:24:59.651Z"}
{"type":"session","version":3,"id":"pi_ses_t2","timestamp":"2026-07-28T18:22:13.943Z","cwd":"/tmp","title":"Current header title","titleSource":"auto"}
{"type":"title_change","id":"3bd94818","parentId":null,"timestamp":"2026-07-28T18:24:59.651Z","title":"Historical change","source":"auto"}
{"type":"message","timestamp":"2026-07-28T18:25:03.877Z","message":{"role":"assistant","model":"gpt-5","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].session_title.as_deref(),
            Some("Current header title")
        );
    }

    #[test]
    fn test_parse_pi_title_change_updates_rolling_title() {
        // given: no header title — title_change entries refine the title as
        // the session runs; messages after the change carry the new title.
        let content = r#"{"type":"session","id":"pi_ses_t3","timestamp":"2026-07-28T18:22:13.943Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-28T18:23:00.000Z","message":{"role":"assistant","model":"gpt-5","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}
{"type":"title_change","id":"3bd94818","parentId":null,"timestamp":"2026-07-28T18:24:59.651Z","title":"Refined title","source":"auto"}
{"type":"message","timestamp":"2026-07-28T18:25:03.877Z","message":{"role":"assistant","model":"gpt-5","provider":"openai","usage":{"input":20,"output":6,"cacheRead":0,"cacheWrite":0,"totalTokens":26}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].session_title, None);
        assert_eq!(messages[1].session_title.as_deref(), Some("Refined title"));
    }

    #[test]
    fn test_parse_pi_trailing_title_change_reaches_the_last_message() {
        // given: no header title, and the session is renamed after its last
        // message — the trailing change has no later message to retitle, so
        // the latest message must carry it for the Sessions tab to show it.
        let content = r#"{"type":"session","id":"pi_ses_t5","timestamp":"2026-07-28T18:22:13.943Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-28T18:23:00.000Z","message":{"role":"assistant","model":"gpt-5","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}
{"type":"title_change","id":"3bd94818","parentId":null,"timestamp":"2026-07-28T18:24:59.651Z","title":"Renamed after the fact","source":"auto"}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].session_title.as_deref(),
            Some("Renamed after the fact")
        );
    }

    #[test]
    fn test_parse_pi_ignores_blank_titles() {
        // given: a blank title never shadows a real one from another record.
        let content = r#"{"type":"title","v":1,"title":"Real title"}
{"type":"title","v":1,"title":"   "}
{"type":"session","id":"pi_ses_t4","timestamp":"2026-07-02T18:07:14.690Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-02T18:08:53.229Z","message":{"role":"assistant","model":"gpt-4o-mini","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_title.as_deref(), Some("Real title"));
    }

    #[test]
    fn test_parse_pi_nested_subagent_file_derives_parent_session() {
        // given: OMP nests subagent transcripts inside a directory named
        // after the parent session's file stem — the parent id must be
        // recovered so the Sessions tab can roll the child into its parent.
        let dir = tempfile::tempdir().unwrap();
        let nested = dir
            .path()
            .join("2026-07-28T20-45-44-298Z_019faa79-e9ea-7000-a4cd-be6083d214ab");
        std::fs::create_dir_all(&nested).unwrap();
        let path = nested.join("ScheduleImpl.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session","id":"child-019faaa2","timestamp":"2026-07-28T21:30:19.369Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-28T21:30:20.000Z","message":{"role":"assistant","model":"gpt-5","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#,
        )
        .unwrap();

        // when
        let messages = parse_pi_format_file(&path, "omp", "omp");

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "child-019faaa2");
        assert_eq!(
            messages[0].parent_session_id.as_deref(),
            Some("019faa79-e9ea-7000-a4cd-be6083d214ab")
        );
    }

    #[test]
    fn test_parse_pi_flat_clients_do_not_derive_parent_from_path() {
        // given: the same nested shape under a client whose transcripts are
        // flat — the layout belongs to OMP, so no parent link may be invented
        // for a directory that merely looks like a parent stem.
        let dir = tempfile::tempdir().unwrap();
        let nested = dir
            .path()
            .join("2026-07-28T20-45-44-298Z_019faa79-e9ea-7000-a4cd-be6083d214ab");
        std::fs::create_dir_all(&nested).unwrap();
        let path = nested.join("Session.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session","id":"pi_ses_flat","timestamp":"2026-07-28T21:30:19.369Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-28T21:30:20.000Z","message":{"role":"assistant","model":"gpt-5","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#,
        )
        .unwrap();

        // when
        let messages = parse_pi_file(&path);

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].parent_session_id, None);
    }

    #[test]
    fn test_parent_session_id_from_path_edge_cases() {
        use std::path::Path;
        // Workspace dirs and non-uuid suffixes never match.
        assert_eq!(
            parent_session_id_from_path(Path::new(
                "/home/u/.omp/agent/sessions/-go-src-proj--/2026-07-28T20-45-44-298Z_019faa79-e9ea-7000-a4cd-be6083d214ab.jsonl"
            )),
            None
        );
        assert_eq!(
            parent_session_id_from_path(Path::new(
                "/sessions/not-a-timestamp_019faa79-e9ea-7000-a4cd-be6083d214ab/Child.jsonl"
            )),
            None
        );
        assert_eq!(
            parent_session_id_from_path(Path::new(
                "/sessions/2026-07-28T20-45-44-298Z_not-a-uuid/Child.jsonl"
            )),
            None
        );
        // A top-level file directly in the workspace dir has no parent.
        let flat = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(parent_session_id_from_path(flat.path()), None);
    }

    #[test]
    fn test_parse_pi_rejects_unknown_leading_record_type() {
        // given: an unrecognized type before `session` is still treated as
        // a malformed file rather than silently scanned through.
        let content = r#"{"type":"totally_unknown_thing","foo":"bar"}
{"type":"session","id":"pi_ses_007","timestamp":"2026-07-02T18:07:14.690Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-02T18:08:53.229Z","message":{"role":"assistant","model":"gpt-4o-mini","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert!(messages.is_empty());
    }

    /// The record shape from the token-monitor synthetic repro
    /// (Javis603/token-monitor#627, tokscale#1306): a Pi branch copies the
    /// parent's assistant entries into a new session file verbatim —
    /// same `message.responseId`, same entry id, same usage — and only the
    /// session header differs.
    fn fork_fixture(session_id: &str, extra_message: Option<&str>) -> String {
        let copied = format!(
            r#"{{"type":"message","id":"entry-fork-copy","parentId":"{session_id}","timestamp":"2026-09-06T12:00:00.000Z","message":{{"role":"assistant","provider":"openai-codex","model":"gpt-6-astra","responseId":"resp-demo-fork","usage":{{"input":100,"output":20,"cacheRead":300,"cacheWrite":0,"totalTokens":420}}}}}}"#
        );
        let mut lines = vec![
            format!(
                r#"{{"type":"session","id":"{session_id}","timestamp":"2026-09-06T12:00:00.000Z","cwd":"/tmp/demo"}}"#
            ),
            copied,
        ];
        if let Some(message) = extra_message {
            lines.push(message.to_string());
        }
        lines.join("\n") + "\n"
    }

    #[test]
    fn fork_copies_across_session_files_share_a_response_level_key() {
        let parent = create_test_file(&fork_fixture("session-demo-1", None));
        let child = create_test_file(&fork_fixture("session-demo-2", None));

        let parent_messages = parse_pi_file(parent.path());
        let child_messages = parse_pi_file(child.path());

        assert_eq!(parent_messages.len(), 1);
        assert_eq!(child_messages.len(), 1);
        // Session-independent identity: the key must not embed the session id.
        let parent_key = parent_messages[0].dedup_key.as_deref().unwrap();
        let child_key = child_messages[0].dedup_key.as_deref().unwrap();
        assert_eq!(parent_key, child_key);
        assert_eq!(parent_key, "pi:response:resp-demo-fork");
    }

    #[test]
    fn fork_copy_plus_new_turns_only_the_prefix_collapses() {
        // given: the child file copies the parent's record and then continues
        // with a genuinely new response. Only the copied prefix must share a
        // key; the new turn must survive.
        let new_turn = r#"{"type":"message","id":"entry-child-new","parentId":"session-demo-2","timestamp":"2026-09-06T12:01:00.000Z","message":{"role":"assistant","provider":"openai-codex","model":"gpt-6-astra","responseId":"resp-demo-child-new","usage":{"input":7,"output":3,"cacheRead":0,"cacheWrite":0,"totalTokens":10}}}"#;
        let parent = create_test_file(&fork_fixture("session-demo-1", None));
        let child = create_test_file(&fork_fixture("session-demo-2", Some(new_turn)));

        let parent_messages = parse_pi_file(parent.path());
        let child_messages = parse_pi_file(child.path());

        assert_eq!(parent_messages.len(), 1);
        assert_eq!(child_messages.len(), 2);
        let child_keys: Vec<&str> = child_messages
            .iter()
            .map(|message| message.dedup_key.as_deref().unwrap())
            .collect();
        assert_eq!(child_keys[0], "pi:response:resp-demo-fork");
        assert_eq!(child_keys[1], "pi:response:resp-demo-child-new");
        assert!(child_keys[1] != child_keys[0]);
    }

    #[test]
    fn different_response_ids_stay_separate_even_with_identical_content() {
        // given: two genuinely separate calls with the same usage but
        // different response ids must not merge (regenerate/retry case).
        let file = create_test_file(concat!(
            r#"{"type":"session","id":"session-demo-1","timestamp":"2026-09-06T12:00:00.000Z","cwd":"/tmp/demo"}"#,
            "\n",
            r#"{"type":"message","id":"entry-a","parentId":"session-demo-1","timestamp":"2026-09-06T12:00:00.000Z","message":{"role":"assistant","provider":"openai-codex","model":"gpt-6-astra","responseId":"resp-A","usage":{"input":100,"output":20,"cacheRead":300,"cacheWrite":0,"totalTokens":420}}}"#,
            "\n",
            r#"{"type":"message","id":"entry-b","parentId":"session-demo-1","timestamp":"2026-09-06T12:00:01.000Z","message":{"role":"assistant","provider":"openai-codex","model":"gpt-6-astra","responseId":"resp-B","usage":{"input":100,"output":20,"cacheRead":300,"cacheWrite":0,"totalTokens":420}}}"#,
            "\n",
        ));

        let messages = parse_pi_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_ne!(messages[0].dedup_key, messages[1].dedup_key);
    }

    #[test]
    fn missing_response_id_falls_back_to_entry_identity_across_files() {
        // given: no responseId, but the fork preserves the entry id and the
        // immutable event fields — those form the fallback key, and it must
        // still be session-independent.
        let record = r#"{"type":"message","id":"entry-no-resp","parentId":"SESSION","timestamp":"2026-09-06T12:00:00.000Z","message":{"role":"assistant","provider":"openai-codex","model":"gpt-6-astra","usage":{"input":100,"output":20,"cacheRead":300,"cacheWrite":0,"totalTokens":420}}}"#;
        let make = |session: &str| {
            format!(
                r#"{{"type":"session","id":"{session}","timestamp":"2026-09-06T12:00:00.000Z","cwd":"/tmp/demo"}}"#
            ) + "\n"
                + &record.replace("SESSION", session)
                + "\n"
        };
        let parent = create_test_file(&make("session-a"));
        let child = create_test_file(&make("session-b"));

        let parent_messages = parse_pi_file(parent.path());
        let child_messages = parse_pi_file(child.path());

        assert_eq!(parent_messages.len(), 1);
        assert_eq!(child_messages.len(), 1);
        let parent_key = parent_messages[0].dedup_key.as_deref().unwrap();
        let child_key = child_messages[0].dedup_key.as_deref().unwrap();
        assert_eq!(parent_key, child_key);
        assert!(parent_key.starts_with("pi:message:entry-no-resp:"));
    }

    #[test]
    fn test_parent_path_inference_is_omp_only_across_shared_clients() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir
            .path()
            .join("2026-07-28T20-45-44-298Z_019faa79-e9ea-7000-a4cd-be6083d214ab");
        std::fs::create_dir_all(&nested).unwrap();
        let path = nested.join("Child.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session","id":"child","title":"Shared title"}
{"type":"message","message":{"role":"assistant","model":"gpt-5","usage":{"input":5,"output":2}}}"#,
        )
        .unwrap();
        for client in [
            ClientId::Pi,
            ClientId::Senpi,
            ClientId::Kimchi,
            ClientId::PrimeAgent,
            ClientId::Omp,
        ] {
            let client = client.as_str();
            let rows = parse_pi_format_file(&path, client, "openai");
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows[0].parent_session_id.is_some(),
                client == "omp",
                "{client}"
            );
            assert_eq!(rows[0].session_title.as_deref(), Some("Shared title"));
        }
    }
}
