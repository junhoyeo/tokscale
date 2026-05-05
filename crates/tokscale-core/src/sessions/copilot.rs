//! GitHub Copilot OTEL parser
//!
//! Parses file-exported OpenTelemetry JSONL emitted by Copilot CLI and VS Code
//! Copilot Chat monitoring. Chat spans and inference log records are preferred;
//! aggregate agent records are only used as a fallback to avoid double counting.

use super::utils::file_modified_timestamp_ms;
use super::UnifiedMessage;
use crate::provider_identity::inferred_provider_from_model;
use crate::TokenBreakdown;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::Path;

pub fn parse_copilot_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };

    let fallback_timestamp = file_modified_timestamp_ms(path);
    let mut records = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(record) = serde_json::from_str::<Value>(trimmed) {
            records.push(record);
        }
    }

    let trace_contexts = collect_trace_contexts(&records);
    let candidates: Vec<CopilotUsageCandidate> = records
        .iter()
        .enumerate()
        .filter_map(|(index, record)| {
            usage_candidate_from_record(record, index, fallback_timestamp, &trace_contexts)
        })
        .collect();

    let chat_traces = candidate_trace_contexts(&candidates, CopilotUsageSource::ChatSpan);
    let inference_traces = candidate_trace_contexts(&candidates, CopilotUsageSource::InferenceLog);
    let agent_turn_traces = candidate_trace_contexts(&candidates, CopilotUsageSource::AgentTurnLog);
    let chat_response_ids = candidate_response_ids(&candidates, CopilotUsageSource::ChatSpan);
    let inference_response_ids =
        candidate_response_ids(&candidates, CopilotUsageSource::InferenceLog);
    let agent_turn_response_ids =
        candidate_response_ids(&candidates, CopilotUsageSource::AgentTurnLog);

    candidates
        .into_iter()
        .filter(|candidate| {
            should_emit_candidate(
                candidate,
                &chat_traces,
                &inference_traces,
                &agent_turn_traces,
                &chat_response_ids,
                &inference_response_ids,
                &agent_turn_response_ids,
            )
        })
        .map(CopilotUsageCandidate::into_message)
        .collect()
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CopilotUsageSource {
    ChatSpan,
    InferenceLog,
    AgentTurnLog,
    AgentSummarySpan,
}

struct TraceContext {
    model: Option<String>,
    session_id: Option<String>,
    session_id_priority: SessionIdPriority,
}

