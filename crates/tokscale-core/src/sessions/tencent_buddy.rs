//! Shared parsers for Tencent CodeBuddy / WorkBuddy session formats.

use super::utils::for_each_json_line;
use super::{normalize_workspace_key, workspace_label_from_key, UnifiedMessage};
use crate::{provider_identity, TokenBreakdown};
use chrono::TimeZone;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

const DEFAULT_PROVIDER: &str = "tencent";

/// Shared base parser version for the Tencent buddy transcript format.
///
/// CodeBuddy and WorkBuddy both parse their detailed JSONL transcripts and
/// IDE extension logs through this module ([`parse_jsonl_file`] /
/// [`parse_extension_log_file`]), so a change here alters what byte-identical
/// sources parse to for both clients at once. Bump this base when that
/// happens; `message_cache::parser_version()` derives each member's version
/// from it (base plus a per-client offset that preserves independent history)
/// so no member can be left serving stale cache entries.
///
/// Version 2 anchors extension-log settlements on the preceding
/// `Model prepared:` line and reports the elapsed generation window as
/// `duration_ms`.
///
/// Version 3 spreads the wall-clock duration recorded by each `turn-metrics`
/// line over the usage rows of that turn, so JSONL transcripts report
/// throughput as well.
///
/// Logs are append-only, so cached rows from an older version have to be
/// re-parsed to gain either field.
pub(crate) const TENCENT_BUDDY_PARSER_BASE_VERSION: u32 = 3;

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
    /// Wall-clock length of a completed user turn, recorded on the
    /// `turn-metrics` line CodeBuddy writes after the final reply.
    #[serde(rename = "durationMs")]
    duration_ms: Option<i64>,
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
    total_tokens: Option<i64>,
    #[serde(rename = "totalTokens")]
    total_tokens_camel: Option<i64>,
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
        let cache_read = first_positive(&[
            self.cache_read_input_tokens,
            self.cache_read_input_tokens_camel,
            self.cache_tokens,
            self.prompt_cache_hit_tokens,
            self.cached_tokens,
        ]);
        let output = first_present(&[
            self.output_tokens,
            self.output_tokens_camel,
            self.completion_tokens,
        ]);
        let cache_write = first_positive(&[
            self.cache_creation_input_tokens,
            self.cache_creation_input_tokens_camel,
            self.cached_write_tokens,
            self.prompt_cache_write_tokens,
        ]);
        let reasoning = first_present(&[
            self.completion_thinking_tokens,
            self.completion_thinking_tokens_camel,
            self.reasoning_tokens,
        ]);
        let tokens = TokenBreakdown {
            input: self.input_exclusive(cache_read, cache_write, reasoning, output),
            output,
            cache_read,
            cache_write,
            cache_write_1h: 0,
            reasoning,
        };

        (tokens.total() > 0).then_some(tokens)
    }

    /// `cachedMissTokens` / `cacheMissTokens` (extension-log style) already
    /// exclude cached input. Other input fields are ambiguous unless a
    /// reported total proves they include the cache buckets.
    fn input_exclusive(
        &self,
        cache_read: i64,
        cache_write: i64,
        reasoning: i64,
        output: i64,
    ) -> i64 {
        if let Some(miss_input) = self
            .cached_miss_tokens
            .or(self.cache_miss_tokens)
            .map(|tokens| tokens.max(0))
        {
            return miss_input;
        }
        let input = first_present(&[
            self.input_tokens,
            self.input_tokens_camel,
            self.prompt_tokens,
        ]);
        let Some(total) = first_option(&[self.total_tokens, self.total_tokens_camel]) else {
            return input;
        };

        let inclusive_total = input.saturating_add(output);
        let exclusive_total = inclusive_total
            .saturating_add(cache_read)
            .saturating_add(cache_write)
            .saturating_add(reasoning);
        if cache_read > 0 && total.max(0) == inclusive_total && inclusive_total != exclusive_total {
            input.saturating_sub(cache_read)
        } else {
            input
        }
    }
}

