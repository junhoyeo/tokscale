pub mod aliases;
pub mod cache;
pub mod litellm;
pub mod lookup;
pub mod openrouter;

use lookup::{LookupResult, PricingLookup};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::OnceCell;

use crate::TokenBreakdown;

pub use litellm::ModelPricing;

static PRICING_SERVICE: OnceCell<Arc<PricingService>> = OnceCell::const_new();

// @keep: documents non-obvious filtering behavior — without this, the next person
// will wonder why github_copilot entries disappear from the pricing data.
/// Provider prefixes in LiteLLM data that use subscription-based pricing ($0.00)
/// and should be excluded from pay-per-token cost estimation.
const EXCLUDED_LITELLM_PREFIXES: &[&str] = &["github_copilot/"];

pub struct PricingService {
    lookup: PricingLookup,
}

impl PricingService {
    pub fn new(
        litellm_data: HashMap<String, ModelPricing>,
        openrouter_data: HashMap<String, ModelPricing>,
    ) -> Self {
        Self {
            lookup: PricingLookup::new(
                litellm_data,
                openrouter_data,
                Self::build_cursor_overrides(),
            ),
        }
    }

    // @keep: the retain logic is non-trivial (lowercase + prefix match); this doc
    // explains *why* these entries are dropped, not just *what* the code does.
    /// Filter out LiteLLM entries from subscription-based providers (e.g. github_copilot/)
    /// whose $0.00 pricing is meaningless for per-token cost estimation.
    fn filter_litellm_data(
        mut data: HashMap<String, ModelPricing>,
    ) -> HashMap<String, ModelPricing> {
        data.retain(|key, _| {
            let lower = key.to_lowercase();
            !EXCLUDED_LITELLM_PREFIXES
                .iter()
                .any(|prefix| lower.starts_with(prefix))
        });
        data
    }

    // @keep: Cursor-sourced pricing for models not yet in LiteLLM/OpenRouter.
    // Checked after exact/prefix matches but before fuzzy matching in PricingLookup,
    // so real upstream entries (including provider-prefixed like openai/gpt-5.3-codex)
    // always win. Source citations are required for audit trail.
    fn build_cursor_overrides() -> HashMap<String, ModelPricing> {
        let entries: &[(&str, f64, f64, Option<f64>)] = &[
            // GPT-5.3 family: $1.75/$14.00 per 1M tokens, $0.175 cache read
            // Source: Cursor docs (cursor.com/en-US/docs/models), llm-stats.com
            ("gpt-5.3", 0.00000175, 0.000014, Some(1.75e-7)),
            ("gpt-5.3-codex", 0.00000175, 0.000014, Some(1.75e-7)),
            ("gpt-5.3-codex-spark", 0.00000175, 0.000014, Some(1.75e-7)),
            // Composer 1: $1.25/$10.00 per 1M tokens, $0.125 cache read
            // Source: Cursor docs (cursor.com/docs/models#model-pricing)
            ("composer 1", 0.00000125, 0.00001, Some(1.25e-7)),
            ("composer-1", 0.00000125, 0.00001, Some(1.25e-7)),
            // Composer 1.5: $3.50/$17.50 per 1M tokens, $0.35 cache read
            // Source: Cursor docs (cursor.com/docs/models#model-pricing), issue #276
            ("composer 1.5", 0.0000035, 0.0000175, Some(3.5e-7)),
            ("composer-1.5", 0.0000035, 0.0000175, Some(3.5e-7)),
            // Composer 2: $0.50/$2.50 per 1M input/output, $0.20/M cache read; cache creation free
            // Composer 2 Fast: $1.50/$7.50 per 1M, $0.35/M cache read; cache creation free
            // Source: Cursor docs (cursor.com/docs/models#model-pricing)
            ("composer 2", 5e-7, 2.5e-6, Some(2e-7)),
            ("composer-2", 5e-7, 2.5e-6, Some(2e-7)),
            ("composer 2 fast", 1.5e-6, 7.5e-6, Some(3.5e-7)),
            ("composer-2-fast", 1.5e-6, 7.5e-6, Some(3.5e-7)),
        ];

        let mut overrides = HashMap::with_capacity(entries.len());
        for (model_id, input, output, cache_read) in entries {
            overrides.insert(
                model_id.to_string(),
                ModelPricing {
                    input_cost_per_token: Some(*input),
                    output_cost_per_token: Some(*output),
                    cache_read_input_token_cost: *cache_read,
                    cache_creation_input_token_cost: None,
                    ..Default::default()
                },
            );
        }
        overrides
    }

    async fn fetch_inner() -> Result<Self, String> {
        let (litellm_result, openrouter_data) =
            tokio::join!(litellm::fetch(), openrouter::fetch_all_mapped());

        let litellm_data = litellm_result.map_err(|e| e.to_string())?;
        let litellm_data = Self::filter_litellm_data(litellm_data);

        Ok(Self::new(litellm_data, openrouter_data))
    }

