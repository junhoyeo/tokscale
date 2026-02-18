pub mod aliases;
pub mod cache;
pub mod litellm;
pub mod lookup;
pub mod openrouter;

use lookup::{PricingLookup, LookupResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::OnceCell;

pub use litellm::ModelPricing;

static PRICING_SERVICE: OnceCell<Arc<PricingService>> = OnceCell::const_new();

/// Provider prefixes in LiteLLM data that use subscription-based pricing ($0.00)
/// and should be excluded from pay-per-token cost estimation.
const EXCLUDED_LITELLM_PREFIXES: &[&str] = &[
    "github_copilot/",
];

pub struct PricingService {
    lookup: PricingLookup,
    /// Lowest-priority fallback pricing for models not yet in LiteLLM/OpenRouter.
    /// Only checked AFTER both upstream sources return no match, so real entries
    /// (including provider-prefixed ones like `openai/gpt-5.3-codex`) always win.
    fallback_overrides: HashMap<String, ModelPricing>,
}

impl PricingService {
    pub fn new(litellm_data: HashMap<String, ModelPricing>, openrouter_data: HashMap<String, ModelPricing>) -> Self {
        Self {
            lookup: PricingLookup::new(litellm_data, openrouter_data),
            fallback_overrides: Self::build_fallback_overrides(),
        }
    }

    /// Filter out LiteLLM entries from subscription-based providers (e.g. github_copilot/)
    /// whose $0.00 pricing is meaningless for per-token cost estimation.
    fn filter_litellm_data(mut data: HashMap<String, ModelPricing>) -> HashMap<String, ModelPricing> {
        data.retain(|key, _| {
            let lower = key.to_lowercase();
            !EXCLUDED_LITELLM_PREFIXES.iter().any(|prefix| lower.starts_with(prefix))
        });
        data
    }

    fn build_fallback_overrides() -> HashMap<String, ModelPricing> {
        let entries: &[(&str, f64, f64, Option<f64>)] = &[
            // GPT-5.3 family: $1.75/$14.00 per 1M tokens, $0.175 cache read
            // Source: Cursor docs (cursor.com/en-US/docs/models), llm-stats.com
            // Pattern: codex variants match base model pricing (gpt-5.2-codex = gpt-5.2)
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

    fn lookup_fallback(&self, model_id: &str) -> Option<LookupResult> {
        let lower = model_id.to_lowercase();
        self.fallback_overrides.get(&lower).map(|pricing| LookupResult {
            pricing: pricing.clone(),
            source: "Fallback".to_string(),
            matched_key: lower,
        })
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
            .or_else(|| self.lookup_fallback(model_id))
    }
    
    pub fn calculate_cost(&self, model_id: &str, input: i64, output: i64, cache_read: i64, cache_write: i64, reasoning: i64) -> f64 {
        let result = match self.lookup.lookup(model_id).or_else(|| self.lookup_fallback(model_id)) {
            Some(r) => r,
            None => return 0.0,
        };

        let p = &result.pricing;
        let safe_price =
            |opt: Option<f64>| opt.filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(0.0);

        let input_cost = input as f64 * safe_price(p.input_cost_per_token);
        let output_cost = (output + reasoning) as f64 * safe_price(p.output_cost_per_token);
        let cache_read_cost = cache_read as f64 * safe_price(p.cache_read_input_token_cost);
        let cache_write_cost = cache_write as f64 * safe_price(p.cache_creation_input_token_cost);

        input_cost + output_cost + cache_read_cost + cache_write_cost
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
    fn test_fallback_returns_pricing_when_not_in_upstream() {
        let service = PricingService::new(HashMap::new(), HashMap::new());

        let result = service.lookup_with_source("gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "Fallback");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.00000175));
        assert_eq!(result.pricing.output_cost_per_token, Some(0.000014));
        assert_eq!(result.pricing.cache_read_input_token_cost, Some(1.75e-7));
    }

    #[test]
    fn test_fallback_yields_to_litellm_entry() {
        let mut litellm = HashMap::new();
        litellm.insert("gpt-5.3-codex".into(), ModelPricing {
            input_cost_per_token: Some(0.002),
            output_cost_per_token: Some(0.016),
            cache_read_input_token_cost: None,
            cache_creation_input_token_cost: None,
        });

        let service = PricingService::new(litellm, HashMap::new());
        let result = service.lookup_with_source("gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "LiteLLM");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.002));
    }

    #[test]
    fn test_fallback_yields_to_openrouter_prefixed_entry() {
        let mut openrouter = HashMap::new();
        openrouter.insert("openai/gpt-5.3-codex".into(), ModelPricing {
            input_cost_per_token: Some(0.003),
            output_cost_per_token: Some(0.012),
            cache_read_input_token_cost: None,
            cache_creation_input_token_cost: None,
        });

        let service = PricingService::new(HashMap::new(), openrouter);
        let result = service.lookup_with_source("gpt-5.3-codex", None).unwrap();
        assert_eq!(result.source, "OpenRouter");
        assert_eq!(result.pricing.input_cost_per_token, Some(0.003));
    }

    #[test]
    fn test_fallback_calculate_cost() {
        let service = PricingService::new(HashMap::new(), HashMap::new());
        let cost = service.calculate_cost("gpt-5.3-codex", 1_000_000, 100_000, 0, 0, 0);
        let expected = 1_000_000.0 * 0.00000175 + 100_000.0 * 0.000014;
        assert!((cost - expected).abs() < 1e-10);
    }
}
