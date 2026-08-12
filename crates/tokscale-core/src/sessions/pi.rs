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
    Standard(std::io::Lines<BufReader<std::fs::File>>),
    Lossy(super::utils::LossyLinesWithBytes<BufReader<std::fs::File>>),
}

impl Iterator for PiLines {
    type Item = LossyLine;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self {
                Self::Standard(lines) => match lines.next() {
                    Some(Ok(line)) => {
                        return Some(LossyLine {
                            bytes: line.as_bytes().to_vec(),
                            text: line,
                        });
                    }
                    Some(Err(error)) if error.kind() == std::io::ErrorKind::InvalidData => continue,
                    Some(Err(_)) => return None,
                    None => return None,
                },
                Self::Lossy(lines) => return lines.next(),
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
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map_or_else(|| "unknown".to_string(), |stem| format!("unknown:{stem}"))
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
        PiLines::Standard(reader.lines())
    };
    let mut messages: Vec<UnifiedMessage> = Vec::with_capacity(64);
    let mut buffer = Vec::with_capacity(4096);

    let mut session_id: Option<String> = None;
    let mut workspace_key: Option<String> = None;
    let mut workspace_label: Option<String> = None;
    let mut agent: Option<String> = None;
    let mut is_rlm_subagent = false;
    for line in lines {
        let trimmed = line.text.trim();
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

        if entry.entry_type != "message" {
            observer.observe_entry(&entry, None);
            continue;
        }

        let Some(message) = entry.message.as_ref() else {
            observer.observe_entry(&entry, None);
            continue;
        };

        if message.role.as_deref() != Some("assistant") {
            observer.observe_entry(&entry, None);
            continue;
        }

        let Some(usage) = message.usage.as_ref() else {
            observer.observe_entry(&entry, None);
            continue;
        };

        let Some(recorded_model) = message.model.as_deref() else {
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
                        unified.dedup_key =
                            Some(damaged_cross_session_dedup_key(namespace, &line.bytes));
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
