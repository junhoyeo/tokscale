//! Pi (badlogic/pi-mono) session parser
//!
//! Parses JSONL files from `~/.pi/agent/sessions/<encoded-cwd>/*.jsonl` (and,
//! via the `pi` client's OMP scan root, `~/.omp/agent/sessions/...`). Current
//! OMP builds write a `title` metadata record before the `session` header in
//! newly-created session files; see [`PRE_SESSION_METADATA_TYPES`].
//!
//! Pi descendants reuse this record layout verbatim, so [`parse_pi_format_file`]
//! is shared: see `sessions::senpi` for Senpi (OmO Native).

use super::utils::{file_modified_timestamp_ms, lossy_lines_with_bytes, LossyLine};
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

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
/// keys, including `rlmDepth�`, remain valid. Invalid UTF-8 is tracked
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
    pub name: Option<String>,
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
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PiUsage {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    #[allow(dead_code)]
    pub total_tokens: Option<i64>,
    /// Parsed so the omission below is a real decision rather than an accident
    /// of the schema, but never summed: see the note at the emit site.
    #[allow(dead_code)]
    pub reasoning: Option<i64>,
    #[serde(flatten)]
    extra_fields: BTreeMap<String, serde_json::Value>,
}

impl PiUsage {
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
        None,
        PiParseOptions::standard(),
        &mut observer,
    )
}

/// Parse a Pi-format session and retain message ids in namespaced dedup keys.
/// Pi-compatible clients that need cross-file deduplication can opt into this
/// without changing the historical output of the shared Pi and Senpi parsers.
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
/// an assistant record that passed the shared parser's validation.
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
        }
    }

    const fn prime_agent() -> Self {
        Self {
            rlm_session_name_as_agent: true,
            cross_session_dedup: true,
            lossy_line_reader: true,
        }
    }
}

enum PiLines {
    Standard {
        lines: std::io::Lines<BufReader<std::fs::File>>,
        at_start: bool,
    },
    Lossy(super::utils::LossyLinesWithBytes<BufReader<std::fs::File>>),
}

enum PiLine {
    Standard(String),
    Lossy(LossyLine),
}

impl PiLine {
    fn text(&self) -> &str {
        match self {
            Self::Standard(text) => text,
            Self::Lossy(line) => &line.text,
        }
    }

    fn raw_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Standard(_) => None,
            Self::Lossy(line) => Some(&line.bytes),
        }
    }
}

impl Iterator for PiLines {
    type Item = PiLine;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self {
                Self::Standard { lines, at_start } => match lines.next() {
                    Some(Ok(mut line)) => {
                        // A UTF-8 BOM decodes cleanly, so it survives as U+FEFF
                        // glued to the front of the first record, where
                        // `str::trim` leaves it (U+FEFF is not White_Space) and
                        // the header then fails to parse. A header that fails to
                        // parse discards the whole transcript rather than one
                        // record, so strip the marker here exactly as the lossy
                        // reader already does for Prime Agent.
                        if std::mem::take(at_start) {
                            if let Some(stripped) = line.strip_prefix('\u{feff}') {
                                line = stripped.to_string();
                            }
                        }
                        return Some(PiLine::Standard(line));
                    }
                    // `reader.lines()` historically skipped all unreadable
                    // records and continued. In particular, an invalid-UTF-8
                    // line must not truncate valid records that follow it.
                    Some(Err(_)) => {
                        // The marker can only precede the first record, and that
                        // record was just consumed.
                        *at_start = false;
                        continue;
                    }
                    None => return None,
                },
                Self::Lossy(lines) => return lines.next().map(PiLine::Lossy),
            }
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
    message: &'a PiMessage,
    usage: &'a PiUsage,
    recorded_model: &'a str,
}

