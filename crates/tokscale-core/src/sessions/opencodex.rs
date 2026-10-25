use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use once_cell::sync::Lazy;

use super::{CostSource, UnifiedMessage};

const MAX_END_SKEW_MS: u64 = 2_000;
// Codex can persist its token_count after OpenCodex records the response end.
// Only allow a wider skew when Codex is late; early events keep the 2s bound.
const MAX_LATE_END_SKEW_MS: i64 = 30_000;
const MAX_START_SKEW_MS: u64 = 120_000;
/// Bounds how far a response's recorded end can sit after its start while a
/// start-less Codex record is still correlated through its end anchor. Longer
/// responses keep the virtual selector (fail closed) instead of widening the
/// scan without limit.
const MAX_RESPONSE_DURATION_MS: i64 = 600_000;
/// One hand-edited or truncated ledger line must not allocate without bound.
const MAX_LEDGER_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PhysicalTarget {
    provider: String,
    model: String,
    service_tier: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cached_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
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

#[derive(Debug, Clone)]
struct IndexedRow {
    timestamp: i64,
    duration_ms: i64,
    /// Resolved once at load: the observed upstream identity behind the row's
    /// single successful reported attempt. `None` still claims the row so an
    /// unclear request blocks attribution instead of yielding to another.
    target: Option<PhysicalTarget>,
    attempts: Vec<PersistedAttempt>,
}

/// One possible ledger row; an unknown target still claims that row.
struct CandidateMatch {
    row_index: usize,
    target: Option<PhysicalTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundedLine {
    Eof,
    Line,
    Oversized,
}

/// Read and consume one JSONL row without allocating beyond the row cap. An
/// oversized row is discarded through its newline and parsing continues.
fn read_bounded_line<R: BufRead>(reader: &mut R, line: &mut Vec<u8>) -> io::Result<BoundedLine> {
    line.clear();
    let mut consumed = 0usize;
    let mut oversized = false;

    loop {
        let (take, has_newline) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(if consumed == 0 {
                    BoundedLine::Eof
                } else if oversized {
                    BoundedLine::Oversized
                } else {
                    BoundedLine::Line
                });
            }
            let take = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            if !oversized {
                if line.len().saturating_add(take) > MAX_LEDGER_LINE_BYTES {
                    oversized = true;
                    line.clear();
                } else {
                    line.extend_from_slice(&available[..take]);
                }
            }
            (take, available[take - 1] == b'\n')
        };
        reader.consume(take);
        consumed = consumed.saturating_add(take);
        if has_newline {
            if !oversized {
                if line.last() == Some(&b'\n') {
                    line.pop();
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                }
                return Ok(BoundedLine::Line);
            }
            return Ok(BoundedLine::Oversized);
        }
    }
}

#[derive(Debug, Default)]
struct AttributionIndex {
    by_selector: HashMap<String, Vec<IndexedRow>>,
}

impl AttributionIndex {
    fn load(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        let mut index = Self::default();

        loop {
            match read_bounded_line(&mut reader, &mut line).ok()? {
                BoundedLine::Eof => break,
                BoundedLine::Oversized => continue,
                BoundedLine::Line => {}
            }
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }

            let Ok(row) = serde_json::from_slice::<PersistedUsageRow>(&line) else {
                continue;
            };
            if row.provider.as_deref() != Some("combo")
                || row.inbound_protocol.as_deref() != Some("responses")
                || !row
                    .status
                    .is_some_and(|status| (200..300).contains(&status))
            {
                continue;
            }

            // The selector is required evidence: a row that cannot name what
            // Codex asked for cannot correlate anything and is not a claim.
            let Some(selector) = row
                .requested_model
                .as_deref()
                .filter(|selector| !selector.is_empty())
            else {
                continue;
            };
            let Some(timestamp) = row.timestamp.filter(|timestamp| *timestamp > 0) else {
                continue;
            };
            let Some(duration_ms) = row.duration_ms.filter(|duration| *duration >= 0) else {
                continue;
            };

            let target = resolve_row_target(&row);
            index
                .by_selector
                .entry(selector.to_string())
                .or_default()
                .push(IndexedRow {
                    timestamp,
                    duration_ms,
                    target,
                    attempts: row.attempts.unwrap_or_default(),
                });
        }

