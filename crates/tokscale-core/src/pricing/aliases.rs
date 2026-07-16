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
    // responseModel aliases in CLI conversation protobufs. The evidence has
    // two distinct roles:
    //
    // - Antigravity Manager is a third-party account/quota manager. Its quota
    //   client documents the server-side metadata source and response shape:
    //   model IDs and display names come from Google Cloud Code Assist's
    //   fetchAvailableModels API.
    //   https://github.com/lbjlaq/Antigravity-Manager/blob/dfe876548d572237da92fe4c3e070a9db33c0910/src-tauri/src/modules/quota.rs
    // - The concrete placeholder and responseModel mappings below come from
    //   Antigravity Context Window Monitor's GetUserStatus/session registry.
    //   https://github.com/AGI-is-going-to-arrive/Antigravity-Context-Window-Monitor/blob/603e3ea00a0ee94f1beecc162cf47a4ed68d3a6f/src/models.ts
    //
    // Keep these as machine-ID aliases. Do not use server-provided display
    // labels as pricing keys because labels may be renamed or localized.
    //
    // M133/`gemini-3-flash-b` and M187/raw `gemini-3.5-flash-low` are two
    // cases where the obvious mapping is wrong, verified against the pinned
    // Antigravity Context Window Monitor SHA above (models.ts@603e3ea):
    //
    // - M133 was renamed from "Gemini 3 Flash" to "Gemini 3.5 Flash (High)"
    //   ("MODEL_PLACEHOLDER_M133": 'Gemini 3.5 Flash (High)', // gemini-3-flash-agent
    //   (renamed from "Gemini 3 Flash")"), and `responseModelAliases` maps
    //   BOTH `gemini-3-flash-agent` and `gemini-3-flash-b` to M133. So M133
    //   and `gemini-3-flash-b` must resolve identically to `gemini-3-flash-agent`
    //   (gemini-3.5-flash-high), not to the retired gemini-3-flash-preview tier.
    // - The raw CLI responseModel string `gemini-3.5-flash-low` is, per the
    //   same pinned source, literally `responseModelAliases['gemini-3.5-flash-low']
    //   = 'MODEL_PLACEHOLDER_M20' // model_id for M20 (3.5 Flash Medium)` — i.e.
    //   despite the name, that wire string identifies the Medium tier, not the
    //   Low tier (M187). resolve_alias is single-hop, so M187 must resolve
    //   directly to the same final target as the raw `gemini-3.5-flash-low`
    //   key instead of to the string `gemini-3.5-flash-low` itself (which
    //   would otherwise re-enter this table on a second hop and collapse to
    //   a different result depending on whether it came from the IDE or the
    //   CLI).
    m.insert("model_placeholder_m16", "gemini-3.1-pro");
    m.insert("model_placeholder_m18", "gemini-3-flash-preview");
    m.insert("model_placeholder_m84", "gemini-3-flash-preview");
    m.insert("model_placeholder_m132", "gemini-3.5-flash-high");
    m.insert("model_placeholder_m133", "gemini-3.5-flash-high");
    m.insert("model_placeholder_m187", "gemini-3.5-flash-medium");
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
    use std::collections::HashMap;

    #[test]
    fn resolves_antigravity_placeholders() {
        let cases = [
            ("MODEL_PLACEHOLDER_M26", "claude-opus-4-6"),
            ("model_placeholder_m37", "gemini-3.1-pro"),
            ("model_placeholder_m16", "gemini-3.1-pro"),
            ("model_placeholder_m18", "gemini-3-flash-preview"),
            ("MODEL_PLACEHOLDER_M84", "gemini-3-flash-preview"),
            ("model_placeholder_m132", "gemini-3.5-flash-high"),
            ("model_placeholder_m133", "gemini-3.5-flash-high"),
            ("model_placeholder_m187", "gemini-3.5-flash-medium"),
            ("model_placeholder_m20", "gemini-3.5-flash-medium"),
            ("gemini-pro-default", "gemini-3.1-pro"),
            ("gemini-pro-agent", "gemini-3.1-pro"),
            ("gemini-3-flash-agent", "gemini-3.5-flash-high"),
            ("gemini-3-flash-b", "gemini-3.5-flash-high"),
            ("gemini-3.5-flash-low", "gemini-3.5-flash-medium"),
            ("MODEL_OPENAI_GPT_OSS_120B_MEDIUM", "gpt-oss-120b-medium"),
            ("gemini-3-flash-c", "gemini-3-flash-preview"),
            ("gemini-3-flash-a", "gemini-3-flash-preview"),
            ("claude-opus-4.6-thinking", "claude-opus-4-6"),
            ("anthropic/claude-4-5-haiku", "claude-haiku-4-5"),
            ("anthropic/claude-4-6-sonnet", "claude-sonnet-4-6"),
        ];

        for (raw, expected) in cases {
            assert_eq!(resolve_alias(raw), Some(expected), "raw model: {raw}");
        }
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

    #[test]
    fn ide_and_cli_low_tier_aliases_price_to_the_same_catalog_entry() {
        // Two-stage regression for the collapsed M187 chain: the IDE emits
        // the opaque placeholder `model_placeholder_m187`, while the CLI
        // emits the raw responseModel string `gemini-3.5-flash-low`. Both
        // must resolve (single-hop) to the same canonical id, and that id
        // must land on the identical priced catalog entry so the two
        // sources merge into one cost bucket instead of splitting.
        let ide_canonical = resolve_alias("model_placeholder_m187").unwrap();
        let cli_canonical = resolve_alias("gemini-3.5-flash-low").unwrap();
        assert_eq!(ide_canonical, "gemini-3.5-flash-medium");
        assert_eq!(ide_canonical, cli_canonical);

        let mut litellm = HashMap::new();
        litellm.insert(
            "gemini-3.5-flash-medium".to_string(),
            super::super::litellm::ModelPricing {
                input_cost_per_token: Some(0.0000005),
                output_cost_per_token: Some(0.000003),
                ..Default::default()
            },
        );
        let lookup =
            super::super::lookup::PricingLookup::new(litellm, HashMap::new(), HashMap::new());

        let ide_result = lookup.lookup(ide_canonical).expect("IDE path must price");
        let cli_result = lookup.lookup(cli_canonical).expect("CLI path must price");

        assert_eq!(ide_result.matched_key, "gemini-3.5-flash-medium");
        assert_eq!(ide_result.matched_key, cli_result.matched_key);
        assert_eq!(
            ide_result.pricing.input_cost_per_token,
            cli_result.pricing.input_cost_per_token
        );
        assert_eq!(
            ide_result.pricing.output_cost_per_token,
            cli_result.pricing.output_cost_per_token
        );
    }
}
