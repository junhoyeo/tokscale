//! Droid (Factory.ai) session parser
//!
//! Parses JSON files from ~/.factory/sessions/

use super::utils::{file_modified_timestamp_ms_opt, read_file_or_none};
use super::UnifiedMessage;
use crate::{provider_identity, TokenBreakdown};
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Droid settings.json structure
#[derive(Debug, Deserialize)]
pub struct DroidSettingsJson {
    pub model: Option<String>,
    #[serde(rename = "providerLock")]
    pub provider_lock: Option<String>,
    #[serde(rename = "providerLockTimestamp")]
    pub provider_lock_timestamp: Option<String>,
    #[serde(rename = "tokenUsage")]
    pub token_usage: Option<DroidTokenUsage>,
}

#[derive(Debug, Deserialize)]
pub struct DroidTokenUsage {
    #[serde(rename = "inputTokens")]
    pub input_tokens: Option<i64>,
    #[serde(rename = "outputTokens")]
    pub output_tokens: Option<i64>,
    #[serde(rename = "cacheCreationTokens")]
    pub cache_creation_tokens: Option<i64>,
    #[serde(rename = "cacheReadTokens")]
    pub cache_read_tokens: Option<i64>,
    #[serde(rename = "thinkingTokens")]
    pub thinking_tokens: Option<i64>,
}

/// Normalize model name from Droid's custom format
/// e.g., "custom:Claude-Opus-4.5-Thinking-[Anthropic]-0" -> "claude-opus-4-5-thinking-0"
/// e.g., "gemini-2.5-pro" -> "gemini-2-5-pro"
/// e.g., "Claude-Sonnet-4-[Anthropic]" -> "claude-sonnet-4"
fn normalize_model_name(model: &str) -> String {
    // Remove "custom:" prefix if present
    let mut normalized = model.strip_prefix("custom:").unwrap_or(model).to_string();

    // Handle bracket notation like "Claude-Opus-4.5-Thinking-[Anthropic]-0"
    // Remove [anything] patterns (like TypeScript's .replace(/\[.*?\]/g, ""))
    let mut result = String::new();
    let mut in_bracket = false;

    for ch in normalized.chars() {
        match ch {
            '[' => in_bracket = true,
            ']' => in_bracket = false,
            _ if !in_bracket => result.push(ch),
            _ => {}
        }
    }

    normalized = result;

    // Remove trailing hyphens only (like TypeScript's .replace(/-+$/, ""))
    // NOTE: Do NOT remove trailing digits - TypeScript keeps them
    normalized = normalized.trim_end_matches('-').to_string();

    // Convert to lowercase (like TypeScript's .toLowerCase())
    normalized = normalized.to_lowercase();

    // Replace dots with hyphens (like TypeScript's .replace(/\./g, "-"))
    normalized = normalized.replace('.', "-");

    // Collapse multiple consecutive hyphens into one (like TypeScript's .replace(/-+/g, "-"))
    let mut collapsed = String::new();
    let mut last_was_hyphen = false;
    for ch in normalized.chars() {
        if ch == '-' {
            if !last_was_hyphen {
                collapsed.push(ch);
            }
            last_was_hyphen = true;
        } else {
            collapsed.push(ch);
            last_was_hyphen = false;
        }
    }

    collapsed
}

fn get_provider_from_model(model: &str) -> &'static str {
    provider_identity::inferred_provider_from_model(model).unwrap_or("unknown")
}

/// Get default model name based on provider when model field is missing
fn get_default_model_from_provider(provider: &str) -> String {
    match provider_identity::canonical_provider(provider)
        .as_deref()
        .unwrap_or(provider)
    {
        "anthropic" => "claude-unknown".to_string(),
        "openai" => "gpt-unknown".to_string(),
        "google" => "gemini-unknown".to_string(),
        "xai" => "grok-unknown".to_string(),
        _ => format!("{}-unknown", provider),
    }
}

