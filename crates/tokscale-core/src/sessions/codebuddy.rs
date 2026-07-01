//! CodeBuddy CLI session parser.
//!
//! CodeBuddy stores per-project JSONL transcripts under
//! `~/.codebuddy/projects/<project-key>/*.jsonl`. Completed assistant
//! messages carry usage in `message.usage`, with provider fallbacks under
//! `providerData.usage` / `providerData.rawUsage`.

use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

const DEFAULT_MODEL: &str = "codebuddy";
const DEFAULT_PROVIDER: &str = "tencent";

#[derive(Debug, Deserialize)]
struct CodeBuddyLine {
    id: Option<String>,
    timestamp: Option<i64>,
    #[serde(rename = "type")]
    line_type: Option<String>,
    role: Option<String>,
    status: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    message: Option<CodeBuddyMessage>,
    #[serde(rename = "providerData")]
    provider_data: Option<CodeBuddyProviderData>,
}

#[derive(Debug, Deserialize)]
struct CodeBuddyMessage {
    model: Option<String>,
    usage: Option<CodeBuddyUsage>,
}

#[derive(Debug, Deserialize)]
struct CodeBuddyProviderData {
    model: Option<String>,
    #[serde(rename = "requestModelId")]
    request_model_id: Option<String>,
    #[serde(rename = "messageId")]
    message_id: Option<String>,
    #[serde(rename = "traceId")]
    trace_id: Option<String>,
    usage: Option<CodeBuddyUsage>,
    #[serde(rename = "rawUsage")]
    raw_usage: Option<CodeBuddyUsage>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodeBuddyUsage {
    #[serde(rename = "input_tokens")]
    input_tokens: Option<i64>,
    #[serde(rename = "inputTokens")]
    input_tokens_camel: Option<i64>,
    prompt_tokens: Option<i64>,
    #[serde(rename = "output_tokens")]
    output_tokens: Option<i64>,
    #[serde(rename = "outputTokens")]
    output_tokens_camel: Option<i64>,
    completion_tokens: Option<i64>,
    #[serde(rename = "cache_read_input_tokens")]
    cache_read_input_tokens: Option<i64>,
    #[serde(rename = "cacheReadInputTokens")]
    cache_read_input_tokens_camel: Option<i64>,
    prompt_cache_hit_tokens: Option<i64>,
    cached_tokens: Option<i64>,
    #[serde(rename = "cache_creation_input_tokens")]
    cache_creation_input_tokens: Option<i64>,
    #[serde(rename = "cacheCreationInputTokens")]
    cache_creation_input_tokens_camel: Option<i64>,
    prompt_cache_write_tokens: Option<i64>,
    #[serde(rename = "completion_thinking_tokens")]
    completion_thinking_tokens: Option<i64>,
    #[serde(rename = "completionThinkingTokens")]
    completion_thinking_tokens_camel: Option<i64>,
}

impl CodeBuddyUsage {
    fn to_breakdown(&self) -> Option<TokenBreakdown> {
        let tokens = TokenBreakdown {
            input: first_present(&[
                self.input_tokens,
                self.input_tokens_camel,
                self.prompt_tokens,
            ]),
            output: first_present(&[
                self.output_tokens,
                self.output_tokens_camel,
                self.completion_tokens,
            ]),
            cache_read: first_positive(&[
                self.cache_read_input_tokens,
                self.cache_read_input_tokens_camel,
                self.prompt_cache_hit_tokens,
                self.cached_tokens,
            ]),
            cache_write: first_positive(&[
                self.cache_creation_input_tokens,
                self.cache_creation_input_tokens_camel,
                self.prompt_cache_write_tokens,
            ]),
            reasoning: first_present(&[
                self.completion_thinking_tokens,
                self.completion_thinking_tokens_camel,
            ]),
        };

        if tokens.total() == 0 {
            None
        } else {
            Some(tokens)
        }
    }
}

fn first_present(values: &[Option<i64>]) -> i64 {
    values.iter().copied().flatten().next().unwrap_or(0).max(0)
}

fn first_positive(values: &[Option<i64>]) -> i64 {
    values
        .iter()
        .copied()
        .flatten()
        .find(|count| *count > 0)
        .or_else(|| values.iter().copied().flatten().next())
        .unwrap_or(0)
        .max(0)
}

pub fn parse_codebuddy_file(path: &Path) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };

    let fallback_session_id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string();
    let fallback_timestamp = super::utils::file_modified_timestamp_ms(path);
    let mut keyed_indices: HashMap<String, usize> = HashMap::new();
    let mut messages: Vec<UnifiedMessage> = Vec::new();

    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut bytes = trimmed.as_bytes().to_vec();
        let item = match simd_json::from_slice::<CodeBuddyLine>(&mut bytes) {
            Ok(item) => item,
            Err(_) => continue,
        };

        if item.line_type.as_deref() != Some("message") || item.role.as_deref() != Some("assistant")
        {
            continue;
        }

        if item
            .status
            .as_deref()
            .is_some_and(|status| status != "completed")
        {
            continue;
        }

        let usage = item
            .message
            .as_ref()
            .and_then(|message| message.usage.as_ref())
            .or_else(|| {
                item.provider_data
                    .as_ref()
                    .and_then(|provider| provider.usage.as_ref())
            })
            .or_else(|| {
                item.provider_data
                    .as_ref()
                    .and_then(|provider| provider.raw_usage.as_ref())
            });
        let Some(tokens) = usage.and_then(CodeBuddyUsage::to_breakdown) else {
            continue;
        };

        let provider_data = item.provider_data.as_ref();
        let model_id = provider_data
            .and_then(|provider| provider.model.as_deref())
            .or_else(|| provider_data.and_then(|provider| provider.request_model_id.as_deref()))
            .or_else(|| {
                item.message
                    .as_ref()
                    .and_then(|message| message.model.as_deref())
            })
            .filter(|model| !model.trim().is_empty())
            .unwrap_or(DEFAULT_MODEL)
            .to_string();
        let provider_id = provider_identity::inferred_provider_from_model(&model_id)
            .unwrap_or(DEFAULT_PROVIDER)
            .to_string();
        let session_id = item
            .session_id
            .unwrap_or_else(|| fallback_session_id.clone());
        let timestamp = item.timestamp.unwrap_or(fallback_timestamp);

        let mut message = UnifiedMessage::new(
            "codebuddy",
            model_id,
            provider_id,
            session_id.clone(),
            timestamp,
            tokens,
            0.0,
        );

        if let Some(workspace_key) = item.cwd.as_deref().and_then(normalize_workspace_key) {
            let workspace_label = workspace_label_from_key(&workspace_key);
            message.set_workspace(Some(workspace_key), workspace_label);
        }

        let dedup_key = provider_data
            .and_then(|provider| provider.message_id.as_deref())
            .or_else(|| provider_data.and_then(|provider| provider.trace_id.as_deref()))
            .or(item.id.as_deref())
            .map(|key| format!("codebuddy:{session_id}:{key}"));
        message.dedup_key = dedup_key.clone();

        if let Some(key) = dedup_key {
            if let Some(existing_index) = keyed_indices.get(&key).copied() {
                if message.tokens.total() >= messages[existing_index].tokens.total() {
                    messages[existing_index] = message;
                }
                continue;
            }
            keyed_indices.insert(key, messages.len());
        }

        messages.push(message);
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codebuddy_file_reads_message_usage() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("projects").join("c-Users-alice-repo");
        std::fs::create_dir_all(&project_dir).unwrap();
        let path = project_dir.join("session-1.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"user-1","timestamp":1780000000000,"type":"message","role":"user","sessionId":"session-1","cwd":"/Users/alice/repo"}
{"id":"assistant-1","timestamp":1780000000100,"type":"message","role":"assistant","status":"completed","sessionId":"session-1","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"msg-1"},"message":{"usage":{"input_tokens":24486,"output_tokens":3,"total_tokens":24489,"cache_read_input_tokens":14720}}}"#,
        )
        .unwrap();

        let messages = parse_codebuddy_file(&path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "codebuddy");
        assert_eq!(message.model_id, "glm-5.2");
        assert_eq!(message.provider_id, "tencent");
        assert_eq!(message.session_id, "session-1");
        assert_eq!(message.tokens.input, 24486);
        assert_eq!(message.tokens.output, 3);
        assert_eq!(message.tokens.cache_read, 14720);
        assert_eq!(message.workspace_label.as_deref(), Some("repo"));
        assert_eq!(
            message.dedup_key.as_deref(),
            Some("codebuddy:session-1:msg-1")
        );
    }

    #[test]
    fn parse_codebuddy_file_falls_back_to_raw_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-2.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"assistant-1","timestamp":1780000000100,"type":"message","role":"assistant","status":"completed","sessionId":"session-2","providerData":{"requestModelId":"glm-5.2","rawUsage":{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":3,"prompt_cache_write_tokens":4,"completion_thinking_tokens":5}}}"#,
        )
        .unwrap();

        let messages = parse_codebuddy_file(&path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 2);
        assert_eq!(messages[0].tokens.cache_read, 3);
        assert_eq!(messages[0].tokens.cache_write, 4);
        assert_eq!(messages[0].tokens.reasoning, 5);
    }
}