fn first_option(values: &[Option<i64>]) -> Option<i64> {
    values.iter().copied().flatten().next()
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
    let fallback_session_id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string();
    let fallback_timestamp = super::utils::file_modified_timestamp_ms(path);
    let mut keyed_indices: HashMap<String, usize> = HashMap::new();
    let mut messages: Vec<UnifiedMessage> = Vec::new();
    // Indices of the usage rows produced since the current user turn started.
    // `turn-metrics` closes the turn; its duration is spread over these rows.
    let mut turn_indices: Vec<usize> = Vec::new();

    for_each_json_line(path, &mut |_index, trimmed| {
        let mut bytes = trimmed.as_bytes().to_vec();
        let item = match simd_json::from_slice::<BuddyLine>(&mut bytes) {
            Ok(item) => item,
            Err(_) => return,
        };

        if item.line_type.as_deref() == Some("turn-metrics") {
            spread_turn_duration(&mut messages, &turn_indices, item.duration_ms);
            turn_indices.clear();
            return;
        }

        if item.line_type.as_deref() == Some("message") && item.role.as_deref() == Some("user") {
            turn_indices.clear();
            return;
        }

        let is_assistant_message = item.line_type.as_deref() == Some("message")
            && item.role.as_deref() == Some("assistant");
        let is_function_call = item.line_type.as_deref() == Some("function_call");
        if !is_assistant_message && !is_function_call {
            return;
        }

        if item
            .status
            .as_deref()
            .is_some_and(|status| status != "completed")
        {
            return;
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
            return;
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
                return;
            }
            keyed_indices.insert(key, messages.len());
        }

        turn_indices.push(messages.len());
        messages.push(message);
    });

    messages
}

/// Spread a turn's measured wall-clock duration over the usage rows that turn
/// produced, in proportion to their output tokens.
///
/// CodeBuddy writes one `turn-metrics` line per finished user turn, carrying the
/// wall-clock length of that turn and no token counts; the tokens live on the
/// per-step rows. Splitting the duration by output keeps the ratio a consumer
/// computes (`sum(output) / sum(duration)`) equal to the turn's real rate instead
/// of attributing the whole turn to a single step.
///
/// The message timestamps are deliberately left alone. `SessionSpan` projects a
/// message as `[timestamp, timestamp + duration_ms]`, and moving the stamp to the
/// turn's start (the dsh approach in junhoyeo/tokscale#1427, where an exact
/// per-request anchor exists) would drag every step of the turn backwards by up
/// to the whole turn and move the day buckets it lands in.
fn spread_turn_duration(
    messages: &mut [UnifiedMessage],
    turn_indices: &[usize],
    duration_ms: Option<i64>,
) {
    let Some(duration_ms) = duration_ms.filter(|duration| *duration > 0) else {
        return;
    };
    // Sum and divide in i128: `duration_ms * output` can exceed i64 for a
    // malformed transcript, and a panic here would take the whole scan down.
    let total_output: i128 = turn_indices
        .iter()
        .map(|index| messages[*index].tokens.output.max(0) as i128)
        .sum();
    if total_output <= 0 {
        return;
    }
    // The integer remainder goes to the last row that actually produced output,
    // so a trailing zero-output row cannot collect time it did not spend.
    let Some(last_positive) = turn_indices
        .iter()
        .rposition(|index| messages[*index].tokens.output > 0)
    else {
        return;
    };

    let mut assigned = 0i64;
    for (position, index) in turn_indices.iter().enumerate() {
        let message = &mut messages[*index];
        let share = if position == last_positive {
            duration_ms - assigned
        } else {
            (duration_ms as i128 * message.tokens.output.max(0) as i128 / total_output) as i64
        };
        assigned += share;
        if share > 0 {
            message.duration_ms = Some(share);
        }
    }
}

/// A `Model prepared:` record, keyed by agent id, that anchors the generation
/// clock for the execution the agent reports next.
struct PreparedAgent {
    model_id: String,
    prepared_at_ms: Option<i64>,
}

