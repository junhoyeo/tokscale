use once_cell::sync::Lazy;
use std::collections::HashMap;

static MODEL_ALIASES: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("big-pickle", "glm-4.7");
    m.insert("big pickle", "glm-4.7");
    m.insert("bigpickle", "glm-4.7");
    m
});

pub fn resolve_alias(model_id: &str) -> Option<&'static str> {
    MODEL_ALIASES.get(model_id.to_lowercase().as_str()).copied()
}

/// Routing prefixes to strip (e.g., `antigravity-claude-opus-4-5` → `claude-opus-4-5`).
const DISPLAY_STRIPPED_PREFIXES: &[&str] = &["antigravity-"];

/// Known display aliases: maps variant/wrong names to canonical display names.
/// Applied after lowercasing, prefix stripping, provider stripping, and date stripping.
static DISPLAY_ALIASES: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("claude-4-sonnet", "claude-sonnet-4");
    m.insert("claude-4-opus", "claude-opus-4");
    m.insert("claude-4-sonnet-thinking", "claude-sonnet-4");
    m.insert("claude-4-opus-thinking", "claude-opus-4");
    m.insert("claude-4.5-opus-high-thinking", "claude-opus-4.5");
    m.insert("claude-4.5-sonnet-thinking", "claude-sonnet-4.5");
    m.insert("claude-3.7-sonnet-thinking-max", "claude-3.7-sonnet");
    m.insert("claude-3.7-sonnet-max", "claude-3.7-sonnet");
    m.insert("claude-3.7-sonnet-thinking", "claude-3.7-sonnet");
    m.insert("claude-3-7-sonnet-thinking-max", "claude-3.7-sonnet");
    m.insert("claude-3-7-sonnet-max", "claude-3.7-sonnet");
    m.insert("claude-3-7-sonnet-thinking", "claude-3.7-sonnet");
    m.insert("gemini-2.5-pro-max", "gemini-2.5-pro");
    m.insert("gpt-5-1-codex-max-0", "gpt-5.1-codex-max");
    m
});

/// Normalize a model ID for display and aggregation purposes.
///
/// This function applies a series of normalization rules so that model ID variants
/// (e.g., `claude-opus-4-1-20250805` and `claude-opus-4-1`) map to the same display key.
///
/// **This does NOT affect pricing lookup** — it is only for display/aggregation.
pub fn normalize_display_model_id(model_id: &str) -> String {
    if model_id.is_empty() {
        return String::new();
    }

    // 1. Lowercase
    let mut s = model_id.to_lowercase();

    // 2. Strip routing prefixes (e.g., "antigravity-")
    for prefix in DISPLAY_STRIPPED_PREFIXES {
        if s.starts_with(prefix) {
            s = s[prefix.len()..].to_string();
        }
    }

    // 3. Strip provider prefixes with "/" (e.g., "qwen/", "accounts/fireworks/models/")
    if let Some(pos) = s.rfind('/') {
        s = s[pos + 1..].to_string();
    }

    // 4. Strip date suffixes: -YYYYMMDD (exactly 8 digits at end)
    s = strip_date_suffix(&s);

    // 5. Strip -preview and -exp suffixes (and variants like -preview-05-06, -exp-03-25)
    s = strip_preview_exp_suffix(&s);

    // 6. Strip -latest suffix
    if s.ends_with("-latest") {
        s = s[..s.len() - 7].to_string();
    }

    // 7. Apply known display aliases
    if let Some(canonical) = DISPLAY_ALIASES.get(s.as_str()) {
        s = canonical.to_string();
    }

    // 8. Strip -thinking suffix from Claude models (also handle -high-thinking, etc.)
    s = strip_claude_thinking_suffix(&s);

    // 9. Version separator: hyphens → dots between single digits (e.g., 4-5 → 4.5, 3-7 → 3.7)
    s = normalize_version_separator(&s);

    s
}

