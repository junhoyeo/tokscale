//! Shared parsers for Tencent CodeBuddy / WorkBuddy session formats.

use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use chrono::TimeZone;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;

const DEFAULT_PROVIDER: &str = "tencent";

#[derive(Debug, Deserialize)]
struct BuddyLine {
    id: Option<String>,
    timestamp: Option<i64>,
    #[serde(rename = "type")]
    line_type: Option<String>,
    role: Option<String>,
    status: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    message: Option<BuddyMessage>,
    #[serde(rename = "providerData")]
    provider_data: Option<BuddyProviderData>,
}

#[derive(Debug, Deserialize)]
struct BuddyMessage {
    model: Option<String>,
    usage: Option<BuddyUsage>,
}

#[derive(Debug, Deserialize)]
struct BuddyProviderData {
    model: Option<String>,
    #[serde(rename = "requestModelId")]
    request_model_id: Option<String>,
    #[serde(rename = "messageId")]
    message_id: Option<String>,
    #[serde(rename = "traceId")]
    trace_id: Option<String>,
    usage: Option<BuddyUsage>,
    #[serde(rename = "rawUsage")]
    raw_usage: Option<BuddyUsage>,
}

#[derive(Debug, Clone, Deserialize)]
struct BuddyUsage {
    #[serde(rename = "cachedMissTokens")]
    cached_miss_tokens: Option<i64>,
    #[serde(rename = "cacheMissTokens")]
    cache_miss_tokens: Option<i64>,
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
    #[serde(rename = "cacheTokens")]
    cache_tokens: Option<i64>,
    prompt_cache_hit_tokens: Option<i64>,
    cached_tokens: Option<i64>,
    #[serde(rename = "cache_creation_input_tokens")]
    cache_creation_input_tokens: Option<i64>,
    #[serde(rename = "cacheCreationInputTokens")]
    cache_creation_input_tokens_camel: Option<i64>,
    #[serde(rename = "cachedWriteTokens")]
    cached_write_tokens: Option<i64>,
    prompt_cache_write_tokens: Option<i64>,
    #[serde(rename = "completion_thinking_tokens")]
    completion_thinking_tokens: Option<i64>,
    #[serde(rename = "completionThinkingTokens")]
    completion_thinking_tokens_camel: Option<i64>,
    #[serde(rename = "reasoningTokens")]
    reasoning_tokens: Option<i64>,
}

