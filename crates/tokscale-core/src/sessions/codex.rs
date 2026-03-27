//! Codex CLI session parser
//!
//! Parses JSONL files from ~/.codex/sessions/
//! Note: This parser has stateful logic to track model and delta calculations.

use super::utils::{
    extract_i64, extract_string, file_modified_timestamp_ms, parse_timestamp_value,
};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use serde_json::Value;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

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
    #[serde(rename = "type")]
    pub payload_type: Option<String>,
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub model_info: Option<CodexModelInfo>,
    pub info: Option<CodexInfo>,
    pub source: Option<String>,
    /// Provider identity from session_meta (e.g. "openai", "azure")
    pub model_provider: Option<String>,
    /// Agent name from session_meta
    pub agent_nickname: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodexModelInfo {
    pub slug: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodexInfo {
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub last_token_usage: Option<CodexTokenUsage>,
    pub total_token_usage: Option<CodexTokenUsage>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CodexTokenUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
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
        TokenBreakdown {
            input: (self.input - clamped_cached).max(0),
            output: self.output.max(0),
            cache_read: clamped_cached,
            cache_write: 0,
            reasoning: self.reasoning.max(0),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct CodexParseState {
    pub current_model: Option<String>,
    pub previous_totals: Option<CodexTotals>,
    pub session_is_headless: bool,
    pub session_provider: Option<String>,
    pub session_agent: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedCodexFile {
    pub messages: Vec<UnifiedMessage>,
    pub fallback_timestamp_indices: Vec<usize>,
    pub consumed_offset: u64,
    pub parse_succeeded: bool,
    pub state: CodexParseState,
}

fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn parse_codex_reader<R: BufRead>(
    mut reader: R,
    session_id: &str,
    fallback_timestamp: i64,
    start_offset: u64,
    mut state: CodexParseState,
) -> ParsedCodexFile {
    let mut messages = Vec::with_capacity(64);
    let mut fallback_timestamp_indices = Vec::new();
    let mut buffer = Vec::with_capacity(4096);
    let mut line = String::with_capacity(4096);
    let mut consumed_offset = start_offset;
    let mut parse_succeeded = true;

    loop {
        line.clear();
        let bytes_read = match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(bytes_read) => bytes_read,
            Err(_) => {
                parse_succeeded = false;
                break;
            }
        };
        consumed_offset += bytes_read as u64;

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut handled = false;
        buffer.clear();
        buffer.extend_from_slice(trimmed.as_bytes());
        if let Ok(entry) = simd_json::from_slice::<CodexEntry>(&mut buffer) {
            if let Some(payload) = entry.payload {
                if entry.entry_type == "session_meta" {
                    if payload.source.as_deref() == Some("exec") {
                        state.session_is_headless = true;
                    }
                    if let Some(ref provider) = payload.model_provider {
                        state.session_provider = Some(provider.clone());
                    }
                    if let Some(ref nickname) = payload.agent_nickname {
                        state.session_agent = Some(nickname.clone());
                    }
                }
                // Extract model from turn_context
                if entry.entry_type == "turn_context" {
                    state.current_model = extract_model(&payload);
                    handled = true;
                }

                // Process token_count events
                if entry.entry_type == "event_msg"
                    && payload.payload_type.as_deref() == Some("token_count")
                {
                    // Try to extract model from payload
                    if let Some(model) = extract_model(&payload) {
                        state.current_model = Some(model);
                    }

                    let info = match payload.info {
                        Some(i) => i,
                        None => continue,
                    };

                    // Try to extract model from info
                    if let Some(model) = info.model.clone().or(info.model_name.clone()) {
                        state.current_model = Some(model);
                    }

                    let model = state
                        .current_model
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());

                    // Use last_token_usage as the primary increment source.
                    // Upstream totals are mutable snapshots (compaction, context-window
                    // capping can rewrite them), so we only use total_token_usage for
                    // dedup and monotonicity checks — never as a direct delta source.
                    let total_usage = info.total_token_usage.as_ref().map(CodexTotals::from_usage);
                    let last_usage = info.last_token_usage.as_ref().map(CodexTotals::from_usage);

                    let (tokens, next_totals) =
                        match (total_usage, last_usage, state.previous_totals) {
                            // Both present with previous baseline (standard path)
                            (Some(total), Some(last), Some(previous)) => {
                                if total == previous {
                                    continue;
                                }
                                if total.delta_from(previous).is_none()
                                    && total.looks_like_stale_regression(previous, last)
                                {
                                    continue;
                                }
                                (last.into_tokens(), Some(total))
                            }
                            // Both present, first event — use last (NOT full total) to
                            // avoid overcounting tokens carried from a resumed session.
                            (Some(total), Some(last), None) => (last.into_tokens(), Some(total)),
                            // Only total, have previous (defensive — upstream schema
                            // requires both when info is present)
                            (Some(total), None, Some(previous)) => {
                                if total == previous {
                                    continue;
                                }
                                if let Some(delta) = total.delta_from(previous) {
                                    (delta.into_tokens(), Some(total))
                                } else {
                                    state.previous_totals = Some(total);
                                    continue;
                                }
                            }
                            // Only total, first event, no last — legacy/degraded path
                            (Some(total), None, None) => (total.into_tokens(), Some(total)),
                            // Only last, have previous
                            (None, Some(last), Some(previous)) => {
                                (last.into_tokens(), Some(previous.saturating_add(last)))
                            }
                            // Only last, no previous
                            (None, Some(last), None) => (last.into_tokens(), None),
                            // Neither
                            (None, None, _) => continue,
                        };

                    // Skip zero-token snapshots without advancing the baseline so
                    // that post-compaction zero totals don't inflate later deltas.
                    if tokens.input == 0
                        && tokens.output == 0
                        && tokens.cache_read == 0
                        && tokens.reasoning == 0
                    {
                        continue;
                    }

                    state.previous_totals = next_totals;

                    let parsed_timestamp = entry
                        .timestamp
                        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
                        .map(|dt| dt.timestamp_millis());
                    let timestamp = parsed_timestamp.unwrap_or(fallback_timestamp);

                    let agent = if state.session_is_headless {
                        Some("headless".to_string())
                    } else {
                        state.session_agent.clone()
                    };

                    let provider = state.session_provider.as_deref().unwrap_or("openai");

                    messages.push(UnifiedMessage::new_with_agent(
                        "codex",
                        model,
                        provider,
                        session_id.to_string(),
                        timestamp,
                        tokens,
                        0.0,
                        agent,
                    ));
                    if parsed_timestamp.is_none() {
                        fallback_timestamp_indices.push(messages.len() - 1);
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

        if let Some((msg, used_fallback_timestamp)) = parse_codex_headless_line(
            trimmed,
            session_id,
            &mut state.current_model,
            fallback_timestamp,
            state.session_provider.as_deref(),
            &state.session_agent,
            state.session_is_headless,
        ) {
            messages.push(msg);
            if used_fallback_timestamp {
                fallback_timestamp_indices.push(messages.len() - 1);
            }
        }
    }

    ParsedCodexFile {
        messages,
        fallback_timestamp_indices,
        consumed_offset,
        parse_succeeded,
        state,
    }
}

/// Parse a Codex JSONL file with stateful tracking
pub fn parse_codex_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let session_id = session_id_from_path(path);
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let reader = BufReader::new(file);
    let parsed = parse_codex_reader(
        reader,
        &session_id,
        fallback_timestamp,
        0,
        CodexParseState::default(),
    );
    parsed.messages
}

pub(crate) fn parse_codex_file_incremental(
    path: &Path,
    start_offset: u64,
    state: CodexParseState,
) -> ParsedCodexFile {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => {
            return ParsedCodexFile {
                messages: Vec::new(),
                fallback_timestamp_indices: Vec::new(),
                consumed_offset: start_offset,
                parse_succeeded: false,
                state,
            };
        }
    };

    if file.seek(SeekFrom::Start(start_offset)).is_err() {
        return ParsedCodexFile {
            messages: Vec::new(),
            fallback_timestamp_indices: Vec::new(),
            consumed_offset: start_offset,
            parse_succeeded: false,
            state,
        };
    }

    let session_id = session_id_from_path(path);
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let reader = BufReader::new(file);
    parse_codex_reader(reader, &session_id, fallback_timestamp, start_offset, state)
}

fn extract_model(payload: &CodexPayload) -> Option<String> {
    payload
        .model_info
        .as_ref()
        .and_then(|mi| mi.slug.clone())
        .filter(|s| !s.is_empty())
        .or(payload.model.clone().filter(|s| !s.is_empty()))
        .or(payload.model_name.clone().filter(|s| !s.is_empty()))
        .or(payload
            .info
            .as_ref()
            .and_then(|i| i.model.clone())
            .filter(|s| !s.is_empty()))
        .or(payload
            .info
            .as_ref()
            .and_then(|i| i.model_name.clone())
            .filter(|s| !s.is_empty()))
}

struct CodexHeadlessUsage {
    input: i64,
    output: i64,
    cached: i64,
    model: Option<String>,
    timestamp_ms: Option<i64>,
}

fn parse_codex_headless_line(
    line: &str,
    session_id: &str,
    current_model: &mut Option<String>,
    fallback_timestamp: i64,
    session_provider: Option<&str>,
    session_agent: &Option<String>,
    session_is_headless: bool,
) -> Option<(UnifiedMessage, bool)> {
    let mut bytes = line.as_bytes().to_vec();
    let value: Value = simd_json::from_slice(&mut bytes).ok()?;

    if let Some(model) = extract_model_from_value(&value) {
        *current_model = Some(model);
    }

    let usage = extract_headless_usage(&value)?;
    let model = usage
        .model
        .or_else(|| current_model.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let timestamp = usage.timestamp_ms.unwrap_or(fallback_timestamp);

    if usage.input == 0 && usage.output == 0 && usage.cached == 0 {
        return None;
    }

    let provider = session_provider.unwrap_or("openai");
    let agent = if session_is_headless {
        Some("headless".to_string())
    } else {
        session_agent.clone()
    };

    Some((
        UnifiedMessage::new_with_agent(
            "codex",
            model,
            provider,
            session_id.to_string(),
            timestamp,
            TokenBreakdown {
                input: usage.input.max(0),
                output: usage.output.max(0),
                cache_read: usage.cached.max(0),
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
            agent,
        ),
        usage.timestamp_ms.is_none(),
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Cursor, Error, ErrorKind, Seek, SeekFrom, Write};
    use tempfile::NamedTempFile;

    fn create_test_file(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
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
            self.inner.fill_buf()
        }

        fn consume(&mut self, amt: usize) {
            self.inner.consume(amt);
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
            r#"{"type":"session_meta","payload":{"source":"chat","model_provider":"openai","agent_nickname":"builder"}}"#,
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
        assert_eq!(messages[0].agent.as_deref(), Some("headless"));
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
        assert_eq!(messages[0].tokens.output, 30);
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
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 3);
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
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 3);
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
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        // Second message: delta from 50→80
        assert_eq!(messages[1].tokens.input, 25);
        assert_eq!(messages[1].tokens.output, 10);
        assert_eq!(messages[1].tokens.cache_read, 5);
        assert_eq!(messages[1].tokens.reasoning, 2);
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
        assert_eq!(tokens.output, 30);
        assert_eq!(tokens.reasoning, 5);
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
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 3);
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
        assert_eq!(messages[0].tokens.output, 30);
        assert_eq!(messages[0].tokens.cache_read, 20);
        assert_eq!(messages[0].tokens.reasoning, 5);

        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 3);
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
        assert_eq!(messages[0].tokens.output, 400);
        assert_eq!(messages[0].tokens.cache_read, 1000);
        assert_eq!(messages[0].tokens.reasoning, 50);

        assert_eq!(messages[1].tokens.input, 20);
        assert_eq!(messages[1].tokens.output, 4);
        assert_eq!(messages[1].tokens.cache_read, 5);
        assert_eq!(messages[1].tokens.reasoning, 1);

        assert_eq!(messages[2].tokens.input, 20);
        assert_eq!(messages[2].tokens.output, 4);
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
        assert_eq!(messages[0].tokens.output, 5);
        assert_eq!(messages[0].tokens.cache_read, 2);
        assert_eq!(messages[0].tokens.reasoning, 1);
        assert_eq!(messages[1].tokens.input, 10);
        assert_eq!(messages[1].tokens.output, 5);
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
        assert_eq!(messages[0].tokens.output, 80);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.reasoning, 10);
        assert_eq!(messages[1].tokens.input, 8);
        assert_eq!(messages[1].tokens.output, 3);
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
        assert_eq!(messages[0].agent.as_deref(), Some("my-agent"));
    }

    #[test]
    fn test_cached_tokens_takes_max_of_both_fields() {
        let usage = CodexTokenUsage {
            input_tokens: Some(100),
            output_tokens: Some(30),
            cached_input_tokens: Some(10),
            cache_read_input_tokens: Some(20),
            reasoning_output_tokens: Some(5),
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
        assert_eq!(messages[1].tokens.output, 10);
        assert_eq!(messages[1].tokens.cache_read, 5);
        assert_eq!(messages[1].tokens.reasoning, 2);
    }

    #[test]
    fn test_headless_fallback_uses_session_provider_and_agent() {
        // session_meta sets provider to "azure" and agent to "my-bot",
        // then a line falls through to headless parsing (no structured entry_type)
        let line1 = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"model_provider":"azure","agent_nickname":"my-bot"}}"#;
        let line2 = r#"{"type":"turn.completed","model":"gpt-4o","usage":{"input_tokens":100,"output_tokens":50}}"#;
        let content = format!("{}\n{}", line1, line2);
        let file = create_test_file(&content);

        let messages = parse_codex_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "azure");
        assert_eq!(messages[0].agent.as_deref(), Some("my-bot"));
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
}