/// Try to extract model name from JSONL file's system-reminder
/// Looks for pattern: "Model: Claude Opus 4.5 Thinking [Anthropic]"
fn extract_model_from_jsonl(jsonl_path: &Path) -> Option<String> {
    let file = std::fs::File::open(jsonl_path).ok()?;
    let reader = BufReader::new(file);

    // Scan more lines for parity with TypeScript which reads entire file
    // Cap at 500 lines to avoid performance issues with very large files
    for line in reader.lines().take(500) {
        let line = line.ok()?;
        // Look for Model: pattern in system-reminder
        if let Some(pos) = line.find("Model:") {
            let after_model = &line[pos + 6..];
            // Extract until [ or end of string/newline
            let model_part: String = after_model
                .chars()
                .take_while(|&c| c != '[' && c != '\\' && c != '"')
                .collect();
            let model_name = model_part.trim();
            if !model_name.is_empty() {
                return Some(normalize_model_name(model_name));
            }
        }
    }

    None
}

/// Return the fallback JSONL consulted when a settings snapshot omits its
/// model. The cache watches this path even when it is currently absent so a
/// later-created transcript invalidates the stored fallback model.
pub(crate) fn droid_jsonl_path(path: &Path) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let stem = file_name.strip_suffix(".settings.json")?;
    Some(path.with_file_name(format!("{stem}.jsonl")))
}

/// Pick the instant a Droid session's cumulative `tokenUsage` is attributed to.
///
/// A Droid `*.settings.json` holds one running total for the whole session and
/// is rewritten every time the agent spends tokens, so the parser emits a
/// single record and has to anchor it somewhere.
///
/// `providerLockTimestamp` is the wrong anchor: it records when the provider
/// was *selected*, not when the tokens were spent. Droid rewrites the totals
/// in place without touching that field, so a session left running across days
/// (a `/loop`, a long autonomous run) keeps reporting its very first instant
/// while the totals climb. Every token it has ever spent lands in the bucket
/// for the day it started, and the session reads as silent in `--today` and
/// `--yesterday` even while it is actively burning tokens.
///
/// The file's mtime is when the totals being read were written, which is the
/// closest available marker for when they were last accrued, so it wins. The
/// lock timestamp becomes a floor rather than the answer: usage cannot predate
/// provider selection, so a stale mtime (a restore or copy that rewound it)
/// cannot drag the record earlier than the session could possibly have run.
///
/// When the filesystem reports no mtime at all, fall back to the lock
/// timestamp, then to now() — a record with real token usage is never dropped
/// just because its timestamp could not be resolved.
/// Parse `providerLockTimestamp` into epoch milliseconds, rejecting values that
/// cannot describe a real provider lock.
///
/// Zero is Droid's unset sentinel, and a negative value is a clock or
/// corruption artifact — neither is a usable anchor. Both collapse to `None` so
/// the resolver treats them as absent, which keeps this anchor's validity rule
/// symmetric with the mtime one: `file_modified_timestamp_ms_opt` already
/// reports `None` for a pre-epoch mtime. Without that symmetry a negative lock
/// would survive as the resolved anchor whenever mtime was unavailable, and the
/// record would land in a 1969 bucket that no date filter can reach.
fn parse_lock_timestamp(raw: Option<&str>) -> Option<i64> {
    raw.and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.timestamp_millis())
        .filter(|&ts| ts > 0)
}

fn resolve_usage_timestamp(lock_timestamp: Option<i64>, modified: Option<i64>) -> i64 {
    match (modified, lock_timestamp) {
        (Some(modified), Some(lock)) => modified.max(lock),
        (Some(modified), None) => modified,
        (None, Some(lock)) => lock,
        (None, None) => chrono::Utc::now().timestamp_millis(),
    }
}