pub(crate) fn parse_extension_log_file(
    client: &'static str,
    default_model: &'static str,
    path: &Path,
) -> Vec<UnifiedMessage> {
    let fallback_timestamp = super::utils::file_modified_timestamp_ms(path);
    let fallback_session_id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("extension-log")
        .to_string();
    let mut prepared_by_agent: HashMap<String, PreparedAgent> = HashMap::new();
    let mut messages = Vec::new();

    for_each_json_line(path, &mut |_index, line| {
        if line.contains("[CraftInvokableAgent]") && line.contains("Model prepared:") {
            if let Some((agent_id, model_id)) = parse_model_prepared_line(line) {
                prepared_by_agent.insert(
                    agent_id,
                    PreparedAgent {
                        model_id,
                        prepared_at_ms: parse_log_timestamp_ms(line),
                    },
                );
            }
            return;
        }

        if !line.contains("[AgentReporter]")
            || !line.contains("Agent execution successful with usage:")
        {
            return;
        }

        let Some(agent_id) = bracket_value_after(line, "[AgentReporter]") else {
            return;
        };
        // Consume the anchor the moment the settlement is identified, before
        // its usage is parsed or validated: one `Model prepared:` record
        // describes one execution, and every check below can bail out on a
        // malformed or empty usage. Leaving the anchor in place would hand the
        // next settlement of the same agent a stale clock and bill the whole
        // gap between two executions as one generation.
        let prepared = prepared_by_agent.remove(&agent_id);
        let Some(usage_json) = line.split("Agent execution successful with usage:").nth(1) else {
            return;
        };
        let usage_json = usage_json.trim();
        let Some(json_end) = usage_json.rfind('}') else {
            return;
        };
        let mut bytes = usage_json.as_bytes()[..=json_end].to_vec();
        let usage = match simd_json::from_slice::<BuddyUsage>(&mut bytes) {
            Ok(usage) => usage,
            Err(_) => return,
        };
        let Some(tokens) = usage.to_breakdown() else {
            return;
        };

        // Keep the parsed settlement separately from the timestamp fallback:
        // the file mtime is only good enough to date the message. Using it as
        // the end of the measured window would invent a duration spanning the
        // whole lifetime of the file.
        let parsed_settled_at = parse_log_timestamp_ms(line);
        let settled_at = parsed_settled_at.unwrap_or(fallback_timestamp);
        let model_id = prepared
            .as_ref()
            .map(|prepared| prepared.model_id.clone())
            .unwrap_or_else(|| default_model.to_string());
        // Date the message at the anchor, not at the settlement. `SessionSpan`
        // projects a message as `[timestamp, timestamp + duration_ms]`, so
        // stamping the settlement would push the interval forward by one
        // generation and move the day buckets it lands in.
        let (timestamp, duration_ms) = match (
            prepared.and_then(|prepared| prepared.prepared_at_ms),
            parsed_settled_at,
        ) {
            (Some(prepared_at), Some(settled_at)) if settled_at > prepared_at => {
                (prepared_at, Some(settled_at - prepared_at))
            }
            _ => (settled_at, None),
        };
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
        message.duration_ms = duration_ms;
        // Key on the SECOND, not the millisecond. The same [AgentReporter] line
        // is written to multiple sinks (the extension's own log AND the host's
        // output-channel log), each prefixed by its own logger a few ms apart, so
        // a millisecond key double-counts every mirrored execution. Distinct
        // executions of the same agent with identical usage are seconds-to-minutes
        // apart, so second granularity still keeps them separate (the timestamp
        // was added to this key precisely to protect those).
        //
        // This keys on the settlement time rather than on `timestamp`: the
        // anchor can differ by a few ms between mirrored sinks, and only the
        // settlement is guaranteed to agree across them.
        let dedup_second = settled_at.div_euclid(1000);
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
            message.session_id = fallback_session_id.clone().into();
        }

        messages.push(message);
    });

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
        assert_eq!(message.tokens.input, 9766);
        assert_eq!(message.tokens.output, 3);
        assert_eq!(message.tokens.cache_read, 14720);
        assert_eq!(message.tokens.total(), 24489);
        assert_eq!(message.workspace_label.as_deref(), Some("repo"));
        assert_eq!(
            message.dedup_key.as_deref(),
            Some("codebuddy:session-1:msg-1")
        );
    }

    #[test]
    fn parse_jsonl_file_keeps_ambiguous_raw_usage_input_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-2.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"call-1","timestamp":1780000000100,"type":"function_call","sessionId":"session-2","providerData":{"requestModelId":"glm-5.2","messageId":"msg-2","rawUsage":{"prompt_tokens":3,"completion_tokens":2,"prompt_cache_hit_tokens":4,"prompt_cache_write_tokens":4,"completion_thinking_tokens":5}}}"#,
        )
        .unwrap();

        let messages = parse_jsonl_file("workbuddy", "workbuddy", &path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client, "workbuddy");
        assert_eq!(messages[0].tokens.input, 3);
        assert_eq!(messages[0].tokens.output, 2);
        assert_eq!(messages[0].tokens.cache_read, 4);
        assert_eq!(messages[0].tokens.cache_write, 4);
        assert_eq!(messages[0].tokens.reasoning, 5);
        assert_eq!(messages[0].tokens.total(), 18);
    }

    #[test]
    fn parse_jsonl_file_does_not_double_count_inclusive_cache_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-cache.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"assistant-1","timestamp":1780000000100,"type":"message","role":"assistant","status":"completed","sessionId":"session-cache","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"msg-1"},"message":{"usage":{"input_tokens":113415,"output_tokens":990,"total_tokens":114405,"cache_read_input_tokens":112224}}}"#,
        )
        .unwrap();

        let messages = parse_jsonl_file("workbuddy", "workbuddy", &path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.tokens.input, 1191);
        assert_eq!(message.tokens.output, 990);
        assert_eq!(message.tokens.cache_read, 112224);
        // Provider-billed total (114405), not input + cache_read + output (226629).
        assert_eq!(message.tokens.total(), 114405);
    }

    #[test]
    fn parse_jsonl_file_keeps_ambiguous_codebuddy_input_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-ambiguous.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"assistant-1","timestamp":1780000000100,"type":"message","role":"assistant","status":"completed","sessionId":"session-ambiguous","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"msg-1"},"message":{"usage":{"inputTokens":7,"outputTokens":2,"cacheTokens":10}}}"#,
        )
        .unwrap();

        let messages = parse_jsonl_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.tokens.input, 7);
        assert_eq!(message.tokens.output, 2);
        assert_eq!(message.tokens.cache_read, 10);
        assert_eq!(message.tokens.total(), 19);
    }

    #[test]
    fn parse_jsonl_file_keeps_zero_cached_miss_tokens_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-zero-cache-miss.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"assistant-1","timestamp":1780000000100,"type":"message","role":"assistant","status":"completed","sessionId":"session-zero-cache-miss","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"msg-1"},"message":{"usage":{"inputTokens":100,"outputTokens":5,"cacheTokens":100,"cachedMissTokens":0}}}"#,
        )
        .unwrap();

        let messages = parse_jsonl_file("workbuddy", "workbuddy", &path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.tokens.input, 0);
        assert_eq!(message.tokens.output, 5);
        assert_eq!(message.tokens.cache_read, 100);
        assert_eq!(message.tokens.total(), 105);
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

    #[test]
    fn parse_extension_log_file_measures_generation_window_from_model_prepared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proj__session.log");
        std::fs::write(
            &path,
            r#"[2026/7/1 16:56:01.100] [info] [CraftInvokableAgent] [agent-1] Model prepared: Kimi-K2.7-Code (kimi-k2.7)
[2026/7/1 16:56:02.200] [info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":140732,"outputTokens":635,"totalTokens":141367}"#,
        )
        .unwrap();

        let messages = parse_extension_log_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.duration_ms, Some(1_100));
        // The message is dated at the anchor and the span closes on the
        // settlement, so the projected interval still ends where the log
        // reports the execution finishing.
        let prepared_at = parse_log_timestamp_ms(
            "[2026/7/1 16:56:01.100] [info] [CraftInvokableAgent] [agent-1] Model prepared: Kimi-K2.7-Code (kimi-k2.7)",
        )
        .unwrap();
        let settled_at = parse_log_timestamp_ms(
            "[2026/7/1 16:56:02.200] [info] [AgentReporter] [agent-1] Agent execution successful with usage: {}",
        )
        .unwrap();
        assert_eq!(message.timestamp, prepared_at);
        assert_eq!(message.timestamp + message.duration_ms.unwrap(), settled_at);
    }

    #[test]
    fn parse_extension_log_file_leaves_settlement_without_anchor_untimed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proj__rotated.log");
        std::fs::write(
            &path,
            r#"[2026/7/1 16:56:02.200] [info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":10,"outputTokens":2,"totalTokens":12}"#,
        )
        .unwrap();

        let messages = parse_extension_log_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].duration_ms, None);
    }

    #[test]
    fn parse_extension_log_file_does_not_reuse_a_consumed_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.log");
        std::fs::write(
            &path,
            r#"[2026/7/1 16:56:01.100] [info] [CraftInvokableAgent] [agent-1] Model prepared: GLM-5.2 (glm-5.2)
[2026/7/1 16:56:02.200] [info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":10,"outputTokens":2,"totalTokens":12}
[2026/7/1 16:57:02.200] [info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":11,"outputTokens":2,"totalTokens":13}"#,
        )
        .unwrap();

        let messages = parse_extension_log_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].duration_ms, Some(1_100));
        // Without a fresh `Model prepared:` line the second settlement stays
        // untimed instead of billing the minute between the two executions.
        assert_eq!(messages[1].duration_ms, None);
    }

    #[test]
    fn parse_extension_log_file_consumes_the_anchor_on_a_malformed_settlement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.log");
        std::fs::write(
            &path,
            r#"[2026/7/1 16:56:01.100] [info] [CraftInvokableAgent] [agent-1] Model prepared: GLM-5.2 (glm-5.2)
[2026/7/1 16:56:02.200] [info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":
[2026/7/1 16:57:02.200] [info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":10,"outputTokens":2,"totalTokens":12}"#,
        )
        .unwrap();

        let messages = parse_extension_log_file("codebuddy", "codebuddy", &path);

        // An unusable settlement still ends the execution it belongs to, so it
        // takes the anchor with it. The later row must not inherit that start
        // time and bill the minute between the two lines as one generation.
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].duration_ms, None);
    }

    #[test]
    fn parse_jsonl_file_spreads_turn_duration_over_its_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turn.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"user-1","timestamp":1780000000000,"type":"message","role":"user","sessionId":"s1","cwd":"/Users/alice/repo"}
{"id":"call-1","timestamp":1780000001000,"type":"function_call","sessionId":"s1","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"m-1","usage":{"prompt_tokens":10,"completion_tokens":100}}}
{"id":"call-2","timestamp":1780000002000,"type":"function_call","sessionId":"s1","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"m-2","usage":{"prompt_tokens":10,"completion_tokens":300}}}
{"id":"turn-1","timestamp":1780000003000,"type":"turn-metrics","sessionId":"s1","durationMs":4000}"#,
        )
        .unwrap();

        let messages = parse_jsonl_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 2);
        // The turn's 4 s is split 1:3 by output tokens, and the ratio a consumer
        // computes from these rows is the turn's own rate.
        assert_eq!(messages[0].duration_ms, Some(1_000));
        assert_eq!(messages[1].duration_ms, Some(3_000));
        // Timestamps stay where the steps were written; only the duration is
        // attributed, so day buckets do not move.
        assert_eq!(messages[0].timestamp, 1_780_000_001_000);
        assert_eq!(messages[1].timestamp, 1_780_000_002_000);
    }

    #[test]
    fn parse_jsonl_file_resets_the_turn_at_the_next_user_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("two-turns.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"user-1","timestamp":1780000000000,"type":"message","role":"user","sessionId":"s1","cwd":"/Users/alice/repo"}
{"id":"call-1","timestamp":1780000001000,"type":"function_call","sessionId":"s1","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"m-1","usage":{"prompt_tokens":10,"completion_tokens":100}}}
{"id":"turn-1","timestamp":1780000002000,"type":"turn-metrics","sessionId":"s1","durationMs":1000}
{"id":"user-2","timestamp":1780000003000,"type":"message","role":"user","sessionId":"s1","cwd":"/Users/alice/repo"}
{"id":"call-2","timestamp":1780000004000,"type":"function_call","sessionId":"s1","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"m-2","usage":{"prompt_tokens":10,"completion_tokens":100}}}
{"id":"turn-2","timestamp":1780000005000,"type":"turn-metrics","sessionId":"s1","durationMs":3000}"#,
        )
        .unwrap();

        let messages = parse_jsonl_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].duration_ms, Some(1_000));
        assert_eq!(messages[1].duration_ms, Some(3_000));
    }

    #[test]
    fn parse_jsonl_file_ignores_zero_length_turn_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zero.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"user-1","timestamp":1780000000000,"type":"message","role":"user","sessionId":"s1","cwd":"/Users/alice/repo"}
{"id":"call-1","timestamp":1780000001000,"type":"function_call","sessionId":"s1","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"m-1","usage":{"prompt_tokens":10,"completion_tokens":100}}}
{"id":"turn-1","timestamp":1780000002000,"type":"turn-metrics","sessionId":"s1","durationMs":0}"#,
        )
        .unwrap();

        let messages = parse_jsonl_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].duration_ms, None);
    }

    #[test]
    fn parse_jsonl_file_gives_the_remainder_to_the_last_timed_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remainder.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"user-1","timestamp":1780000000000,"type":"message","role":"user","sessionId":"s1","cwd":"/Users/alice/repo"}
{"id":"call-1","timestamp":1780000001000,"type":"function_call","sessionId":"s1","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"m-1","usage":{"prompt_tokens":10,"completion_tokens":100}}}
{"id":"call-2","timestamp":1780000002000,"type":"function_call","sessionId":"s1","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"m-2","usage":{"prompt_tokens":10,"completion_tokens":100}}}
{"id":"call-3","timestamp":1780000003000,"type":"function_call","sessionId":"s1","cwd":"/Users/alice/repo","providerData":{"model":"glm-5.2","messageId":"m-3","usage":{"prompt_tokens":10,"completion_tokens":0}}}
{"id":"turn-1","timestamp":1780000004000,"type":"turn-metrics","sessionId":"s1","durationMs":1000}"#,
        )
        .unwrap();

        let messages = parse_jsonl_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].duration_ms, Some(500));
        // The 1000 ms cannot divide evenly across two rows, so the remainder
        // lands on the last row that produced output rather than on the
        // trailing row that generated nothing.
        assert_eq!(messages[1].duration_ms, Some(500));
        assert_eq!(messages[2].duration_ms, None);
    }

    #[test]
    fn parse_extension_log_file_does_not_time_a_settlement_without_its_own_clock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proj__session.log");
        std::fs::write(
            &path,
            r#"[2026/7/1 16:56:01.100] [info] [CraftInvokableAgent] [agent-1] Model prepared: GLM-5.2 (glm-5.2)
[info] [AgentReporter] [agent-1] Agent execution successful with usage: {"inputTokens":10,"outputTokens":2,"totalTokens":12}"#,
        )
        .unwrap();

        let messages = parse_extension_log_file("codebuddy", "codebuddy", &path);

        assert_eq!(messages.len(), 1);
        // The settlement line has no parsable clock. Falling back to the file
        // mtime would have invented a duration spanning the whole file's life.
        assert_eq!(messages[0].duration_ms, None);
    }
}
