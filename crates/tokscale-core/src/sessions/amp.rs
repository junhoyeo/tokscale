//! Amp (Sourcegraph) session parser
//!
//! Parses JSON files from ~/.local/share/amp/threads/

use super::utils::read_file_or_none;
use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use std::path::Path;

/// Amp usage event from usageLedger
#[derive(Debug, Deserialize)]
pub struct AmpUsageEvent {
    pub timestamp: Option<String>,
    pub model: Option<String>,
    pub credits: Option<f64>,
    pub tokens: Option<AmpTokens>,
    #[serde(rename = "operationType")]
    pub _operation_type: Option<String>,
    #[serde(rename = "fromMessageId")]
    pub _from_message_id: Option<i64>,
    #[serde(rename = "toMessageId")]
    pub to_message_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AmpTokens {
    pub input: Option<i64>,
    pub output: Option<i64>,
    #[serde(rename = "cacheReadInputTokens")]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(rename = "cacheCreationInputTokens")]
    pub cache_creation_input_tokens: Option<i64>,
}

/// Amp message usage (per-message, more detailed)
#[derive(Debug, Deserialize)]
pub struct AmpMessageUsage {
    pub model: Option<String>,
    #[serde(rename = "inputTokens")]
    pub input_tokens: Option<i64>,
    #[serde(rename = "outputTokens")]
    pub output_tokens: Option<i64>,
    #[serde(rename = "cacheReadInputTokens")]
    pub cache_read_input_tokens: Option<i64>,
    #[serde(rename = "cacheCreationInputTokens")]
    pub cache_creation_input_tokens: Option<i64>,
    pub credits: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct AmpMessage {
    pub role: Option<String>,
    #[serde(rename = "messageId")]
    pub message_id: Option<i64>,
    pub usage: Option<AmpMessageUsage>,
}

#[derive(Debug, Deserialize)]
pub struct AmpUsageLedger {
    pub events: Option<Vec<AmpUsageEvent>>,
}

#[derive(Debug, Deserialize)]
pub struct AmpThread {
    pub id: Option<String>,
    pub created: Option<i64>,
    pub messages: Option<Vec<AmpMessage>>,
    #[serde(rename = "usageLedger")]
    pub usage_ledger: Option<AmpUsageLedger>,
}

/// Get provider from model name
fn get_provider_from_model(model: &str) -> &'static str {
    provider_identity::inferred_provider_from_model(model).unwrap_or("anthropic")
}

#[derive(Debug, Clone)]
struct AmpUsageRecord {
    model: String,
    timestamp: i64,
    has_explicit_timestamp: bool,
    message_id: Option<i64>,
    ledger_to_message_id: Option<i64>,
    tokens: TokenBreakdown,
    cost: f64,
}

impl AmpUsageRecord {
    fn matches_message_usage(&self, other: &Self) -> bool {
        self.model == other.model && self.tokens == other.tokens
    }

