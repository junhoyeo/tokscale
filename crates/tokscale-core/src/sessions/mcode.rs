//! MiniMax Code headless stream parser.
//!
//! `mcode exec --output-format stream-json` emits projected Runtime events as
//! JSONL followed by one `exec.result`. Assistant `message` events carry
//! authoritative per-call usage, while the final result carries the actual
//! provider/model selected for the turn. Usage is buffered until that result
//! arrives so Tokscale never guesses a model from display text or local
//! configuration.

use super::utils::{file_modified_timestamp_ms, lossy_lines};
use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde_json::Value;
use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;

#[derive(Debug)]
struct PendingUsage {
    timestamp: i64,
    tokens: TokenBreakdown,
}

/// Parse one Tokscale-captured MiniMax Code JSONL stream.
///
/// A partial stream without a model-bearing `exec.result` is deliberately
/// ignored. The token counts are still present in that case, but attributing
/// them to a guessed model could attach the wrong price.
pub fn parse_mcode_file(path: &Path) -> Vec<UnifiedMessage> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let fallback_timestamp = file_modified_timestamp_ms(path);
    let mut pending_by_turn: HashMap<String, Vec<PendingUsage>> = HashMap::new();
    let mut messages = Vec::new();

    // `lines().map_while(Result::ok)` ends the iteration at the first line that
    // is not valid UTF-8 rather than skipping it (#1031). That is worse here
    // than in most parsers: usage is buffered in `pending_by_turn` and only
    // flushed when the trailing `exec.result` is seen, so one stray byte
    // anywhere before it drops the whole capture to zero messages instead of
    // truncating a tail. `lossy_lines` keeps a bad byte local to its own line
    // and strips a leading BOM.
    for line in lossy_lines(BufReader::new(file)) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        match value.get("type").and_then(Value::as_str) {
            Some("message") => {
                let Some(message) = value.get("message") else {
                    continue;
                };
                if message.get("role").and_then(Value::as_str) != Some("assistant") {
                    continue;
                }
                let Some(turn_id) = message
                    .get("turnId")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|turn| !turn.is_empty())
                else {
                    continue;
                };
                let Some(usage) = message.get("usage") else {
                    continue;
                };
                let tokens = tokens_from_usage(usage);
                if tokens.total() == 0 {
                    continue;
                }
                let timestamp = normalize_timestamp(
                    message
                        .get("timestamp")
                        .and_then(Value::as_i64)
                        .unwrap_or(fallback_timestamp),
                );
                if timestamp <= 0 {
                    continue;
                }
                pending_by_turn
                    .entry(turn_id.to_string())
                    .or_default()
                    .push(PendingUsage { timestamp, tokens });
            }
            Some("exec.result") => {
                let Some(turn_id) = value
                    .get("turnId")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|turn| !turn.is_empty())
                else {
                    continue;
                };
                let Some(session_id) = value
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|session| !session.is_empty())
                else {
                    continue;
                };
                let Some(model) = value.get("model") else {
                    continue;
                };
                let Some(provider_id) = model
                    .get("providerId")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|provider| !provider.is_empty())
                else {
                    continue;
                };
                let Some(model_id) = model
                    .get("modelId")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|model| !model.is_empty())
                else {
                    continue;
                };
                let Some(pending) = pending_by_turn.remove(turn_id) else {
                    continue;
                };

                for (index, usage) in pending.into_iter().enumerate() {
                    let dedup_key = format!(
                        "mcode:{session_id}:{turn_id}:{index}:{}:{}:{}:{}",
                        usage.tokens.input,
                        usage.tokens.output,
                        usage.tokens.cache_read,
                        usage.tokens.cache_write
                    );
                    let mut message = UnifiedMessage::new_with_dedup(
                        "mcode",
                        model_id,
                        provider_id,
                        session_id,
                        usage.timestamp,
                        usage.tokens,
                        0.0,
                        Some(dedup_key),
                    );
                    message.agent = Some("headless".to_string());
                    message.is_turn_start = index == 0;
                    messages.push(message);
                }
            }
            _ => {}
        }
    }

    messages
}

