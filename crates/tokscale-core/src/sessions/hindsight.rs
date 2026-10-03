//! Hindsight memory service LLM usage parser.
//!
//! Hindsight (<https://github.com/vectorize-io/hindsight>) is a self-hosted
//! memory service for AI agents. It does not produce local session log files;
//! instead, it records LLM trace metadata into an internal database and exposes
//! it via `GET /v1/{tenant}/banks/{bank_id}/llm-requests`.
//!
//! Because Hindsight's internal trace table is a rolling window (records older
//! than the current service run or after ~90 minutes are evicted), the client
//! integration relies on `tokscale hindsight sync` to mirror API records into a
//! local, append-only JSONL log at `$HINDSIGHT_HOME/usage/<YYYY-MM>.jsonl`
//! (defaulting to `~/.hindsight/usage/<YYYY-MM>.jsonl`). This parser reads that
//! ledger.
//!
//! # Token Semantics and Provider Behavior
//!
//! - **Cached tokens:** OpenAI-compatible, Responses API, and Gemini adapters
//!   report `cached_tokens` inside `input_tokens`; the cached subset is split
//!   into `cache_read` so totals and pricing count it once. Anthropic reports
//!   cache reads beside input, so its input is kept unchanged. Providers with
//!   unverified semantics retain the existing separate buckets. Missing or
//!   null cached values (including observed Ollama Cloud rows) become `0`.
//! - **Reasoning tokens:** Reported as `0`. Empirical testing against Ollama
//!   Cloud verified that `completion_tokens` already includes reasoning output
//!   (for example, a response with 4 characters of visible content and 409
//!   characters of reasoning was billed 138 completion tokens). While Hindsight's
//!   `TokenUsage` schema documents `output_tokens` as excluding reasoning and
//!   defines an unpopulated `thoughts_tokens`, in practice on this path reasoning
//!   is already inside `output_tokens`. Setting `reasoning: 0` prevents
//!   double-counting or erroneous subtraction from output tokens.
//! - **Total tokens:** `total_tokens == input_tokens + output_tokens` on every
//!   observed row.
//! - **Failed calls:** Only successful token-bearing calls are retained; failed
//!   calls yield null token fields and are skipped.

use super::utils::lossy_lines;
use super::UnifiedMessage;
use crate::TokenBreakdown;
use chrono::DateTime;
use serde::Deserialize;
use std::io::BufReader;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct HindsightRecord {
    id: String,
    #[serde(default)]
    trace_id: Option<String>,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    operation: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    duration_ms: Option<i64>,
    #[serde(default)]
    input_tokens: Option<i64>,
    #[serde(default)]
    output_tokens: Option<i64>,
    #[serde(default)]
    cached_tokens: Option<i64>,
    #[serde(default)]
    total_tokens: Option<i64>,
    #[serde(default)]
    bank: Option<String>,
}

