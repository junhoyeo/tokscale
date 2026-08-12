//! Parser for Reasonix's authoritative append-only statistics records.
//!
//! Reasonix writes one JSON object per provider request to
//! `<REASONIX_HOME>/stats/YYYY-MM-DD.jsonl`. Session transcript JSONL is not
//! scanned: it has no authoritative usage counters and would overlap stats.

use super::pi::has_replacement_character;
use super::utils::{lossy_lines, parse_timestamp_value};
use super::UnifiedMessage;
use crate::provider_identity::{
    canonical_provider, inferred_provider_from_model, inferred_provider_from_model_delimited,
};
use crate::TokenBreakdown;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::BufReader;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct ReasonixStat {
    ts: serde_json::Value,
    #[serde(default)]
    model: String,
    #[serde(default)]
    prompt: i64,
    #[serde(default)]
    completion: i64,
    #[serde(default)]
    reasoning: i64,
    #[serde(default)]
    cache_hit: i64,
    cache_miss: Option<i64>,
    #[serde(default)]
    total: i64,
    #[serde(default)]
    requests: i64,
    #[serde(default)]
    turn: bool,
    #[serde(flatten)]
    extra_fields: BTreeMap<String, serde_json::Value>,
}

impl ReasonixStat {
    fn has_damaged_key(&self) -> bool {
        self.extra_fields
            .keys()
            .any(|key| has_replacement_character(key))
    }
}

fn split_model_ref(model_ref: &str) -> (String, String) {
    let model_ref = model_ref.trim();
    if let Some((provider, model)) = model_ref.split_once('/') {
        let model = if has_replacement_character(model) {
            "unknown"
        } else {
            model
        };
        let provider = if has_replacement_character(provider) {
            inferred_provider_from_model_delimited(model)
                .unwrap_or("reasonix")
                .to_string()
        } else {
            canonical_provider(provider).unwrap_or_else(|| provider.to_string())
        };
        return (provider, model.to_string());
    }
    if has_replacement_character(model_ref) {
        return ("reasonix".to_string(), "unknown".to_string());
    }
    let provider = inferred_provider_from_model(model_ref)
        .unwrap_or("reasonix")
        .to_string();
    (provider, model_ref.to_string())
}

fn non_negative(value: i64) -> i64 {
    value.max(0)
}