    fn from_cached_datasets(
        litellm_data: Option<HashMap<String, ModelPricing>>,
        openrouter_data: Option<HashMap<String, ModelPricing>>,
    ) -> Option<Self> {
        if litellm_data.is_none() && openrouter_data.is_none() {
            return None;
        }

        Some(Self::new(
            Self::filter_litellm_data(litellm_data.unwrap_or_default()),
            openrouter_data.unwrap_or_default(),
        ))
    }

    pub fn load_cached_any_age() -> Option<Self> {
        Self::from_cached_datasets(
            litellm::load_cached_any_age(),
            openrouter::load_cached_any_age(),
        )
    }

    pub async fn get_or_init() -> Result<Arc<PricingService>, String> {
        PRICING_SERVICE
            .get_or_try_init(|| async { Self::fetch_inner().await.map(Arc::new) })
            .await
            .map(Arc::clone)
    }

    pub fn lookup_with_source(
        &self,
        model_id: &str,
        force_source: Option<&str>,
    ) -> Option<LookupResult> {
        self.lookup.lookup_with_source(model_id, force_source)
    }

    pub fn lookup_with_source_and_provider(
        &self,
        model_id: &str,
        force_source: Option<&str>,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        self.lookup
            .lookup_with_source_and_provider(model_id, force_source, provider_id)
    }

    pub fn calculate_cost(
        &self,
        model_id: &str,
        input: i64,
        output: i64,
        cache_read: i64,
        cache_write: i64,
        reasoning: i64,
    ) -> f64 {
        let usage = TokenBreakdown {
            input,
            output,
            cache_read,
            cache_write,
            reasoning,
        };
        self.calculate_cost_with_provider(model_id, None, &usage)
    }

    pub fn calculate_cost_with_provider(
        &self,
        model_id: &str,
        provider_id: Option<&str>,
        usage: &TokenBreakdown,
    ) -> f64 {
        self.lookup
            .calculate_cost_with_provider(model_id, provider_id, usage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_excludes_github_copilot() {
        let mut data = HashMap::new();
        data.insert(
            "github_copilot/gpt-5.3-codex".into(),
            ModelPricing::default(),
        );
        data.insert("github_copilot/gpt-4o".into(), ModelPricing::default());
        data.insert(
            "gpt-5.2".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000175),
                ..Default::default()
            },
        );
        data.insert("openai/gpt-5.2".into(), ModelPricing::default());

        let filtered = PricingService::filter_litellm_data(data);
        assert!(!filtered.contains_key("github_copilot/gpt-5.3-codex"));
        assert!(!filtered.contains_key("github_copilot/gpt-4o"));
        assert!(filtered.contains_key("gpt-5.2"));
        assert!(filtered.contains_key("openai/gpt-5.2"));
    }