impl BuddyUsage {
    fn to_breakdown(&self) -> Option<TokenBreakdown> {
        let tokens = TokenBreakdown {
            input: first_present(&[
                self.cached_miss_tokens,
                self.cache_miss_tokens,
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
                self.cache_tokens,
                self.prompt_cache_hit_tokens,
                self.cached_tokens,
            ]),
            cache_write: first_positive(&[
                self.cache_creation_input_tokens,
                self.cache_creation_input_tokens_camel,
                self.cached_write_tokens,
                self.prompt_cache_write_tokens,
            ]),
            reasoning: first_present(&[
                self.completion_thinking_tokens,
                self.completion_thinking_tokens_camel,
                self.reasoning_tokens,
            ]),
        };

        (tokens.total() > 0).then_some(tokens)
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

pub(crate) fn parse_jsonl_file(
    client: &'static str,
    default_model: &'static str,
    path: &Path,
) -> Vec<UnifiedMessage> {
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
        let item = match simd_json::from_slice::<BuddyLine>(&mut bytes) {
            Ok(item) => item,
            Err(_) => continue,
        };

        let is_assistant_message = item.line_type.as_deref() == Some("message")
            && item.role.as_deref() == Some("assistant");
        let is_function_call = item.line_type.as_deref() == Some("function_call");
        if !is_assistant_message && !is_function_call {
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
        let Some(tokens) = usage.and_then(BuddyUsage::to_breakdown) else {
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
            .unwrap_or(default_model)
            .to_string();
        let provider_id = provider_identity::inferred_provider_from_model(&model_id)
            .unwrap_or(DEFAULT_PROVIDER)
            .to_string();
        let session_id = item
            .session_id
            .unwrap_or_else(|| fallback_session_id.clone());
        let timestamp = item.timestamp.unwrap_or(fallback_timestamp);

        let mut message = UnifiedMessage::new(
            client,
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
            .map(|key| format!("{client}:{session_id}:{key}"));
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

pub(crate) fn parse_extension_log_file(
    client: &'static str,
    default_model: &'static str,
    path: &Path,
) -> Vec<UnifiedMessage> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };

    let fallback_timestamp = super::utils::file_modified_timestamp_ms(path);
    let fallback_session_id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("extension-log")
        .to_string();
    let mut models_by_agent: HashMap<String, String> = HashMap::new();
    let mut messages = Vec::new();

    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            continue;
        };

        if line.contains("[CraftInvokableAgent]") && line.contains("Model prepared:") {
            if let Some((agent_id, model_id)) = parse_model_prepared_line(&line) {
                models_by_agent.insert(agent_id, model_id);
            }
            continue;
        }

        if !line.contains("[AgentReporter]")
            || !line.contains("Agent execution successful with usage:")
        {
            continue;
        }

        let Some(agent_id) = bracket_value_after(&line, "[AgentReporter]") else {
            continue;
        };
        let Some(usage_json) = line.split("Agent execution successful with usage:").nth(1) else {
            continue;
        };
        let usage_json = usage_json.trim();
        let Some(json_end) = usage_json.rfind('}') else {
            continue;
        };
        let mut bytes = usage_json.as_bytes()[..=json_end].to_vec();
        let usage = match simd_json::from_slice::<BuddyUsage>(&mut bytes) {
            Ok(usage) => usage,
            Err(_) => continue,
        };
        let Some(tokens) = usage.to_breakdown() else {
            continue;
        };

        let timestamp = parse_log_timestamp_ms(&line).unwrap_or(fallback_timestamp);
        let model_id = models_by_agent
            .get(&agent_id)
            .cloned()
            .unwrap_or_else(|| default_model.to_string());
        let provider_id = provider_identity::inferred_provider_from_model(&model_id)
            .unwrap_or(DEFAULT_PROVIDER)
            .to_string();
        let mut message = UnifiedMessage::new(
            client,
            model_id,
            provider_id,
            agent_id.clone(),
            timestamp,
            tokens,
            0.0,
        );
        // Key on the SECOND, not the millisecond. The same [AgentReporter] line
        // is written to multiple sinks (the extension's own log AND the host's
        // output-channel log), each prefixed by its own logger a few ms apart, so
        // a millisecond key double-counts every mirrored execution. Distinct
        // executions of the same agent with identical usage are seconds-to-minutes
        // apart, so second granularity still keeps them separate (the timestamp
        // was added to this key precisely to protect those).
        let dedup_second = timestamp.div_euclid(1000);
        message.dedup_key = Some(format!(
            "{client}:extension-log:{agent_id}:{dedup_second}:{}:{}:{}:{}:{}",
            message.tokens.input,
            message.tokens.output,
            message.tokens.cache_read,
            message.tokens.cache_write,
            message.tokens.reasoning
        ));

        if let Some(workspace_key) = workspace_from_log_path(path) {
            let workspace_label = workspace_label_from_key(&workspace_key);
            message.set_workspace(Some(workspace_key), workspace_label);
        }
        if message.session_id.is_empty() {
            message.session_id = fallback_session_id.clone();
        }

        messages.push(message);
    }

    messages
}

pub(crate) fn is_jsonl_source(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
}

pub(crate) fn is_extension_log_source(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("log"))
}

fn parse_model_prepared_line(line: &str) -> Option<(String, String)> {
    let agent_id = bracket_value_after(line, "[CraftInvokableAgent]")?;
    let marker = "Model prepared:";
    let after_marker = line.split(marker).nth(1)?.trim();
    let model_id = after_marker
        .rsplit_once('(')
        .and_then(|(_, tail)| tail.split_once(')').map(|(model, _)| model.trim()))
        .filter(|model| !model.is_empty())
        .unwrap_or(after_marker)
        .to_string();
    Some((agent_id, model_id))
}

fn bracket_value_after(line: &str, marker: &str) -> Option<String> {
    let after_marker = line.split(marker).nth(1)?;
    let start = after_marker.find('[')?;
    let after_open = &after_marker[start + 1..];
    let end = after_open.find(']')?;
    let value = after_open[..end].trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn parse_log_timestamp_ms(line: &str) -> Option<i64> {
    let raw = if let Some(rest) = line.strip_prefix('[') {
        rest.split_once(']')?.0.trim()
    } else {
        line.split_once(" [")
            .map(|(timestamp, _)| timestamp.trim())
            .unwrap_or(line.trim())
    };

    let (date, time) = raw.split_once(' ')?;
    let separator = if date.contains('/') { '/' } else { '-' };
    let parts = date
        .split(separator)
        .filter_map(|part| part.parse::<u32>().ok())
        .collect::<Vec<_>>();
    if parts.len() != 3 {
        return super::utils::parse_timestamp_str(raw);
    }

    let normalized = format!("{:04}-{:02}-{:02} {}", parts[0], parts[1], parts[2], time);
    parse_local_naive_timestamp_ms(&normalized)
        .or_else(|| super::utils::parse_timestamp_str(&normalized))
}

fn parse_local_naive_timestamp_ms(value: &str) -> Option<i64> {
    for format in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(value, format) {
            return match chrono::Local.from_local_datetime(&naive) {
                chrono::LocalResult::Single(dt) => Some(dt.timestamp_millis()),
                chrono::LocalResult::Ambiguous(earlier, _) => Some(earlier.timestamp_millis()),
                chrono::LocalResult::None => None,
            };
        }
    }

    None
}

fn workspace_from_log_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let workspace = stem.split("__").next().unwrap_or(stem);
    normalize_workspace_key(workspace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_jsonl_file_reads_assistant_usage() {
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

        let messages = parse_jsonl_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "codebuddy");
        assert_eq!(message.model_id, "glm-5.2");
        // `inferred_provider_from_model` recognizes "glm" and infers "zai"
        // (Zhipu AI), taking precedence over the DEFAULT_PROVIDER ("tencent")
        // fallback — consistent with how every other model family (claude,
        // gpt, etc.) is attributed to its real provider rather than the
        // client name. DEFAULT_PROVIDER only applies when inference can't
        // identify the model at all.
        assert_eq!(message.provider_id, "zai");
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
    fn parse_jsonl_file_reads_function_call_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-2.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"call-1","timestamp":1780000000100,"type":"function_call","sessionId":"session-2","providerData":{"requestModelId":"glm-5.2","messageId":"msg-2","rawUsage":{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":3,"prompt_cache_write_tokens":4,"completion_thinking_tokens":5}}}"#,
        )
        .unwrap();

        let messages = parse_jsonl_file("workbuddy", "workbuddy", &path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "workbuddy");
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.output, 2);
        assert_eq!(messages[0].tokens.cache_read, 3);
        assert_eq!(messages[0].tokens.cache_write, 4);
        assert_eq!(messages[0].tokens.reasoning, 5);
    }