/// Parse a Hindsight append-only monthly usage JSONL file.
///
/// Malformed rows and rows with unparseable timestamps or empty identities are
/// skipped defensively.
pub fn parse_hindsight_file(path: &Path) -> Vec<UnifiedMessage> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };

    lossy_lines(BufReader::new(file))
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }

            let record: HindsightRecord = serde_json::from_str(line).ok()?;

            // Hindsight nulls every token field on a failed call, and
            // `tokscale hindsight sync` refuses to write those rows. Guard
            // again here, because the ledger is a plain file a user may edit:
            // a stated total that is not positive is a row asserting it has no
            // usage, and the token check below covers a row that never stated
            // one.
            if record.total_tokens.is_some_and(|total| total <= 0) {
                return None;
            }

            let started_at = record.started_at.as_deref()?;
            let timestamp = DateTime::parse_from_rfc3339(started_at)
                .ok()?
                .timestamp_millis();

            let session_id = record
                .trace_id
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| record.id.clone());

            let input = record.input_tokens.unwrap_or(0).max(0);
            let output = record.output_tokens.unwrap_or(0).max(0);
            if input == 0 && output == 0 {
                return None;
            }
            let cache_read = record.cached_tokens.unwrap_or(0).max(0);
            // Adapter routing in vectorize-io/hindsight at 017b3f5d:
            // engine/llm_wrapper.py maps these providers to OpenAI-compatible,
            // Responses API, or Gemini usage with cache-inclusive input.
            // Anthropic and unknown adapters keep their existing semantics;
            // a cache count above input is not a valid subset to subtract.
            let input = if cache_read > 0
                && cache_read <= input
                && matches!(
                    record.provider.to_ascii_lowercase().as_str(),
                    "openai"
                        | "groq"
                        | "ollama"
                        | "ollama-cloud"
                        | "lmstudio"
                        | "minimax"
                        | "deepseek"
                        | "volcano"
                        | "openrouter"
                        | "requesty"
                        | "zai"
                        | "opencode-go"
                        | "atlas"
                        | "meta"
                        | "openai-responses"
                        | "gemini"
                        | "vertexai"
                ) {
                input - cache_read
            } else {
                input
            };
            let cache_write = 0;
            // Reasoning is already inside `output_tokens`. Hindsight's
            // `TokenUsage` schema documents the field as excluding reasoning,
            // and that documentation does not hold for this path: a probe
            // returning 4 characters of visible content plus 409 characters of
            // reasoning was billed 138 completion tokens. Splitting a share
            // out here would double-count it.
            let reasoning = 0;

            let tokens = TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write,
                cache_write_1h: 0,
                reasoning,
            };

            let dedup_key = format!("hindsight:{}", record.id);

            let mut message = UnifiedMessage::new_with_dedup(
                "hindsight",
                record.model,
                record.provider,
                session_id,
                timestamp,
                tokens,
                0.0,
                Some(dedup_key),
            );

            let bank = record.bank.filter(|b| !b.trim().is_empty());
            message.workspace_key = bank.clone();
            message.workspace_label = bank;
            message.duration_ms = record.duration_ms.map(|d| d.max(0));
            // Hindsight's own labels for the call: `consolidation`,
            // `retain`/`retain_extract_facts`, `refresh_mental_model`, and so
            // on. The scope narrows the operation, so it only earns a place in
            // the title when it says something the operation does not.
            message.session_title = match (
                record.operation.filter(|o| !o.trim().is_empty()),
                record.scope.filter(|s| !s.trim().is_empty()),
            ) {
                (Some(operation), Some(scope)) if scope != operation => {
                    Some(format!("{operation} / {scope}"))
                }
                (Some(operation), _) => Some(operation),
                (None, scope) => scope,
            };

            Some(message)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const SAMPLE_FIXTURE: &str = r#"{"id":"aa4cc970-33b8-4234-996d-b6e05a5a1bb2","trace_id":"51b96021-07fc-4a64-b333-5c8c72833a44","provider":"ollama-cloud","model":"deepseek-v4-flash:0731","operation":"retain","scope":"retain_extract_facts","started_at":"2026-09-01T08:16:51.357104+00:00","ended_at":"2026-09-01T08:17:43.904885+00:00","duration_ms":52547,"input_tokens":3667,"output_tokens":4063,"cached_tokens":null,"total_tokens":7730,"bank":"omp"}
{"id":"77319709-1e81-400e-8aa9-2e31779f9065","trace_id":"76c6cba6-3dab-43f5-9777-4770202df639","provider":"ollama-cloud","model":"deepseek-v4-flash:0731","operation":"consolidation","scope":"consolidation","started_at":"2026-09-01T08:17:51.558167+00:00","ended_at":"2026-09-01T08:18:48.283320+00:00","duration_ms":56725,"input_tokens":69860,"output_tokens":6532,"cached_tokens":null,"total_tokens":76392,"bank":"omp"}
{"id":"2f85da10-188e-4746-8638-72d3157ae68e","trace_id":"f01c47c4-6283-4631-99bf-b1417a1ce4c1","provider":"ollama-cloud","model":"deepseek-v4-flash:0731","operation":"refresh_mental_model","scope":"reflect_tool_call","started_at":"2026-09-01T12:53:27.493913+00:00","ended_at":"2026-09-01T12:53:50.992949+00:00","duration_ms":23499,"input_tokens":2787,"output_tokens":360,"cached_tokens":null,"total_tokens":3147,"bank":"george"}
{"id":"c18ec2c3-2fa2-47b7-86ab-11be3ee6dd29","trace_id":"e7ba34ff-baf8-42aa-bd7a-b09c127a1df6","provider":"ollama-cloud","model":"deepseek-v4-flash:0731","operation":"retain","scope":"retain_extract_facts","started_at":"2026-09-01T12:55:33.373387+00:00","ended_at":"2026-09-01T12:56:10.594632+00:00","duration_ms":37221,"input_tokens":3586,"output_tokens":3969,"cached_tokens":null,"total_tokens":7555,"bank":"george"}
"#;

    #[test]
    fn parses_happy_path_and_maps_all_fields() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(SAMPLE_FIXTURE.as_bytes()).unwrap();
        file.flush().unwrap();

        let messages = parse_hindsight_file(file.path());
        assert_eq!(messages.len(), 4);

        let first = &messages[0];
        assert_eq!(first.client, "hindsight");
        assert_eq!(first.model_id, "deepseek-v4-flash:0731");
        assert_eq!(first.provider_id, "ollama-cloud");
        assert_eq!(first.session_id, "51b96021-07fc-4a64-b333-5c8c72833a44");
        assert_eq!(first.workspace_key, Some("omp".to_string()));
        assert_eq!(first.workspace_label, Some("omp".to_string()));
        assert_eq!(
            first.timestamp,
            DateTime::parse_from_rfc3339("2026-09-01T08:16:51.357104+00:00")
                .unwrap()
                .timestamp_millis()
        );
        assert_eq!(first.date, "2026-09-01");
        assert_eq!(first.tokens.input, 3667);
        assert_eq!(first.tokens.output, 4063);
        assert_eq!(first.tokens.cache_read, 0);
        assert_eq!(first.tokens.cache_write, 0);
        assert_eq!(first.tokens.reasoning, 0);
        assert_eq!(first.tokens.total(), 7730);
        assert_eq!(first.duration_ms, Some(52547));
        assert_eq!(first.message_count, 1);
        assert_eq!(first.cost, 0.0);
        assert_eq!(
            first.dedup_key,
            Some("hindsight:aa4cc970-33b8-4234-996d-b6e05a5a1bb2".to_string())
        );
        assert_eq!(
            first.session_title,
            Some("retain / retain_extract_facts".to_string())
        );

        // Verify that bank "george" is mapped correctly on the third record
        let third = &messages[2];
        assert_eq!(third.workspace_key, Some("george".to_string()));
        assert_eq!(third.workspace_label, Some("george".to_string()));
        assert_eq!(third.session_id, "f01c47c4-6283-4631-99bf-b1417a1ce4c1");
        assert_eq!(third.tokens.input, 2787);
        assert_eq!(third.tokens.output, 360);
        assert_eq!(third.tokens.total(), 3147);
        assert_eq!(third.duration_ms, Some(23499));
    }

    #[test]
    fn skips_malformed_and_garbage_lines() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"not json at all\n").unwrap();
        file.write_all(b"{\"id\":123}\n").unwrap(); // id should be string
        file.write_all(b"\n").unwrap();
        file.write_all(SAMPLE_FIXTURE.as_bytes()).unwrap();
        file.write_all(b"{\"id\":\"trailing-broken-json\"\n")
            .unwrap();
        file.flush().unwrap();

        let messages = parse_hindsight_file(file.path());
        assert_eq!(messages.len(), 4);
    }

    #[test]
    fn maps_missing_or_null_cached_tokens_to_zero() {
        let mut file = NamedTempFile::new().unwrap();
        // Record with explicit null cached_tokens
        file.write_all(br#"{"id":"test-null-cache","provider":"ollama-cloud","model":"deepseek-v4-flash:0731","started_at":"2026-09-01T08:16:51.357104+00:00","input_tokens":100,"output_tokens":50,"cached_tokens":null,"bank":"omp"}"#).unwrap();
        file.write_all(b"\n").unwrap();
        // Record with omitted cached_tokens
        file.write_all(br#"{"id":"test-omitted-cache","provider":"ollama-cloud","model":"deepseek-v4-flash:0731","started_at":"2026-09-01T08:16:51.357104+00:00","input_tokens":200,"output_tokens":75,"bank":"omp"}"#).unwrap();
        file.write_all(b"\n").unwrap();
        // Negative counts clamp to zero, which leaves the row with no usage at
        // all, so it is dropped rather than counted as an empty message.
        file.write_all(br#"{"id":"test-negative-tokens","provider":"ollama-cloud","model":"deepseek-v4-flash:0731","started_at":"2026-09-01T08:16:51.357104+00:00","input_tokens":-10,"output_tokens":-5,"cached_tokens":-1,"bank":"omp"}"#).unwrap();
        file.write_all(b"\n").unwrap();
        // A failed Hindsight call: every token field null.
        file.write_all(br#"{"id":"test-failed-call","provider":"ollama-cloud","model":"deepseek-v4-flash:0731","started_at":"2026-09-01T08:16:51.357104+00:00","input_tokens":null,"output_tokens":null,"cached_tokens":null,"total_tokens":null,"bank":"omp"}"#).unwrap();
        file.write_all(b"\n").unwrap();
        file.flush().unwrap();

        let messages = parse_hindsight_file(file.path());
        assert_eq!(messages.len(), 2);

        assert_eq!(messages[0].tokens.cache_read, 0);
        assert_eq!(messages[0].tokens.input, 100);
        assert_eq!(messages[0].tokens.output, 50);

        assert_eq!(messages[1].tokens.cache_read, 0);
        assert_eq!(messages[1].tokens.input, 200);
        assert_eq!(messages[1].tokens.output, 75);
    }

    #[test]
    fn skips_unparseable_started_at_timestamps() {
        let mut file = NamedTempFile::new().unwrap();
        // Missing started_at
        file.write_all(br#"{"id":"test-missing-time","provider":"ollama-cloud","model":"deepseek-v4-flash:0731","input_tokens":100,"output_tokens":50,"bank":"omp"}"#).unwrap();
        file.write_all(b"\n").unwrap();
        // Invalid non-RFC3339 string
        file.write_all(br#"{"id":"test-invalid-time","provider":"ollama-cloud","model":"deepseek-v4-flash:0731","started_at":"not-a-timestamp","input_tokens":100,"output_tokens":50,"bank":"omp"}"#).unwrap();
        file.write_all(b"\n").unwrap();
        // Valid started_at
        file.write_all(br#"{"id":"test-valid-time","provider":"ollama-cloud","model":"deepseek-v4-flash:0731","started_at":"2026-09-01T08:16:51.357104+00:00","input_tokens":100,"output_tokens":50,"bank":"omp"}"#).unwrap();
        file.write_all(b"\n").unwrap();
        file.flush().unwrap();

        let messages = parse_hindsight_file(file.path());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].session_id, "test-valid-time");
    }

    #[test]
    fn dedup_keys_are_unique_per_request_id() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(SAMPLE_FIXTURE.as_bytes()).unwrap();
        file.flush().unwrap();

        let messages = parse_hindsight_file(file.path());
        assert_eq!(messages.len(), 4);

        let mut seen = HashSet::new();
        for msg in &messages {
            let key = msg.dedup_key.as_ref().expect("dedup_key must be populated");
            assert!(
                key.starts_with("hindsight:"),
                "dedup_key should have hindsight prefix: {key}"
            );
            assert!(seen.insert(key.clone()), "duplicate dedup key found: {key}");
        }
        assert_eq!(seen.len(), 4);
    }
}

#[cfg(test)]
mod cache_token_tests {
    use super::*;
    use crate::pricing::{ModelPricing, PricingService};
    use std::collections::HashMap;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn parse_usage(provider: &str, input: i64, output: i64, cached: i64) -> UnifiedMessage {
        let row = serde_json::json!({
            "id": "cached-request", "provider": provider, "model": "gpt-test",
            "started_at": "2026-09-01T08:16:51Z", "input_tokens": input,
            "output_tokens": output, "cached_tokens": cached,
        });
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "{row}").unwrap();
        let mut messages = parse_hindsight_file(file.path());
        assert_eq!(messages.len(), 1);
        messages.pop().unwrap()
    }

    #[test]
    fn splits_cache_inclusive_adapter_usage_without_inflating_totals() {
        for provider in [
            "openai",
            "groq",
            "ollama",
            "ollama-cloud",
            "lmstudio",
            "minimax",
            "deepseek",
            "volcano",
            "openrouter",
            "requesty",
            "zai",
            "opencode-go",
            "atlas",
            "meta",
            "openai-responses",
            "gemini",
            "vertexai",
            "OpenAI",
        ] {
            let message = parse_usage(provider, 100, 20, 40);
            assert_eq!(message.tokens.input, 60, "{provider}");
            assert_eq!(message.tokens.cache_read, 40, "{provider}");
            assert_eq!(message.tokens.total(), 120, "{provider}");
            assert_eq!(message.provider_id, provider);
        }
    }

    #[test]
    fn preserves_anthropic_and_unverified_adapter_usage() {
        for provider in [
            "anthropic",
            "litellm",
            "bedrock",
            "fireworks",
            "unknown",
            "",
        ] {
            let message = parse_usage(provider, 100, 20, 40);
            assert_eq!(message.tokens.input, 100, "{provider}");
            assert_eq!(message.tokens.cache_read, 40, "{provider}");
            assert_eq!(message.tokens.total(), 160, "{provider}");
        }
    }

    #[test]
    fn retains_fully_cached_input_with_no_output() {
        let message = parse_usage("openai", 100, 0, 100);
        assert_eq!(message.tokens.input, 0);
        assert_eq!(message.tokens.cache_read, 100);
        assert_eq!(message.tokens.total(), 100);
    }

    #[test]
    fn preserves_inconsistent_cache_counts_and_clamps_negative_counts() {
        let inconsistent = parse_usage("openai", 10, 20, 40);
        assert_eq!(inconsistent.tokens.input, 10);
        assert_eq!(inconsistent.tokens.cache_read, 40);
        let negative_cache = parse_usage("openai", 100, 20, -40);
        assert_eq!(negative_cache.tokens.input, 100);
        assert_eq!(negative_cache.tokens.cache_read, 0);
        let negative_input = parse_usage("openai", -10, 20, 40);
        assert_eq!(negative_input.tokens.input, 0);
        assert_eq!(negative_input.tokens.cache_read, 40);
    }

    #[test]
    fn prices_cached_input_only_at_the_cache_rate() {
        let message = parse_usage("openai", 100, 20, 40);
        let pricing = PricingService::new(
            HashMap::from([(
                "gpt-test".to_string(),
                ModelPricing {
                    input_cost_per_token: Some(0.01),
                    output_cost_per_token: Some(0.02),
                    cache_read_input_token_cost: Some(0.001),
                    ..Default::default()
                },
            )]),
            HashMap::new(),
        );
        let cost = pricing.calculate_cost_with_provider(
            &message.model_id,
            Some(&message.provider_id),
            &message.tokens,
        );
        assert!((cost - 1.04).abs() < 1e-9, "cost was {cost}");
    }
}