struct CopilotUsageCandidate {
    source: CopilotUsageSource,
    trace_id: Option<String>,
    response_id: Option<String>,
    model: String,
    provider_id: String,
    session_id: String,
    timestamp_ms: i64,
    tokens: TokenBreakdown,
    dedup_key: String,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum SessionIdPriority {
    Missing,
    Response,
    Interaction,
    Session,
}

impl CopilotUsageCandidate {
    fn into_message(self) -> UnifiedMessage {
        UnifiedMessage::new_with_dedup(
            "copilot",
            self.model,
            self.provider_id,
            self.session_id,
            self.timestamp_ms,
            self.tokens,
            0.0,
            Some(self.dedup_key),
        )
    }
}

fn collect_trace_contexts(records: &[Value]) -> HashMap<String, TraceContext> {
    let mut contexts = HashMap::new();

    for record in records {
        let Some(trace_id) = trace_id_from_record(record) else {
            continue;
        };

        let Some(attributes) = record.get("attributes").and_then(Value::as_object) else {
            continue;
        };

        let context = contexts
            .entry(trace_id.to_string())
            .or_insert(TraceContext {
                model: None,
                session_id: None,
                session_id_priority: SessionIdPriority::Missing,
            });

        if context.model.is_none() {
            context.model = first_non_empty_attr(attributes, MODEL_ATTRS).map(str::to_string);
        }

        if let Some((session_id, priority)) = best_session_attr(attributes) {
            if priority > context.session_id_priority {
                context.session_id = Some(session_id.to_string());
                context.session_id_priority = priority;
            }
        }
    }

    contexts
}

fn usage_candidate_from_record(
    record: &Value,
    index: usize,
    fallback_timestamp: i64,
    trace_contexts: &HashMap<String, TraceContext>,
) -> Option<CopilotUsageCandidate> {
    let attributes = record.get("attributes").and_then(Value::as_object)?;
    let trace_id = trace_id_from_record(record).map(str::to_string);
    let trace_context = trace_id
        .as_deref()
        .and_then(|trace_id| trace_contexts.get(trace_id));

    if is_chat_span_record(record, attributes) {
        return candidate_from_attributes(
            CopilotUsageSource::ChatSpan,
            record,
            attributes,
            trace_id,
            trace_context,
            index,
            fallback_timestamp,
        );
    }

    if is_inference_log_record(record, attributes) {
        return candidate_from_attributes(
            CopilotUsageSource::InferenceLog,
            record,
            attributes,
            trace_id,
            trace_context,
            index,
            fallback_timestamp,
        );
    }

    if is_agent_turn_log_record(record, attributes) {
        return candidate_from_attributes(
            CopilotUsageSource::AgentTurnLog,
            record,
            attributes,
            trace_id,
            trace_context,
            index,
            fallback_timestamp,
        );
    }

    if is_agent_summary_span_record(record, attributes) {
        return candidate_from_attributes(
            CopilotUsageSource::AgentSummarySpan,
            record,
            attributes,
            trace_id,
            trace_context,
            index,
            fallback_timestamp,
        );
    }

    None
}

fn candidate_from_attributes(
    source: CopilotUsageSource,
    record: &Value,
    attributes: &Map<String, Value>,
    trace_id: Option<String>,
    trace_context: Option<&TraceContext>,
    index: usize,
    fallback_timestamp: i64,
) -> Option<CopilotUsageCandidate> {
    let input = attr_i64_first(attributes, &["gen_ai.usage.input_tokens"]);
    let output = attr_i64_first(attributes, &["gen_ai.usage.output_tokens"]);
    let cache_read = attr_i64_first(attributes, &["gen_ai.usage.cache_read.input_tokens"]);
    let cache_write = attr_i64_first(
        attributes,
        &[
            "gen_ai.usage.cache_write.input_tokens",
            "gen_ai.usage.cache_creation.input_tokens",
        ],
    );
    let reasoning = attr_i64_first(
        attributes,
        &[
            "gen_ai.usage.reasoning.output_tokens",
            "gen_ai.usage.reasoning_tokens",
        ],
    );

    let tokens = normalize_input_tokens(input, output, cache_read, cache_write, reasoning);
    if tokens.total() == 0 {
        return None;
    }

    let response_id = attributes
        .get("gen_ai.response.id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let model = first_non_empty_attr(attributes, MODEL_ATTRS)
        .or_else(|| trace_context.and_then(|context| context.model.as_deref()))
        .unwrap_or("unknown")
        .to_string();
    let provider_id = inferred_provider_from_model(&model)
        .unwrap_or("github-copilot")
        .to_string();
    let session_id = best_session_attr(attributes)
        .map(|(session_id, _)| session_id)
        .or_else(|| trace_context.and_then(|context| context.session_id.as_deref()))
        .or(trace_id.as_deref())
        .unwrap_or("unknown-session")
        .to_string();
    let timestamp_ms = timestamp_ms_from_record(record).unwrap_or(fallback_timestamp);
    let dedup_key = dedup_key_for_record(
        source,
        record,
        attributes,
        trace_id.as_deref(),
        &session_id,
        timestamp_ms,
        index,
    );

    Some(CopilotUsageCandidate {
        source,
        trace_id,
        response_id,
        model,
        provider_id,
        session_id,
        timestamp_ms,
        tokens,
        dedup_key,
    })
}

fn candidate_trace_contexts(
    candidates: &[CopilotUsageCandidate],
    source: CopilotUsageSource,
) -> HashSet<String> {
    candidates
        .iter()
        .filter(|candidate| candidate.source == source)
        .filter_map(|candidate| candidate.trace_id.clone())
        .collect()
}

fn candidate_response_ids(
    candidates: &[CopilotUsageCandidate],
    source: CopilotUsageSource,
) -> HashSet<String> {
    candidates
        .iter()
        .filter(|candidate| candidate.source == source)
        .filter_map(|candidate| candidate.response_id.clone())
        .collect()
}

fn should_emit_candidate(
    candidate: &CopilotUsageCandidate,
    chat_traces: &HashSet<String>,
    inference_traces: &HashSet<String>,
    agent_turn_traces: &HashSet<String>,
    chat_response_ids: &HashSet<String>,
    inference_response_ids: &HashSet<String>,
    agent_turn_response_ids: &HashSet<String>,
) -> bool {
    // Cross-source priority filtering keys off two stable per-event identifiers:
    // the OTel `trace_id` and `gen_ai.response.id`. Either match is sufficient
    // to suppress a lower-priority lane, which closes the mixed-trace gap where
    // one record carries a trace_id and another (describing the same response)
    // does not. Coarse session attributes such as gen_ai.conversation.id span
    // multiple turns and are intentionally NOT used here.
    let trace_id = candidate.trace_id.as_deref();
    let response_id = candidate.response_id.as_deref();

    let trace_match = |traces: &HashSet<String>| trace_id.is_some_and(|id| traces.contains(id));
    let response_match =
        |response_ids: &HashSet<String>| response_id.is_some_and(|id| response_ids.contains(id));

    match candidate.source {
        CopilotUsageSource::ChatSpan => true,
        CopilotUsageSource::InferenceLog => {
            !trace_match(chat_traces) && !response_match(chat_response_ids)
        }
        CopilotUsageSource::AgentTurnLog => {
            !trace_match(chat_traces)
                && !trace_match(inference_traces)
                && !response_match(chat_response_ids)
                && !response_match(inference_response_ids)
        }
        CopilotUsageSource::AgentSummarySpan => {
            !trace_match(chat_traces)
                && !trace_match(inference_traces)
                && !trace_match(agent_turn_traces)
                && !response_match(chat_response_ids)
                && !response_match(inference_response_ids)
                && !response_match(agent_turn_response_ids)
        }
    }
}

const MODEL_ATTRS: &[&str] = &["gen_ai.response.model", "gen_ai.request.model"];
const SESSION_ATTRS: &[(&str, SessionIdPriority)] = &[
    ("gen_ai.conversation.id", SessionIdPriority::Session),
    ("copilot_chat.session_id", SessionIdPriority::Session),
    ("copilot_chat.chat_session_id", SessionIdPriority::Session),
    ("session.id", SessionIdPriority::Session),
    (
        "github.copilot.interaction_id",
        SessionIdPriority::Interaction,
    ),
    ("gen_ai.response.id", SessionIdPriority::Response),
];

fn is_chat_span_record(value: &Value, attributes: &Map<String, Value>) -> bool {
    if !is_span_record(value) {
        return false;
    }

    if attr_str(attributes, "gen_ai.operation.name") == Some("chat") {
        return true;
    }

    value
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| name.starts_with("chat "))
}

fn is_agent_summary_span_record(value: &Value, attributes: &Map<String, Value>) -> bool {
    if !is_span_record(value) {
        return false;
    }

    if attr_str(attributes, "gen_ai.operation.name") == Some("invoke_agent") {
        return true;
    }

    value
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| name.starts_with("invoke_agent "))
}

