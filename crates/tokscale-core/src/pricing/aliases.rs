use once_cell::sync::Lazy;
use std::collections::HashMap;

static MODEL_ALIASES: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("big-pickle", "glm-4.7");
    m.insert("big pickle", "glm-4.7");
    m.insert("bigpickle", "glm-4.7");
    m.insert("k2p5", "kimi-k2-thinking");
    m.insert("k2-p5", "kimi-k2-thinking");
    m.insert("k2p6", "kimi-k2.6");
    m.insert("k2-p6", "kimi-k2.6");
    m.insert("kimi-k2p6", "kimi-k2.6");
    m.insert("kimi-k2.5-thinking", "kimi-k2-thinking");
    m.insert("kimi-for-coding", "kimi-k2.5");

    m.insert("model_placeholder_m26", "claude-opus-4-6");
    m.insert("model_placeholder_m35", "claude-sonnet-4-6");
    m.insert("model_placeholder_m36", "gemini-3.1-pro");
    m.insert("model_placeholder_m37", "gemini-3.1-pro");
    // Antigravity uses opaque placeholder IDs in IDE metadata and shorter
    // responseModel aliases in CLI conversation protobufs. These mappings are
    // cross-checked against two independent open-source integrations:
    //
    // - Antigravity Manager is a third-party account/quota manager. Its quota
    //   client shows that Antigravity obtains the available model IDs and
    //   display names from Google Cloud Code Assist's fetchAvailableModels API:
    //   https://github.com/lbjlaq/Antigravity-Manager/blob/dfe876548d572237da92fe4c3e070a9db33c0910/src-tauri/src/modules/quota.rs
    // - Antigravity Context Window Monitor reads the running language server's
    //   GetUserStatus model config and records the placeholder/responseModel
    //   aliases observed in Antigravity session metadata:
    //   https://github.com/AGI-is-going-to-arrive/Antigravity-Context-Window-Monitor/blob/603e3ea00a0ee94f1beecc162cf47a4ed68d3a6f/src/models.ts
    //
    // Keep these as machine-ID aliases. Do not use server-provided display
    // labels as pricing keys because labels may be renamed or localized.
    m.insert("model_placeholder_m16", "gemini-3.1-pro");
    m.insert("model_placeholder_m18", "gemini-3-flash-preview");
    m.insert("model_placeholder_m84", "gemini-3-flash-preview");
    m.insert("model_placeholder_m132", "gemini-3.5-flash-high");
    m.insert("model_placeholder_m133", "gemini-3.5-flash-high");
    m.insert("model_placeholder_m187", "gemini-3.5-flash-low");
    m.insert("model_placeholder_m20", "gemini-3.5-flash-medium");
    m.insert("gemini-pro-default", "gemini-3.1-pro");
    m.insert("gemini-pro-agent", "gemini-3.1-pro");
    m.insert("gemini-3-flash-agent", "gemini-3.5-flash-high");
    m.insert("gemini-3-flash-b", "gemini-3.5-flash-high");
    m.insert("gemini-3.5-flash-low", "gemini-3.5-flash-medium");
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
    m.insert("anthropic/claude-4-5-opus", "claude-opus-4-5");
    m.insert("anthropic/claude-4-5-sonnet", "claude-sonnet-4-5");
    m.insert("anthropic/claude-4-5-haiku", "claude-haiku-4-5");
    m.insert("anthropic/claude-4-6-opus", "claude-opus-4-6");
    m.insert("anthropic/claude-4-6-sonnet", "claude-sonnet-4-6");
    m.insert("anthropic/claude-4-6-haiku", "claude-haiku-4-6");
    m.insert("gemini-3.1-pro-high", "gemini-3.1-pro");
    m.insert("gemini-3.1-pro-low", "gemini-3.1-pro");
    m.insert("gemini-3-pro-high", "gemini-3-pro");
    m.insert("gemini-3-pro-low", "gemini-3-pro");
    m.insert("gemini-3-flash", "gemini-3-flash-preview");
    m.insert("gemini-3-flash-c", "gemini-3-flash-preview");
    m.insert("gemini-3-flash-a", "gemini-3-flash-preview");
    m.insert("grok-composer-2.5", "composer-2.5");
    m.insert("grok-composer-2.5-fast", "composer-2.5-fast");

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
            resolve_alias("model_placeholder_m16"),
            Some("gemini-3.1-pro")
        );
        assert_eq!(
            resolve_alias("MODEL_PLACEHOLDER_M84"),
            Some("gemini-3-flash-preview")
        );
        assert_eq!(
            resolve_alias("model_placeholder_m133"),
            Some("gemini-3.5-flash-high")
        );
        assert_eq!(
            resolve_alias("gemini-3-flash-agent"),
            Some("gemini-3.5-flash-high")
        );
        assert_eq!(
            resolve_alias("gemini-3.5-flash-low"),
            Some("gemini-3.5-flash-medium")
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
            resolve_alias("gemini-3-flash-a"),
            Some("gemini-3-flash-preview")
        );
        assert_eq!(
            resolve_alias("claude-opus-4.6-thinking"),
            Some("claude-opus-4-6")
        );
        assert_eq!(
            resolve_alias("anthropic/claude-4-5-haiku"),
            Some("claude-haiku-4-5")
        );
        assert_eq!(
            resolve_alias("anthropic/claude-4-6-sonnet"),
            Some("claude-sonnet-4-6")
        );
    }

    #[test]
    fn resolves_kimi_k2p6_aliases_without_regressing_k2p5() {
        assert_eq!(resolve_alias("k2p6"), Some("kimi-k2.6"));
        assert_eq!(resolve_alias("k2-p6"), Some("kimi-k2.6"));
        assert_eq!(resolve_alias("kimi-k2p6"), Some("kimi-k2.6"));
        assert_eq!(resolve_alias("KIMI-K2P6"), Some("kimi-k2.6"));

        assert_eq!(resolve_alias("k2p5"), Some("kimi-k2-thinking"));
        assert_eq!(resolve_alias("k2-p5"), Some("kimi-k2-thinking"));
    }

    #[test]
    fn resolves_grok_composer_aliases_to_cursor_composer_prices() {
        assert_eq!(resolve_alias("grok-composer-2.5"), Some("composer-2.5"));
        assert_eq!(
            resolve_alias("GROK-COMPOSER-2.5-FAST"),
            Some("composer-2.5-fast")
        );
    }
}
