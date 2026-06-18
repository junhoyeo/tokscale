//! Junie session parser
//!
//! Junie stores local sessions under `~/.junie/sessions/<session-id>/events.jsonl`.

use super::utils::file_modified_timestamp_ms;
use super::UnifiedMessage;
use crate::{pricing, provider_identity, TokenBreakdown};
use chrono::{Local, LocalResult, NaiveDateTime, TimeZone};
use serde_json::Value;
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;

const USAGE_EVENT_KIND: &str = "LlmResponseMetadataEvent";
const USER_PROMPT_KIND: &str = "UserPromptEvent";
const SKIP_EVENT_KINDS: &[&str] = &[
    "AgentStateUpdatedEvent",
    "AgentCurrentStatusUpdatedEvent",
    "AgentPatchCreatedEvent",
];

pub fn parse_junie_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };

    let session_id = session_id_from_path(path);
    let default_timestamp =
        session_timestamp_from_id(&session_id).unwrap_or_else(|| file_modified_timestamp_ms(path));
    let mut pending_turn_start = false;
    let mut messages = Vec::new();
    let mut seen = HashSet::new();

    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            continue;
        };
        // Junie state snapshots can be very large and do not carry the usage
        // rows Tokscale needs; skip them before JSON parsing.
        if SKIP_EVENT_KINDS.iter().any(|kind| line.contains(kind)) {
            continue;
        }
        if !line.contains(USAGE_EVENT_KIND) && !line.contains(USER_PROMPT_KIND) {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if event_kind(&value) == Some(USER_PROMPT_KIND) {
            pending_turn_start = true;
            continue;
        }

        let Some(agent_event) = value
            .pointer("/event/agentEvent")
            .filter(|event| string_field(event, "kind") == Some(USAGE_EVENT_KIND))
        else {
            continue;
        };

        let timestamp = number_field(&value, "timestampMs")
            .filter(|timestamp| *timestamp > 0)
            .unwrap_or(default_timestamp);
        let agent = agent_name(agent_event);
        let Some(usages) = agent_event.get("modelUsage").and_then(Value::as_array) else {
            continue;
        };

        for (usage_index, usage) in usages.iter().enumerate() {
            let Some(model_raw) = string_field(usage, "model") else {
                continue;
            };
            let model_id = pricing::aliases::resolve_alias(model_raw)
                .unwrap_or(model_raw)
                .to_string();
            let provider_id = provider_from_usage(usage, &model_id);
            let tokens = tokens_from_usage(usage);
            let cost = float_field(usage, "cost")
                .filter(|cost| cost.is_finite() && *cost >= 0.0)
                .unwrap_or(0.0);
            if tokens.total() == 0 && cost == 0.0 {
                continue;
            }

            let dedup_key = format!(
                "junie:{session_id}:{timestamp}:{model_id}:{}:{}:{}:{}:{}:{:.12}:{usage_index}",
                tokens.input,
                tokens.output,
                tokens.cache_read,
                tokens.cache_write,
                tokens.reasoning,
                cost
            );
            if !seen.insert(dedup_key.clone()) {
                continue;
            }

            let mut message = UnifiedMessage::new_with_agent(
                "junie",
                model_id,
                provider_id,
                &session_id,
                timestamp,
                tokens,
                cost,
                agent.clone(),
            );
            message.dedup_key = Some(dedup_key);
            message.duration_ms = number_field(usage, "time").filter(|duration| *duration > 0);
            if pending_turn_start {
                message.is_turn_start = true;
                pending_turn_start = false;
            }
            messages.push(message);
        }
    }

    messages
}

fn session_id_from_path(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn session_timestamp_from_id(session_id: &str) -> Option<i64> {
    let mut parts = session_id.split('-');
    if parts.next()? != "session" {
        return None;
    }
    let date = parts.next()?;
    let time = parts.next()?;
    if date.len() != 6
        || time.len() != 6
        || !date.bytes().all(|byte| byte.is_ascii_digit())
        || !time.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let naive = NaiveDateTime::parse_from_str(&format!("{date}{time}"), "%y%m%d%H%M%S").ok()?;
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(datetime) => Some(datetime.timestamp_millis()),
        LocalResult::Ambiguous(earliest, _) => Some(earliest.timestamp_millis()),
        LocalResult::None => None,
    }
}

fn event_kind(value: &Value) -> Option<&str> {
    string_field(value, "kind")
}

fn agent_name(agent_event: &Value) -> Option<String> {
    let agent = agent_event.get("agent")?;
    string_field(agent, "name")
        .or_else(|| string_field(agent, "id"))
        .map(str::to_string)
}

fn provider_from_usage(usage: &Value, model_id: &str) -> String {
    string_field(usage, "provider")
        .and_then(provider_identity::canonical_provider)
        .or_else(|| provider_identity::inferred_provider_from_model(model_id).map(str::to_string))
        .unwrap_or_else(|| "junie".to_string())
}

fn tokens_from_usage(usage: &Value) -> TokenBreakdown {
    TokenBreakdown {
        input: first_number_field(usage, &["inputTokens", "input"]),
        output: first_number_field(usage, &["outputTokens", "output"]),
        cache_read: first_number_field(
            usage,
            &["cacheInputTokens", "cacheReadInputTokens", "cacheRead"],
        ),
        cache_write: first_number_field(
            usage,
            &[
                "cacheCreateTokens",
                "cacheCreationInputTokens",
                "cacheWrite",
            ],
        ),
        reasoning: first_number_field(
            usage,
            &["reasoningTokens", "reasoningOutputTokens", "thinkingTokens"],
        ),
    }
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn first_number_field(value: &Value, fields: &[&str]) -> i64 {
    fields
        .iter()
        .find_map(|field| number_field(value, field))
        .unwrap_or(0)
}

fn number_field(value: &Value, field: &str) -> Option<i64> {
    number_value(value.get(field)?)
}

fn number_value(value: &Value) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return Some(value.max(0));
    }
    if let Some(value) = value.as_u64() {
        return Some(value.min(i64::MAX as u64) as i64);
    }
    if let Some(value) = value.as_f64() {
        return value.is_finite().then_some(value.max(0.0) as i64);
    }
    value
        .as_str()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .and_then(|value| value.is_finite().then_some(value.max(0.0) as i64))
}

fn float_field(value: &Value, field: &str) -> Option<f64> {
    let value = value.get(field)?;
    if let Some(number) = value.as_f64() {
        return Some(number);
    }
    value.as_str()?.trim().parse().ok()
}