/// Strip date suffix: removes `-YYYYMMDD` (exactly 8 digits) at the end.
fn strip_date_suffix(s: &str) -> String {
    // Check if the string ends with -<8 digits>
    if s.len() < 10 {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let len = bytes.len();
    // Check for hyphen at position len-9
    if bytes[len - 9] != b'-' {
        return s.to_string();
    }
    // Check that the last 8 chars are all digits
    let suffix = &s[len - 8..];
    if suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()) {
        // Verify it looks like a date (starts with 19 or 20)
        if suffix.starts_with("19") || suffix.starts_with("20") {
            return s[..len - 9].to_string();
        }
    }
    s.to_string()
}

/// Strip `-preview`, `-preview-XX-XX`, `-exp`, `-exp-XX-XX` suffixes.
fn strip_preview_exp_suffix(s: &str) -> String {
    // Try stripping -preview-DD-DD or -exp-DD-DD first (longer patterns)
    let re_patterns: &[&str] = &["-preview-", "-exp-"];
    for pat in re_patterns {
        if let Some(pos) = s.find(pat) {
            let after = &s[pos + pat.len()..];
            // Check if what follows is DD-DD (date-like)
            if after.len() >= 5
                && after[..2].chars().all(|c| c.is_ascii_digit())
                && after.as_bytes()[2] == b'-'
                && after[3..5].chars().all(|c| c.is_ascii_digit())
                && after.len() == 5
            {
                return s[..pos].to_string();
            }
        }
    }

    if let Some(stripped) = s.strip_suffix("-preview") {
        return stripped.to_string();
    }
    if let Some(stripped) = s.strip_suffix("-exp") {
        return stripped.to_string();
    }

    s.to_string()
}

fn strip_claude_thinking_suffix(s: &str) -> String {
    if !s.starts_with("claude-") {
        return s.to_string();
    }

    if let Some(base) = s.strip_suffix("-thinking") {
        if let Some(stripped) = base.strip_suffix("-high") {
            return stripped.to_string();
        }
        return base.to_string();
    }

    s.to_string()
}

