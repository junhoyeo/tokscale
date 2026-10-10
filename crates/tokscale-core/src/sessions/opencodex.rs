use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use once_cell::sync::Lazy;

use super::{CostSource, UnifiedMessage};

const MAX_END_SKEW_MS: u64 = 2_000;
const MAX_START_SKEW_MS: u64 = 120_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PhysicalTarget {
    provider: String,
    model: String,
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
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedAttempt {
    provider: Option<String>,
    model: Option<String>,
    status: Option<i32>,
    usage_status: Option<String>,
    usage: Option<PersistedUsage>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedUsageRow {
    timestamp: Option<i64>,
    provider: Option<String>,
    model: Option<String>,
    requested_model: Option<String>,
    inbound_protocol: Option<String>,
    status: Option<i32>,
    duration_ms: Option<i64>,
    attempts: Option<Vec<PersistedAttempt>>,
}

#[derive(Debug, Clone)]
struct IndexedRow {
    timestamp: i64,
    duration_ms: i64,
    attempts: Vec<PersistedAttempt>,
}

#[derive(Debug, Default)]
struct AttributionIndex {
    by_selector: HashMap<String, Vec<IndexedRow>>,
}

impl AttributionIndex {
    fn load(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let reader = BufReader::new(file);
        let mut index = Self::default();

        for line in reader.lines() {
            let Ok(line) = line else {
                continue;
            };
            if line.trim().is_empty() {
                continue;
            }

            let Ok(row) = serde_json::from_str::<PersistedUsageRow>(&line) else {
                continue;
            };
            if row.provider.as_deref() != Some("combo") {
                continue;
            }
            if row.inbound_protocol.as_deref() != Some("responses") {
                continue;
            }
            if !row.status.is_some_and(|status| (200..300).contains(&status)) {
                continue;
            }

            let Some(selector) = row.requested_model.as_deref().or(row.model.as_deref()) else {
                continue;
            };
            let Some(timestamp) = row.timestamp.filter(|timestamp| *timestamp > 0) else {
                continue;
            };
            let attempts = row.attempts.unwrap_or_default();
            if attempts.is_empty() {
                continue;
            }

            index
                .by_selector
                .entry(selector.to_string())
                .or_default()
                .push(IndexedRow {
                    timestamp,
                    duration_ms: row.duration_ms.unwrap_or(0).max(0),
                    attempts,
                });
        }

        Some(index)
    }

    fn resolve(&self, message: &UnifiedMessage) -> Option<PhysicalTarget> {
        let rows = self.by_selector.get(&message.model_id)?;
        let mut targets = HashSet::new();

        for row in rows {
            if !timing_matches(message, row) {
                continue;
            }

            for attempt in &row.attempts {
                if !attempt
                    .status
                    .is_some_and(|status| (200..300).contains(&status))
                {
                    continue;
                }
                if attempt
                    .usage_status
                    .as_deref()
                    .is_some_and(|status| status != "reported")
                {
                    continue;
                }
                let Some(usage) = attempt.usage.as_ref() else {
                    continue;
                };
                if !usage_matches(message, usage) {
                    continue;
                }
                let (Some(provider), Some(model)) =
                    (attempt.provider.as_ref(), attempt.model.as_ref())
                else {
                    continue;
                };
                if provider.is_empty() || model.is_empty() {
                    continue;
                }

                targets.insert(PhysicalTarget {
                    provider: provider.clone(),
                    model: model.clone(),
                });
            }
        }

        if targets.len() == 1 {
            targets.into_iter().next()
        } else {
            None
        }
    }
}

fn timing_matches(message: &UnifiedMessage, row: &IndexedRow) -> bool {
    if message.timestamp <= 0 {
        return false;
    }

    let start_skew = message.timestamp.abs_diff(row.timestamp);
    if let Some(duration_ms) = message.duration_ms.filter(|duration| *duration >= 0) {
        if start_skew > MAX_START_SKEW_MS {
            return false;
        }
        let message_end = message.timestamp.saturating_add(duration_ms);
        let row_end = row.timestamp.saturating_add(row.duration_ms);
        message_end.abs_diff(row_end) <= MAX_END_SKEW_MS
    } else {
        start_skew <= MAX_END_SKEW_MS
    }
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

    let cached = usage.cached_input_tokens.max(usage.cache_read_input_tokens);
    if cached.is_some_and(|cached| cached != message.tokens.cache_read) {
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
/// must line up. If more than one physical target survives those checks, the
/// original selector is retained.
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

    let mut attributed = 0;
    for message in messages
        .iter_mut()
        .filter(|message| message.client == "codex")
    {
        let Some(target) = index.resolve(message) else {
            continue;
        };

        if message.provider_id == target.provider && message.model_id == target.model {
            continue;
        }

        message.provider_id = target.provider;
        message.model_id = target.model;
        // Any estimate computed for the virtual selector is invalid after
        // attribution. The caller reprices this physical identity.
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

    const SELECTOR: &str = "sol-luna-jev-combo";
    const START: i64 = 1_791_132_925_632;
    const DURATION: i64 = 21_597;

    fn codex_message() -> UnifiedMessage {
        let mut message = UnifiedMessage::new(
            "codex",
            SELECTOR,
            "openai",
            "session",
            START,
            TokenBreakdown {
                input: 1_147,
                output: 181,
                cache_read: 95_488,
                cache_write: 0,
                cache_write_1h: 0,
                reasoning: 134,
            },
            7.0,
        );
        message.duration_ms = Some(DURATION);
        message.cost_source = CostSource::Estimated;
        message
    }

    fn usage() -> Value {
        json!({
            "inputTokens": 96635,
            "outputTokens": 315,
            "totalTokens": 96950,
            "cachedInputTokens": 95488,
            "reasoningOutputTokens": 134
        })
    }

    fn row(target: &str, status: i32, attempt_status: i32, usage_value: Option<Value>) -> Value {
        json!({
            "requestId": "ocx-test",
            "timestamp": START + 12_952,
            "provider": "combo",
            "model": SELECTOR,
            "requestedModel": SELECTOR,
            "inboundProtocol": "responses",
            "status": status,
            "durationMs": 8_595,
            "usageStatus": if usage_value.is_some() { "reported" } else { "unreported" },
            "attempts": [{
                "ordinal": 1,
                "provider": "openai",
                "model": target,
                "adapter": "openai-responses",
                "status": attempt_status,
                "durationMs": 8_595,
                "sendCount": 1,
                "recoveryKinds": [],
                "usageStatus": if usage_value.is_some() { "reported" } else { "unreported" },
                "usage": usage_value
            }]
        })
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
    fn attributes_matching_combo_attempt_and_clears_virtual_cost() {
        let home = TempDir::new().unwrap();
        write_ledger(home.path(), &[row("gpt-6-luna", 200, 200, Some(usage()))], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            1
        );
        assert_eq!(messages[0].provider_id, "openai");
        assert_eq!(messages[0].model_id, "gpt-6-luna");
        assert_eq!(messages[0].cost, 0.0);
        assert_eq!(messages[0].cost_source, CostSource::Unknown);
    }

    #[test]
    fn malformed_lines_are_skipped_without_hiding_valid_rows() {
        let home = TempDir::new().unwrap();
        write_ledger(
            home.path(),
            &[row("gpt-6.1-sol", 200, 200, Some(usage()))],
            true,
        );
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            1
        );
        assert_eq!(messages[0].model_id, "gpt-6.1-sol");
    }

    #[test]
    fn cancelled_or_unreported_attempt_is_not_used() {
        let home = TempDir::new().unwrap();
        write_ledger(home.path(), &[row("gpt-6-luna", 499, 499, None)], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn ambiguous_physical_targets_fail_closed() {
        let home = TempDir::new().unwrap();
        write_ledger(
            home.path(),
            &[
                row("gpt-6-luna", 200, 200, Some(usage())),
                row("gpt-6.1-sol", 200, 200, Some(usage())),
            ],
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
    fn failed_failover_attempt_does_not_override_successful_attempt() {
        let home = TempDir::new().unwrap();
        let mut value = row("gpt-6-luna", 200, 200, Some(usage()));
        value["attempts"] = json!([
            {
                "ordinal": 1,
                "provider": "openai",
                "model": "gpt-6.1-sol",
                "adapter": "openai-responses",
                "status": 502,
                "durationMs": 100,
                "sendCount": 1,
                "recoveryKinds": [],
                "usageStatus": "reported",
                "usage": usage()
            },
            {
                "ordinal": 2,
                "provider": "openai",
                "model": "gpt-6-luna",
                "adapter": "openai-responses",
                "status": 200,
                "durationMs": 8_400,
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
        assert_eq!(messages[0].model_id, "gpt-6-luna");
    }

    #[test]
    fn token_mismatch_does_not_attribute_even_when_time_matches() {
        let home = TempDir::new().unwrap();
        let mut wrong = usage();
        wrong["outputTokens"] = json!(316);
        wrong["totalTokens"] = json!(96951);
        write_ledger(home.path(), &[row("gpt-6-luna", 200, 200, Some(wrong))], false);
        let mut messages = vec![codex_message()];

        assert_eq!(
            attribute_codex_messages(home.path().to_str().unwrap(), false, &mut messages),
            0
        );
        assert_eq!(messages[0].model_id, SELECTOR);
    }

    #[test]
    fn distant_matching_usage_does_not_attribute() {
        let home = TempDir::new().unwrap();
        let mut distant = row("gpt-6-luna", 200, 200, Some(usage()));
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

        let body = serde_json::to_string(&row("gpt-6-luna", 200, 200, Some(usage()))).unwrap()
            + "\n";
        fs::write(override_home.path().join("usage.jsonl"), body).unwrap();

        let mut messages = vec![codex_message()];
        assert_eq!(
            attribute_codex_messages(default_home.path().to_str().unwrap(), true, &mut messages),
            1
        );
        assert_eq!(messages[0].model_id, "gpt-6-luna");
    }

    #[test]
    fn combo_rows_without_protocol_are_ignored() {
        let home = TempDir::new().unwrap();
        let mut value = row("gpt-6-luna", 200, 200, Some(usage()));
        value
            .as_object_mut()
            .unwrap()
            .remove("inboundProtocol");
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
        let mut value = row("gpt-6-luna", 200, 200, Some(usage()));
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