fn is_inference_log_record(value: &Value, attributes: &Map<String, Value>) -> bool {
    if is_span_record(value) {
        return false;
    }

    attr_str(attributes, "event.name") == Some("gen_ai.client.inference.operation.details")
        || record_body(value).is_some_and(|body| body.starts_with("GenAI inference:"))
}

fn is_agent_turn_log_record(value: &Value, attributes: &Map<String, Value>) -> bool {
    if is_span_record(value) {
        return false;
    }

    attr_str(attributes, "event.name") == Some("copilot_chat.agent.turn")
        || record_body(value).is_some_and(|body| body.starts_with("copilot_chat.agent.turn"))
}

fn is_span_record(value: &Value) -> bool {
    // VS Code Copilot Chat exports omit `type: "span"`, so when `type` is absent
    // we infer span-ness from a top-level `name` plus span identity (spanId or
    // traceId), span timing, or `kind`. This is intentionally permissive for
    // VS Code support. Inference-log and agent-turn-log records do NOT carry a
    // top-level `name` field — that is the property that disambiguates them
    // here. If a future record shape adds a top-level `name`, revisit this.
    match value.get("type").and_then(Value::as_str) {
        Some("span") => return true,
        Some(_) => return false,
        None => {}
    }

    let has_name = value.get("name").and_then(Value::as_str).is_some();
    let has_span_identity = value.get("spanId").and_then(Value::as_str).is_some()
        || value.get("traceId").and_then(Value::as_str).is_some();
    let has_span_timing = value.get("startTime").is_some()
        || value.get("endTime").is_some()
        || value.get("duration").is_some();

    has_name && (has_span_identity || has_span_timing || value.get("kind").is_some())
}

fn trace_id_from_record(value: &Value) -> Option<&str> {
    value.get("traceId").and_then(Value::as_str).or_else(|| {
        value
            .get("spanContext")
            .and_then(Value::as_object)
            .and_then(|context| context.get("traceId"))
            .and_then(Value::as_str)
    })
}

fn span_id_from_record(value: &Value) -> Option<&str> {
    value.get("spanId").and_then(Value::as_str).or_else(|| {
        value
            .get("spanContext")
            .and_then(Value::as_object)
            .and_then(|context| context.get("spanId"))
            .and_then(Value::as_str)
    })
}

fn dedup_key_for_record(
    source: CopilotUsageSource,
    record: &Value,
    attributes: &Map<String, Value>,
    trace_id: Option<&str>,
    session_id: &str,
    timestamp_ms: i64,
    index: usize,
) -> String {
    let span_id = span_id_from_record(record);

    match source {
        CopilotUsageSource::ChatSpan | CopilotUsageSource::AgentSummarySpan => {
            match (trace_id, span_id) {
                (Some(trace_id), Some(span_id)) => format!("{trace_id}:{span_id}"),
                _ => format!("span:{session_id}:{timestamp_ms}:{index}"),
            }
        }
        CopilotUsageSource::InferenceLog => match (trace_id, span_id) {
            (Some(trace_id), Some(span_id)) => format!("log:{trace_id}:{span_id}"),
            _ => format!("log:{session_id}:{timestamp_ms}:{index}"),
        },
        CopilotUsageSource::AgentTurnLog => {
            // When the record actually carries a turn.index, use it so the key
            // is stable across re-runs. Otherwise fall back to the line index
            // so two turn-less agent-turn records in the same trace do not
            // collide on a `0` sentinel.
            let turn_part = ["turn.index", "copilot_chat.turn.index"]
                .iter()
                .find_map(|key| attributes.get(*key).and_then(value_as_i64))
                .map(|value| value.to_string())
                .unwrap_or_else(|| format!("idx-{index}"));
            if let Some(trace_id) = trace_id {
                format!("agent-turn:{trace_id}:{turn_part}")
            } else {
                format!("agent-turn:{session_id}:{turn_part}:{index}")
            }
        }
    }
}

fn attr_str<'a>(attributes: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    attributes.get(key).and_then(Value::as_str)
}

fn attr_i64(attributes: &Map<String, Value>, key: &str) -> i64 {
    attributes
        .get(key)
        .and_then(value_as_i64)
        .unwrap_or(0)
        .max(0)
}

fn attr_i64_first(attributes: &Map<String, Value>, keys: &[&str]) -> i64 {
    keys.iter()
        .map(|key| attr_i64(attributes, key))
        .find(|value| *value > 0)
        .unwrap_or(0)
}

