fn canonicalize_provider_segment(segment: &str) -> Option<String> {
    let normalized = segment
        .trim()
        .trim_end_matches('/')
        .to_lowercase()
        .replace('-', "_");

    let canonical = match normalized.as_str() {
        "" | "unknown" => return None,
        "x_ai" | "xai" => "xai",
        "z_ai" | "zai" => "zai",
        "moonshot" | "moonshotai" => "moonshotai",
        "meta" | "meta_llama" => "meta_llama",
        "azure" | "azure_ai" => "azure_ai",
        "anthropic" | "vertex" | "vertex_ai" => "anthropic",
        "together" | "together_ai" => "together_ai",
        "fireworks" | "fireworks_ai" => "fireworks_ai",
        "google" | "gemini" => "google",
        "openai" | "openai_codex" => "openai",
        "mistral" | "mistralai" => "mistralai",
        "ai21" => "ai21",
        // For unknown segments, reject if they contain digits — those are
        // almost certainly model-name fragments (e.g., "gpt-4", "claude-3")
        // rather than provider identifiers.
        other if other.chars().any(|ch| ch.is_ascii_digit()) => return None,
        other => other,
    };

    Some(canonical.into())
}

pub fn canonical_provider(raw: &str) -> Option<String> {
    provider_tags(raw).into_iter().next()
}

pub fn provider_tags(raw: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut push = |segment: &str| {
        if let Some(tag) = canonicalize_provider_segment(segment) {
            if !tags.iter().any(|existing| existing == &tag) {
                tags.push(tag);
            }
        }
    };

    for segment in raw.trim().trim_end_matches('/').split('/') {
        push(segment);
        if segment.contains('.') {
            for dotted in segment.split('.') {
                push(dotted);
            }
        }
    }

    tags
}

pub fn key_provider_tags(dataset_key: &str) -> Vec<String> {
    let key_parts: Vec<&str> = dataset_key.split('/').collect();
    if key_parts.len() < 2 {
        return Vec::new();
    }

    let mut tags = Vec::new();
    let mut push_all = |value: &str| {
        for tag in provider_tags(value) {
            if !tags.iter().any(|existing| existing == &tag) {
                tags.push(tag);
            }
        }
    };

    for segment in &key_parts[..key_parts.len() - 1] {
        push_all(segment);
    }
    for dotted in key_parts[key_parts.len() - 1].split('.') {
        push_all(dotted);
    }

    tags
}

pub fn matches_provider_hint(dataset_key: &str, provider_id: Option<&str>) -> bool {
    let Some(provider_id) = provider_id else {
        return false;
    };

    let hint_tags = provider_tags(provider_id);
    matches_provider_hint_with_tags(dataset_key, &hint_tags)
}

pub fn matches_provider_hint_with_tags(dataset_key: &str, hint_tags: &[String]) -> bool {
    if hint_tags.is_empty() {
        return false;
    }

    let key_tags = key_provider_tags(dataset_key);
    if key_tags.is_empty() {
        return false;
    }

    key_tags
        .iter()
        .any(|key_tag| hint_tags.iter().any(|hint_tag| hint_tag == key_tag))
}

