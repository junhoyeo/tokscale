//! OpenCodex gateway adapter.
//!
//! Codex rollouts only know the caller-facing selector, while OpenCodex's
//! durable usage ledger records the physical attempts. This adapter turns that
//! ledger into normalized [`GatewayRecord`]s; the shared matcher correlates
//! them with Codex usage rows and the shared resolver applies the result.
//!
//! A row qualifies only when it is a successful Responses-protocol combo
//! request with a requested selector, a start time, and a recorded duration.
//! Its physical identity comes from `servedModel`, the model the upstream
//! response reported serving — never from the wire model a route intended to
//! send. The row must have exactly one successful reported attempt; anything
//! else leaves the row reserved with no target, so it blocks attribution
//! rather than yielding to a weaker candidate.

use std::ffi::OsStr;
use std::path::PathBuf;

use crate::model_attribution::matcher::{GatewayIndex, GatewayRecord, MatchPolicy, TokenSignature};
use crate::model_attribution::{cached_index, read_ledger};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cached_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    cache_write_input_tokens: Option<i64>,
    reasoning_output_tokens: Option<i64>,
    estimated: Option<bool>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedTierOutcome {
    response_service_tier: Option<String>,
    response_tier_authoritative: Option<bool>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedAttempt {
    ordinal: Option<i64>,
    provider: Option<String>,
    model: Option<String>,
    adapter: Option<String>,
    status: Option<i32>,
    duration_ms: Option<i64>,
    send_count: Option<i64>,
    usage_status: Option<String>,
    usage: Option<PersistedUsage>,
    stream_aborted: Option<bool>,
    locally_answered: Option<bool>,
    tier_outcome: Option<PersistedTierOutcome>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedUsageRow {
    timestamp: Option<i64>,
    provider: Option<String>,
    requested_model: Option<String>,
    served_model: Option<String>,
    inbound_protocol: Option<String>,
    status: Option<i32>,
    duration_ms: Option<i64>,
    attempts: Option<Vec<PersistedAttempt>>,
}

/// Decode one ledger row into normalized evidence, or nothing when the row
/// cannot describe a successful Responses combo call.
fn decode_row(line: &[u8]) -> Option<GatewayRecord> {
    let row: PersistedUsageRow = serde_json::from_slice(line).ok()?;
    if row.provider.as_deref() != Some("combo")
        || row.inbound_protocol.as_deref() != Some("responses")
        || !row
            .status
            .is_some_and(|status| (200..300).contains(&status))
    {
        return None;
    }

    // The selector is required evidence: a row that cannot name what Codex
    // asked for cannot correlate anything and is not a claim.
    let selector = row
        .requested_model
        .as_deref()
        .filter(|selector| !selector.is_empty())?
        .to_string();
    let timestamp = row.timestamp.filter(|timestamp| *timestamp > 0)?;
    let duration_ms = row.duration_ms.filter(|duration| *duration >= 0)?;

    let attempts = row.attempts.unwrap_or_default();
    let eligible: Vec<&PersistedAttempt> = attempts
        .iter()
        .filter(|attempt| attempt_eligible(attempt))
        .collect();
    let signatures = eligible
        .iter()
        .filter_map(|attempt| attempt.usage.as_ref().and_then(usage_signature))
        .collect::<Vec<_>>();
    let unique_target = (eligible.len() == 1).then(|| &eligible[0]);
    let provider = unique_target.and_then(|attempt| attempt.provider.clone());
    let served_model = row.served_model.clone().filter(|model| !model.is_empty());
    let service_tier = unique_target
        .and_then(|attempt| attempt.tier_outcome.as_ref())
        .filter(|tier| tier.response_tier_authoritative == Some(true))
        .and_then(|tier| tier.response_service_tier.clone())
        .filter(|tier| !tier.is_empty());

    Some(GatewayRecord {
        adapter: "opencodex",
        selector: Some(selector),
        selected_model: None,
        served_model,
        provider,
        service_tier,
        session_ids: Vec::new(),
        stable_ids: Vec::new(),
        start_ms: timestamp,
        duration_ms: Some(duration_ms),
        signatures,
        completed: true,
        target_eligible: eligible.len() == 1,
        // A confirmed selection is not enough here: the row must report the
        // model the response named.
        allow_selected_fallback: false,
    })
}

/// A 2xx reported attempt whose required bookkeeping is intact. Estimates are
/// not observations, and a locally answered or stream-aborted attempt is not
/// the response Codex received.
fn attempt_eligible(attempt: &PersistedAttempt) -> bool {
    attempt.ordinal.is_some_and(|ordinal| ordinal > 0)
        && attempt
            .provider
            .as_deref()
            .is_some_and(|provider| !provider.is_empty())
        && attempt
            .model
            .as_deref()
            .is_some_and(|model| !model.is_empty())
        && attempt
            .adapter
            .as_deref()
            .is_some_and(|adapter| !adapter.is_empty())
        && attempt
            .status
            .is_some_and(|status| (200..300).contains(&status))
        && attempt.duration_ms.is_some_and(|duration| duration >= 0)
        && attempt.send_count.is_some_and(|sends| sends >= 0)
        && attempt.usage_status.as_deref() == Some("reported")
        && attempt.stream_aborted != Some(true)
        && attempt.locally_answered != Some(true)
        && attempt
            .usage
            .as_ref()
            .is_some_and(|usage| usage.estimated != Some(true))
}

/// The attempt's counters as a comparable signature.
///
/// OpenCodex reports OpenAI-shaped usage: `inputTokens` includes the cache
/// read, and `outputTokens` includes reasoning. A row whose own counters
/// contradict each other is malformed evidence, not a candidate. Counter
/// buckets the row omits stay unreported (`None`), so a ledger written before
/// a counter existed still matches a client that reports it.
fn usage_signature(usage: &PersistedUsage) -> Option<TokenSignature> {
    let input = usage.input_tokens?;
    let output = usage.output_tokens?;
    if input < 0 || output < 0 {
        return None;
    }
    if usage
        .total_tokens
        .is_some_and(|total| total != input.saturating_add(output))
    {
        return None;
    }
    // Both spellings of the cache counter are observations; disagreeing
    // counters mean malformed evidence, not a choice between them.
    let cache_read = match (usage.cached_input_tokens, usage.cache_read_input_tokens) {
        (Some(left), Some(right)) if left != right => return None,
        (left, right) => left.or(right),
    };
    let cache_write = usage.cache_write_input_tokens.filter(|value| *value >= 0);
    Some(TokenSignature::from_wire_totals(
        input,
        output,
        cache_read.filter(|value| *value >= 0),
        cache_write,
        usage.reasoning_output_tokens.filter(|value| *value >= 0),
    ))
}

fn expand_opencodex_home(raw: &OsStr, home_dir: &str) -> PathBuf {
    let raw = raw.to_string_lossy();
    if raw == "~" {
        return PathBuf::from(home_dir);
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return PathBuf::from(home_dir).join(rest);
    }
    PathBuf::from(raw.as_ref())
}

fn usage_log_path(home_dir: &str, use_env_roots: bool) -> PathBuf {
    if use_env_roots {
        if let Some(raw) = std::env::var_os("OPENCODEX_HOME").filter(|value| !value.is_empty()) {
            return expand_opencodex_home(&raw, home_dir).join("usage.jsonl");
        }
    }
    PathBuf::from(home_dir)
        .join(".opencodex")
        .join("usage.jsonl")
}

/// Load OpenCodex's ledger as a gateway index. Returns `None` when no ledger
/// exists or it holds no usable row.
pub(crate) fn load_index(
    home_dir: &str,
    use_env_roots: bool,
) -> Option<std::sync::Arc<GatewayIndex>> {
    let path = usage_log_path(home_dir, use_env_roots);
    cached_index(&path, MatchPolicy::opencodex(), |path| {
        read_ledger(path, decode_row).ok()
    })
}

#[cfg(test)]
mod tests {
    use crate::sessions::CostSource;
    use crate::sessions::UnifiedMessage;
    use crate::TokenBreakdown;
    use serde_json::{json, Value};
    use serial_test::serial;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Drive the production attribution path over a slice of parsed rows.
    ///
    /// The adapter itself only decodes evidence; correlation, conflict
    /// detection, and application are shared with every other gateway, so a
    /// regression test here exercises the same code a real scan runs.
    fn attribute_codex_messages(
        home_dir: &str,
        use_env_roots: bool,
        messages: &mut [UnifiedMessage],
    ) -> usize {
        crate::model_attribution::clear_index_cache();
        let Some(overlay) = crate::model_attribution::GatewayOverlay::load(home_dir, use_env_roots)
        else {
            return 0;
        };

        let mut slots: Vec<Option<UnifiedMessage>> = messages
            .iter()
            .map(|message| Some(message.clone()))
            .collect();
        for slot in &mut slots {
            let is_codex = slot
                .as_ref()
                .is_some_and(|message| message.client == "codex");
            if !is_codex {
                continue;
            }
            let message = slot.take().expect("checked above");
            overlay
                .park(message, None)
                .expect("a fixture never approaches the park cap");
        }

        let mut released = Vec::new();
        overlay.finish(None, |message, _| released.push(message));
        let mut released = released.into_iter();
        let mut attributed = 0;
        for slot in &mut slots {
            if slot.is_none() {
                *slot = Some(released.next().expect("every parked row is released"));
            }
        }

        for (message, slot) in messages.iter_mut().zip(slots) {
            let next = slot.expect("every fixture row is accounted for");
            if next.model_id != message.model_id {
                attributed += 1;
            }
            *message = next;
        }
        attributed
    }

    const SELECTOR: &str = "combo/test-fixture";
    const START: i64 = 1_700_000_000_000;
    const DURATION: i64 = 10_000;
    const ROW_START_OFFSET: i64 = 4_000;
    const ROW_DURATION: i64 = 6_000;
    const WIRE_MODEL: &str = "wire-model-fixture";
    const SERVED_MODEL: &str = "served-model-fixture";
    const OTHER_SERVED_MODEL: &str = "other-served-model-fixture";
    const PROVIDER: &str = "fixture-provider";

    fn codex_message() -> UnifiedMessage {
        let mut message = UnifiedMessage::new(
            "codex",
            SELECTOR,
            "openai",
            "fixture-session",
            START,
            TokenBreakdown {
                input: 100,
                output: 20,
                cache_read: 400,
                cache_write: 0,
                reasoning: 5,
                ..Default::default()
            },
            7.0,
        );
        message.duration_ms = Some(DURATION);
        message.cost_source = CostSource::Estimated;
        message
    }

    fn usage() -> Value {
        json!({
            "inputTokens": 500,
            "outputTokens": 25,
            "totalTokens": 525,
            "cachedInputTokens": 400,
            "cacheReadInputTokens": 400,
            "reasoningOutputTokens": 5
        })
    }

    fn row_with_served(
        served: &str,
        status: i32,
        attempt_status: i32,
        usage_value: Option<Value>,
    ) -> Value {
        json!({
            "requestId": "fixture-request",
            "timestamp": START + ROW_START_OFFSET,
            "provider": "combo",
            "model": SELECTOR,
            "requestedModel": SELECTOR,
            "servedModel": served,
            "inboundProtocol": "responses",
            "status": status,
            "durationMs": ROW_DURATION,
            "usageStatus": if usage_value.is_some() { "reported" } else { "unreported" },
            "attempts": [{
                "ordinal": 1,
                "provider": PROVIDER,
                "model": WIRE_MODEL,
                "adapter": "fixture-responses",
                "status": attempt_status,
                "durationMs": ROW_DURATION,
                "sendCount": 1,
                "recoveryKinds": [],
                "usageStatus": if usage_value.is_some() { "reported" } else { "unreported" },
                "usage": usage_value
            }]
        })
    }

    fn row(status: i32, attempt_status: i32, usage_value: Option<Value>) -> Value {
        row_with_served(SERVED_MODEL, status, attempt_status, usage_value)
    }

    fn write_ledger(home: &Path, rows: &[Value], malformed_prefix: bool) {
        let dir = home.join(".opencodex");
        fs::create_dir_all(&dir).unwrap();
        let mut body = String::new();
        if malformed_prefix {
            body.push_str("{ definitely-not-json }\n");
        }
        for row in rows {
            body.push_str(&serde_json::to_string(row).unwrap());
            body.push('\n');
        }
        fs::write(dir.join("usage.jsonl"), body).unwrap();
    }

    #[test]
    fn attributes_unique_reported_attempt_to_observed_served_model() {
        let home = TempDir::new().unwrap();
        write_ledger(home.path(), &[row(200, 200, Some(usage()))], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            1
        );
        assert_eq!(messages[0].provider_id, PROVIDER);
        assert_eq!(messages[0].model_id, SERVED_MODEL);
        assert_ne!(messages[0].model_id, WIRE_MODEL);
        assert_eq!(messages[0].cost, 0.0);
        assert_eq!(messages[0].cost_source, CostSource::Unknown);
    }

    #[test]
    fn malformed_and_oversized_lines_do_not_hide_a_valid_row() {
        let home = TempDir::new().unwrap();
        let dir = home.path().join(".opencodex");
        fs::create_dir_all(&dir).unwrap();
        let mut body = b"{ definitely-not-json }\n".to_vec();
        body.extend(std::iter::repeat_n(
            b'x',
            crate::model_attribution::MAX_LEDGER_LINE_BYTES + 1,
        ));
        body.push(b'\n');
        body.extend(serde_json::to_vec(&row(200, 200, Some(usage()))).unwrap());
        body.push(b'\n');
        fs::write(dir.join("usage.jsonl"), body).unwrap();
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            1
        );
        assert_eq!(messages[0].model_id, SERVED_MODEL);
    }

    #[test]
    fn cancelled_or_unreported_attempt_is_not_used() {
        let home = TempDir::new().unwrap();
        write_ledger(home.path(), &[row(499, 499, None)], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn missing_usage_status_fails_closed() {
        let home = TempDir::new().unwrap();
        let mut value = row(200, 200, Some(usage()));
        value["attempts"][0]
            .as_object_mut()
            .unwrap()
            .remove("usageStatus");
        write_ledger(home.path(), &[value], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn missing_observed_served_model_fails_closed() {
        let home = TempDir::new().unwrap();
        let mut value = row(200, 200, Some(usage()));
        value.as_object_mut().unwrap().remove("servedModel");
        write_ledger(home.path(), &[value], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
        assert_eq!(messages[0].cost, 7.0);
    }

    #[test]
    fn missing_requested_selector_fails_closed() {
        let home = TempDir::new().unwrap();
        let mut value = row(200, 200, Some(usage()));
        value.as_object_mut().unwrap().remove("requestedModel");
        write_ledger(home.path(), &[value], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
    }

    #[test]
    fn estimated_usage_is_not_authoritative() {
        let home = TempDir::new().unwrap();
        let mut estimated = usage();
        estimated["estimated"] = json!(true);
        write_ledger(home.path(), &[row(200, 200, Some(estimated))], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn stream_aborted_attempt_is_not_authoritative() {
        let home = TempDir::new().unwrap();
        let mut value = row(200, 200, Some(usage()));
        value["attempts"][0]["streamAborted"] = json!(true);
        write_ledger(home.path(), &[value], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
    }

    #[test]
    fn authoritative_service_tier_is_carried_to_repricing() {
        let home = TempDir::new().unwrap();
        let mut value = row(200, 200, Some(usage()));
        value["attempts"][0]["tierOutcome"] = json!({
            "responseServiceTier": "priority",
            "responseTierAuthoritative": true
        });
        write_ledger(home.path(), &[value], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            1
        );
        assert_eq!(messages[0].service_tier.as_deref(), Some("priority"));
    }

    #[test]
    fn unauthoritative_tier_is_not_carried() {
        let home = TempDir::new().unwrap();
        let mut value = row(200, 200, Some(usage()));
        value["attempts"][0]["tierOutcome"] = json!({
            "responseServiceTier": "priority",
            "responseTierAuthoritative": false
        });
        write_ledger(home.path(), &[value], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            1
        );
        assert_eq!(messages[0].service_tier, None);
    }

    #[test]
    fn failed_failover_attempt_does_not_hide_unique_success() {
        let home = TempDir::new().unwrap();
        let mut value = row(200, 200, Some(usage()));
        value["attempts"] = json!([
            {
                "ordinal": 1,
                "provider": "fixture-provider-a",
                "model": "failed-wire-model-fixture",
                "adapter": "fixture-responses",
                "status": 502,
                "durationMs": 100,
                "sendCount": 1,
                "recoveryKinds": [],
                "usageStatus": "reported",
                "usage": usage()
            },
            {
                "ordinal": 2,
                "provider": PROVIDER,
                "model": WIRE_MODEL,
                "adapter": "fixture-responses",
                "status": 200,
                "durationMs": ROW_DURATION,
                "sendCount": 1,
                "recoveryKinds": [],
                "usageStatus": "reported",
                "usage": usage()
            }
        ]);
        write_ledger(home.path(), &[value], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            1
        );
        assert_eq!(messages[0].model_id, SERVED_MODEL);
    }

    #[test]
    fn two_eligible_attempts_cannot_claim_one_served_model() {
        let home = TempDir::new().unwrap();
        let mut value = row(200, 200, Some(usage()));
        let mut second = value["attempts"][0].clone();
        second["ordinal"] = json!(2);
        second["model"] = json!("another-wire-model-fixture");
        value["attempts"].as_array_mut().unwrap().push(second);
        write_ledger(home.path(), &[value], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn token_mismatch_does_not_attribute_even_when_time_matches() {
        let home = TempDir::new().unwrap();
        let mut wrong = usage();
        wrong["outputTokens"] = json!(26);
        wrong["totalTokens"] = json!(526);
        write_ledger(home.path(), &[row(200, 200, Some(wrong))], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn conflicting_cache_counters_fail_closed() {
        let home = TempDir::new().unwrap();
        let mut conflicting = usage();
        conflicting["cacheReadInputTokens"] = json!(399);
        write_ledger(home.path(), &[row(200, 200, Some(conflicting))], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
    }

    #[test]
    fn no_duration_record_uses_its_response_time_as_end_anchor() {
        let home = TempDir::new().unwrap();
        write_ledger(home.path(), &[row(200, 200, Some(usage()))], false);
        // An unconfirmed token_usage_record carries the record time and no
        // duration: START + DURATION is the response's end. Matching on the
        // request start would put this ROW_START_OFFSET + ROW_DURATION
        // milliseconds away and fail.
        let mut message = codex_message();
        message.timestamp = START + DURATION;
        message.duration_ms = None;
        let mut messages = vec![message];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            1
        );
        assert_eq!(messages[0].model_id, SERVED_MODEL);
    }

    #[test]
    fn delayed_codex_usage_is_attributed_with_and_without_duration() {
        let home = TempDir::new().unwrap();
        write_ledger(home.path(), &[row(200, 200, Some(usage()))], false);

        for has_duration in [true, false] {
            let mut message = codex_message();
            if has_duration {
                message.duration_ms = Some(DURATION + 25_000);
            } else {
                message.timestamp = START + DURATION + 25_000;
                message.duration_ms = None;
            }
            let mut messages = vec![message];

            assert_eq!(
                attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
                1
            );
            assert_eq!(messages[0].model_id, SERVED_MODEL);
        }
    }

    #[test]
    fn delayed_codex_usage_keeps_strict_early_and_late_boundaries() {
        let home = TempDir::new().unwrap();
        write_ledger(home.path(), &[row(200, 200, Some(usage()))], false);

        for (skew_ms, expected_count) in [(-2_001, 0), (-2_000, 1), (30_000, 1), (30_001, 0)] {
            let mut message = codex_message();
            message.duration_ms = Some(DURATION + skew_ms);
            let mut messages = vec![message];

            assert_eq!(
                attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
                expected_count,
                "unexpected attribution at skew {skew_ms}ms"
            );
            assert_eq!(
                messages[0].model_id,
                if expected_count == 1 {
                    SERVED_MODEL
                } else {
                    SELECTOR
                }
            );
        }
    }

    #[test]
    fn delayed_codex_usage_still_rejects_multiple_candidate_rows() {
        let home = TempDir::new().unwrap();
        let mut competing = row_with_served(OTHER_SERVED_MODEL, 200, 200, Some(usage()));
        competing["timestamp"] = json!(START + ROW_START_OFFSET + 10_000);
        write_ledger(
            home.path(),
            &[row(200, 200, Some(usage())), competing],
            false,
        );

        let mut message = codex_message();
        message.duration_ms = Some(DURATION + 25_000);
        let mut messages = vec![message];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn distant_matching_usage_does_not_attribute() {
        let home = TempDir::new().unwrap();
        let mut distant = row(200, 200, Some(usage()));
        distant["timestamp"] = json!(START + 600_000);
        write_ledger(home.path(), &[distant], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn one_ledger_request_cannot_attribute_two_codex_messages() {
        let home = TempDir::new().unwrap();
        write_ledger(home.path(), &[row(200, 200, Some(usage()))], false);
        let mut messages = vec![codex_message(), codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert!(messages.iter().all(|message| message.model_id == SELECTOR));
        assert!(messages.iter().all(|message| message.cost == 7.0));
    }

    #[test]
    fn late_usage_does_not_claim_a_row_shared_by_two_codex_messages() {
        let home = TempDir::new().unwrap();
        write_ledger(home.path(), &[row(200, 200, Some(usage()))], false);
        let first = codex_message();
        let mut second = codex_message();
        second.timestamp += 8_000;
        let mut messages = vec![first, second];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert!(messages.iter().all(|message| message.model_id == SELECTOR));
    }

    #[test]
    fn multiple_candidate_requests_fail_closed_even_for_the_same_target() {
        let home = TempDir::new().unwrap();
        write_ledger(
            home.path(),
            &[row(200, 200, Some(usage())), row(200, 200, Some(usage()))],
            false,
        );
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn distant_requests_claim_their_own_rows_after_sorting() {
        let home = TempDir::new().unwrap();
        let mut later = row_with_served(OTHER_SERVED_MODEL, 200, 200, Some(usage()));
        later["timestamp"] = json!(START + 600_000);
        write_ledger(home.path(), &[row(200, 200, Some(usage())), later], false);
        let mut later_message = codex_message();
        later_message.timestamp = START + 600_000 - ROW_START_OFFSET;
        let mut messages = vec![later_message, codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            2
        );
        assert_eq!(messages[0].model_id, OTHER_SERVED_MODEL);
        assert_eq!(messages[1].model_id, SERVED_MODEL);
    }

    #[test]
    fn appended_rows_invalidate_the_cached_index_and_restore_ambiguity() {
        let home = TempDir::new().unwrap();
        let dir = home.path().join(".opencodex");
        fs::create_dir_all(&dir).unwrap();
        let ledger = dir.join("usage.jsonl");
        fs::write(
            &ledger,
            serde_json::to_vec(&row(200, 200, Some(usage()))).unwrap(),
        )
        .unwrap();
        let mut messages = vec![codex_message()];
        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            1
        );

        let mut body = fs::read(&ledger).unwrap();
        body.push(b'\n');
        body.extend(serde_json::to_vec(&row(200, 200, Some(usage()))).unwrap());
        fs::write(&ledger, body).unwrap();
        let mut messages = vec![codex_message()];
        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn missing_ledger_keeps_existing_codex_behavior() {
        let home = TempDir::new().unwrap();
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].cost, 7.0);
    }

    #[test]
    #[serial]
    fn opencodex_home_override_is_honored() {
        let default_home = TempDir::new().unwrap();
        let override_home = TempDir::new().unwrap();
        let mut env = crate::paths::test_env::EnvGuard::capture(&["OPENCODEX_HOME"]);
        env.set("OPENCODEX_HOME", override_home.path());

        let body = serde_json::to_string(&row(200, 200, Some(usage()))).unwrap() + "\n";
        fs::write(override_home.path().join("usage.jsonl"), body).unwrap();

        let mut messages = vec![codex_message()];
        assert_eq!(
            attribute_codex_messages(default_home.path().to_str().unwrap(), true, &mut messages),
            1
        );
        assert_eq!(messages[0].model_id, SERVED_MODEL);
    }

    #[test]
    fn combo_rows_without_protocol_are_ignored() {
        let home = TempDir::new().unwrap();
        let mut value = row(200, 200, Some(usage()));
        value.as_object_mut().unwrap().remove("inboundProtocol");
        write_ledger(home.path(), &[value], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn non_responses_combo_rows_are_ignored() {
        let home = TempDir::new().unwrap();
        let mut value = row(200, 200, Some(usage()));
        value["inboundProtocol"] = json!("messages");
        write_ledger(home.path(), &[value], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }
}