/// Parse a Droid settings.json file
pub fn parse_droid_file(path: &Path) -> Vec<UnifiedMessage> {
    let Some(data) = read_file_or_none(path) else {
        return Vec::new();
    };

    let mut bytes = data;
    let settings: DroidSettingsJson = match simd_json::from_slice(&mut bytes) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    // Skip if no token usage data
    let usage = match settings.token_usage {
        Some(u) => u,
        None => return Vec::new(),
    };

    // Calculate total tokens to check if any were used
    let total_tokens = usage.input_tokens.unwrap_or(0)
        + usage.output_tokens.unwrap_or(0)
        + usage.cache_creation_tokens.unwrap_or(0)
        + usage.cache_read_tokens.unwrap_or(0)
        + usage.thinking_tokens.unwrap_or(0);

    if total_tokens == 0 {
        return Vec::new();
    }

    // Extract session ID from filename (e.g., "uuid.settings.json" -> "uuid")
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
        .replace(".settings", "");

    // Get model and provider
    let provider = settings.provider_lock.clone().unwrap_or_else(|| {
        get_provider_from_model(settings.model.as_deref().unwrap_or("")).to_string()
    });

    let model = if let Some(m) = settings.model {
        normalize_model_name(&m)
    } else {
        // Try to extract from JSONL file
        let jsonl_path = droid_jsonl_path(path);

        if let Some(ref jsonl) = jsonl_path {
            extract_model_from_jsonl(jsonl)
                .unwrap_or_else(|| get_default_model_from_provider(&provider))
        } else {
            get_default_model_from_provider(&provider)
        }
    };

    let lock_timestamp = parse_lock_timestamp(settings.provider_lock_timestamp.as_deref());

    let timestamp = resolve_usage_timestamp(lock_timestamp, file_modified_timestamp_ms_opt(path));

    vec![UnifiedMessage::new(
        "droid",
        model,
        provider,
        session_id,
        timestamp,
        TokenBreakdown {
            input: usage.input_tokens.unwrap_or(0).max(0),
            output: usage.output_tokens.unwrap_or(0).max(0),
            cache_read: usage.cache_read_tokens.unwrap_or(0).max(0),
            cache_write: usage.cache_creation_tokens.unwrap_or(0).max(0),
            reasoning: usage.thinking_tokens.unwrap_or(0).max(0),
        },
        0.0,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_model_name_custom_prefix() {
        // TypeScript keeps trailing digits: "claude-opus-4-5-thinking-0"
        assert_eq!(
            normalize_model_name("custom:Claude-Opus-4.5-Thinking-[Anthropic]-0"),
            "claude-opus-4-5-thinking-0"
        );
    }

    #[test]
    fn test_normalize_model_name_simple() {
        // Dots become hyphens: "gemini-2.5-pro" -> "gemini-2-5-pro"
        assert_eq!(normalize_model_name("gemini-2.5-pro"), "gemini-2-5-pro");
    }

    #[test]
    fn test_normalize_model_name_brackets() {
        // TypeScript keeps trailing digits: "claude-sonnet-4"
        assert_eq!(
            normalize_model_name("Claude-Sonnet-4-[Anthropic]"),
            "claude-sonnet-4"
        );
    }

    #[test]
    fn test_get_provider_from_model() {
        assert_eq!(get_provider_from_model("claude-3-sonnet"), "anthropic");
        assert_eq!(get_provider_from_model("opus-4"), "anthropic");
        assert_eq!(get_provider_from_model("sonnet-4"), "anthropic");
        assert_eq!(get_provider_from_model("haiku-3"), "anthropic");
        assert_eq!(get_provider_from_model("gpt-4o"), "openai");
        assert_eq!(get_provider_from_model("o1-preview"), "openai");
        assert_eq!(get_provider_from_model("o3-mini"), "openai");
        assert_eq!(get_provider_from_model("gemini-pro"), "google");
        assert_eq!(get_provider_from_model("grok-2"), "xai");
        assert_eq!(get_provider_from_model("unknown-model"), "unknown");
    }

    #[test]
    fn test_get_default_model_from_provider() {
        assert_eq!(
            get_default_model_from_provider("anthropic"),
            "claude-unknown"
        );
        assert_eq!(get_default_model_from_provider("openai"), "gpt-unknown");
        assert_eq!(get_default_model_from_provider("google"), "gemini-unknown");
        assert_eq!(get_default_model_from_provider("xai"), "grok-unknown");
        assert_eq!(get_default_model_from_provider("custom"), "custom-unknown");
    }

    #[test]
    fn test_parse_lock_timestamp_rejects_unusable_anchors() {
        // Zero is Droid's unset sentinel.
        assert_eq!(parse_lock_timestamp(Some("1970-01-01T00:00:00Z")), None);
        // A pre-epoch lock is a clock/corruption artifact. Keeping it would
        // outlive the mtime fallback and bucket the record in 1969, where no
        // date filter can reach it.
        assert_eq!(parse_lock_timestamp(Some("1969-07-20T20:17:00Z")), None);
        assert_eq!(parse_lock_timestamp(Some("not-a-timestamp")), None);
        assert_eq!(parse_lock_timestamp(None), None);

        assert_eq!(
            parse_lock_timestamp(Some("2026-08-07T03:32:46.663Z")),
            Some(1_786_073_566_663)
        );
    }

    #[test]
    fn test_pre_epoch_lock_falls_through_to_now_without_mtime() {
        // The resolver only reaches its now() fallback if the rejected lock
        // arrives as None, so this pins the two halves together: an unusable
        // lock plus an unavailable mtime must not yield a pre-epoch anchor.
        let lock = parse_lock_timestamp(Some("1969-07-20T20:17:00Z"));

        assert!(resolve_usage_timestamp(lock, None) > 1_700_000_000_000);
    }

    #[test]
    fn test_resolve_usage_timestamp_prefers_mtime_over_stale_lock() {
        // The regression: a session locked its provider on day 1 and was still
        // spending tokens on day 4. Anchoring on the lock timestamp reported
        // all of it against day 1 and left the session invisible in --today.
        let lock = 1_700_000_000_000;
        let modified = lock + 3 * 86_400_000;

        assert_eq!(
            resolve_usage_timestamp(Some(lock), Some(modified)),
            modified
        );
    }

    #[test]
    fn test_resolve_usage_timestamp_floors_stale_mtime_at_lock() {
        // A copy or restore can rewind mtime below the instant the provider was
        // locked. Usage cannot predate provider selection, so the lock wins.
        let lock = 1_700_000_000_000;
        let modified = lock - 86_400_000;

        assert_eq!(resolve_usage_timestamp(Some(lock), Some(modified)), lock);
    }

    #[test]
    fn test_resolve_usage_timestamp_falls_back_across_missing_inputs() {
        let lock = 1_700_000_000_000;
        let modified = 1_700_000_500_000;

        // Droid omits providerLockTimestamp on plenty of sessions.
        assert_eq!(resolve_usage_timestamp(None, Some(modified)), modified);
        // Filesystem reported no mtime: the lock is still better than now().
        assert_eq!(resolve_usage_timestamp(Some(lock), None), lock);
    }

    #[test]
    fn test_resolve_usage_timestamp_without_any_anchor_is_not_pre_epoch() {
        // Neither anchor available: the record still carries real token usage,
        // so it must land in a present-day bucket rather than at the epoch.
        assert!(resolve_usage_timestamp(None, None) > 1_700_000_000_000);
    }

    #[test]
    fn test_parse_droid_file_anchors_long_running_session_at_last_write() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = temp_dir
            .path()
            .join("11111111-2222-3333-4444-555555555555.settings.json");

        // providerLockTimestamp far in the past, totals written just now —
        // the shape of a session that has been looping for days.
        let lock_ms = chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        std::fs::write(
            &path,
            r#"{
                "model": "custom:Kimi-K3-(free)-0",
                "providerLockTimestamp": "2024-01-01T00:00:00Z",
                "tokenUsage": { "inputTokens": 1000, "outputTokens": 200 }
            }"#,
        )
        .unwrap();

        let messages = parse_droid_file(&path);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].session_id,
            "11111111-2222-3333-4444-555555555555"
        );
        assert!(
            messages[0].timestamp > lock_ms,
            "cumulative usage anchored on the stale lock timestamp ({lock_ms}), \
             got {}",
            messages[0].timestamp
        );
    }

    #[test]
    fn test_parse_droid_settings_structure() {
        let json = r#"{
            "model": "custom:Claude-Opus-4.5-Thinking-[Anthropic]-0",
            "providerLock": "anthropic",
            "providerLockTimestamp": "2024-12-26T12:00:00Z",
            "tokenUsage": {
                "inputTokens": 1234,
                "outputTokens": 567,
                "cacheCreationTokens": 89,
                "cacheReadTokens": 12,
                "thinkingTokens": 34
            }
        }"#;

        let mut bytes = json.as_bytes().to_vec();
        let settings: DroidSettingsJson = simd_json::from_slice(&mut bytes).unwrap();

        assert_eq!(
            settings.model,
            Some("custom:Claude-Opus-4.5-Thinking-[Anthropic]-0".to_string())
        );
        assert_eq!(settings.provider_lock, Some("anthropic".to_string()));

        let usage = settings.token_usage.unwrap();
        assert_eq!(usage.input_tokens, Some(1234));
        assert_eq!(usage.output_tokens, Some(567));
        assert_eq!(usage.cache_creation_tokens, Some(89));
        assert_eq!(usage.cache_read_tokens, Some(12));
        assert_eq!(usage.thinking_tokens, Some(34));
    }
}