fn normalize_input_tokens(
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    reasoning: i64,
) -> TokenBreakdown {
    // OTEL reports input_tokens inclusive of cache reads. Normalize only the
    // cached-read portion out of input, but preserve the reported cache buckets
    // intact because pricing totals account for them separately.
    let cache_read_for_input = cache_read.max(0).min(input.max(0));

    TokenBreakdown {
        input: input.saturating_sub(cache_read_for_input).max(0),
        output: output.max(0),
        cache_read: cache_read.max(0),
        cache_write: cache_write.max(0),
        reasoning: reasoning.max(0),
    }
}

fn first_non_empty_attr<'a>(attributes: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .filter_map(|key| attributes.get(*key).and_then(Value::as_str))
        .find(|value| !value.trim().is_empty())
}

fn best_session_attr(attributes: &Map<String, Value>) -> Option<(&str, SessionIdPriority)> {
    SESSION_ATTRS
        .iter()
        .filter_map(|(key, priority)| {
            let value = attributes.get(*key).and_then(Value::as_str)?;
            if value.trim().is_empty() {
                return None;
            }

            Some((value, *priority))
        })
        .max_by_key(|(_, priority)| *priority)
}

fn record_body(value: &Value) -> Option<&str> {
    value
        .get("body")
        .and_then(Value::as_str)
        .or_else(|| value.get("_body").and_then(Value::as_str))
}

fn value_as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_f64().map(|value| value as i64))
        .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
}

fn timestamp_ms_from_record(value: &Value) -> Option<i64> {
    value
        .get("endTime")
        .and_then(timestamp_ms_from_value)
        .or_else(|| value.get("startTime").and_then(timestamp_ms_from_value))
        .or_else(|| value.get("hrTime").and_then(timestamp_ms_from_value))
        .or_else(|| value.get("_hrTime").and_then(timestamp_ms_from_value))
        .or_else(|| value.get("time").and_then(timestamp_ms_from_value))
        .or_else(|| value.get("timestamp").and_then(timestamp_ms_from_scalar))
        .or_else(|| {
            value
                .get("observedTimestamp")
                .and_then(timestamp_ms_from_scalar)
        })
        .or_else(|| {
            value
                .get("timeUnixNano")
                .and_then(timestamp_ms_from_unix_nanos)
        })
}

fn timestamp_ms_from_value(value: &Value) -> Option<i64> {
    let parts = value.as_array()?;
    let seconds = parts.first().and_then(value_as_i64)?;
    let nanos = parts.get(1).and_then(value_as_i64)?;
    Some(seconds.saturating_mul(1000) + nanos / 1_000_000)
}

fn timestamp_ms_from_scalar(value: &Value) -> Option<i64> {
    let raw = value_as_i64(value)?;
    Some(match raw.abs() {
        100_000_000_000_000_000.. => raw / 1_000_000,
        100_000_000_000_000.. => raw / 1_000,
        100_000_000_000.. => raw,
        _ => raw.saturating_mul(1000),
    })
}

