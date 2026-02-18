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
}

impl PricingService {
    pub fn new(litellm_data: HashMap<String, ModelPricing>, openrouter_data: HashMap<String, ModelPricing>) -> Self {
        Self {
            lookup: PricingLookup::new(litellm_data, openrouter_data),
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

    /// Inject hardcoded pricing overrides for models not yet available on the API
    /// but with known pricing from first-party sources (e.g. Cursor docs).
    /// These are inserted with lower priority — they won't overwrite existing entries.
    fn inject_pricing_overrides(data: &mut HashMap<String, ModelPricing>) {
        let overrides: &[(&str, f64, f64, Option<f64>)] = &[
            // GPT-5.3 family: $1.75/$14.00 per 1M tokens, $0.175 cache read
            // Source: Cursor docs (cursor.com/en-US/docs/models), llm-stats.com
            // Pattern: codex variants match base model pricing (gpt-5.2-codex = gpt-5.2)
            ("gpt-5.3",             0.00000175, 0.000014, Some(1.75e-7)),
            ("gpt-5.3-codex",       0.00000175, 0.000014, Some(1.75e-7)),
            ("gpt-5.3-codex-spark", 0.00000175, 0.000014, Some(1.75e-7)),
        ];

        for (model_id, input, output, cache_read) in overrides {
            data.entry(model_id.to_string()).or_insert_with(|| ModelPricing {
                input_cost_per_token: Some(*input),
                output_cost_per_token: Some(*output),
                cache_read_input_token_cost: *cache_read,
                cache_creation_input_token_cost: None,
            });
        }
    }
    
    async fn fetch_inner() -> Result<Self, String> {
        let (litellm_result, openrouter_data) = tokio::join!(
            litellm::fetch(),
            openrouter::fetch_all_mapped()
        );
        
        let mut litellm_data = litellm_result.map_err(|e| e.to_string())?;
        litellm_data = Self::filter_litellm_data(litellm_data);
        Self::inject_pricing_overrides(&mut litellm_data);
        
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
    fn test_inject_overrides_adds_missing_models() {
        let mut data = HashMap::new();
        PricingService::inject_pricing_overrides(&mut data);

        assert!(data.contains_key("gpt-5.3"));
        assert!(data.contains_key("gpt-5.3-codex"));
        assert!(data.contains_key("gpt-5.3-codex-spark"));

        let pricing = &data["gpt-5.3-codex"];
        assert_eq!(pricing.input_cost_per_token, Some(0.00000175));
        assert_eq!(pricing.output_cost_per_token, Some(0.000014));
        assert_eq!(pricing.cache_read_input_token_cost, Some(1.75e-7));
    }

    #[test]
    fn test_inject_overrides_does_not_overwrite_existing() {
        let mut data = HashMap::new();
        data.insert("gpt-5.3-codex".into(), ModelPricing {
            input_cost_per_token: Some(0.002),
            output_cost_per_token: Some(0.016),
            cache_read_input_token_cost: None,
            cache_creation_input_token_cost: None,
        });

        PricingService::inject_pricing_overrides(&mut data);

        let pricing = &data["gpt-5.3-codex"];
        assert_eq!(pricing.input_cost_per_token, Some(0.002));
        assert_eq!(pricing.output_cost_per_token, Some(0.016));
    }
}
