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
        // Cheap pre-filter only: Junie state snapshots can be very large and do
        // not carry the usage rows Tokscale needs, so skip lines that mention
        // neither relevant kind before paying for JSON parsing. The authoritative
        // skip decision is made on the parsed event kind below.
        if !line.contains(USAGE_EVENT_KIND) && !line.contains(USER_PROMPT_KIND) {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // Skip noise events by matching the parsed event kind, not a raw substring
        // search: a legitimate usage/prompt line may merely *mention* a skipped
        // kind in its text and must not be dropped.
        if let Some(kind) = parsed_event_kind(&value) {
            if SKIP_EVENT_KINDS.contains(&kind) {
                continue;
            }
        }
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

        let mut turn_start_assigned = false;
        for (usage_index, usage) in usages.iter().enumerate() {
            // The uniqueness suffix is the row's position *within this event's*
            // `modelUsage` array, not a file-global counter. This keeps the
            // dedup key derived from the event itself so a replayed identical
            // event reproduces the same key (and is collapsed by the in-file
            // `seen` set and the cross-file `should_keep_deduped_message`
            // filter), while multiple distinct rows inside one event still get
            // distinct indices. Genuinely-distinct LLM calls differ in their
            // `timestampMs` (already in the key), so they stay separate.
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
            if pending_turn_start && !turn_start_assigned {
                message.is_turn_start = true;
                turn_start_assigned = true;
            }
            messages.push(message);
        }
        // The prompt's turn-start is consumed by the first usage event that
        // follows it. Clear it here — once per usage event — so a prompt that
        // produced no counted usage does not leak `is_turn_start` onto a later,
        // unrelated turn's usage event.
        pending_turn_start = false;
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

/// Resolve the event's kind from the parsed JSON, preferring the top-level
/// `kind` and falling back to the nested `event.agentEvent.kind`. Used to make
/// skip decisions on the actual event type rather than a raw substring search.
fn parsed_event_kind(value: &Value) -> Option<&str> {
    event_kind(value).or_else(|| {
        value
            .pointer("/event/agentEvent")
            .and_then(|event| string_field(event, "kind"))
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    /// Write the given JSONL `content` to `events.jsonl` inside a session
    /// directory whose name drives `session_id_from_path`, then parse it.
    fn parse_events(content: &str) -> Vec<UnifiedMessage> {
        let dir = TempDir::new().unwrap();
        let session_dir = dir.path().join("session-250622-101010");
        std::fs::create_dir_all(&session_dir).unwrap();
        let path = session_dir.join("events.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        parse_junie_file(&path)
    }

    fn usage_event(timestamp_ms: i64, model: &str, input: i64, output: i64) -> String {
        format!(
            r#"{{"timestampMs":{timestamp_ms},"event":{{"agentEvent":{{"kind":"LlmResponseMetadataEvent","modelUsage":[{{"model":"{model}","inputTokens":{input},"outputTokens":{output}}}]}}}}}}"#
        )
    }

    #[test]
    fn distinct_usage_rows_with_identical_tokens_are_both_counted() {
        // Two separate LLM response events with identical token counts but
        // distinct `timestampMs` (the realistic shape of #727: back-to-back
        // calls returning the same usage). Both must be counted. The original
        // #727 bug dropped the second because the per-`modelUsage` index reset
        // to 0; here the differing timestamp keeps the keys distinct.
        let content = format!(
            "{}\n{}\n",
            usage_event(1_750_000_000_000, "gpt-5", 100, 50),
            usage_event(1_750_000_001_000, "gpt-5", 100, 50),
        );
        let messages = parse_events(&content);

        assert_eq!(
            messages.len(),
            2,
            "both distinct calls with identical token counts must be counted"
        );
        for message in &messages {
            assert_eq!(message.tokens.input, 100);
            assert_eq!(message.tokens.output, 50);
        }
        assert_ne!(
            messages[0].dedup_key, messages[1].dedup_key,
            "distinct usage rows must receive distinct dedup keys"
        );
    }

    #[test]
    fn replayed_identical_event_is_deduplicated_to_one() {
        // Junie can append/replay the exact same `LlmResponseMetadataEvent`.
        // A byte-for-byte replayed event (same timestamp, model, and tokens)
        // must collapse to a single counted row — otherwise the same tokens
        // and cost are double-counted. The dedup suffix is derived from the
        // event's own within-array index, so the replay reproduces the same
        // dedup key and is dropped by the `seen` set.
        let content = format!(
            "{}\n{}\n",
            usage_event(1_750_000_000_000, "gpt-5", 100, 50),
            usage_event(1_750_000_000_000, "gpt-5", 100, 50),
        );
        let messages = parse_events(&content);

        assert_eq!(
            messages.len(),
            1,
            "a replayed identical usage event must collapse to a single row"
        );
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);
    }

    #[test]
    fn identical_rows_within_one_event_are_both_counted() {
        // Multiple identical rows inside a single `modelUsage` array must also
        // each survive: they get distinct within-event indices (0 and 1).
        let content = "{\"timestampMs\":1750000000000,\"event\":{\"agentEvent\":{\"kind\":\"LlmResponseMetadataEvent\",\"modelUsage\":[{\"model\":\"gpt-5\",\"inputTokens\":100,\"outputTokens\":50},{\"model\":\"gpt-5\",\"inputTokens\":100,\"outputTokens\":50}]}}}\n";
        let messages = parse_events(content);
        assert_eq!(messages.len(), 2);
        assert_ne!(messages[0].dedup_key, messages[1].dedup_key);
    }

    #[test]
    fn pending_turn_start_does_not_leak_when_prompt_yields_no_usage() {
        // Prompt A opens a turn but its response event carries no counted usage
        // (zero tokens). Prompt B then opens its own turn with real usage. The
        // turn-start must attach to B's usage, and the empty A response must not
        // leak the flag onto an unrelated later usage event.
        let empty_usage = r#"{"timestampMs":1750000000000,"event":{"agentEvent":{"kind":"LlmResponseMetadataEvent","modelUsage":[{"model":"gpt-5","inputTokens":0,"outputTokens":0}]}}}"#;
        let content = format!(
            "{}\n{}\n{}\n{}\n",
            r#"{"kind":"UserPromptEvent"}"#,
            empty_usage,
            r#"{"kind":"UserPromptEvent"}"#,
            usage_event(1_750_000_100_000, "gpt-5", 100, 50),
        );
        let messages = parse_events(&content);

        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].is_turn_start,
            "turn-start should attach to prompt B's real usage"
        );
    }

    #[test]
    fn turn_start_marks_only_the_first_usage_after_a_prompt() {
        let content = format!(
            "{}\n{}\n{}\n",
            r#"{"kind":"UserPromptEvent"}"#,
            usage_event(1_750_000_000_000, "gpt-5", 100, 50),
            usage_event(1_750_000_100_000, "gpt-5", 200, 60),
        );
        let messages = parse_events(&content);

        assert_eq!(messages.len(), 2);
        assert!(messages[0].is_turn_start);
        assert!(
            !messages[1].is_turn_start,
            "only the first usage event after a prompt is a turn-start"
        );
    }

    #[test]
    fn usage_line_mentioning_skipped_kind_is_not_dropped() {
        // The user prompt text legitimately mentions a skipped kind name; the
        // following usage event must still be counted because the skip decision
        // is made on the parsed event kind, not a raw substring match.
        let content = format!(
            "{}\n{}\n",
            r#"{"kind":"UserPromptEvent","prompt":"please review the AgentStateUpdatedEvent handling"}"#,
            usage_event(1_750_000_000_000, "gpt-5", 100, 50),
        );
        let messages = parse_events(&content);

        assert_eq!(
            messages.len(),
            1,
            "a usage event must not be dropped just because a prior line mentioned a skipped kind"
        );
        assert!(messages[0].is_turn_start);
    }

    #[test]
    fn skipped_event_kind_is_ignored() {
        let content = format!(
            "{}\n{}\n",
            r#"{"kind":"AgentStateUpdatedEvent","event":{"agentEvent":{"kind":"LlmResponseMetadataEvent","modelUsage":[{"model":"gpt-5","inputTokens":100,"outputTokens":50}]}}}"#,
            usage_event(1_750_000_000_000, "gpt-5", 100, 50),
        );
        let messages = parse_events(&content);
        // Only the genuine usage event counts; the snapshot tagged with a
        // skipped top-level kind is ignored even though it embeds a usage shape.
        assert_eq!(messages.len(), 1);
    }
}
