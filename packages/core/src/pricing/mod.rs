pub mod aliases;
pub mod cache;
pub mod litellm;
pub mod lookup;
pub mod openrouter;

use lookup::PricingLookup;
pub use lookup::LookupResult;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::OnceCell;

pub use litellm::ModelPricing;

static PRICING_SERVICE: OnceCell<Arc<PricingService>> = OnceCell::const_new();

// @keep: documents non-obvious filtering behavior — without this, the next person
// will wonder why github_copilot entries disappear from the pricing data.
/// Provider prefixes in LiteLLM data that use subscription-based pricing ($0.00)
/// and should be excluded from pay-per-token cost estimation.
const EXCLUDED_LITELLM_PREFIXES: &[&str] = &[
    "github_copilot/",
];

pub struct PricingService {
    lookup: PricingLookup,
}

impl PricingService {
    pub fn new(litellm_data: HashMap<String, ModelPricing>, openrouter_data: HashMap<String, ModelPricing>) -> Self {
        Self {
            lookup: PricingLookup::new(litellm_data, openrouter_data, Self::build_cursor_overrides()),
        }
    }

    // @keep: the retain logic is non-trivial (lowercase + prefix match); this doc
    // explains *why* these entries are dropped, not just *what* the code does.
    /// Filter out LiteLLM entries from subscription-based providers (e.g. github_copilot/)
    /// whose $0.00 pricing is meaningless for per-token cost estimation.
    fn filter_litellm_data(mut data: HashMap<String, ModelPricing>) -> HashMap<String, ModelPricing> {
        data.retain(|key, _| {
            let lower = key.to_lowercase();
            !EXCLUDED_LITELLM_PREFIXES.iter().any(|prefix| lower.starts_with(prefix))
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
            ("gpt-5.3",             0.00000175, 0.000014, Some(1.75e-7)),
            ("gpt-5.3-codex",       0.00000175, 0.000014, Some(1.75e-7)),
            ("gpt-5.3-codex-spark", 0.00000175, 0.000014, Some(1.75e-7)),
        ];

        let mut overrides = HashMap::with_capacity(entries.len());
        for (model_id, input, output, cache_read) in entries {
            overrides.insert(model_id.to_string(), ModelPricing {
                input_cost_per_token: Some(*input),
                output_cost_per_token: Some(*output),
                cache_read_input_token_cost: *cache_read,
                cache_creation_input_token_cost: None,
            });
        }
        overrides
    }
    
    async fn fetch_inner() -> Result<Self, String> {
        let (litellm_result, openrouter_data) = tokio::join!(
            litellm::fetch(),
            openrouter::fetch_all_mapped()
        );
        
        let litellm_data = litellm_result.map_err(|e| e.to_string())?;
        let litellm_data = Self::filter_litellm_data(litellm_data);
        
        Ok(Self::new(litellm_data, openrouter_data))
    }
    
    pub async fn get_or_init() -> Result<Arc<PricingService>, String> {
        PRICING_SERVICE.get_or_try_init(|| async {
            Self::fetch_inner().await.map(Arc::new)
        }).await.map(Arc::clone)
    }

    pub fn lookup_with_source(&self, model_id: &str, force_source: Option<&str>) -> Option<LookupResult> {
        self.lookup.lookup_with_source(model_id, force_source)
    }
    
    pub fn calculate_cost(&self, model_id: &str, input: i64, output: i64, cache_read: i64, cache_write: i64, reasoning: i64) -> f64 {
        self.lookup.calculate_cost(model_id, input, output, cache_read, cache_write, reasoning)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_excludes_github_copilot() {
        let mut data = HashMap::new();
        data.insert("github_copilot/gpt-5.3-codex".into(), ModelPricing::default());
        data.insert("github_copilot/gpt-4o".into(), ModelPricing::default());
        data.insert("gpt-5.2".into(), ModelPricing {
            input_cost_per_token: Some(0.00000175),
            ..Default::default()
        });
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
        litellm.insert("gpt-5.3-codex".into(), ModelPricing {
            input_cost_per_token: Some(0.002),
            output_cost_per_token: Some(0.016),
            ..Default::default()
        });
        let service = PricingService::new(litellm, HashMap::new());
        let result = service.lookup_with_source("gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.002));
    }

    #[test]
    fn test_cursor_yields_to_openrouter_prefix() {
        let mut openrouter = HashMap::new();
        openrouter.insert("openai/gpt-5.3-codex".into(), ModelPricing {
            input_cost_per_token: Some(0.003),
            output_cost_per_token: Some(0.012),
            ..Default::default()
        });
        let service = PricingService::new(HashMap::new(), openrouter);
        let result = service.lookup_with_source("gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "OpenRouter");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.003));
    }

    #[test]
    fn test_cursor_skipped_when_force_source_set() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        assert!(service.lookup_with_source("gpt-5.3-codex", Some("litellm")).is_none());
        assert!(service.lookup_with_source("gpt-5.3-codex", Some("openrouter")).is_none());
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
        let result = service.lookup_with_source("openai/gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "Cursor");
        assert_eq!(result.matched_key, "gpt-5.3-codex");
    }

    #[test]
    fn test_cursor_provider_prefix_yields_to_upstream() {
        let mut openrouter = HashMap::new();
        openrouter.insert("openai/gpt-5.3-codex".into(), ModelPricing {
            input_cost_per_token: Some(0.003),
            output_cost_per_token: Some(0.012),
            ..Default::default()
        });
        let service = PricingService::new(HashMap::new(), openrouter);
        let result = service.lookup_with_source("openai/gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "OpenRouter");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.003));
    }

    #[test]
    fn test_cursor_matches_via_suffix_stripping() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let result = service.lookup_with_source("gpt-5.3-codex-high", None).unwrap();
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
}
