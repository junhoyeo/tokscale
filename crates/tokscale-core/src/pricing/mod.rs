pub mod aliases;
pub mod cache;
pub mod custom;
pub mod litellm;
pub mod lookup;
pub mod models_dev;
pub mod openrouter;

use custom::CustomPricing;
use lookup::{compute_cost, LookupResult, PricingLookup};
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
    custom: CustomPricing,
    lookup: PricingLookup,
}

impl PricingService {
    pub fn new(
        litellm_data: HashMap<String, ModelPricing>,
        openrouter_data: HashMap<String, ModelPricing>,
    ) -> Self {
        Self::new_with_custom(CustomPricing::default(), litellm_data, openrouter_data)
    }

    pub fn new_with_custom(
        custom: CustomPricing,
        litellm_data: HashMap<String, ModelPricing>,
        openrouter_data: HashMap<String, ModelPricing>,
    ) -> Self {
        Self::new_with_custom_and_models_dev(custom, litellm_data, openrouter_data, HashMap::new())
    }

    pub fn new_with_custom_and_models_dev(
        custom: CustomPricing,
        litellm_data: HashMap<String, ModelPricing>,
        openrouter_data: HashMap<String, ModelPricing>,
        models_dev_data: HashMap<String, ModelPricing>,
    ) -> Self {
        Self {
            custom,
            lookup: PricingLookup::new_with_models_dev(
                litellm_data,
                openrouter_data,
                Self::build_cursor_overrides(),
                models_dev_data,
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
            // Composer 2: $0.50/$2.50 per 1M input/output, $0.20/M cache read; cache creation free
            // Composer 2 Fast: $1.50/$7.50 per 1M, $0.35/M cache read; cache creation free
            // Source: Cursor docs (cursor.com/docs/models#model-pricing)
            ("composer-2.5", 5e-7, 2.5e-6, Some(2e-7)),
            ("composer-2.5-fast", 1.5e-6, 7.5e-6, Some(3.5e-7)),
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
        let (litellm_result, openrouter_data, models_dev_result) = tokio::join!(
            litellm::fetch(),
            openrouter::fetch_all_mapped(),
            models_dev::fetch()
        );

        let litellm_data = litellm_result.map_err(|e| e.to_string())?;
        let litellm_data = Self::filter_litellm_data(litellm_data);
        let models_dev_data = match models_dev_result {
            Ok(data) => data,
            Err(e) => {
                eprintln!("[tokscale] models.dev fetch failed: {}", e);
                HashMap::new()
            }
        };

        Ok(Self::new_with_custom_and_models_dev(
            CustomPricing::load_from_default_path(),
            litellm_data,
            openrouter_data,
            models_dev_data,
        ))
    }

    fn from_cached_datasets(
        litellm_data: Option<HashMap<String, ModelPricing>>,
        openrouter_data: Option<HashMap<String, ModelPricing>>,
        models_dev_data: Option<HashMap<String, ModelPricing>>,
    ) -> Option<Self> {
        if litellm_data.is_none() && openrouter_data.is_none() && models_dev_data.is_none() {
            return None;
        }

        Some(Self::new_with_custom_and_models_dev(
            CustomPricing::load_from_default_path(),
            Self::filter_litellm_data(litellm_data.unwrap_or_default()),
            openrouter_data.unwrap_or_default(),
            models_dev_data.unwrap_or_default(),
        ))
    }

    pub fn load_cached_any_age() -> Option<Self> {
        Self::from_cached_datasets(
            litellm::load_cached_any_age(),
            openrouter::load_cached_any_age(),
            models_dev::load_cached_any_age(),
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
        match force_source {
            Some(source) if source.eq_ignore_ascii_case("custom") => {
                return self.lookup_custom(model_id);
            }
            None => {
                if let Some(result) = self.lookup_custom(model_id) {
                    return Some(result);
                }
            }
            Some(_) => {}
        }

        self.lookup.lookup_with_source(model_id, force_source)
    }

    pub fn lookup_with_source_and_provider(
        &self,
        model_id: &str,
        force_source: Option<&str>,
        provider_id: Option<&str>,
    ) -> Option<LookupResult> {
        match force_source {
            Some(source) if source.eq_ignore_ascii_case("custom") => {
                return self.lookup_custom(model_id);
            }
            None => {
                if let Some(result) = self.lookup_custom(model_id) {
                    return Some(result);
                }
            }
            Some(_) => {}
        }

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
        if let Some(result) = self.custom.lookup_with_key(model_id) {
            return compute_cost(
                result.pricing,
                usage.input,
                usage.output,
                usage.cache_read,
                usage.cache_write,
                usage.reasoning,
            );
        }

        self.lookup
            .calculate_cost_with_provider(model_id, provider_id, usage)
    }

    fn lookup_custom(&self, model_id: &str) -> Option<LookupResult> {
        self.custom
            .lookup_with_key(model_id)
            .map(|result| LookupResult {
                pricing: result.pricing.clone(),
                source: "Custom".into(),
                matched_key: result.matched_key.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_pricing(input: f64, output: f64) -> ModelPricing {
        ModelPricing {
            input_cost_per_token: Some(input),
            output_cost_per_token: Some(output),
            ..Default::default()
        }
    }

    fn custom_service(
        custom: HashMap<String, ModelPricing>,
        litellm: HashMap<String, ModelPricing>,
        openrouter: HashMap<String, ModelPricing>,
    ) -> PricingService {
        PricingService::new_with_custom(CustomPricing::from_models(custom), litellm, openrouter)
    }

    fn fixture_models_dev() -> HashMap<String, ModelPricing> {
        models_dev::parse_dataset(include_str!("../../tests/fixtures/models_dev_pricing.json"))
            .unwrap()
    }

    fn custom_service_with_models_dev(
        custom: HashMap<String, ModelPricing>,
        litellm: HashMap<String, ModelPricing>,
        openrouter: HashMap<String, ModelPricing>,
        models_dev: HashMap<String, ModelPricing>,
    ) -> PricingService {
        PricingService::new_with_custom_and_models_dev(
            CustomPricing::from_models(custom),
            litellm,
            openrouter,
            models_dev,
        )
    }

    #[test]
    fn models_dev_parses_fixture_prices_per_token() {
        let data = fixture_models_dev();
        let pricing = data.get("openai/gpt-fixture-model").unwrap();

        assert_eq!(pricing.input_cost_per_token, Some(0.00000125));
        assert_eq!(pricing.output_cost_per_token, Some(0.00001));
        assert_eq!(pricing.cache_read_input_token_cost, Some(0.000000125));
        assert_eq!(pricing.cache_creation_input_token_cost, Some(0.000001875));
        assert!(!data.contains_key("openai/missing-output-price"));
    }

    #[test]
    fn models_dev_fills_provider_aware_fallback_prices() {
        let service = custom_service_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            fixture_models_dev(),
        );

        let result = service
            .lookup_with_source_and_provider("gpt-fixture-model", None, Some("openai"))
            .unwrap();

        assert_eq!(result.source, "Models.dev");
        assert_eq!(result.matched_key, "openai/gpt-fixture-model");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.00000125));
    }

    #[test]
    fn models_dev_cache_prices_are_used_for_cost_fallback() {
        let service = custom_service_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            fixture_models_dev(),
        );
        let usage = TokenBreakdown {
            input: 1_000_000,
            output: 100_000,
            cache_read: 50_000,
            cache_write: 20_000,
            reasoning: 0,
        };

        let cost =
            service.calculate_cost_with_provider("gpt-fixture-model", Some("openai"), &usage);

        let expected = 1.25 + 1.0 + 0.00625 + 0.0375;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn existing_sources_beat_models_dev_fallback() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "gpt-fixture-model".into(),
            model_pricing(0.000002, 0.000008),
        );
        let mut openrouter = HashMap::new();
        openrouter.insert(
            "anthropic/claude-fixture-sonnet".into(),
            model_pricing(0.000004, 0.000016),
        );

        let service = custom_service_with_models_dev(
            HashMap::new(),
            litellm,
            openrouter,
            fixture_models_dev(),
        );

        let litellm_result = service
            .lookup_with_source_and_provider("gpt-fixture-model", None, Some("openai"))
            .unwrap();
        assert_eq!(litellm_result.source, "LiteLLM");
        assert_eq!(litellm_result.pricing.input_cost_per_token, Some(0.000002));

        let openrouter_result = service
            .lookup_with_source_and_provider("claude-fixture-sonnet", None, Some("anthropic"))
            .unwrap();
        assert_eq!(openrouter_result.source, "OpenRouter");
        assert_eq!(
            openrouter_result.pricing.input_cost_per_token,
            Some(0.000004)
        );
    }

    #[test]
    fn models_dev_respects_forced_source_boundaries() {
        let service = custom_service_with_models_dev(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            fixture_models_dev(),
        );

        assert!(service
            .lookup_with_source_and_provider("gpt-fixture-model", Some("litellm"), Some("openai"))
            .is_none());
        assert!(service
            .lookup_with_source_and_provider(
                "gpt-fixture-model",
                Some("openrouter"),
                Some("openai")
            )
            .is_none());

        let result = service
            .lookup_with_source_and_provider(
                "gpt-fixture-model",
                Some("models.dev"),
                Some("openai"),
            )
            .unwrap();
        assert_eq!(result.source, "Models.dev");
    }

    #[test]
    fn custom_override_beats_models_dev_fallback() {
        let mut custom = HashMap::new();
        custom.insert(
            "gpt-fixture-model".into(),
            model_pricing(0.000009, 0.000018),
        );

        let service = custom_service_with_models_dev(
            custom,
            HashMap::new(),
            HashMap::new(),
            fixture_models_dev(),
        );

        let result = service
            .lookup_with_source_and_provider("gpt-fixture-model", None, Some("openai"))
            .unwrap();

        assert_eq!(result.source, "Custom");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.000009));
    }

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
    fn test_cursor_returns_pricing_for_composer_2_5() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("composer-2.5", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-2.5");
        assert_eq!(result.pricing.input_cost_per_token, Some(5e-7));
        assert_eq!(result.pricing.output_cost_per_token, Some(2.5e-6));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(2e-7));
        assert_eq!(result.pricing.cache_creation_input_token_cost, None);
    }

    #[test]
    fn test_cursor_returns_pricing_for_composer_2_5_fast() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service
            .lookup_with_source("composer-2.5-fast", None)
            .unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "composer-2.5-fast");
        assert_eq!(result.pricing.input_cost_per_token, Some(1.5e-6));
        assert_eq!(result.pricing.output_cost_per_token, Some(7.5e-6));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(3.5e-7));
        assert_eq!(result.pricing.cache_creation_input_token_cost, None);
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_2_5() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("composer-2.5", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 5e-7 + 100_000.0 * 2.5e-6 + 50_000.0 * 2e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_composer_2_5_cache_write_free() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let with_write = service.calculate_cost("composer-2.5", 0, 0, 0, 500_000, 0);
        let without_write = service.calculate_cost("composer-2.5", 0, 0, 0, 0, 0);
        assert!((with_write - without_write).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_for_composer_2_5_fast() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("composer-2.5-fast", 1_000_000, 100_000, 50_000, 0, 0);
        let expected = 1_000_000.0 * 1.5e-6 + 100_000.0 * 7.5e-6 + 50_000.0 * 3.5e-7;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn test_cursor_calculate_cost_composer_2_5_fast_cache_write_free() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let with_write = service.calculate_cost("composer-2.5-fast", 0, 0, 0, 500_000, 0);
        let without_write = service.calculate_cost("composer-2.5-fast", 0, 0, 0, 0, 0);
        assert!(
            (with_write - without_write).abs() < 1e-10,
            "Cache creation should be free for Composer 2.5 Fast"
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
        assert!(PricingService::from_cached_datasets(None, None, None).is_none());
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

        let service = PricingService::from_cached_datasets(Some(litellm), None, None).unwrap();

        assert!(service
            .lookup_with_source("github_copilot/gpt-5.3-codex", Some("litellm"))
            .is_none());
        assert!(service
            .lookup_with_source("gpt-5.2", Some("litellm"))
            .is_some());
    }

    #[test]
    fn test_from_cached_datasets_uses_models_dev_when_other_sources_missing() {
        let service =
            PricingService::from_cached_datasets(None, None, Some(fixture_models_dev())).unwrap();

        let result = service
            .lookup_with_source_and_provider("gpt-fixture-model", None, Some("openai"))
            .unwrap();

        assert_eq!(result.source, "Models.dev");
        assert_eq!(result.matched_key, "openai/gpt-fixture-model");
    }

    #[test]
    fn custom_override_wins_over_litellm() {
        let mut custom = HashMap::new();
        custom.insert("gpt-4o".into(), model_pricing(0.000002, 0.000008));
        let mut litellm = HashMap::new();
        litellm.insert("gpt-4o".into(), model_pricing(0.00001, 0.00003));

        let service = custom_service(custom, litellm, HashMap::new());
        let result = service.lookup_with_source("gpt-4o", None).unwrap();

        assert_eq!(result.source, "Custom");
        assert_eq!(result.matched_key, "gpt-4o");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.000002));
    }

    #[test]
    fn custom_override_wins_over_openrouter() {
        let mut custom = HashMap::new();
        custom.insert("grok-code".into(), model_pricing(0.000002, 0.000008));
        let mut openrouter = HashMap::new();
        openrouter.insert("x-ai/grok-code".into(), model_pricing(0.00001, 0.00003));

        let service = custom_service(custom, HashMap::new(), openrouter);
        let result = service.lookup_with_source("grok-code", None).unwrap();

        assert_eq!(result.source, "Custom");
        assert_eq!(result.matched_key, "grok-code");
        assert_eq!(result.pricing.output_cost_per_token, Some(0.000008));
    }

    #[test]
    fn custom_override_respects_force_source() {
        let mut custom = HashMap::new();
        custom.insert("gpt-4o".into(), model_pricing(0.000002, 0.000008));
        let mut litellm = HashMap::new();
        litellm.insert("gpt-4o".into(), model_pricing(0.00001, 0.00003));
        let mut openrouter = HashMap::new();
        openrouter.insert("openai/gpt-4o".into(), model_pricing(0.000003, 0.000012));

        let service = custom_service(custom, litellm, openrouter);

        let litellm_result = service
            .lookup_with_source("gpt-4o", Some("litellm"))
            .unwrap();
        assert_eq!(litellm_result.source, "LiteLLM");
        assert_eq!(litellm_result.pricing.input_cost_per_token, Some(0.00001));

        let openrouter_result = service
            .lookup_with_source("gpt-4o", Some("openrouter"))
            .unwrap();
        assert_eq!(openrouter_result.source, "OpenRouter");
        assert_eq!(
            openrouter_result.pricing.input_cost_per_token,
            Some(0.000003)
        );

        let custom_result = service
            .lookup_with_source("gpt-4o", Some("custom"))
            .unwrap();
        assert_eq!(custom_result.source, "Custom");
        assert_eq!(custom_result.pricing.input_cost_per_token, Some(0.000002));
    }

    #[test]
    fn custom_force_source_does_not_fall_through_on_miss() {
        let mut litellm = HashMap::new();
        litellm.insert("gpt-4o".into(), model_pricing(0.0000025, 0.00001));

        let service = custom_service(HashMap::new(), litellm, HashMap::new());

        assert!(service
            .lookup_with_source("gpt-4o", Some("custom"))
            .is_none());
    }

    #[test]
    fn custom_override_raw_match_wins() {
        let mut custom = HashMap::new();
        custom.insert(
            "accounts/fireworks/routers/kimi-k2p6-turbo".into(),
            model_pricing(0.000002, 0.000008),
        );
        let mut litellm = HashMap::new();
        litellm.insert("kimi-k2.6".into(), model_pricing(0.00000095, 0.000004));

        let service = custom_service(custom, litellm, HashMap::new());
        let result = service
            .lookup_with_source("accounts/fireworks/routers/kimi-k2p6-turbo", None)
            .unwrap();

        assert_eq!(result.source, "Custom");
        assert_eq!(
            result.matched_key,
            "accounts/fireworks/routers/kimi-k2p6-turbo"
        );
        assert_eq!(result.pricing.input_cost_per_token, Some(0.000002));
    }

    #[test]
    fn custom_override_normalized_match_wins() {
        let mut custom = HashMap::new();
        custom.insert("kimi-k2p6".into(), model_pricing(0.00000095, 0.000004));
        let mut litellm = HashMap::new();
        litellm.insert("gpt-4-turbo".into(), model_pricing(0.00001, 0.00003));

        let service = custom_service(custom, litellm, HashMap::new());
        let result = service
            .lookup_with_source("accounts/fireworks/models/kimi-k2p6", None)
            .unwrap();

        assert_eq!(result.source, "Custom");
        assert_eq!(result.matched_key, "kimi-k2p6");
        assert_eq!(result.pricing.output_cost_per_token, Some(0.000004));
    }

    #[test]
    fn custom_override_raw_beats_normalized() {
        let mut custom = HashMap::new();
        custom.insert("kimi-k2p6-turbo".into(), model_pricing(0.000001, 0.000004));
        custom.insert(
            "accounts/fireworks/models/kimi-k2p6-turbo".into(),
            model_pricing(0.000002, 0.000008),
        );

        let service = custom_service(custom, HashMap::new(), HashMap::new());
        let result = service
            .lookup_with_source("accounts/fireworks/models/kimi-k2p6-turbo", None)
            .unwrap();

        assert_eq!(
            result.matched_key,
            "accounts/fireworks/models/kimi-k2p6-turbo"
        );
        assert_eq!(result.pricing.input_cost_per_token, Some(0.000002));
    }

    #[test]
    fn custom_override_skips_fuzzy_chain() {
        let mut custom = HashMap::new();
        custom.insert("kimi-k2p6-turbo".into(), model_pricing(0.000002, 0.000008));

        let service = custom_service(custom, HashMap::new(), HashMap::new());

        assert!(service
            .lookup_with_source("my-kimi-k2p6-turbo", None)
            .is_none());
    }

    #[test]
    fn no_custom_falls_through_to_litellm() {
        let mut litellm = HashMap::new();
        litellm.insert("gpt-4o".into(), model_pricing(0.0000025, 0.00001));

        let service = custom_service(HashMap::new(), litellm, HashMap::new());
        let result = service.lookup_with_source("gpt-4o", None).unwrap();

        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.0000025));
    }

    #[test]
    fn custom_calculate_cost_uses_override() {
        let mut custom = HashMap::new();
        custom.insert(
            "accounts/fireworks/routers/kimi-k2p6-turbo".into(),
            model_pricing(0.000002, 0.000008),
        );
        let mut litellm = HashMap::new();
        litellm.insert(
            "accounts/fireworks/routers/kimi-k2p6-turbo".into(),
            model_pricing(0.00001, 0.00003),
        );

        let service = custom_service(custom, litellm, HashMap::new());
        let cost = service.calculate_cost(
            "accounts/fireworks/routers/kimi-k2p6-turbo",
            1_000_000,
            100_000,
            0,
            0,
            0,
        );

        let expected = 1_000_000.0 * 0.000002 + 100_000.0 * 0.000008;
        assert!((cost - expected).abs() < 1e-10);
    }
}