fn contains_delimited(haystack: &str, needle: &str) -> bool {
    for (pos, _) in haystack.match_indices(needle) {
        let before_ok = pos == 0 || !haystack.as_bytes()[pos - 1].is_ascii_alphanumeric();
        let after_pos = pos + needle.len();
        let after_ok =
            after_pos == haystack.len() || !haystack.as_bytes()[after_pos].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

pub fn inferred_provider_from_model(model: &str) -> Option<&'static str> {
    let lower = model.to_lowercase();

    if lower.contains("claude")
        || lower.contains("anthropic")
        || contains_delimited(&lower, "opus")
        || contains_delimited(&lower, "sonnet")
        || contains_delimited(&lower, "haiku")
    {
        return Some("anthropic");
    }

    if lower.contains("gpt")
        || lower.contains("openai")
        || contains_delimited(&lower, "o1")
        || contains_delimited(&lower, "o3")
        || contains_delimited(&lower, "o4")
    {
        return Some("openai");
    }

    if lower.contains("gemini") || lower.contains("google") {
        return Some("google");
    }

    if lower.contains("grok") {
        return Some("xai");
    }

    if lower.contains("deepseek") {
        return Some("deepseek");
    }

    if lower.contains("mistral") || lower.contains("mixtral") {
        return Some("mistral");
    }

    if lower.contains("llama") || contains_delimited(&lower, "meta") {
        return Some("meta");
    }

    if lower.contains("qwen") {
        return Some("qwen");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_tags_normalize_known_aliases() {
        let cases = [
            ("openai-codex", vec!["openai"]),
            ("gemini", vec!["google"]),
            ("vertex", vec!["anthropic"]),
            ("azure", vec!["azure_ai"]),
            ("fireworks", vec!["fireworks_ai"]),
            ("openrouter/google", vec!["openrouter", "google"]),
            ("bedrock/anthropic", vec!["bedrock", "anthropic"]),
        ];

        for (raw, expected) in cases {
            assert_eq!(provider_tags(raw), expected);
        }
    }

    #[test]
    fn test_canonical_provider_returns_first_canonical_tag() {
        assert_eq!(canonical_provider("openai-codex"), Some("openai".into()));
        assert_eq!(
            canonical_provider("openrouter/google"),
            Some("openrouter".into())
        );
        assert_eq!(canonical_provider("unknown"), None);
    }

    #[test]
    fn test_key_provider_tags_extract_nested_provider_segments() {
        assert_eq!(
            key_provider_tags("openrouter/google/gemini-3-pro-preview"),
            vec!["openrouter", "google"]
        );
        assert_eq!(
            key_provider_tags("bedrock/anthropic.claude-sonnet-4"),
            vec!["bedrock", "anthropic"]
        );
    }

    #[test]
    fn test_matches_provider_hint_for_known_aliases_and_nested_keys() {
        assert!(matches_provider_hint(
            "openai/gpt-5.2-preview",
            Some("openai-codex")
        ));
        assert!(matches_provider_hint(
            "openrouter/google/gemini-3-pro-preview",
            Some("google")
        ));
        assert!(matches_provider_hint("azure/openai/gpt-4", Some("azure")));
        assert!(matches_provider_hint(
            "fireworks_ai/deepseek-v3-0324",
            Some("fireworks")
        ));
        assert!(!matches_provider_hint("openai/gpt-4", Some("anthropic")));
    }

    #[test]
    fn test_inferred_provider_from_model() {
        assert_eq!(
            inferred_provider_from_model("claude-sonnet-4"),
            Some("anthropic")
        );
        assert_eq!(inferred_provider_from_model("gpt-5.2"), Some("openai"));
        assert_eq!(inferred_provider_from_model("gpt-5.5"), Some("openai"));
        assert_eq!(
            inferred_provider_from_model("gemini-2.5-pro"),
            Some("google")
        );
        assert_eq!(
            inferred_provider_from_model("grok-code-fast-1"),
            Some("xai")
        );
        assert_eq!(
            inferred_provider_from_model("deepseek-v3"),
            Some("deepseek")
        );
        assert_eq!(
            inferred_provider_from_model("mixtral-8x7b"),
            Some("mistral")
        );
        assert_eq!(
            inferred_provider_from_model("mistral-large"),
            Some("mistral")
        );
        assert_eq!(inferred_provider_from_model("llama-3"), Some("meta"));
        assert_eq!(inferred_provider_from_model("qwen3-coder"), Some("qwen"));
        assert_eq!(inferred_provider_from_model("unknown-model"), None);
    }

    #[test]
    fn test_inferred_provider_no_false_positives() {
        assert_eq!(inferred_provider_from_model("protocol1-fast"), None);
        assert_eq!(inferred_provider_from_model("proto3-server"), None);
        assert_eq!(inferred_provider_from_model("co4pilot-v2"), None);
        assert_eq!(inferred_provider_from_model("metadata-model"), None);
        assert_eq!(inferred_provider_from_model("metamorphic-v1"), None);
    }

    #[test]
    fn test_inferred_provider_boundary_matches() {
        assert_eq!(inferred_provider_from_model("o1-preview"), Some("openai"));
        assert_eq!(inferred_provider_from_model("o3-mini"), Some("openai"));
        assert_eq!(inferred_provider_from_model("o4-mini"), Some("openai"));
        assert_eq!(inferred_provider_from_model("meta-llama-3"), Some("meta"));
    }

    #[test]
    fn test_provider_tags_mistral_alias() {
        assert_eq!(provider_tags("mistral"), vec!["mistralai"]);
        assert_eq!(provider_tags("mistralai"), vec!["mistralai"]);
    }

    #[test]
    fn test_matches_provider_hint_mistral_keys() {
        assert!(matches_provider_hint(
            "mistralai/mistral-large",
            Some("mistral")
        ));
        assert!(matches_provider_hint(
            "mistralai/mixtral-8x7b",
            Some("mistralai")
        ));
    }

    #[test]
    fn test_provider_tags_ai21_with_digits() {
        assert_eq!(provider_tags("ai21"), vec!["ai21"]);
    }

    #[test]
    fn test_matches_provider_hint_none_and_empty() {
        assert!(!matches_provider_hint("openai/gpt-4", None));
        assert!(!matches_provider_hint("openai/gpt-4", Some("")));
        assert!(!matches_provider_hint("openai/gpt-4", Some("unknown")));
    }
}