pub fn parse_reasonix_file(path: &Path) -> Vec<UnifiedMessage> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };

    lossy_lines(BufReader::new(file))
        .enumerate()
        .filter_map(|(line_index, line)| {
            let record: ReasonixStat = serde_json::from_str(line.trim()).ok()?;
            if record.has_damaged_key()
                || record.turn
                || record.model.trim().is_empty()
                || (record.total <= 0 && record.requests <= 0)
            {
                return None;
            }
            let timestamp = parse_timestamp_value(&record.ts)?;
            let (provider_id, model_id) = split_model_ref(&record.model);
            let cache_read = non_negative(record.cache_hit);
            let raw_input = non_negative(record.prompt);
            // An explicit nonzero cache miss is Reasonix's authoritative
            // ordinary-input bucket. Older records omit it, so derive that
            // bucket from prompt tokens and cache hits in that case.
            let input = match record.cache_miss {
                Some(cache_miss) if cache_miss != 0 => non_negative(cache_miss),
                _ => raw_input.saturating_sub(cache_read),
            };
            let reasoning = non_negative(record.reasoning).min(non_negative(record.completion));
            let tokens = TokenBreakdown {
                input,
                output: non_negative(record.completion).saturating_sub(reasoning),
                cache_read,
                cache_write: 0,
                reasoning,
            };
            let mut message = UnifiedMessage::new_with_dedup(
                "reasonix",
                model_id,
                provider_id,
                format!("reasonix-stats:{}", path.display()),
                timestamp,
                tokens,
                0.0,
                Some(format!(
                    "reasonix:{}:{}:{}:{}",
                    path.display(),
                    line_index,
                    record.requests,
                    record.total
                )),
            );
            message.message_count = record.requests.clamp(1, i64::from(i32::MAX)) as i32;
            Some(message)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn accepts_bom_crlf_and_later_records_after_invalid_utf8() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            b"\xef\xbb\xbf{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"deepseek/chat\",\"prompt\":100,\"completion\":20,\"cache_hit\":30,\"cache_miss\":70,\"total\":120}\r\n",
        )
        .unwrap();
        file.write_all(b"invalid \xff record\r\n").unwrap();
        file.write_all(
            br#"{"ts":"2026-08-04T09:11:11Z","model":"deepseek/chat","prompt":10,"completion":2,"total":12}"#,
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_reasonix_file(file.path());

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tokens.total(), 120);
        assert_eq!(messages[1].tokens.total(), 12);
    }

    #[test]
    fn rejects_replacement_mangled_counter_keys() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            b"{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"deepseek/chat\",\"prom\xffpt\":100,\"completion\":20,\"total\":120}\n",
        )
        .unwrap();
        file.write_all(
            "{\"ts\":\"2026-08-04T09:10:12Z\",\"model\":\"deepseek/chat\",\"cache_�hit\":30,\"prompt\":100,\"completion\":20,\"total\":120}\n"
                .as_bytes(),
        )
        .unwrap();
        file.write_all(
            b"{\"ts\":\"2026-08-04T09:10:13Z\",\"model\":\"deepseek/chat\",\"prompt\":10,\"completion\":2,\"total\":12}\n",
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_reasonix_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.total(), 12);
    }

    #[test]
    fn sanitizes_replacement_mangled_provider_and_model() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(
            b"{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"bad-\xff/bad-\xff-model\",\"prompt\":100,\"completion\":20,\"total\":120}\n",
        )
        .unwrap();
        file.flush().unwrap();

        let messages = parse_reasonix_file(file.path());

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "reasonix");
        assert_eq!(messages[0].model_id, "unknown");
    }

    #[test]
    fn damaged_provider_uses_delimiter_aware_model_inference() {
        for (model, expected_provider) in [
            ("agpt-foo", "reasonix"),
            ("declaude-x", "reasonix"),
            ("unqwened-model", "reasonix"),
            ("minimaximal", "reasonix"),
            ("gpt-5", "openai"),
            ("gpt4-turbo", "openai"),
            ("claude-sonnet-4", "anthropic"),
            ("claude3-opus", "anthropic"),
            ("qwen3-coder", "qwen"),
        ] {
            let mut file = NamedTempFile::new().unwrap();
            writeln!(
                file,
                "{{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"bad-�/{model}\",\"prompt\":100,\"completion\":20,\"total\":120}}"
            )
            .unwrap();
            file.flush().unwrap();

            let messages = parse_reasonix_file(file.path());

            assert_eq!(messages.len(), 1, "{model}");
            assert_eq!(messages[0].provider_id, expected_provider, "{model}");
            assert_eq!(messages[0].model_id, model, "{model}");
        }
    }

    #[test]
    fn parses_authoritative_stats_with_provider_usage_and_timestamp() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            concat!(
                "{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"opencode/deepseek-v4\",\"prompt\":100,\"completion\":20,\"reasoning\":5,\"cache_hit\":30,\"cache_miss\":70,\"total\":120,\"requests\":1}\n",
                "{\"ts\":\"2026-08-04T09:11:11Z\",\"turn\":true}\n",
            ),
        )
        .unwrap();

        let messages = parse_reasonix_file(file.path());
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert_eq!(message.client, "reasonix");
        assert_eq!(message.provider_id, "opencode");
        assert_eq!(message.model_id, "deepseek-v4");
        assert_eq!(message.tokens.input, 70);
        assert_eq!(message.tokens.output, 15);
        assert_eq!(message.tokens.reasoning, 5);
        assert_eq!(message.tokens.cache_read, 30);
        assert_eq!(message.tokens.cache_write, 0);
        assert_eq!(message.tokens.total(), 120);
        assert_eq!(message.message_count, 1);
        assert_eq!(
            message.timestamp,
            parse_timestamp_value(&serde_json::json!("2026-08-04T09:10:11Z")).unwrap()
        );
    }

    #[test]
    fn skips_turn_markers_malformed_and_zero_usage_records() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            concat!(
                "not json\n",
                "{\"ts\":\"2026-08-04T09:10:11Z\",\"turn\":true}\n",
                "{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"deepseek/test\",\"total\":0}\n",
            ),
        )
        .unwrap();
        assert!(parse_reasonix_file(file.path()).is_empty());
    }

    #[test]
    fn preserves_unknown_model_provider_as_reasonix_only_when_not_inferable() {
        assert_eq!(
            split_model_ref("deepseek/chat"),
            ("deepseek".into(), "chat".into())
        );
        assert_eq!(
            split_model_ref("openrouter/google/gemini-2.5-pro"),
            ("openrouter".into(), "google/gemini-2.5-pro".into())
        );
        assert_eq!(
            split_model_ref("claude-sonnet-4"),
            ("anthropic".into(), "claude-sonnet-4".into())
        );
    }

    #[test]
    fn preserves_explicit_cache_miss_when_it_disagrees_with_prompt_input() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"deepseek/chat\",\"prompt\":100,\"completion\":20,\"cache_hit\":30,\"cache_miss\":10,\"total\":120}\n",
        )
        .unwrap();

        let messages = parse_reasonix_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 10);
        assert_eq!(messages[0].tokens.cache_read, 30);
        assert_eq!(messages[0].tokens.total(), 60);
    }

    #[test]
    fn falls_back_to_prompt_minus_cache_hit_when_cache_miss_is_absent() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"deepseek/chat\",\"prompt\":100,\"completion\":20,\"cache_hit\":30,\"total\":120}\n",
        )
        .unwrap();

        let messages = parse_reasonix_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.input, 70);
        assert_eq!(messages[0].tokens.total(), 120);
    }

    #[test]
    fn maps_authoritative_request_count_to_bounded_message_count() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            concat!(
                "{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"deepseek/chat\",\"prompt\":1,\"completion\":1,\"total\":2,\"requests\":3}\n",
                "{\"ts\":\"2026-08-04T09:11:11Z\",\"model\":\"deepseek/chat\",\"prompt\":1,\"completion\":1,\"total\":2,\"requests\":0}\n",
                "{\"ts\":\"2026-08-04T09:12:11Z\",\"model\":\"deepseek/chat\",\"prompt\":1,\"completion\":1,\"total\":2,\"requests\":9999999999}\n",
            ),
        )
        .unwrap();

        let messages = parse_reasonix_file(file.path());
        assert_eq!(
            messages
                .iter()
                .map(|message| message.message_count)
                .collect::<Vec<_>>(),
            vec![3, 1, i32::MAX]
        );
    }

    #[test]
    fn preserves_tokenless_request_counts_but_skips_plain_zero_rows() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            concat!(
                "{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"deepseek/chat\",\"total\":0,\"requests\":2}\n",
                "{\"ts\":\"2026-08-04T09:11:11Z\",\"model\":\"deepseek/chat\",\"total\":0}\n",
            ),
        )
        .unwrap();

        let messages = parse_reasonix_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tokens.total(), 0);
        assert_eq!(messages[0].message_count, 2);
    }
}