fn normalize_version_separator(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();

    for i in 0..chars.len() {
        if chars[i] == '-'
            && i > 0
            && i < chars.len() - 1
            && chars[i - 1].is_ascii_digit()
            && chars[i + 1].is_ascii_digit()
        {
            let is_multi_digit_before = i >= 2 && chars[i - 2].is_ascii_digit();
            let is_multi_digit_after = i + 2 < chars.len() && chars[i + 2].is_ascii_digit();

            if !is_multi_digit_before && !is_multi_digit_after {
                result.push('.');
            } else {
                result.push('-');
            }
        } else {
            result.push(chars[i]);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // normalize_display_model_id tests
    // =========================================================================

    #[test]
    fn test_display_lowercase() {
        assert_eq!(
            normalize_display_model_id("Claude-Opus-4-5"),
            "claude-opus-4.5"
        );
        assert_eq!(normalize_display_model_id("GPT-5.1"), "gpt-5.1");
    }

    #[test]
    fn test_display_strip_antigravity_prefix() {
        assert_eq!(
            normalize_display_model_id("antigravity-claude-opus-4-5-thinking"),
            "claude-opus-4.5"
        );
        assert_eq!(
            normalize_display_model_id("antigravity-gemini-3-flash"),
            "gemini-3-flash"
        );
    }

    #[test]
    fn test_display_strip_provider_prefix() {
        assert_eq!(normalize_display_model_id("qwen/qwen3-32b"), "qwen3-32b");
        assert_eq!(
            normalize_display_model_id("moonshotai/kimi-k2-instruct"),
            "kimi-k2-instruct"
        );
        assert_eq!(
            normalize_display_model_id("accounts/fireworks/models/kimi-k2-instruct"),
            "kimi-k2-instruct"
        );
        assert_eq!(
            normalize_display_model_id("meta-llama/llama-3-70b"),
            "llama-3-70b"
        );
    }

    #[test]
    fn test_display_strip_date_suffix() {
        assert_eq!(
            normalize_display_model_id("claude-opus-4-1-20250805"),
            "claude-opus-4.1"
        );
        assert_eq!(
            normalize_display_model_id("claude-sonnet-4-20250514"),
            "claude-sonnet-4"
        );
        assert_eq!(
            normalize_display_model_id("claude-opus-4-5-20251101"),
            "claude-opus-4.5"
        );
        assert_eq!(
            normalize_display_model_id("claude-sonnet-4-5-20250929"),
            "claude-sonnet-4.5"
        );
        assert_eq!(
            normalize_display_model_id("claude-haiku-4-5-20251001"),
            "claude-haiku-4.5"
        );
        assert_eq!(
            normalize_display_model_id("claude-3-haiku-20240307"),
            "claude-3-haiku"
        );
        assert_eq!(
            normalize_display_model_id("claude-3-5-haiku-20241022"),
            "claude-3.5-haiku"
        );
        assert_eq!(
            normalize_display_model_id("claude-3-5-sonnet-20241022"),
            "claude-3.5-sonnet"
        );
    }

    #[test]
    fn test_display_no_strip_short_version() {
        assert_eq!(normalize_display_model_id("gpt-5.1"), "gpt-5.1");
        assert_eq!(normalize_display_model_id("gpt-5.2"), "gpt-5.2");
    }

    #[test]
    fn test_display_strip_preview_suffix() {
        assert_eq!(
            normalize_display_model_id("gemini-3-flash-preview"),
            "gemini-3-flash"
        );
        assert_eq!(
            normalize_display_model_id("gemini-2.5-pro-preview-05-06"),
            "gemini-2.5-pro"
        );
        assert_eq!(
            normalize_display_model_id("gemini-2.5-flash-preview-05-20"),
            "gemini-2.5-flash"
        );
        assert_eq!(
            normalize_display_model_id("gemini-2.5-flash-preview-04-17"),
            "gemini-2.5-flash"
        );
    }

    #[test]
    fn test_display_strip_exp_suffix() {
        assert_eq!(
            normalize_display_model_id("gemini-2.5-pro-exp-03-25"),
            "gemini-2.5-pro"
        );
    }

    #[test]
    fn test_display_strip_latest_suffix() {
        assert_eq!(
            normalize_display_model_id("gemini-2.5-flash-latest"),
            "gemini-2.5-flash"
        );
    }

    #[test]
    fn test_display_alias_claude_thinking_max() {
        assert_eq!(
            normalize_display_model_id("claude-3.7-sonnet-thinking-max"),
            "claude-3.7-sonnet"
        );
    }

    #[test]
    fn test_display_alias_claude_max() {
        assert_eq!(
            normalize_display_model_id("claude-3.7-sonnet-max"),
            "claude-3.7-sonnet"
        );
    }

    #[test]
    fn test_display_alias_claude_thinking() {
        assert_eq!(
            normalize_display_model_id("claude-3.7-sonnet-thinking"),
            "claude-3.7-sonnet"
        );
    }

    #[test]
    fn test_display_alias_gemini_max() {
        assert_eq!(
            normalize_display_model_id("gemini-2.5-pro-max"),
            "gemini-2.5-pro"
        );
    }

    #[test]
    fn test_display_alias_gpt_codex_variant() {
        assert_eq!(
            normalize_display_model_id("gpt-5-1-codex-max-0"),
            "gpt-5.1-codex-max"
        );
    }

    #[test]
    fn test_display_codex_max_is_real_model() {
        // codex-max is the ONLY -max variant that is a real distinct model
        // It must NOT be stripped
        assert_eq!(
            normalize_display_model_id("gpt-5.1-codex-max"),
            "gpt-5.1-codex-max"
        );
        assert_eq!(
            normalize_display_model_id("gpt-5-codex-max"),
            "gpt-5-codex-max"
        );
    }

    #[test]
    fn test_display_alias_claude_wrong_order() {
        assert_eq!(
            normalize_display_model_id("claude-4-sonnet"),
            "claude-sonnet-4"
        );
        assert_eq!(normalize_display_model_id("claude-4-opus"), "claude-opus-4");
    }

    #[test]
    fn test_display_alias_claude_wrong_order_thinking() {
        assert_eq!(
            normalize_display_model_id("claude-4-sonnet-thinking"),
            "claude-sonnet-4"
        );
        assert_eq!(
            normalize_display_model_id("claude-4-opus-thinking"),
            "claude-opus-4"
        );
    }

    #[test]
    fn test_display_alias_claude_4_5_variants() {
        assert_eq!(
            normalize_display_model_id("claude-4.5-opus-high-thinking"),
            "claude-opus-4.5"
        );
        assert_eq!(
            normalize_display_model_id("claude-4.5-sonnet-thinking"),
            "claude-sonnet-4.5"
        );
    }

    #[test]
    fn test_display_strip_claude_thinking() {
        assert_eq!(
            normalize_display_model_id("claude-opus-4-5-thinking"),
            "claude-opus-4.5"
        );
        assert_eq!(
            normalize_display_model_id("claude-sonnet-4-5-thinking"),
            "claude-sonnet-4.5"
        );
        assert_eq!(
            normalize_display_model_id("claude-sonnet-4-thinking"),
            "claude-sonnet-4"
        );
    }

    #[test]
    fn test_display_version_separator_hyphen_to_dot() {
        assert_eq!(
            normalize_display_model_id("claude-3-5-sonnet"),
            "claude-3.5-sonnet"
        );
        assert_eq!(
            normalize_display_model_id("claude-3-7-sonnet"),
            "claude-3.7-sonnet"
        );
        assert_eq!(
            normalize_display_model_id("claude-opus-4-5"),
            "claude-opus-4.5"
        );
        assert_eq!(
            normalize_display_model_id("claude-opus-4-1"),
            "claude-opus-4.1"
        );
        assert_eq!(normalize_display_model_id("gpt-5-codex"), "gpt-5-codex");
        assert_eq!(normalize_display_model_id("gpt-5.1"), "gpt-5.1");
        assert_eq!(normalize_display_model_id("gpt-5.2"), "gpt-5.2");
        assert_eq!(
            normalize_display_model_id("gemini-2.5-pro"),
            "gemini-2.5-pro"
        );
    }

    #[test]
    fn test_display_version_separator_preserves_multi_digit() {
        assert_eq!(normalize_display_model_id("qwen3-32b"), "qwen3-32b");
        assert_eq!(normalize_display_model_id("llama-3-70b"), "llama-3-70b");
        assert_eq!(
            normalize_display_model_id("minimax-m2.1-free"),
            "minimax-m2.1-free"
        );
    }

    #[test]
    fn test_display_empty_string() {
        assert_eq!(normalize_display_model_id(""), "");
    }

    #[test]
    fn test_display_already_normalized() {
        assert_eq!(
            normalize_display_model_id("claude-opus-4.5"),
            "claude-opus-4.5"
        );
        assert_eq!(
            normalize_display_model_id("gemini-2.5-pro"),
            "gemini-2.5-pro"
        );
        assert_eq!(normalize_display_model_id("gpt-5.1"), "gpt-5.1");
    }

    #[test]
    fn test_display_unknown_model_passthrough() {
        assert_eq!(
            normalize_display_model_id("some-unknown-model"),
            "some-unknown-model"
        );
    }

    #[test]
    fn test_display_combined_prefix_and_date() {
        assert_eq!(
            normalize_display_model_id("antigravity-claude-opus-4-5-20251101"),
            "claude-opus-4.5"
        );
    }

    #[test]
    fn test_display_combined_provider_and_date() {
        assert_eq!(
            normalize_display_model_id("anthropic/claude-sonnet-4-20250514"),
            "claude-sonnet-4"
        );
    }

    #[test]
    fn test_display_combined_all_rules() {
        assert_eq!(
            normalize_display_model_id("anthropic/claude-3-5-sonnet-20241022"),
            "claude-3.5-sonnet"
        );
    }

    #[test]
    fn test_display_gemini_preview_with_date() {
        assert_eq!(
            normalize_display_model_id("gemini-2.5-pro-preview-05-06"),
            "gemini-2.5-pro"
        );
    }

    #[test]
    fn test_display_claude_3_5_with_dot_and_date() {
        assert_eq!(
            normalize_display_model_id("claude-3.5-sonnet-20241022"),
            "claude-3.5-sonnet"
        );
    }
}
