use super::{
    lookup::{LookupResult, ResolutionEvidence, ResolutionKind},
    ModelPricing,
};
use crate::provider_identity;

pub fn lookup(model_id: &str, provider_id: Option<&str>) -> Option<LookupResult> {
    provider_id
        .filter(|provider| provider_identity::is_self_hosted_provider(provider))
        .map(|_| LookupResult {
            pricing: ModelPricing {
                input_cost_per_token: Some(0.0),
                output_cost_per_token: Some(0.0),
                cache_read_input_token_cost: Some(0.0),
                cache_creation_input_token_cost: Some(0.0),
                ..Default::default()
            },
            source: "Self-hosted".to_string(),
            matched_key: model_id.to_string(),
            evidence: ResolutionEvidence::deterministic(ResolutionKind::BuiltIn),
        })
}