fn tokens_from_usage(usage: &Value) -> TokenBreakdown {
    TokenBreakdown {
        input: int_field(usage, "inputTokens"),
        output: int_field(usage, "outputTokens"),
        cache_read: int_field(usage, "cacheReadTokens"),
        cache_write: int_field(usage, "cacheWriteTokens"),
        reasoning: 0,
    }
}

fn int_field(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0).max(0)
}

fn normalize_timestamp(timestamp: i64) -> i64 {
    if timestamp > 0 && timestamp < 10_000_000_000 {
        timestamp.saturating_mul(1_000)
    } else {
        timestamp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn an_undecodable_byte_does_not_discard_the_rest_of_the_capture() {
        // `lines().map_while(Result::ok)` would end the iteration here, and
        // because usage is only flushed when `exec.result` is reached, the
        // whole capture would come back empty rather than merely truncated.
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(
            br#"{"type":"message","message":{"turnId":"turn-1","role":"assistant","timestamp":1786800000000,"usage":{"inputTokens":11,"outputTokens":3}}}"#,
        )
        .unwrap();
        file.write_all(b"\n").unwrap();
        // A projected event carrying a raw non-UTF-8 byte, before the result.
        file.write_all(br#"{"type":"message","message":{"note":""#)
            .unwrap();
        file.write_all(&[0xff]).unwrap();
        file.write_all(br#""}}"#).unwrap();
        file.write_all(b"\n").unwrap();
        file.write_all(
            br#"{"schemaVersion":1,"type":"exec.result","sessionId":"session-1","turnId":"turn-1","status":"succeeded","model":{"providerId":"minimax","modelId":"MiniMax-M2.5"},"durationMs":10}"#,
        )
        .unwrap();
        file.write_all(b"\n").unwrap();
        file.flush().unwrap();

        let messages = parse_mcode_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].model_id, "MiniMax-M2.5");
        assert_eq!(messages[0].tokens.input, 11);
        assert_eq!(messages[0].tokens.output, 3);
    }

    #[test]
    fn pairs_authoritative_usage_with_the_final_model_identity() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"{{"type":"message","message":{{"turnId":"turn-1","role":"assistant","timestamp":1786800000000,"usage":{{"totalTokens":24165,"inputTokens":20468,"outputTokens":46,"cacheReadTokens":3651}}}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"message","message":{{"turnId":"turn-1","role":"assistant","timestamp":1786800001000,"usage":{{"inputTokens":-10,"outputTokens":5,"cacheWriteTokens":7}}}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"schemaVersion":1,"type":"exec.result","sessionId":"session-1","turnId":"turn-1","status":"succeeded","model":{{"providerId":"minimax","modelId":"MiniMax-M2.5","variant":"fast"}},"durationMs":10}}"#
        )
        .unwrap();

        let messages = parse_mcode_file(file.path());
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].client, "mcode");
        assert_eq!(messages[0].provider_id, "minimax");
        assert_eq!(messages[0].model_id, "MiniMax-M2.5");
        assert_eq!(messages[0].session_id, "session-1");
        assert_eq!(messages[0].tokens.input, 20_468);
        assert_eq!(messages[0].tokens.output, 46);
        assert_eq!(messages[0].tokens.cache_read, 3_651);
        assert_eq!(messages[0].agent.as_deref(), Some("headless"));
        assert!(messages[0].is_turn_start);
        assert_eq!(messages[1].tokens.input, 0);
        assert_eq!(messages[1].tokens.output, 5);
        assert_eq!(messages[1].tokens.cache_write, 7);
        assert!(!messages[1].is_turn_start);
    }

    #[test]
    fn ignores_partial_or_unattributed_streams() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "not json").unwrap();
        writeln!(
            file,
            r#"{{"type":"message","message":{{"turnId":"turn-1","role":"assistant","usage":{{"inputTokens":10}}}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"schemaVersion":1,"type":"exec.result","sessionId":"session-1","turnId":"turn-1","status":"succeeded","durationMs":10}}"#
        )
        .unwrap();

        assert!(parse_mcode_file(file.path()).is_empty());
    }
}