fn timestamp_ms_from_unix_nanos(value: &Value) -> Option<i64> {
    // OTel `timeUnixNano` is unsigned-by-spec; a negative or zero value is
    // malformed. Refuse it and let the caller fall through to the next
    // timestamp source (or the file modified time) instead of producing a
    // pre-1970 timestamp downstream.
    value_as_i64(value)
        .filter(|raw| *raw > 0)
        .map(|raw| raw / 1_000_000)
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
    fn test_parse_copilot_chat_span() {
        let content = r#"{"type":"metric","name":"gen_ai.client.token.usage"}
{"type":"span","traceId":"trace-1","spanId":"span-1","name":"chat claude-sonnet-4","startTime":[1775934260,133000000],"endTime":[1775934264,967317833],"attributes":{"gen_ai.operation.name":"chat","gen_ai.request.model":"claude-sonnet-4","gen_ai.response.model":"claude-sonnet-4","gen_ai.conversation.id":"conv-1","gen_ai.usage.input_tokens":19452,"gen_ai.usage.output_tokens":281,"gen_ai.usage.cache_read.input_tokens":123,"gen_ai.usage.reasoning.output_tokens":128,"github.copilot.interaction_id":"interaction-1"}}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "copilot");
        assert_eq!(message.model_id, "claude-sonnet-4");
        assert_eq!(message.provider_id, "anthropic");
        assert_eq!(message.session_id, "conv-1");
        assert_eq!(message.tokens.input, 19_329);
        assert_eq!(message.tokens.output, 281);
        assert_eq!(message.tokens.cache_read, 123);
        assert_eq!(message.tokens.reasoning, 128);
        assert_eq!(message.timestamp, 1_775_934_264_967);
        assert_eq!(message.dedup_key.as_deref(), Some("trace-1:span-1"));
    }

    #[test]
    fn test_parse_copilot_ignores_non_chat_spans() {
        let content = r#"{"type":"span","traceId":"trace-1","spanId":"tool-1","name":"execute_tool rg","attributes":{"gen_ai.operation.name":"execute_tool","gen_ai.tool.name":"rg"}}
{"type":"span","traceId":"trace-1","spanId":"invoke-1","name":"invoke_agent","attributes":{"gen_ai.operation.name":"invoke_agent","gen_ai.usage.input_tokens":999,"gen_ai.usage.output_tokens":111}}
{"type":"span","traceId":"trace-1","spanId":"chat-1","name":"chat gpt-5.4-mini","endTime":[1775934264,967317833],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.usage.input_tokens":10,"gen_ai.usage.output_tokens":5}}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("trace-1:chat-1"));
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 5);
    }

    #[test]
    fn test_parse_copilot_falls_back_to_trace_and_provider() {
        let content = r#"{"type":"span","traceId":"trace-fallback","spanId":"span-fallback","name":"chat custom-model","attributes":{"gen_ai.operation.name":"chat","gen_ai.request.model":"custom-model","gen_ai.usage.input_tokens":"7","gen_ai.usage.output_tokens":"9"}}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "github-copilot");
        assert_eq!(messages[0].session_id, "trace-fallback");
        assert_eq!(messages[0].tokens.input, 7);
        assert_eq!(messages[0].tokens.output, 9);
    }

    #[test]
    fn test_parse_copilot_normalizes_only_cache_read_from_input() {
        let content = r#"{"type":"span","traceId":"trace-cache","spanId":"span-cache","name":"chat gpt-5.4","endTime":[1775934264,967317833],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4","gen_ai.usage.input_tokens":1000,"gen_ai.usage.output_tokens":20,"gen_ai.usage.cache_read.input_tokens":200,"gen_ai.usage.cache_write.input_tokens":50}}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 800);
        assert_eq!(messages[0].tokens.output, 20);
        assert_eq!(messages[0].tokens.cache_read, 200);
        assert_eq!(messages[0].tokens.cache_write, 50);
    }

    #[test]
    fn test_parse_copilot_clamps_only_cache_read_to_input() {
        let content = r#"{"type":"span","traceId":"trace-clamp","spanId":"span-clamp","name":"chat gpt-5.4-mini","endTime":[1775934264,967317833],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.usage.input_tokens":100,"gen_ai.usage.output_tokens":5,"gen_ai.usage.cache_read.input_tokens":90,"gen_ai.usage.cache_write.input_tokens":20}}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.cache_read, 90);
        assert_eq!(messages[0].tokens.cache_write, 20);
    }

    #[test]
    fn test_parse_copilot_keeps_cache_only_message() {
        let content = r#"{"type":"span","traceId":"trace-zero","spanId":"span-zero","name":"chat gpt-5.4-mini","endTime":[1775934264,967317833],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.usage.input_tokens":0,"gen_ai.usage.cache_read.input_tokens":50,"gen_ai.usage.cache_write.input_tokens":20}}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 0);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.cache_write, 20);
    }

    #[test]
    fn test_parse_copilot_keeps_cache_read_when_input_is_missing() {
        let content = r#"{"type":"span","traceId":"trace-cache-read","spanId":"span-cache-read","name":"chat gpt-5.4-mini","endTime":[1775934264,967317833],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.usage.cache_read.input_tokens":50}}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 0);
        assert_eq!(messages[0].tokens.cache_read, 50);
        assert_eq!(messages[0].tokens.cache_write, 0);
    }

    #[test]
    fn test_parse_copilot_vscode_chat_span_without_type() {
        let content = r#"{"resource":{"attributes":{"service.name":"copilot-chat"}},"instrumentationScope":{"name":"copilot-chat","version":"0.44.0"},"traceId":"trace-vscode","spanId":"span-vscode","name":"chat claude-sonnet-4.5","kind":2,"endTime":[1775934264,967317833],"attributes":{"gen_ai.operation.name":"chat","gen_ai.provider.name":"github","gen_ai.request.model":"claude-sonnet-4.5","gen_ai.response.model":"claude-sonnet-4.5","gen_ai.conversation.id":"conv-vscode","gen_ai.usage.input_tokens":1000,"gen_ai.usage.output_tokens":50,"gen_ai.usage.cache_read.input_tokens":200,"gen_ai.usage.cache_creation.input_tokens":75,"gen_ai.usage.reasoning_tokens":12}}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-4.5");
        assert_eq!(messages[0].provider_id, "anthropic");
        assert_eq!(messages[0].session_id, "conv-vscode");
        assert_eq!(messages[0].tokens.input, 800);
        assert_eq!(messages[0].tokens.output, 50);
        assert_eq!(messages[0].tokens.cache_read, 200);
        assert_eq!(messages[0].tokens.cache_write, 75);
        assert_eq!(messages[0].tokens.reasoning, 12);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("trace-vscode:span-vscode")
        );
    }

    #[test]
    fn test_parse_copilot_vscode_inference_log_when_span_is_unavailable() {
        let content = r#"{"hrTime":[1775934264,967317833],"spanContext":{"traceId":"trace-log","spanId":"span-log","traceFlags":1},"instrumentationScope":{"name":"copilot-chat","version":"0.44.0"},"attributes":{"event.name":"gen_ai.client.inference.operation.details","gen_ai.operation.name":"chat","gen_ai.request.model":"gpt-5.4-mini","gen_ai.response.model":"gpt-5.4-mini","gen_ai.response.id":"response-log","gen_ai.usage.input_tokens":42,"gen_ai.usage.output_tokens":7},"_body":"GenAI inference: gpt-5.4-mini"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "gpt-5.4-mini");
        assert_eq!(messages[0].session_id, "response-log");
        assert_eq!(messages[0].tokens.input, 42);
        assert_eq!(messages[0].tokens.output, 7);
        assert_eq!(messages[0].timestamp, 1_775_934_264_967);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("log:trace-log:span-log")
        );
    }

    #[test]
    fn test_parse_copilot_prefers_chat_spans_over_agent_summary() {
        let content = r#"{"traceId":"trace-dupe","spanId":"agent-1","name":"invoke_agent GitHub Copilot Chat","endTime":[1775934270,0],"attributes":{"gen_ai.operation.name":"invoke_agent","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"conv-dupe","gen_ai.usage.input_tokens":100,"gen_ai.usage.output_tokens":30}}
{"traceId":"trace-dupe","spanId":"chat-1","name":"chat gpt-5.4-mini","endTime":[1775934264,967317833],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"conv-dupe","gen_ai.usage.input_tokens":60,"gen_ai.usage.output_tokens":10}}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("trace-dupe:chat-1"));
        assert_eq!(messages[0].tokens.input, 60);
        assert_eq!(messages[0].tokens.output, 10);
    }

    #[test]
    fn test_parse_copilot_agent_turn_log_uses_trace_context_as_last_resort() {
        let content = r#"{"hrTime":[1775934260,0],"spanContext":{"traceId":"trace-turn","spanId":"session-log","traceFlags":1},"attributes":{"event.name":"copilot_chat.session.start","session.id":"conv-turn","gen_ai.request.model":"claude-sonnet-4.5"},"_body":"copilot_chat.session.start"}
{"hrTime":[1775934264,967317833],"spanContext":{"traceId":"trace-turn","spanId":"turn-log","traceFlags":1},"attributes":{"event.name":"copilot_chat.agent.turn","turn.index":3,"gen_ai.usage.input_tokens":120,"gen_ai.usage.output_tokens":9},"_body":"copilot_chat.agent.turn: 3"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-4.5");
        assert_eq!(messages[0].session_id, "conv-turn");
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].tokens.output, 9);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("agent-turn:trace-turn:3")
        );
    }

    #[test]
    fn test_parse_copilot_prefers_chat_span_over_agent_turn_in_same_trace() {
        let content = r#"{"type":"span","traceId":"trace-mix","spanId":"chat-mix","name":"chat gpt-5.4-mini","endTime":[1775934264,967317833],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"conv-mix","gen_ai.usage.input_tokens":50,"gen_ai.usage.output_tokens":8}}
{"hrTime":[1775934265,0],"spanContext":{"traceId":"trace-mix","spanId":"turn-mix","traceFlags":1},"attributes":{"event.name":"copilot_chat.agent.turn","turn.index":1,"gen_ai.usage.input_tokens":50,"gen_ai.usage.output_tokens":8},"_body":"copilot_chat.agent.turn: 1"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].dedup_key.as_deref(), Some("trace-mix:chat-mix"));
        assert_eq!(messages[0].tokens.input, 50);
        assert_eq!(messages[0].tokens.output, 8);
    }

    #[test]
    fn test_parse_copilot_traceless_records_do_not_cross_suppress() {
        // Two traceless records describing distinct OTel responses must both
        // emit even when they share a coarse session attribute (here
        // gen_ai.conversation.id, which spans an entire chat). Cross-source
        // suppression must key on the per-response identifier
        // (gen_ai.response.id), not on chat-wide session attributes.
        let content = r#"{"type":"span","spanId":"chat-traceless","name":"chat gpt-5.4-mini","endTime":[1775934260,0],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"conv-shared","gen_ai.response.id":"resp-A","gen_ai.usage.input_tokens":11,"gen_ai.usage.output_tokens":3}}
{"hrTime":[1775934262,0],"attributes":{"event.name":"gen_ai.client.inference.operation.details","gen_ai.request.model":"gpt-5.4-mini","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"conv-shared","gen_ai.response.id":"resp-B","gen_ai.usage.input_tokens":22,"gen_ai.usage.output_tokens":4},"_body":"GenAI inference: gpt-5.4-mini"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 2);
        let total_input: i64 = messages.iter().map(|m| m.tokens.input).sum();
        let total_output: i64 = messages.iter().map(|m| m.tokens.output).sum();
        assert_eq!(total_input, 33);
        assert_eq!(total_output, 7);
    }

    #[test]
    fn test_parse_copilot_agent_turn_log_without_turn_index_uses_line_index() {
        // Two agent-turn records in the same trace with no turn.index attribute
        // must produce distinct dedup keys (no `0` sentinel collision).
        let content = r#"{"hrTime":[1775934260,0],"spanContext":{"traceId":"trace-noidx","spanId":"turn-a","traceFlags":1},"attributes":{"event.name":"copilot_chat.agent.turn","gen_ai.request.model":"gpt-5.4-mini","gen_ai.usage.input_tokens":10,"gen_ai.usage.output_tokens":2},"_body":"copilot_chat.agent.turn"}
{"hrTime":[1775934261,0],"spanContext":{"traceId":"trace-noidx","spanId":"turn-b","traceFlags":1},"attributes":{"event.name":"copilot_chat.agent.turn","gen_ai.request.model":"gpt-5.4-mini","gen_ai.usage.input_tokens":11,"gen_ai.usage.output_tokens":3},"_body":"copilot_chat.agent.turn"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 2);
        let mut keys: Vec<String> = messages
            .iter()
            .filter_map(|m| m.dedup_key.clone())
            .collect();
        keys.sort();
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0], keys[1], "dedup keys must be unique: {keys:?}");
        for key in &keys {
            assert!(
                key.starts_with("agent-turn:trace-noidx:idx-"),
                "expected line-index fallback shape in {key}",
            );
        }
    }

    #[test]
    fn test_parse_copilot_inference_log_uses_time_unix_nano_timestamp() {
        let content = r#"{"timeUnixNano":1775934264967317833,"spanContext":{"traceId":"trace-nano","spanId":"span-nano","traceFlags":1},"attributes":{"event.name":"gen_ai.client.inference.operation.details","gen_ai.response.model":"gpt-5.4-mini","gen_ai.response.id":"resp-nano","gen_ai.usage.input_tokens":5,"gen_ai.usage.output_tokens":2},"_body":"GenAI inference: gpt-5.4-mini"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].timestamp, 1_775_934_264_967);
    }

    #[test]
    fn test_parse_copilot_agent_turn_log_uses_scalar_timestamp() {
        let content = r#"{"timestamp":1775934264967,"spanContext":{"traceId":"trace-ts","spanId":"turn-ts","traceFlags":1},"attributes":{"event.name":"copilot_chat.agent.turn","turn.index":2,"gen_ai.request.model":"gpt-5.4-mini","gen_ai.usage.input_tokens":7,"gen_ai.usage.output_tokens":1},"_body":"copilot_chat.agent.turn: 2"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].timestamp, 1_775_934_264_967);
    }

    #[test]
    fn test_parse_copilot_mixed_trace_double_count_suppressed_via_response_id() {
        // Mixed-trace gap: a traceless chat span and a traced inference log
        // describe the same OTel response (same gen_ai.response.id). With no
        // shared trace_id, the response-id key is what links them; only the
        // higher-priority chat span should emit.
        let content = r#"{"type":"span","spanId":"chat-mixed","name":"chat gpt-5.4-mini","endTime":[1775934260,0],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"conv-mixed","gen_ai.response.id":"resp-mixed","gen_ai.usage.input_tokens":40,"gen_ai.usage.output_tokens":7}}
{"hrTime":[1775934261,0],"spanContext":{"traceId":"trace-mixed-inf","spanId":"inf-mixed","traceFlags":1},"attributes":{"event.name":"gen_ai.client.inference.operation.details","gen_ai.request.model":"gpt-5.4-mini","gen_ai.response.model":"gpt-5.4-mini","gen_ai.response.id":"resp-mixed","gen_ai.usage.input_tokens":40,"gen_ai.usage.output_tokens":7},"_body":"GenAI inference: gpt-5.4-mini"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "conv-mixed");
        assert_eq!(messages[0].tokens.input, 40);
        assert_eq!(messages[0].tokens.output, 7);
    }

    #[test]
    fn test_parse_copilot_traced_chat_suppresses_traceless_inference_via_response_id() {
        // Inverse of the mixed-trace gap: a traced chat span suppresses a
        // traceless inference log via shared gen_ai.response.id, even though
        // the log carries no trace_id to link it through.
        let content = r#"{"type":"span","traceId":"trace-chat-inv","spanId":"chat-inv","name":"chat gpt-5.4-mini","endTime":[1775934260,0],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"conv-inv","gen_ai.response.id":"resp-inv","gen_ai.usage.input_tokens":33,"gen_ai.usage.output_tokens":5}}
{"hrTime":[1775934261,0],"attributes":{"event.name":"gen_ai.client.inference.operation.details","gen_ai.request.model":"gpt-5.4-mini","gen_ai.response.model":"gpt-5.4-mini","gen_ai.response.id":"resp-inv","gen_ai.usage.input_tokens":33,"gen_ai.usage.output_tokens":5},"_body":"GenAI inference: gpt-5.4-mini"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("trace-chat-inv:chat-inv"),
        );
        assert_eq!(messages[0].tokens.input, 33);
        assert_eq!(messages[0].tokens.output, 5);
    }

    #[test]
    fn test_parse_copilot_inference_log_negative_time_unix_nano_falls_back() {
        // Malformed `timeUnixNano` must not produce a negative timestamp; the
        // parser should fall through to the next available timestamp source
        // (here, the file modified time, which is non-negative).
        let content = r#"{"timeUnixNano":-1,"spanContext":{"traceId":"trace-bad","spanId":"span-bad","traceFlags":1},"attributes":{"event.name":"gen_ai.client.inference.operation.details","gen_ai.response.model":"gpt-5.4-mini","gen_ai.response.id":"resp-bad","gen_ai.usage.input_tokens":5,"gen_ai.usage.output_tokens":2},"_body":"GenAI inference: gpt-5.4-mini"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].timestamp >= 0,
            "negative timeUnixNano should not leak into output, got {}",
            messages[0].timestamp,
        );
    }

    #[test]
    fn test_parse_copilot_interleaved_multi_trace_suppression_is_per_trace() {
        // Two traces interleaved on the wire. Source-priority suppression must
        // be scoped per-trace; both invoke_agent records should be dropped in
        // favor of their own trace's chat span, regardless of line order.
        let content = r#"{"type":"span","traceId":"trace-A","spanId":"agent-A","name":"invoke_agent","endTime":[1775934260,0],"attributes":{"gen_ai.operation.name":"invoke_agent","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"conv-A","gen_ai.usage.input_tokens":100,"gen_ai.usage.output_tokens":30}}
{"type":"span","traceId":"trace-B","spanId":"chat-B","name":"chat gpt-5.4-mini","endTime":[1775934261,0],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"conv-B","gen_ai.usage.input_tokens":50,"gen_ai.usage.output_tokens":8}}
{"type":"span","traceId":"trace-A","spanId":"chat-A","name":"chat gpt-5.4-mini","endTime":[1775934262,0],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"conv-A","gen_ai.usage.input_tokens":40,"gen_ai.usage.output_tokens":6}}
{"type":"span","traceId":"trace-B","spanId":"agent-B","name":"invoke_agent","endTime":[1775934263,0],"attributes":{"gen_ai.operation.name":"invoke_agent","gen_ai.response.model":"gpt-5.4-mini","gen_ai.conversation.id":"conv-B","gen_ai.usage.input_tokens":80,"gen_ai.usage.output_tokens":20}}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 2);
        let mut keys: Vec<String> = messages
            .iter()
            .filter_map(|m| m.dedup_key.clone())
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["trace-A:chat-A".to_string(), "trace-B:chat-B".to_string()],
        );
    }

    #[test]
    fn test_parse_copilot_agent_turn_log_with_top_level_trace_id() {
        // Some VS Code variants emit `traceId` at the top level rather than
        // nested inside `spanContext`. The agent-turn classifier should still
        // resolve the trace and produce a stable per-turn dedup key.
        let content = r#"{"hrTime":[1775934264,0],"traceId":"trace-toplevel","spanId":"turn-toplevel","attributes":{"event.name":"copilot_chat.agent.turn","turn.index":5,"gen_ai.request.model":"claude-sonnet-4.5","gen_ai.usage.input_tokens":15,"gen_ai.usage.output_tokens":4},"_body":"copilot_chat.agent.turn: 5"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-4.5");
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("agent-turn:trace-toplevel:5"),
        );
    }

    #[test]
    fn test_parse_copilot_traced_span_does_not_suppress_traceless_record_with_colliding_session() {
        // A traced chat span has trace_id "T-collide". A separate traceless
        // inference log uses "T-collide" as its session-fallback (gen_ai.response.id).
        // The traceless record must NOT be suppressed by the traced chat span's
        // context_key, because they are unrelated events. Both should emit.
        let content = r#"{"type":"span","traceId":"T-collide","spanId":"chat-traced","name":"chat gpt-5.4-mini","endTime":[1775934260,0],"attributes":{"gen_ai.operation.name":"chat","gen_ai.response.model":"gpt-5.4-mini","gen_ai.usage.input_tokens":10,"gen_ai.usage.output_tokens":2}}
{"hrTime":[1775934261,0],"attributes":{"event.name":"gen_ai.client.inference.operation.details","gen_ai.request.model":"gpt-5.4-mini","gen_ai.response.model":"gpt-5.4-mini","gen_ai.response.id":"T-collide","gen_ai.usage.input_tokens":20,"gen_ai.usage.output_tokens":3},"_body":"GenAI inference: gpt-5.4-mini"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 2);
        let total_input: i64 = messages.iter().map(|m| m.tokens.input).sum();
        let total_output: i64 = messages.iter().map(|m| m.tokens.output).sum();
        assert_eq!(total_input, 30);
        assert_eq!(total_output, 5);
    }

    #[test]
    fn test_parse_copilot_trace_context_prefers_session_id_over_response_id() {
        let content = r#"{"hrTime":[1775934260,0],"spanContext":{"traceId":"trace-session-upgrade","spanId":"response-log","traceFlags":1},"attributes":{"event.name":"gen_ai.client.inference.operation.details","gen_ai.response.id":"response-scoped-id","gen_ai.request.model":"claude-sonnet-4.5"},"_body":"GenAI inference: claude-sonnet-4.5"}
{"hrTime":[1775934261,0],"spanContext":{"traceId":"trace-session-upgrade","spanId":"session-log","traceFlags":1},"attributes":{"event.name":"copilot_chat.session.start","session.id":"durable-session-id"},"_body":"copilot_chat.session.start"}
{"hrTime":[1775934264,967317833],"spanContext":{"traceId":"trace-session-upgrade","spanId":"turn-log","traceFlags":1},"attributes":{"event.name":"copilot_chat.agent.turn","turn.index":4,"gen_ai.usage.input_tokens":120,"gen_ai.usage.output_tokens":9},"_body":"copilot_chat.agent.turn: 4"}"#;
        let file = create_test_file(content);

        let messages = parse_copilot_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "claude-sonnet-4.5");
        assert_eq!(messages[0].session_id, "durable-session-id");
        assert_eq!(messages[0].tokens.input, 120);
        assert_eq!(messages[0].tokens.output, 9);
        assert_eq!(
            messages[0].dedup_key.as_deref(),
            Some("agent-turn:trace-session-upgrade:4")
        );
    }
}
