use once_cell::sync::Lazy;
use std::collections::HashMap;

static CURSOR_PRICING_ALIASES: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut aliases = HashMap::new();
    for tier in [
        "cursor-grok-4.6-high",
        "cursor-grok-4.6-high-fast",
        "cursor-grok-4.6-low",
        "cursor-grok-4.6-low-fast",
        "cursor-grok-4.6-medium",
        "cursor-grok-4.6-medium-fast",
        "cursor-grok-4.6-xhigh",
    ] {
        aliases.insert(tier, "grok-4.6");
    }
    aliases.insert("grok-composer-2.5", "composer-2.5");
    aliases.insert("grok-composer-2.5-fast", "composer-2.5-fast");
    aliases
});

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
    // Kimi CLI reports `kimi-for-coding` for Kimi K2.7 Code (its config sets
    // `display_name = "K2.7 Coding"`). It was previously aliased to k2.5, which
    // priced it about a third under the real rate.
    m.insert("kimi-for-coding", "kimi-k2.7-code");
    m.insert("kimi-for-coding-highspeed", "kimi-k2.7-code-highspeed");
    m.insert("k3", "kimi-k3");
    // models.dev also publishes `kimi-for-coding/k3-256k` at $0.00, so the
    // long-context spelling must resolve to the same real moonshotai row as
    // bare `k3` instead of landing on the zero-priced subscription namespace.
    m.insert("k3-256k", "kimi-k3");
    // Kimi Work (the Kimi desktop app's agent mode) embeds the same kimi-code
    // kernel and writes the same wire protocol, but reports its own ids.
    // Unaliased they fuzzy-match badly: `k2d6-agent` landed on
    // `xai/grok-4.20-multi-agent-beta-0309`, and the `k3-agent*` ids fell into
    // the Models.dev `kimi-for-coding/*` subscription namespace, which prices
    // at $0.00.
    m.insert("k2d6-agent", "kimi-k2.6");
    m.insert("k3-agent", "kimi-k3");
    m.insert("k3-agent-swarm", "kimi-k3");

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
    // OpenAI documents the API spelling below as a moving alias for
    // `gpt-5.6-sol`; Codex records the same alias with its `gpt-` prefix.
    // Keep the API, Codex, and provider-qualified spellings pinned to the
    // currently documented target so the upstream GPT-5.6 Sol row supplies
    // all token-bucket rates. The qualified form must be explicit because
    // provider-prefix stripping does not run alias resolution a second time.
    // Sources (accessed 2026-08-17):
    // https://developers.openai.com/api/docs/guides/safety-checks/cybersecurity
    // https://developers.openai.com/api/docs/pricing
    m.insert("daybreak-blue-latest", "gpt-5.6-sol");
    m.insert("gpt-daybreak-blue-latest", "gpt-5.6-sol");
    m.insert("openai/gpt-daybreak-blue-latest", "gpt-5.6-sol");
    m.insert("openai/daybreak-blue-latest", "gpt-5.6-sol");

    // Stealth preview shorthands for the August 2026 Z.AI GLM-5.3-Flash free
    // preview. Sessions record the bare `ox-alpha` (and the router-qualified
    // `stealth/ox-alpha` gateways emit), while upstream models.dev tracks the
    // canonical free incarnations `opencode-go/ox-alpha-free` and
    // `opencode/x-preview-f-free` at $0.00 (both deprecated). Pin the
    // shorthand spellings to those canonical keys -- the same "canonical
    // first-party key" pattern as `minimax-m3` above -- so the live upstream
    // $0 row prices them instead of an unverified reseller guess. The
    // qualified `stealth/` form must be explicit because provider-prefix
    // stripping does not run alias resolution a second time.
    // Sources (accessed 2026-09-03):
    // https://openrouter.ai/stealth/ox-alpha ("free to use", ZAI reveal)
    // https://docs.z.ai/guides/vlm/glm-5.3-flash ("tested anonymously as ox-alpha")
    // https://models.dev/api.json (providers.opencode.models.x-preview-f-free
    // and providers.opencode-go.models.ox-alpha-free at input = 0,
    // output = 0, cache_read = 0, both deprecated)
    m.insert("ox-alpha", "opencode-go/ox-alpha-free");
    m.insert("stealth/ox-alpha", "opencode-go/ox-alpha-free");
    m.insert("x-preview-f-free", "opencode/x-preview-f-free");

    // Synthetic model variants (only where resolver needs help)
    m.insert("kimi-k2.5-nvfp4", "kimi-k2.5"); // Quantization variant → base model pricing
    m.insert("kimi-k2-instruct-0905", "kimi-k2.5"); // Specific version → base (avoids reseller)
    m
});