        for rows in index.by_selector.values_mut() {
            rows.sort_unstable_by_key(|row| row.timestamp);
        }
        Some(index)
    }

    /// Inspect only the selector's bounded time window. Candidates with
    /// unknown or conflicting physical targets still reserve their ledger row.
    fn candidates(&self, message: &UnifiedMessage) -> Vec<CandidateMatch> {
        if message.timestamp <= 0 {
            return Vec::new();
        }
        let Some(rows) = self.by_selector.get(&message.model_id) else {
            return Vec::new();
        };
        // The ledger row is stamped at its request start, which sits within
        // the start skew of a timed message and anywhere inside the response
        // span of a start-less record. End anchors bound both windows.
        let span = message
            .duration_ms
            .filter(|duration| *duration >= 0)
            .unwrap_or(MAX_RESPONSE_DURATION_MS);
        let back = if message.duration_ms.is_some_and(|duration| duration >= 0) {
            MAX_START_SKEW_MS as i64
        } else {
            MAX_LATE_END_SKEW_MS + MAX_RESPONSE_DURATION_MS
        };
        let first =
            rows.partition_point(|row| row.timestamp < message.timestamp.saturating_sub(back));
        let end = rows.partition_point(|row| {
            row.timestamp
                <= message
                    .timestamp
                    .saturating_add(span)
                    .saturating_add(MAX_END_SKEW_MS as i64)
        });
        let mut candidates = Vec::new();

        for (offset, row) in rows[first..end].iter().enumerate() {
            if !timing_matches(message, row) {
                continue;
            }
            let matched = row.attempts.iter().any(|attempt| {
                attempt_eligible(attempt)
                    && attempt
                        .usage
                        .as_ref()
                        .is_some_and(|usage| usage_matches(message, usage))
            });
            if matched {
                candidates.push(CandidateMatch {
                    row_index: first + offset,
                    target: row.target.clone(),
                });
            }
        }
        candidates
    }
}

