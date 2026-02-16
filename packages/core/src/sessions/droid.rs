//! Droid (Factory.ai) session parser
//!
//! Parses JSON files from ~/.factory/sessions/

use super::UnifiedMessage;
use crate::TokenBreakdown;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::Path;

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
    let lower = model.to_lowercase();

    if lower.contains("claude")
        || lower.contains("anthropic")
        || lower.contains("opus")
        || lower.contains("sonnet")
        || lower.contains("haiku")
    {
        return "anthropic";
    }
    if lower.contains("gpt")
        || lower.contains("openai")
        || lower.contains("o1")
        || lower.contains("o3")
    {
        return "openai";
    }
    if lower.contains("gemini") || lower.contains("google") {
        return "google";
    }
    if lower.contains("grok") {
        return "xai";
    }

    "unknown"
}

/// Get default model name based on provider when model field is missing
fn get_default_model_from_provider(provider: &str) -> String {
    match provider.to_lowercase().as_str() {
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

/// Parse a Droid settings.json file
pub fn parse_droid_file(path: &Path) -> Vec<UnifiedMessage> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
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
        let jsonl_path = path
            .to_str()
            .map(|s| s.replace(".settings.json", ".jsonl"))
            .map(std::path::PathBuf::from);

        if let Some(ref jsonl) = jsonl_path {
            extract_model_from_jsonl(jsonl)
                .unwrap_or_else(|| get_default_model_from_provider(&provider))
        } else {
            get_default_model_from_provider(&provider)
        }
    };

    // Get timestamp from providerLockTimestamp or file mtime
    let timestamp = settings
        .provider_lock_timestamp
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
        .map(|dt| dt.timestamp_millis())
        .or_else(|| {
            std::fs::metadata(path)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0)
                })
        })
        .unwrap_or(0);

    if timestamp == 0 {
        return Vec::new();
    }

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