pub fn resolve_alias(model_id: &str) -> Option<&'static str> {
    let lowered = model_id.to_lowercase();
    if let Some(target) = MODEL_ALIASES.get(lowered.as_str()) {
        return Some(target);
    }
    if let Some(target) = CURSOR_PRICING_ALIASES.get(lowered.as_str()) {
        return Some(target);
    }
    // kimi-code reports some rows as `kimi-code/<id>`. The Kimi parser strips
    // that prefix before pricing, but any other path reaching pricing with the
    // qualified form would otherwise miss every alias above and fall through to
    // the Models.dev `kimi-for-coding/*` namespace, which prices at $0.00 — so
    // the qualified and bare spellings of the same model would disagree.
    let bare = lowered.strip_prefix("kimi-code/")?;
    MODEL_ALIASES.get(bare).copied()
}

pub fn uses_cursor_pricing(model_id: &str) -> bool {
    CURSOR_PRICING_ALIASES.contains_key(model_id.to_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::{resolve_alias, uses_cursor_pricing};
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
        // The long-context spelling has its own zero-priced
        // `kimi-for-coding/k3-256k` row on models.dev, so it must resolve to
        // the same real moonshotai row as bare `k3`.
        assert_eq!(resolve_alias("k3-256k"), Some("kimi-k3"));
    }

    #[test]
    fn resolves_kimi_work_agent_ids_to_their_underlying_models() {
        // Kimi Work reports its own ids. Without these they fuzzy-match badly:
        // `k2d6-agent` resolved to `xai/grok-4.20-multi-agent-beta-0309`, and
        // the `k3-agent*` ids landed in the zero-priced `kimi-for-coding/*`
        // subscription namespace.
        assert_eq!(resolve_alias("k2d6-agent"), Some("kimi-k2.6"));
        assert_eq!(resolve_alias("k3-agent"), Some("kimi-k3"));
        assert_eq!(resolve_alias("k3-agent-swarm"), Some("kimi-k3"));
        assert_eq!(resolve_alias("K3-AGENT-SWARM"), Some("kimi-k3"));
    }

    #[test]
    fn kimi_for_coding_prices_as_k2p7_code_not_k2p5() {
        // Kimi CLI's own config names this "K2.7 Coding"; the previous k2.5
        // target priced it about a third under the real rate.
        assert_eq!(resolve_alias("kimi-for-coding"), Some("kimi-k2.7-code"));
        assert_eq!(
            resolve_alias("kimi-for-coding-highspeed"),
            Some("kimi-k2.7-code-highspeed")
        );
    }

    #[test]
    fn cursor_grok_reasoning_tiers_resolve_to_the_base_model() {
        for tier in [
            "cursor-grok-4.6-high",
            "cursor-grok-4.6-high-fast",
            "cursor-grok-4.6-low",
            "cursor-grok-4.6-low-fast",
            "cursor-grok-4.6-medium",
            "cursor-grok-4.6-medium-fast",
            "cursor-grok-4.6-xhigh",
        ] {
            assert_eq!(resolve_alias(tier), Some("grok-4.6"), "tier: {tier}");
            assert!(uses_cursor_pricing(tier), "tier: {tier}");
        }
    }

    #[test]
    fn cursor_pricing_alias_keys_stay_disjoint_from_model_aliases() {
        // `resolve_alias` consults MODEL_ALIASES first, so a key present in
        // both maps would resolve through MODEL_ALIASES while
        // `uses_cursor_pricing` still forced the Cursor catalog for it.
        for key in super::CURSOR_PRICING_ALIASES.keys() {
            assert!(
                !super::MODEL_ALIASES.contains_key(key),
                "{key} is in both CURSOR_PRICING_ALIASES and MODEL_ALIASES"
            );
        }
    }

    #[test]
    fn qualified_kimi_code_ids_resolve_like_their_bare_form() {
        // A `kimi-code/<id>` row must not price differently from `<id>`.
        for bare in ["k3", "kimi-for-coding", "k2d6-agent", "k3-agent"] {
            let qualified = format!("kimi-code/{bare}");
            assert_eq!(
                resolve_alias(&qualified),
                resolve_alias(bare),
                "qualified id {qualified} must resolve like {bare}"
            );
            assert!(resolve_alias(&qualified).is_some());
        }
        // An unknown id stays unknown whether or not it carries the prefix.
        assert_eq!(resolve_alias("kimi-code/not-a-real-model"), None);
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
    fn resolves_openai_daybreak_blue_aliases_to_gpt_5_6_sol() {
        assert_eq!(resolve_alias("daybreak-blue-latest"), Some("gpt-5.6-sol"));
        assert_eq!(
            resolve_alias("gpt-daybreak-blue-latest"),
            Some("gpt-5.6-sol")
        );
        assert_eq!(
            resolve_alias("GPT-DAYBREAK-BLUE-LATEST"),
            Some("gpt-5.6-sol")
        );
        // Both qualified spellings, for the same reason the comment on the
        // table gives: prefix stripping does not re-run alias resolution, so
        // neither can fall back to its bare form.
        assert_eq!(
            resolve_alias("openai/daybreak-blue-latest"),
            Some("gpt-5.6-sol")
        );
        assert_eq!(
            resolve_alias("openai/gpt-daybreak-blue-latest"),
            Some("gpt-5.6-sol")
        );
    }

    #[test]
    fn resolves_stealth_preview_shorthands_to_canonical_upstream_keys() {
        assert_eq!(resolve_alias("ox-alpha"), Some("opencode-go/ox-alpha-free"));
        assert_eq!(resolve_alias("OX-ALPHA"), Some("opencode-go/ox-alpha-free"));
        // The qualified form must be explicit because provider-prefix
        // stripping does not run alias resolution a second time.
        assert_eq!(
            resolve_alias("stealth/ox-alpha"),
            Some("opencode-go/ox-alpha-free")
        );
        assert_eq!(
            resolve_alias("x-preview-f-free"),
            Some("opencode/x-preview-f-free")
        );
        assert_eq!(
            resolve_alias("X-PREVIEW-F-FREE"),
            Some("opencode/x-preview-f-free")
        );
    }

    #[test]
    fn stealth_preview_shorthands_use_the_upstream_zero_row() {
        fn zero_row() -> super::super::litellm::ModelPricing {
            super::super::litellm::ModelPricing {
                input_cost_per_token: Some(0.0),
                output_cost_per_token: Some(0.0),
                cache_read_input_token_cost: Some(0.0),
                ..Default::default()
            }
        }

        // Mirrors the live models.dev rows (both deprecated $0, no
        // cache-write bucket published).
        let service = super::super::PricingService::new_with_custom_and_models_dev(
            super::super::custom::CustomPricing::default(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([
                ("opencode-go/ox-alpha-free".to_string(), zero_row()),
                ("opencode/x-preview-f-free".to_string(), zero_row()),
            ]),
        );
        // Cache-write-bearing: the all-zero row covers every shape, so even
        // usage the upstream row's buckets cannot describe stays priced.
        let usage = crate::TokenBreakdown {
            input: 1_000_000,
            output: 100_000,
            cache_read: 50_000,
            cache_write: 10_000,
            reasoning: 5_000,
        };

        for (provider, model, expected_key) in [
            (
                Some("command-code"),
                "ox-alpha",
                "opencode-go/ox-alpha-free",
            ),
            (Some("stealth"), "ox-alpha", "opencode-go/ox-alpha-free"),
            (
                Some("nous"),
                "stealth/ox-alpha",
                "opencode-go/ox-alpha-free",
            ),
            (
                Some("hermes"),
                "stealth/ox-alpha",
                "opencode-go/ox-alpha-free",
            ),
            (
                Some("opencode-zen"),
                "x-preview-f-free",
                "opencode/x-preview-f-free",
            ),
            (
                Some("opencode_zen"),
                "x-preview-f-free",
                "opencode/x-preview-f-free",
            ),
        ] {
            let resolved = service
                .resolve_for_usage_with_provider(model, provider, &usage)
                .unwrap_or_else(|| {
                    panic!("{provider:?}/{model} must resolve the canonical zero row")
                });
            assert_eq!(resolved.source, "Models.dev", "id: {model}");
            assert_eq!(resolved.matched_key, expected_key, "id: {model}");
            assert!(resolved.evidence.alias_applied, "id: {model}");
            assert!(resolved.evidence.is_submission_safe(), "id: {model}");
            assert!(
                service.covers_usage_with_provider(model, provider, &usage),
                "id: {model}"
            );
            assert_eq!(
                service.calculate_cost_with_provider(model, provider, &usage),
                0.0,
                "id: {model}"
            );
        }
    }

    #[test]
    fn codex_daybreak_blue_usage_uses_the_underlying_openai_price() {
        let pricing = super::super::litellm::ModelPricing {
            input_cost_per_token: Some(5e-6),
            output_cost_per_token: Some(30e-6),
            cache_read_input_token_cost: Some(0.5e-6),
            cache_creation_input_token_cost: Some(6.25e-6),
            ..Default::default()
        };
        let service = super::super::PricingService::new(
            HashMap::from([("gpt-5.6-sol".to_string(), pricing)]),
            HashMap::new(),
        );
        let usage = crate::TokenBreakdown {
            input: 1_000,
            output: 100,
            cache_read: 500,
            cache_write: 200,
            reasoning: 0,
        };

        let expected = 1_000.0 * 5e-6 + 100.0 * 30e-6 + 500.0 * 0.5e-6 + 200.0 * 6.25e-6;
        for model_id in [
            "daybreak-blue-latest",
            "gpt-daybreak-blue-latest",
            "openai/daybreak-blue-latest",
            "openai/gpt-daybreak-blue-latest",
        ] {
            let result = service
                .lookup_with_source_and_provider(model_id, None, Some("openai"))
                .expect("the Codex alias must resolve to the GPT-5.6 Sol row");
            assert_eq!(result.source, "LiteLLM");
            assert_eq!(result.matched_key, "gpt-5.6-sol");
            assert!(result.evidence.alias_applied);
            assert!(service.covers_usage_with_provider(model_id, Some("openai"), &usage));

            let cost = service.calculate_cost_with_provider(model_id, Some("openai"), &usage);
            assert!(
                (cost - expected).abs() < 1e-12,
                "unexpected cost for {model_id}: {cost}"
            );
        }
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