    #[test]
    fn parse_extension_log_file_splits_cached_miss_and_cache_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("moza-configurator__session.log");
        std::fs::write(
            &path,
            r#"[2026/7/1 16:56:01.100] [info] [CraftInvokableAgent] [agent-1] Model prepared: Kimi-K2.7-Code (kimi-k2.7)
[2026/7/1 16:56:02.200] [info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":140732,"outputTokens":635,"totalTokens":141367,"cacheTokens":76032,"cachedWriteTokens":0,"cachedMissTokens":64700}"#,
        )
        .unwrap();

        let messages = parse_extension_log_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "kimi-k2.7");
        assert_eq!(messages[0].tokens.input, 64700);
        assert_eq!(messages[0].tokens.output, 635);
        assert_eq!(messages[0].tokens.cache_read, 76032);
        assert_eq!(messages[0].tokens.total(), 141367);
        assert_eq!(
            messages[0].workspace_label.as_deref(),
            Some("moza-configurator")
        );
    }

    #[test]
    fn parse_extension_log_file_keeps_repeated_agent_usage_at_different_times() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.log");
        std::fs::write(
            &path,
            r#"[2026/7/1 16:56:01.100] [info] [CraftInvokableAgent] [agent-1] Model prepared: GLM-5.2 (glm-5.2)
[2026/7/1 16:56:02.200] [info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":10,"outputTokens":2,"totalTokens":12}
[2026/7/1 16:57:02.200] [info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":10,"outputTokens":2,"totalTokens":12}"#,
        )
        .unwrap();

        let messages = parse_extension_log_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 2);
        assert_ne!(messages[0].dedup_key, messages[1].dedup_key);
    }

    #[test]
    fn parse_extension_log_file_mirrored_sinks_share_dedup_key_despite_ms_skew() {
        // The same agent execution is logged to the extension's own sink and to
        // the host's output-channel sink; each writer stamps its own prefix a few
        // ms apart (and in a different format). Both copies must produce the same
        // dedup key so cross-file dedup collapses them.
        let dir = tempfile::tempdir().unwrap();

        let extension_sink = dir.path().join("proj__session.log");
        std::fs::write(
            &extension_sink,
            r#"[2026/7/1 16:56:02.200] [info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":140732,"outputTokens":635,"totalTokens":141367}"#,
        )
        .unwrap();

        let host_sink = dir.path().join("proj__host.log");
        std::fs::write(
            &host_sink,
            r#"2026-07-01 16:56:02.201 [info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":140732,"outputTokens":635,"totalTokens":141367}"#,
        )
        .unwrap();

        let from_extension = parse_extension_log_file("codebuddy", "codebuddy", &extension_sink);
        let from_host = parse_extension_log_file("codebuddy", "codebuddy", &host_sink);

        assert_eq!(from_extension.len(), 1);
        assert_eq!(from_host.len(), 1);
        assert!(from_extension[0].dedup_key.is_some());
        assert_eq!(from_extension[0].dedup_key, from_host[0].dedup_key);
    }
}