    fn into_unified(self, thread_id: &str) -> UnifiedMessage {
        UnifiedMessage::new(
            "amp",
            &self.model,
            get_provider_from_model(&self.model),
            thread_id.to_string(),
            self.timestamp,
            self.tokens,
            self.cost,
        )
    }
}

fn parse_amp_timestamp(timestamp: Option<String>) -> Option<i64> {
    timestamp
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
        .map(|dt| dt.timestamp_millis())
        .filter(|timestamp| *timestamp != 0)
}

fn fallback_amp_timestamp(
    explicit: Option<i64>,
    thread_created_ms: i64,
    file_mtime_ms: i64,
) -> i64 {
    explicit
        .filter(|timestamp| *timestamp != 0)
        .or_else(|| (thread_created_ms != 0).then_some(thread_created_ms))
        .unwrap_or(file_mtime_ms)
}

fn parse_amp_ledger_records(
    usage_ledger: Option<AmpUsageLedger>,
    thread_created_ms: i64,
    file_mtime_ms: i64,
) -> Vec<AmpUsageRecord> {
    let Some(ledger) = usage_ledger else {
        return Vec::new();
    };
    let Some(events) = ledger.events else {
        return Vec::new();
    };

    events
        .into_iter()
        .filter_map(|event| {
            let model = event.model?;
            let explicit_timestamp = parse_amp_timestamp(event.timestamp);
            let timestamp =
                fallback_amp_timestamp(explicit_timestamp, thread_created_ms, file_mtime_ms);
            let tokens = event.tokens.unwrap_or(AmpTokens {
                input: Some(0),
                output: Some(0),
                cache_read_input_tokens: Some(0),
                cache_creation_input_tokens: Some(0),
            });

            Some(AmpUsageRecord {
                model,
                timestamp,
                has_explicit_timestamp: explicit_timestamp.is_some(),
                message_id: None,
                ledger_to_message_id: event.to_message_id.filter(|id| *id > 0),
                tokens: TokenBreakdown {
                    input: tokens.input.unwrap_or(0).max(0),
                    output: tokens.output.unwrap_or(0).max(0),
                    cache_read: tokens.cache_read_input_tokens.unwrap_or(0).max(0),
                    cache_write: tokens.cache_creation_input_tokens.unwrap_or(0).max(0),
                    reasoning: 0,
                },
                cost: event.credits.unwrap_or(0.0).max(0.0),
            })
        })
        .collect()
}

fn parse_amp_message_records(
    thread_messages: Option<Vec<AmpMessage>>,
    thread_created_ms: i64,
    file_mtime_ms: i64,
) -> Vec<AmpUsageRecord> {
    let Some(thread_messages) = thread_messages else {
        return Vec::new();
    };

    let base_timestamp = if thread_created_ms != 0 {
        thread_created_ms
    } else {
        file_mtime_ms
    };

    thread_messages
        .into_iter()
        .filter_map(|msg| {
            if msg.role.as_deref() != Some("assistant") {
                return None;
            }

            let usage = msg.usage?;
            let model = usage.model?;
            let message_id = msg.message_id.unwrap_or(0).max(0);
            let timestamp = base_timestamp.saturating_add(message_id.saturating_mul(1000));

            Some(AmpUsageRecord {
                model,
                timestamp,
                has_explicit_timestamp: false,
                message_id: Some(message_id).filter(|id| *id > 0),
                ledger_to_message_id: None,
                tokens: TokenBreakdown {
                    input: usage.input_tokens.unwrap_or(0).max(0),
                    output: usage.output_tokens.unwrap_or(0).max(0),
                    cache_read: usage.cache_read_input_tokens.unwrap_or(0).max(0),
                    cache_write: usage.cache_creation_input_tokens.unwrap_or(0).max(0),
                    reasoning: 0,
                },
                cost: usage.credits.unwrap_or(0.0).max(0.0),
            })
        })
        .collect()
}

fn find_matching_ledger_record(
    ledger_records: &[AmpUsageRecord],
    consumed: &[bool],
    search_start: usize,
    message_record: &AmpUsageRecord,
) -> Option<usize> {
    let find_match = |predicate: &dyn Fn(usize) -> bool| {
        (search_start..ledger_records.len())
            .find(|&index| predicate(index))
            .or_else(|| (0..search_start).find(|&index| predicate(index)))
    };

    if let Some(message_id) = message_record.message_id {
        if let Some(index) = find_match(&|index| {
            !consumed[index] && ledger_records[index].ledger_to_message_id == Some(message_id)
        }) {
            return Some(index);
        }
    }

    find_match(&|index| {
        !consumed[index] && ledger_records[index].matches_message_usage(message_record)
    })
}

fn merge_amp_records(
    ledger_record: AmpUsageRecord,
    message_record: &AmpUsageRecord,
) -> AmpUsageRecord {
    if ledger_record.has_explicit_timestamp {
        if ledger_record.cost > 0.0 || message_record.cost <= 0.0 {
            ledger_record
        } else {
            AmpUsageRecord {
                cost: message_record.cost,
                message_id: message_record.message_id,
                ..ledger_record
            }
        }
    } else {
        AmpUsageRecord {
            model: ledger_record.model,
            timestamp: message_record.timestamp,
            has_explicit_timestamp: false,
            message_id: message_record.message_id,
            ledger_to_message_id: ledger_record.ledger_to_message_id,
            tokens: ledger_record.tokens,
            cost: if ledger_record.cost > 0.0 {
                ledger_record.cost
            } else {
                message_record.cost
            },
        }
    }
}

/// Parse an Amp thread JSON file
pub fn parse_amp_file(path: &Path) -> Vec<UnifiedMessage> {
    let Some(content) = read_file_or_none(path) else {
        return Vec::new();
    };

    // Get file mtime as last-resort timestamp fallback
    let file_mtime_ms = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let mut bytes = content;
    let thread: AmpThread = match simd_json::from_slice(&mut bytes) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let thread_id = thread.id.clone().unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    });

    let thread_created_ms = thread.created.unwrap_or(0);
    let mut ledger_records =
        parse_amp_ledger_records(thread.usage_ledger, thread_created_ms, file_mtime_ms);
    let message_records =
        parse_amp_message_records(thread.messages, thread_created_ms, file_mtime_ms);

    if ledger_records.is_empty() {
        let mut message_records = message_records;
        message_records.sort_by_key(|record| record.timestamp);
        return message_records
            .into_iter()
            .map(|record| record.into_unified(&thread_id))
            .collect();
    }

    let mut consumed = vec![false; ledger_records.len()];
    let mut search_start = 0usize;
    let mut unmatched_message_records = Vec::new();

    for message_record in &message_records {
        if let Some(index) =
            find_matching_ledger_record(&ledger_records, &consumed, search_start, message_record)
        {
            consumed[index] = true;
            search_start = index.saturating_add(1);
            let merged = merge_amp_records(ledger_records[index].clone(), message_record);
            ledger_records[index] = merged;
        } else {
            unmatched_message_records.push(message_record.clone());
        }
    }

    ledger_records.extend(unmatched_message_records);
    ledger_records.sort_by_key(|record| record.timestamp);
    ledger_records
        .into_iter()
        .map(|record| record.into_unified(&thread_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_amp_file;
    use std::path::Path;

    fn write_amp_thread(path: &Path, content: &str) {
        std::fs::write(path, content).unwrap();
    }

    fn timestamp_ms(value: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp_millis()
    }

    fn local_date(timestamp_ms: i64) -> String {
        use chrono::TimeZone;

        chrono::Local
            .timestamp_millis_opt(timestamp_ms)
            .single()
            .unwrap()
            .format("%Y-%m-%d")
            .to_string()
    }

    #[test]
    fn test_parse_amp_reconciles_partial_ledger_with_message_usage() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = temp_dir.path().join("T-partial.json");
        let thread_created = timestamp_ms("2026-04-04T12:00:00Z");
        let ledger_timestamp = "2026-04-08T12:00:00Z";

        write_amp_thread(
            &path,
            &serde_json::json!({
                "id": "thread-partial",
                "created": thread_created,
                "usageLedger": {
                    "events": [
                        {
                            "timestamp": ledger_timestamp,
                            "model": "claude-sonnet-4-0",
                            "credits": 0.75,
                            "tokens": { "input": 100, "output": 20 }
                        }
                    ]
                },
                "messages": [
                    {
                        "role": "assistant",
                        "messageId": 1,
                        "usage": {
                            "model": "claude-sonnet-4-0",
                            "inputTokens": 100,
                            "outputTokens": 20,
                            "credits": 0.75
                        }
                    },
                    {
                        "role": "assistant",
                        "messageId": 2,
                        "usage": {
                            "model": "claude-sonnet-4-0",
                            "inputTokens": 50,
                            "outputTokens": 10,
                            "credits": 0.40
                        }
                    }
                ]
            })
            .to_string(),
        );

        let messages = parse_amp_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].date, local_date(thread_created + 2000));
        assert_eq!(messages[1].date, local_date(timestamp_ms(ledger_timestamp)));
        assert_eq!(messages[0].tokens.input, 50);
        assert_eq!(messages[1].tokens.input, 100);
    }

    #[test]
    fn test_parse_amp_does_not_double_count_full_ledger() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = temp_dir.path().join("T-full.json");
        let thread_created = timestamp_ms("2026-04-04T12:00:00Z");
        let first_ledger_timestamp = "2026-04-04T12:00:00Z";
        let second_ledger_timestamp = "2026-04-05T12:00:00Z";

        write_amp_thread(
            &path,
            &serde_json::json!({
                "id": "thread-full",
                "created": thread_created,
                "usageLedger": {
                    "events": [
                        {
                            "timestamp": first_ledger_timestamp,
                            "model": "claude-sonnet-4-0",
                            "credits": 0.20,
                            "tokens": { "input": 20, "output": 5 }
                        },
                        {
                            "timestamp": second_ledger_timestamp,
                            "model": "claude-sonnet-4-0",
                            "credits": 0.25,
                            "tokens": { "input": 25, "output": 5 }
                        }
                    ]
                },
                "messages": [
                    {
                        "role": "assistant",
                        "messageId": 1,
                        "usage": {
                            "model": "claude-sonnet-4-0",
                            "inputTokens": 20,
                            "outputTokens": 5,
                            "credits": 0.20
                        }
                    },
                    {
                        "role": "assistant",
                        "messageId": 2,
                        "usage": {
                            "model": "claude-sonnet-4-0",
                            "inputTokens": 25,
                            "outputTokens": 5,
                            "credits": 0.25
                        }
                    }
                ]
            })
            .to_string(),
        );

        let messages = parse_amp_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].date,
            local_date(timestamp_ms(first_ledger_timestamp))
        );
        assert_eq!(
            messages[1].date,
            local_date(timestamp_ms(second_ledger_timestamp))
        );
    }

    #[test]
    fn test_parse_amp_prefers_message_id_match_over_token_heuristic() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = temp_dir.path().join("T-message-id-match.json");
        let thread_created = timestamp_ms("2026-04-04T12:00:00Z");
        let first_ledger_timestamp = "2026-04-10T12:00:00Z";
        let second_ledger_timestamp = "2026-04-05T12:00:00Z";

        write_amp_thread(
            &path,
            &serde_json::json!({
                "id": "thread-message-id-match",
                "created": thread_created,
                "usageLedger": {
                    "events": [
                        {
                            "timestamp": first_ledger_timestamp,
                            "model": "claude-sonnet-4-0",
                            "credits": 0.20,
                            "tokens": { "input": 20, "output": 5 },
                            "toMessageId": 2
                        },
                        {
                            "timestamp": second_ledger_timestamp,
                            "model": "claude-sonnet-4-0",
                            "credits": 0.20,
                            "tokens": { "input": 20, "output": 5 },
                            "toMessageId": 1
                        }
                    ]
                },
                "messages": [
                    {
                        "role": "assistant",
                        "messageId": 1,
                        "usage": {
                            "model": "claude-sonnet-4-0",
                            "inputTokens": 20,
                            "outputTokens": 5,
                            "credits": 0.20
                        }
                    },
                    {
                        "role": "assistant",
                        "messageId": 2,
                        "usage": {
                            "model": "claude-sonnet-4-0",
                            "inputTokens": 20,
                            "outputTokens": 5,
                            "credits": 0.20
                        }
                    }
                ]
            })
            .to_string(),
        );

        let messages = parse_amp_file(&path);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].timestamp, timestamp_ms(second_ledger_timestamp));
        assert_eq!(messages[1].timestamp, timestamp_ms(first_ledger_timestamp));
    }

    #[test]
    fn test_parse_amp_prefers_message_timestamp_when_ledger_timestamp_missing() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = temp_dir.path().join("T-missing-ledger-ts.json");
        let thread_created = timestamp_ms("2026-04-04T12:00:00Z");

        write_amp_thread(
            &path,
            &serde_json::json!({
                "id": "thread-missing-ts",
                "created": thread_created,
                "usageLedger": {
                    "events": [
                        {
                            "model": "claude-sonnet-4-0",
                            "credits": 0.20,
                            "tokens": { "input": 20, "output": 5 }
                        }
                    ]
                },
                "messages": [
                    {
                        "role": "assistant",
                        "messageId": 7,
                        "usage": {
                            "model": "claude-sonnet-4-0",
                            "inputTokens": 20,
                            "outputTokens": 5,
                            "credits": 0.20
                        }
                    }
                ]
            })
            .to_string(),
        );

        let messages = parse_amp_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].timestamp, thread_created + 7000);
    }

    #[test]
    fn test_parse_amp_uses_file_mtime_when_thread_created_missing() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = temp_dir.path().join("T-no-created.json");

        write_amp_thread(
            &path,
            r#"{
                "id": "thread-no-created",
                "messages": [
                    {
                        "role": "assistant",
                        "messageId": 5,
                        "usage": {
                            "model": "claude-sonnet-4-0",
                            "inputTokens": 10,
                            "outputTokens": 2,
                            "credits": 0.11
                        }
                    }
                ]
            }"#,
        );

        let file_mtime_ms = crate::sessions::utils::file_modified_timestamp_ms(&path);
        let messages = parse_amp_file(&path);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].timestamp >= file_mtime_ms);
        assert_ne!(messages[0].date, "1970-01-01");
    }
}