    #[test]
    fn test_cursor_returns_pricing_when_not_in_upstream() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.00000175));
        assert_eq!(result.pricing.output_cost_per_token, Some(0.000014));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(1.75e-7));
    }

    #[test]
    fn test_cursor_yields_to_litellm_exact() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-5.3-codex".into(),
            ModelPricing {
                input_cost_per_token: Some(0.002),
                output_cost_per_token: Some(0.016),
                ..Default::default()
            },
        );
        let service = PricingService::new(litellm, HashMap::new());
        let result = service.lookup_with_source("gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.002));
    }

    #[test]
    fn test_cursor_yields_to_openrouter_prefix() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "openai/gpt-5.3-codex".into(),
            ModelPricing {
                input_cost_per_token: Some(0.003),
                output_cost_per_token: Some(0.012),
                ..Default::default()
            },
        );
        let service = PricingService::new(HashMap::new(), openrouter);
        let result = service.lookup_with_source("gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "OpenRouter");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.003));
    }

    #[test]
    fn test_cursor_skipped_when_force_source_set() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        assert!(service
            .lookup_with_source("gpt-5.3-codex", Some("litellm"))
            .is_none());
        assert!(service
            .lookup_with_source("gpt-5.3-codex", Some("openrouter"))
            .is_none());
    }

    #[test]
    fn test_cursor_matches_after_version_normalization() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("gpt-5-3-codex", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "gpt-5.3-codex");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.00000175));
    }

    #[test]
    fn test_cursor_matches_provider_prefixed_input() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service
            .lookup_with_source("openai/gpt-5.3-codex", None)
            .unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "gpt-5.3-codex");
    }

    #[test]
    fn test_cursor_provider_prefix_yields_to_upstream() {
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "openai/gpt-5.3-codex".into(),
            ModelPricing {
                input_cost_per_token: Some(0.003),
                output_cost_per_token: Some(0.012),
                ..Default::default()
            },
        );
        let service = PricingService::new(HashMap::new(), openrouter);
        let result = service
            .lookup_with_source("openai/gpt-5.3-codex", None)
            .unwrap();
        assert_eq!(result.source, "OpenRouter");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.003));
    }

    #[test]
    fn test_cursor_matches_via_suffix_stripping() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service
            .lookup_with_source("gpt-5.3-codex-high", None)
            .unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "gpt-5.3-codex");
    }

    #[test]
    fn test_cursor_calculate_cost() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("gpt-5.3-codex", 1_000_000, 100_000, 0, 0, 0);
        let expected = 1_000_000.0 * 0.00000175 + 100_000.0 * 0.000014;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_1() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("Composer 1", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer 1");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.00000125));
        assert_eq!(result.pricing.output_cost_per_token, Some(0.00001));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(1.25e-7));
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_1() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("Composer 1", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 0.00000125 + 100_000.0 * 0.00001 + 50_000.0 * 1.25e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_returns_pricing_for_hyphenated_composer_1() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("composer-1", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-1");
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_1_5() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("Composer 1.5", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer 1.5");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.0000035));
        assert_eq!(result.pricing.output_cost_per_token, Some(0.0000175));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(3.5e-7));
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_1_5() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("Composer 1.5", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 0.0000035 + 100_000.0 * 0.0000175 + 50_000.0 * 3.5e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_returns_pricing_for_hyphenated_composer_1_5() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("composer-1.5", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-1.5");
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_2() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("composer-2", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-2");
        assert_eq!(result.pricing.input_cost_per_token, Some(5e-7));
        assert_eq!(result.pricing.output_cost_per_token, Some(2.5e-6));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(2e-7));
        assert_eq!(result.pricing.cache_creation_input_token_cost, None);
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_2_spaced() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("Composer 2", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer 2");
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_2_fast() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("composer-2-fast", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-2-fast");
        assert_eq!(result.pricing.input_cost_per_token, Some(1.5e-6));
        assert_eq!(result.pricing.output_cost_per_token, Some(7.5e-6));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(3.5e-7));
        assert_eq!(result.pricing.cache_creation_input_token_cost, None);
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_2_fast_spaced() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("Composer 2 Fast", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer 2 fast");
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_2() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("composer-2", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 5e-7 + 100_000.0 * 2.5e-6 + 50_000.0 * 2e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_composer_2_cache_write_free() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let with_write = service.calculate_cost("composer-2", 0, 0, 0, 500_000, 0);
        let without_write = service.calculate_cost("composer-2", 0, 0, 0, 0, 0);
        assert!((with_write - without_write).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_2_fast() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("composer-2-fast", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 1.5e-6 + 100_000.0 * 7.5e-6 + 50_000.0 * 3.5e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_composer_2_fast_cache_write_free() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let with_write = service.calculate_cost("composer-2-fast", 0, 0, 0, 500_000, 0);
        let without_write = service.calculate_cost("composer-2-fast", 0, 0, 0, 0, 0);
        assert!(
            (with_write - without_write).abs() < 1e-10,
            "Cache creation should be free for Composer 2 Fast"
        );
    }

    #[test]
    fn test_cursor_composer_lookup_case_insensitive() {
        let service = PricingService::new(HashMap::new(), HashMap::new());

        let lower = service.lookup_with_source("composer 1", None);
        let upper = service.lookup_with_source("COMPOSER 1", None);
        let mixed = service.lookup_with_source("Composer 1", None);

        assert!(lower.is_some(), "lowercase should resolve");
        assert!(upper.is_some(), "UPPERCASE should resolve");
        assert!(mixed.is_some(), "Mixed Case should resolve");

        assert_eq!(
            lower.unwrap().pricing.input_cost_per_token,
            upper.unwrap().pricing.input_cost_per_token
        );
    }

    #[test]
    fn test_from_cached_datasets_returns_none_when_both_sources_missing() {
        assert!(PricingService::from_cached_datasets(None, None).is_none());
    }

    #[test]
    fn test_from_cached_datasets_filters_subscription_only_litellm_entries() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "github_copilot/gpt-5.3-codex".into(),
            ModelPricing {
                input_cost_per_token: Some(0.0),
                ..Default::default()
            },
        );
        litellm.insert(
            "gpt-5.2".into(),
            ModelPricing {
                input_cost_per_token: Some(0.00000175),
                ..Default::default()
            },
        );

        let service = PricingService::from_cached_datasets(Some(litellm), None).unwrap();

        assert!(service
            .lookup_with_source("github_copilot/gpt-5.3-codex", Some("litellm"))
            .is_none());
        assert!(service
            .lookup_with_source("gpt-5.2", Some("litellm"))
            .is_some());
    }
}