fn pi_emitted_record<'a>(
    entry: &'a PiSessionEntry,
    options: PiParseOptions,
    is_rlm_subagent: bool,
) -> Option<PiEmittedRecord<'a>> {
    if entry.entry_type != "message" {
        return None;
    }

    let message = entry.message.as_ref()?;
    if message.role.as_deref() != Some("assistant") {
        return None;
    }

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

    let usage = message.usage.as_ref()?;
    if options.lossy_line_reader && usage.has_damaged_key() {
        return None;
    }

    let recorded_model = message.model.as_deref()?;
    Some(PiEmittedRecord {
        message,
        usage,
        recorded_model,
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
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let fallback_timestamp = file_modified_timestamp_ms(path);

    let reader = BufReader::new(file);
    let lines = if options.lossy_line_reader {
        PiLines::Lossy(lossy_lines_with_bytes(reader))
    } else {
        PiLines::Standard {
            lines: reader.lines(),
            at_start: true,
        }
    };
    let mut messages: Vec<UnifiedMessage> = Vec::with_capacity(64);
    let mut buffer = Vec::with_capacity(4096);

    let mut session_id: Option<String> = None;
    let mut workspace_key: Option<String> = None;
    let mut workspace_label: Option<String> = None;
    let mut agent: Option<String> = None;
    let mut is_rlm_subagent = false;
    for line in lines {
        let trimmed = line.text().trim();
        if trimmed.is_empty() {
            continue;
        }

        if session_id.is_none() {
            buffer.clear();
            buffer.extend_from_slice(trimmed.as_bytes());
            let entry_type = match simd_json::from_slice::<PiEntryTypeProbe>(&mut buffer) {
                Ok(probe) => probe.entry_type,
                Err(_)
                    if options.lossy_line_reader && pre_header_line_is_skippable(trimmed, None) =>
                {
                    continue;
                }
                Err(_) => return Vec::new(),
            };

            if entry_type != "session" {
                if PRE_SESSION_METADATA_TYPES.contains(&entry_type.as_str()) {
                    continue;
                }
                return Vec::new();
            }

            buffer.clear();
            buffer.extend_from_slice(trimmed.as_bytes());
            let header = match simd_json::from_slice::<PiSessionHeader>(&mut buffer) {
                Ok(h) => h,
                Err(_) => return Vec::new(),
            };
            let has_raw_damaged_lineage_key = line
                .raw_bytes()
                .is_some_and(raw_json_has_damaged_lineage_header_key);
            if options.lossy_line_reader
                && (header.has_invalid_lineage() || has_raw_damaged_lineage_key)
            {
                return Vec::new();
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
            continue;
        }

        buffer.clear();
        buffer.extend_from_slice(trimmed.as_bytes());
        let entry = match simd_json::from_slice::<PiSessionEntry>(&mut buffer) {
            Ok(e) => e,
            Err(_) => continue,
        };

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
            continue;
        }

        let Some(PiEmittedRecord {
            message,
            usage,
            recorded_model,
        }) = pi_emitted_record(&entry, options, is_rlm_subagent)
        else {
            observer.observe_entry(&entry, None);
            continue;
        };

        let model = if !accepts_replacement_field(recorded_model, options.lossy_line_reader) {
            "unknown"
        } else {
            recorded_model
        };

        // A missing/blank provider field is recoverable: infer it from the
        // model name (e.g. a Pi "gpt-5" message with no provider maps to
        // "openai"), falling back to "pi" only when inference can't
        // identify the model, rather than dropping a message that carries
        // valid tokens.
        let provider = match message.provider.as_deref() {
            Some(provider)
                if !provider.is_empty()
                    && accepts_replacement_field(provider, options.lossy_line_reader) =>
            {
                provider.to_string()
            }
            _ => inferred_provider_from_model(model)
                .unwrap_or(fallback_provider)
                .to_string(),
        };

        let recorded_timestamp = entry
            .timestamp
            .as_deref()
            .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
            .map(|timestamp| timestamp.timestamp_millis());
        let timestamp = recorded_timestamp.unwrap_or(fallback_timestamp);

        // `usage.reasoning` is read but deliberately not mapped onto
        // `TokenBreakdown::reasoning`. In the Pi format reasoning tokens are a
        // subset of `output` (Pi's own `totalTokens` excludes them), whereas
        // tokscale totals `reasoning` as its own additive bucket. Mapping it
        // through would double count.
        let mut unified = UnifiedMessage::new_with_agent(
            client,
            model,
            provider.as_str(),
            session_id.clone().unwrap_or_else(|| "unknown".to_string()),
            timestamp,
            TokenBreakdown {
                input: usage.input.unwrap_or(0).max(0),
                output: usage.output.unwrap_or(0).max(0),
                cache_read: usage.cache_read.unwrap_or(0).max(0),
                cache_write: usage.cache_write.unwrap_or(0).max(0),
                reasoning: 0,
            },
            0.0,
            agent.clone(),
        );
        if let Some(namespace) = dedup_namespace {
            if options.cross_session_dedup {
                let clean_response_key = message
                    .response_id
                    .as_deref()
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
                            "{namespace}:message:{id}:{stable_timestamp}:{provider}:{model}:{}:{}:{}:{}",
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
                    }) || message.response_id.as_deref().is_some_and(|id| {
                        !accepts_replacement_field(id, options.lossy_line_reader)
                    });
                    if has_damaged_id {
                        let raw_line = line.raw_bytes().unwrap_or_else(|| line.text().as_bytes());
                        unified.dedup_key =
                            Some(damaged_cross_session_dedup_key(namespace, raw_line));
                    }
                }
            } else if let Some(message_id) = entry.id.as_deref().filter(|id| !id.trim().is_empty())
            {
                let session_id = session_id.as_deref().unwrap_or("unknown");
                unified.dedup_key = Some(format!("{namespace}:{session_id}:{message_id}"));
            }
        }
        unified.set_workspace(workspace_key.clone(), workspace_label.clone());
        observer.observe_entry(&entry, Some(&unified));
        messages.push(unified);
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
    }

    #[test]
    fn test_parse_pi_skips_multiple_leading_title_records() {
        // given: defensive against more than one pre-session metadata line
        // in a row (e.g. a title record rewritten by a later auto-rename).
        let content = r#"{"type":"title","v":1,"title":"first"}
{"type":"title","v":1,"title":"renamed"}
{"type":"session","id":"pi_ses_006","timestamp":"2026-07-02T18:07:14.690Z","cwd":"/tmp"}
{"type":"message","timestamp":"2026-07-02T18:08:53.229Z","message":{"role":"assistant","model":"gpt-4o-mini","provider":"openai","usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15}}}"#;
        let file = create_test_file(content);

        // when
        let messages = parse_pi_file(file.path());

        // then
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "pi_ses_006");
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
}
