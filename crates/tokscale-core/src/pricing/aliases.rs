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
    m.insert("kimi-for-coding-highspeed", "kimi-k2.7-code-highspeed");
    m.insert("k3", "kimi-k3");

    // MiniMax M3: Ollama Cloud and other routers report the model with the
    // lowercase bare id `minimax-m3` (and mixed-case variants), while the
    // authoritative dataset key is `minimax/MiniMax-M3` (litellm). The bare id
    // has no exact hit in any dataset, so with no usable provider hint it falls
    // through to model-part matching across every row whose model part is
    // `minimax-m3` — and models.dev publishes that model part under dozens of
    // third parties, several at 0.0/0.0 (`kenari/minimax-m3`,
    // `nvidia/minimaxai/minimax-m3`). Electing one of those prices real usage
    // at exactly $0, which is the "pricing missing" symptom in #935. Pin the
    // canonical first-party key so the id prices deterministically.
    m.insert("minimax-m3", "minimax/MiniMax-M3");

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
    // M133/`gemini-3-flash-b`, `gemini-3-flash-a`, and M187/raw
    // `gemini-3.5-flash-low` are cases where the obvious mapping is wrong,
    // verified against the pinned Antigravity Context Window Monitor SHA
    // above (models.ts@603e3ea):
    //
    // - M133 was renamed from "Gemini 3 Flash" to "Gemini 3.5 Flash (High)"
    //   ("MODEL_PLACEHOLDER_M133": 'Gemini 3.5 Flash (High)', // gemini-3-flash-agent
    //   (renamed from "Gemini 3 Flash")"), and `responseModelAliases` maps
    //   BOTH `gemini-3-flash-agent` and `gemini-3-flash-b` to M133. So M133
    //   and `gemini-3-flash-b` must resolve identically to `gemini-3-flash-agent`
    //   (gemini-3.5-flash-high), not to the retired gemini-3-flash-preview tier.
    // - `responseModelAliases['gemini-3-flash-a'] = 'MODEL_PLACEHOLDER_M132'`
    //   ("legacy responseModel for 3.5 Flash"), and
    //   `STATIC_MODEL_NAME_FALLBACKS['MODEL_PLACEHOLDER_M132'] =
    //   'Gemini 3.5 Flash (High)' // retired predecessor of M133`. So
    //   `gemini-3-flash-a` prices as the retired-predecessor High tier
    //   (gemini-3.5-flash-high) — the same catalog entry as M133/M132/
    //   `gemini-3-flash-b` — not as the unrelated gemini-3-flash-preview
    //   family (M18/M84), which is a different, older backend command model.
    // - M20's `activeModelSpecs` entry has `modelId: 'gemini-3.5-flash-low'`
    //   with `displayName: 'Gemini 3.5 Flash (Medium)'` — the wire string
    //   says "low" but the tier is actually Medium. M187 is a distinct
    //   placeholder whose own `activeModelSpecs` entry has
    //   `modelId: 'gemini-3.5-flash-extra-low'` and
    //   `displayName: 'Gemini 3.5 Flash (Low)'` — the true Low tier. M187
    //   and M20/raw `gemini-3.5-flash-low` must NOT collapse to the same
    //   canonical alias target: M187 maps to `gemini-3.5-flash-extra-low`
    //   (its own machine ID), while M20 and the raw wire string map to
    //   `gemini-3.5-flash-medium`.
    m.insert("model_placeholder_m16", "gemini-3.1-pro");
    m.insert("model_placeholder_m18", "gemini-3-flash-preview");
    m.insert("model_placeholder_m84", "gemini-3-flash-preview");
    m.insert("model_placeholder_m132", "gemini-3.5-flash-high");
    m.insert("model_placeholder_m133", "gemini-3.5-flash-high");
    m.insert("model_placeholder_m187", "gemini-3.5-flash-extra-low");
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
    // Anthropic's "-0" suffix is their documented moving alias for the latest
    // snapshot of a model line (claude-opus-4-0 -> newest Opus 4). Datasets
    // publish the dated key instead, so the alias form resolved to nothing and
    // real first-party usage was excluded from submission as unpriced.
    m.insert("claude-opus-4-0", "claude-opus-4");
    m.insert("claude-sonnet-4-0", "claude-sonnet-4");
    // GitHub Copilot reports Claude 4.1 without the separator. Copilot usage is
    // priced at the underlying model's rates (its own $0.00 subscription rows
    // are filtered out by EXCLUDED_LITELLM_PREFIXES), so this must resolve the
    // same way github_copilot/gpt-4o already resolves to gpt-4o.
    // Deliberately opus-only: `claude-sonnet-4-1` currently resolves to
    // `databricks/databricks-claude-sonnet-4-1` via a cross-vendor fuzzy match
    // (#1062), so aliasing the Copilot spelling onto it would route Sonnet 4.1
    // usage to Databricks rates. Add it once #1062 makes that target safe.
    m.insert("claude-opus-41", "claude-opus-4-1");
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
    m.insert("gemini-3-flash-a", "gemini-3.5-flash-high");
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
            ("model_placeholder_m187", "gemini-3.5-flash-extra-low"),
            ("model_placeholder_m20", "gemini-3.5-flash-medium"),
            ("gemini-pro-default", "gemini-3.1-pro"),
            ("gemini-pro-agent", "gemini-3.1-pro"),
            ("gemini-3-flash-agent", "gemini-3.5-flash-high"),
            ("gemini-3-flash-b", "gemini-3.5-flash-high"),
            ("gemini-3.5-flash-low", "gemini-3.5-flash-medium"),
            ("MODEL_OPENAI_GPT_OSS_120B_MEDIUM", "gpt-oss-120b-medium"),
            ("gemini-3-flash-c", "gemini-3-flash-preview"),
            ("gemini-3-flash-a", "gemini-3.5-flash-high"),
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
    fn resolves_kimi_coding_plan_ids_to_underlying_models() {
        // kimi-code writes `kimi-code/<id>`; the parser strips the prefix, so
        // pricing sees the bare id. Without these, models.dev matches them under
        // its `kimi-for-coding/*` subscription namespace at $0.00.
        assert_eq!(
            resolve_alias("kimi-for-coding-highspeed"),
            Some("kimi-k2.7-code-highspeed")
        );
        assert_eq!(resolve_alias("k3"), Some("kimi-k3"));
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
    fn m187_and_m20_resolve_to_distinct_tiers_but_both_still_price() {
        // M187 (true Low tier, machine id `gemini-3.5-flash-extra-low`) and
        // M20/raw CLI `gemini-3.5-flash-low` (actually the Medium tier) must
        // NOT collapse to the same canonical alias target — that would
        // silently merge two different-priced tiers into one cost bucket.
        // Verified against the pinned Antigravity Context Window Monitor SHA
        // (models.ts@603e3ea): M187's own `activeModelSpecs` entry has
        // `modelId: 'gemini-3.5-flash-extra-low'`, distinct from M20's
        // `modelId: 'gemini-3.5-flash-low'`.
        let m187_canonical = resolve_alias("model_placeholder_m187").unwrap();
        let m20_canonical = resolve_alias("model_placeholder_m20").unwrap();
        let cli_low_canonical = resolve_alias("gemini-3.5-flash-low").unwrap();

        assert_eq!(m187_canonical, "gemini-3.5-flash-extra-low");
        assert_eq!(m20_canonical, "gemini-3.5-flash-medium");
        assert_ne!(
            m187_canonical, m20_canonical,
            "M187 (Low) and M20 (Medium) must not resolve to the same tier"
        );
        // The raw CLI wire string tracks M20 (Medium), not M187 (Low).
        assert_eq!(cli_low_canonical, m20_canonical);

        // Both tiers must still reach a priced catalog entry: the pricing
        // dataset only carries one generic `google/gemini-3.5-flash` entry,
        // and the lookup's suffix-stripping normalization must land both the
        // `-extra-low` and `-medium` canonical ids on it.
        let mut models_dev = HashMap::new();
        models_dev.insert(
            "google/gemini-3.5-flash".to_string(),
            super::super::litellm::ModelPricing {
                input_cost_per_token: Some(0.0000015),
                output_cost_per_token: Some(0.000009),
                ..Default::default()
            },
        );
        let lookup = super::super::lookup::PricingLookup::new_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            models_dev,
        );

        let m187_result = lookup
            .lookup(m187_canonical)
            .expect("M187 target must still price via lookup normalization");
        let m20_result = lookup
            .lookup(m20_canonical)
            .expect("M20 target must still price via lookup normalization");

        assert_eq!(m187_result.matched_key, "google/gemini-3.5-flash");
        assert_eq!(m20_result.matched_key, "google/gemini-3.5-flash");
    }

    /// Regression: Ollama Cloud and other routers report MiniMax M3 as the
    /// bare lowercase id `minimax-m3`, which matches no dataset key exactly.
    /// Unhinted, it fell through to the model-part fallback and could elect any
    /// third-party row publishing that model part — including the 0.0/0.0 rows
    /// models.dev carries — instead of the first-party `minimax/MiniMax-M3`
    /// key (#935).
    #[test]
    fn resolves_minimax_m3_bare_and_case_variants() {
        // resolve_alias is case-insensitive, since clients report mixed casing.
        assert_eq!(
            super::resolve_alias("minimax-m3"),
            Some("minimax/MiniMax-M3")
        );
        assert_eq!(
            super::resolve_alias("MiniMax-M3"),
            Some("minimax/MiniMax-M3")
        );
        assert_eq!(
            super::resolve_alias("MINIMAX-M3"),
            Some("minimax/MiniMax-M3")
        );
        // The qualified id already resolves via exact match; aliasing it too
        // would be harmless, but the bare form is the reported gap.
        assert_eq!(super::resolve_alias("minimax/minimax-m3"), None);
    }

    /// Regression: Anthropic's "-0" suffix is a documented moving alias for the
    /// latest snapshot of a model line, and GitHub Copilot reports 4.1 without
    /// the separator. Neither form resolved, so real first-party usage was
    /// excluded from submission as unpriced — 41M tokens of claude-opus-4-0 in
    /// one reported case.
    #[test]
    fn anthropic_moving_aliases_and_copilot_spelling_resolve() {
        assert_eq!(
            super::resolve_alias("claude-opus-4-0"),
            Some("claude-opus-4")
        );
        assert_eq!(
            super::resolve_alias("claude-sonnet-4-0"),
            Some("claude-sonnet-4")
        );
        assert_eq!(
            super::resolve_alias("claude-opus-41"),
            Some("claude-opus-4-1")
        );
        // Case-insensitive, since clients report mixed casing.
        assert_eq!(
            super::resolve_alias("Claude-Opus-4-0"),
            Some("claude-opus-4")
        );

        // Deliberately absent: `claude-sonnet-4-1` resolves cross-vendor to
        // `databricks/databricks-claude-sonnet-4-1` today (#1062), so aliasing
        // the Copilot spelling onto it would route usage to the wrong rates.
        assert_eq!(super::resolve_alias("claude-sonnet-41"), None);
    }
}