/// One reported success per row, fully validated against the upstream attempt
/// contract. The observed `servedModel` — not the wire model a route intended
/// to send — is the only accepted physical identity, and only when exactly one
/// successful reported attempt can claim it. Anything else leaves the row
/// reserved with no target.
fn resolve_row_target(row: &PersistedUsageRow) -> Option<PhysicalTarget> {
    let served_model = row
        .served_model
        .as_deref()
        .filter(|model| !model.is_empty())?;
    let eligible: Vec<&PersistedAttempt> = row
        .attempts
        .as_ref()?
        .iter()
        .filter(|attempt| attempt_eligible(attempt))
        .collect();
    if eligible.len() != 1 {
        return None;
    }
    let attempt = eligible[0];
    let provider = attempt.provider.as_deref()?;
    let service_tier = attempt
        .tier_outcome
        .as_ref()
        .filter(|tier| tier.response_tier_authoritative == Some(true))
        .and_then(|tier| tier.response_service_tier.as_deref())
        .filter(|tier| !tier.is_empty())
        .map(str::to_string);
    Some(PhysicalTarget {
        provider: provider.to_string(),
        model: served_model.to_string(),
        service_tier,
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

fn timing_matches(message: &UnifiedMessage, row: &IndexedRow) -> bool {
    if message.timestamp <= 0 {
        return false;
    }

    let row_end = row.timestamp.saturating_add(row.duration_ms);
    let message_end =
        if let Some(duration_ms) = message.duration_ms.filter(|duration| *duration >= 0) {
            if message.timestamp.abs_diff(row.timestamp) > MAX_START_SKEW_MS {
                return false;
            }
            message.timestamp.saturating_add(duration_ms)
        } else {
            // An unconfirmed `token_usage_record` is timestamped when the response
            // lands, so its time anchors on the request's end, not its start.
            message.timestamp
        };
    (-(MAX_END_SKEW_MS as i64)..=MAX_LATE_END_SKEW_MS)
        .contains(&message_end.saturating_sub(row_end))
}

fn usage_matches(message: &UnifiedMessage, usage: &PersistedUsage) -> bool {
    let raw_input = message
        .tokens
        .input
        .saturating_add(message.tokens.cache_read);
    let raw_output = message
        .tokens
        .output
        .saturating_add(message.tokens.reasoning);

    if usage.input_tokens != Some(raw_input) || usage.output_tokens != Some(raw_output) {
        return false;
    }

    if let Some(total_tokens) = usage.total_tokens {
        if total_tokens != raw_input.saturating_add(raw_output) {
            return false;
        }
    }

    // Both spellings of the cache counter are observations; disagreeing
    // counters mean malformed evidence, not a choice between them.
    if usage
        .cached_input_tokens
        .is_some_and(|cached| cached != message.tokens.cache_read)
        || usage
            .cache_read_input_tokens
            .is_some_and(|cached| cached != message.tokens.cache_read)
    {
        return false;
    }

    if usage
        .reasoning_output_tokens
        .is_some_and(|reasoning| reasoning != message.tokens.reasoning)
    {
        return false;
    }

    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LedgerFingerprint {
    len: u64,
    modified: Option<SystemTime>,
}

#[derive(Debug, Clone)]
struct CachedAttributionIndex {
    fingerprint: LedgerFingerprint,
    index: Arc<AttributionIndex>,
}

static ATTRIBUTION_CACHE: Lazy<Mutex<HashMap<PathBuf, CachedAttributionIndex>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn load_attribution_index(path: &Path) -> Option<Arc<AttributionIndex>> {
    let metadata = fs::metadata(path).ok()?;
    let fingerprint = LedgerFingerprint {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    };

    {
        let cache = ATTRIBUTION_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cached) = cache.get(path) {
            if cached.fingerprint == fingerprint {
                return Some(Arc::clone(&cached.index));
            }
        }
    }

    let index = Arc::new(AttributionIndex::load(path)?);
    let mut cache = ATTRIBUTION_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        path.to_path_buf(),
        CachedAttributionIndex {
            fingerprint,
            index: Arc::clone(&index),
        },
    );
    Some(index)
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

/// Replace Codex's virtual OpenCodex combo selector with the physical target
/// that actually served the request.
///
/// Codex rollouts only know the caller-facing selector. OpenCodex's durable
/// usage ledger records the physical attempts. We correlate the two without
/// adding any new usage rows, so total tokens remain unchanged and the overlay
/// cannot double count Codex traffic.
///
/// Matching deliberately fails closed. A row must be a successful Responses
/// combo request, the requested selector must match exactly, a successful
/// reported attempt must match Codex's token signature, and request timing
/// must line up. Each ledger row may serve at most one Codex usage row.
/// Ambiguity retains the original selector.
pub(crate) fn attribute_codex_messages(
    home_dir: &str,
    use_env_roots: bool,
    messages: &mut [UnifiedMessage],
) -> usize {
    if !messages.iter().any(|message| message.client == "codex") {
        return 0;
    }

    let path = usage_log_path(home_dir, use_env_roots);
    let Some(index) = load_attribution_index(&path) else {
        return 0;
    };

    // Collect every feasible ledger claim before changing any model IDs.
    // Even a message with multiple candidates prevents another message from
    // claiming its shared row: uncertain attribution must fail closed.
    let candidates: Vec<Vec<CandidateMatch>> = messages
        .iter()
        .map(|message| {
            if message.client == "codex" {
                index.candidates(message)
            } else {
                Vec::new()
            }
        })
        .collect();
    let mut claims: HashMap<(String, usize), usize> = HashMap::new();
    for (message, matches) in messages.iter().zip(&candidates) {
        for candidate in matches {
            *claims
                .entry((message.model_id.clone(), candidate.row_index))
                .or_default() += 1;
        }
    }

    let mut attributed = 0;
    for (message, matches) in messages.iter_mut().zip(candidates) {
        if message.client != "codex" || matches.len() != 1 {
            continue;
        }
        let candidate = matches.into_iter().next().unwrap();
        if claims.get(&(message.model_id.clone(), candidate.row_index)) != Some(&1) {
            continue;
        }
        let Some(target) = candidate.target else {
            continue;
        };
        let tier_changed = target
            .service_tier
            .as_ref()
            .is_some_and(|tier| message.service_tier.as_ref() != Some(tier));
        if message.provider_id == target.provider
            && message.model_id == target.model
            && !tier_changed
        {
            continue;
        }

        message.provider_id = target.provider;
        message.model_id = target.model;
        if let Some(service_tier) = target.service_tier {
            message.service_tier = Some(service_tier);
        }
        // Estimates for the virtual selector cannot price the physical model.
        message.cost = 0.0;
        message.cost_source = CostSource::Unknown;
        attributed += 1;
    }

    attributed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TokenBreakdown;
    use serde_json::{json, Value};
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

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
                cache_write_1h: 0,
                reasoning: 5,
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
        body.extend(std::iter::repeat_n(b'x', MAX_LEDGER_LINE_BYTES + 1));
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
