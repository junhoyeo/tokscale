use once_cell::sync::Lazy;
use std::collections::HashMap;

static MODEL_ALIASES: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("big-pickle", "glm-4.7");
    m.insert("big pickle", "glm-4.7");
    m.insert("bigpickle", "glm-4.7");
    m.insert("k2p5", "kimi-k2-thinking");
    m.insert("k2-p5", "kimi-k2-thinking");
    m.insert("kimi-k2.5-thinking", "kimi-k2-thinking");
    m.insert("kimi-for-coding", "kimi-k2.5");

    m.insert("model_placeholder_m26", "claude-opus-4-6");
    m.insert("model_placeholder_m35", "claude-sonnet-4-6");
    m.insert("model_placeholder_m36", "gemini-3.1-pro");
    m.insert("model_placeholder_m37", "gemini-3.1-pro");
    m.insert("model_placeholder_m47", "gemini-3-flash-preview");
    m.insert("model_openai_gpt_oss_120b_medium", "gpt-oss-120b-medium");
    m.insert("claude-opus-4-6-thinking", "claude-opus-4-6");
    m.insert("claude-sonnet-4-6-thinking", "claude-sonnet-4-6");
    m.insert("claude-opus-4.6-thinking", "claude-opus-4-6");
    m.insert("claude-sonnet-4.6-thinking", "claude-sonnet-4-6");
    m.insert("claude-opus-4-6", "claude-opus-4-6");
    m.insert("claude-sonnet-4-6", "claude-sonnet-4-6");
    m.insert("claude-haiku-4-6", "claude-haiku-4-6");
    m.insert("claude-opus-4.6", "claude-opus-4-6");
    m.insert("claude-sonnet-4.6", "claude-sonnet-4-6");
    m.insert("claude-haiku-4.6", "claude-haiku-4-6");
    m.insert("gemini-3.1-pro-high", "gemini-3.1-pro");
    m.insert("gemini-3.1-pro-low", "gemini-3.1-pro");
    m.insert("gemini-3-pro-high", "gemini-3-pro");
    m.insert("gemini-3-pro-low", "gemini-3-pro");
    m.insert("gemini-3-flash", "gemini-3-flash-preview");
    m.insert("gemini-3-flash-c", "gemini-3-flash-preview");

    // Synthetic model variants (only where resolver needs help)
    m.insert("kimi-k2.5-nvfp4", "kimi-k2.5"); // Quantization variant → base model pricing
    m.insert("kimi-k2-instruct-0905", "kimi-k2.5"); // Specific version → base (avoids reseller)
    m
});

pub fn resolve_alias(model_id: &str) -> Option<&'static str> {
    MODEL_ALIASES.get(model_id.to_lowercase().as_str()).copied()
}

#[cfg(test)]
mod tests {
    use super::resolve_alias;

    #[test]
    fn resolves_antigravity_placeholders() {
        assert_eq!(
            resolve_alias("MODEL_PLACEHOLDER_M26"),
            Some("claude-opus-4-6")
        );
        assert_eq!(
            resolve_alias("model_placeholder_m37"),
            Some("gemini-3.1-pro")
        );
        assert_eq!(
            resolve_alias("MODEL_OPENAI_GPT_OSS_120B_MEDIUM"),
            Some("gpt-oss-120b-medium")
        );
        assert_eq!(
            resolve_alias("gemini-3-flash-c"),
            Some("gemini-3-flash-preview")
        );
        assert_eq!(
            resolve_alias("claude-opus-4.6-thinking"),
            Some("claude-opus-4-6")
        );
    }
}
