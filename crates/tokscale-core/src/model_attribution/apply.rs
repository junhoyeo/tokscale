//! Safe application of a resolved identity to an existing usage row.
//!
//! The source row stays the accounting authority: token buckets, timestamps,
//! dedup identity, message counts, and parent/session linkage are never
//! touched. Only the model/provider/service-tier identity and the pricing that
//! depended on it are corrected, which is why attribution can never introduce
//! a second counted request.

use crate::model_attribution::matcher::ResolvedTarget;
use crate::pricing::PricingService;
use crate::sessions::{CostSource, EstimateStatus, UnifiedMessage};

/// Apply a resolved physical identity and reprice from scratch.
///
/// Returns true when the row changed. A provider-reported amount is
/// authoritative and survives untouched; an estimate is discarded rather than
/// carried onto a model it never priced, and the caller's pricing service
/// derives a new one from the resolved identity.
pub(crate) fn apply_target(
    message: &mut UnifiedMessage,
    target: &ResolvedTarget,
    pricing: Option<&PricingService>,
) -> bool {
    // A parser that read the provider's own response model has the strongest
    // evidence there is. A gateway selection describes what the gateway asked
    // for, which a router may still redirect, so it must never replace the
    // model the response reported.
    if message.model_evidence == crate::sessions::ModelEvidence::ResponseReported {
        return false;
    }
    let tier_changed = target
        .service_tier
        .as_ref()
        .is_some_and(|tier| message.service_tier.as_ref() != Some(tier));
    let identity_changed =
        message.provider_id != target.provider || message.model_id != target.model || tier_changed;
    if !identity_changed {
        return false;
    }

    message.provider_id = target.provider.clone();
    message.model_id = target.model.clone();
    if let Some(service_tier) = &target.service_tier {
        message.service_tier = Some(service_tier.clone());
    }
    // The pre-attribution raw model was the selector or the client's spelling
    // of it. Pricing must now resolve the model that actually served the call.
    message.pricing_model_id = None;

    if message.cost_source != CostSource::ProviderReported {
        message.cost = 0.0;
        message.cost_source = CostSource::Unknown;
        message.estimate_status = EstimateStatus::Unpriced;
        crate::apply_pricing_if_available(message, pricing);
    }

    true
}

/// Record contradictory authoritative evidence.
///
/// The row keeps the identity its source reported — that is still the only
/// thing the client proved — but it is marked so pricing does not treat a
/// router label or a contradicted spelling as a verified physical model. An
/// authoritative amount is left alone; a previous estimate is dropped because
/// it was computed for an identity that is now in doubt.
pub(crate) fn mark_attribution_conflict(message: &mut UnifiedMessage) {
    message.model_attribution_conflicted = true;
    if message.cost_source != CostSource::ProviderReported {
        message.cost = 0.0;
        message.cost_source = CostSource::Unknown;
        message.estimate_status = EstimateStatus::Unpriced;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_attribution::matcher::EvidenceStrength;
    use crate::TokenBreakdown;
    use std::collections::HashMap;

    fn message() -> UnifiedMessage {
        let mut message = UnifiedMessage::new(
            "codex",
            "combo/auto",
            "openai",
            "session",
            1_700_000_000_000,
            TokenBreakdown {
                input: 100,
                output: 20,
                cache_read: 400,
                ..Default::default()
            },
            7.0,
        );
        message.cost_source = CostSource::Estimated;
        message.estimate_status = EstimateStatus::CurrentEstimate;
        message.pricing_model_id = Some("combo/auto".into());
        message.dedup_key = Some("codex:usage-record:thread:resp-1".into());
        message
    }

    fn target(model: &str, provider: &str, strength: EvidenceStrength) -> ResolvedTarget {
        ResolvedTarget {
            provider: provider.into(),
            model: model.into(),
            service_tier: None,
            strength,
            adapter: "fixture",
        }
    }

    #[test]
    fn replaces_identity_and_recomputes_the_estimate() {
        let mut litellm = HashMap::new();
        litellm.insert(
            "served-model".to_string(),
            crate::pricing::ModelPricing {
                input_cost_per_token: Some(0.001),
                output_cost_per_token: Some(0.002),
                ..Default::default()
            },
        );
        let pricing = PricingService::new(litellm, HashMap::new());
        let mut message = message();

        assert!(apply_target(
            &mut message,
            &target(
                "served-model",
                "provider-a",
                EvidenceStrength::ResponseReported
            ),
            Some(&pricing),
        ));

        assert_eq!(message.model_id, "served-model");
        assert_eq!(message.provider_id, "provider-a");
        assert_eq!(message.pricing_model_id, None);
        // 100 input and 20 output; the fixture has no cache-read rate, so
        // those 400 tokens stay unpriced rather than billed at the input rate.
        assert!((message.cost - 0.14).abs() < 1e-9, "{}", message.cost);
        assert_eq!(message.cost_source, CostSource::Estimated);
    }

    #[test]
    fn never_touches_token_buckets_or_dedup_identity() {
        let mut message = message();
        let before = message.clone();

        apply_target(
            &mut message,
            &target(
                "served-model",
                "provider-a",
                EvidenceStrength::ResponseReported,
            ),
            None,
        );

        assert_eq!(message.tokens, before.tokens);
        assert_eq!(message.timestamp, before.timestamp);
        assert_eq!(message.dedup_key, before.dedup_key);
        assert_eq!(message.message_count, before.message_count);
        assert_eq!(message.session_id, before.session_id);
    }

    /// Native response evidence outranks any gateway observation, so the
    /// application step itself refuses to rewrite such a row even if a caller
    /// somehow hands it a resolved target.
    #[test]
    fn never_replaces_a_response_reported_identity() {
        let mut message = message();
        message.model_id = "native-model".into();
        message.model_evidence = crate::sessions::ModelEvidence::ResponseReported;
        let before = message.clone();

        assert!(!apply_target(
            &mut message,
            &target(
                "served-model",
                "provider-a",
                EvidenceStrength::ResponseReported
            ),
            None,
        ));

        assert_eq!(message.model_id, "native-model");
        assert_eq!(message.provider_id, before.provider_id);
        assert_eq!(message.cost, before.cost);
        assert_eq!(message.model_evidence, before.model_evidence);
    }

    #[test]
    fn keeps_a_provider_reported_amount() {
        let mut message = message();
        message.cost = 3.25;
        message.mark_provider_reported_cost();

        apply_target(
            &mut message,
            &target(
                "served-model",
                "provider-a",
                EvidenceStrength::ResponseReported,
            ),
            None,
        );

        assert_eq!(message.cost, 3.25);
        assert_eq!(message.cost_source, CostSource::ProviderReported);
        assert_eq!(message.estimate_status, EstimateStatus::Authoritative);
    }

    #[test]
    fn same_identity_is_not_a_change() {
        let mut message = message();
        message.model_id = "served-model".into();
        message.provider_id = "provider-a".into();
        message.pricing_model_id = None;

        assert!(!apply_target(
            &mut message,
            &target(
                "served-model",
                "provider-a",
                EvidenceStrength::ResponseReported
            ),
            None,
        ));
    }

    /// An explicit zero-rate rule is a priced row, not a missing one. The
    /// distinction has to survive re-attribution: after the model changes, the
    /// new identity is looked up again, and an explicit free rate must still
    /// read as covered rather than as absent pricing.
    #[test]
    fn an_explicit_free_rate_stays_distinct_from_missing_pricing() {
        let mut custom = HashMap::new();
        custom.insert(
            "served-model".to_string(),
            crate::pricing::ModelPricing {
                input_cost_per_token: Some(0.0),
                output_cost_per_token: Some(0.0),
                cache_read_input_token_cost: Some(0.0),
                cache_creation_input_token_cost: Some(0.0),
                ..Default::default()
            },
        );
        let pricing = PricingService::new_with_custom(
            crate::pricing::custom::CustomPricing::from_models(custom),
            HashMap::new(),
            HashMap::new(),
        );

        let mut free = message();
        assert!(apply_target(
            &mut free,
            &target(
                "served-model",
                "provider-a",
                EvidenceStrength::ResponseReported
            ),
            Some(&pricing),
        ));
        assert_eq!(free.cost, 0.0);
        assert!(
            free.estimate_status == EstimateStatus::CurrentEstimate,
            "an explicit zero rate is complete evidence, got {:?}",
            free.estimate_status
        );

        // The same row, attributed to a model with no rate at all, must not
        // inherit that completeness.
        let mut unknown = message();
        apply_target(
            &mut unknown,
            &target(
                "unpriced-model",
                "provider-a",
                EvidenceStrength::ResponseReported,
            ),
            Some(&pricing),
        );
        assert_eq!(unknown.cost, 0.0);
        assert_eq!(unknown.estimate_status, EstimateStatus::Unpriced);
    }

    #[test]
    fn conflict_drops_an_estimate_but_keeps_the_reported_amount() {
        let mut estimated = message();
        mark_attribution_conflict(&mut estimated);
        assert!(estimated.model_attribution_conflicted);
        assert_eq!(estimated.cost, 0.0);
        assert_eq!(estimated.cost_source, CostSource::Unknown);
        assert_eq!(estimated.estimate_status, EstimateStatus::Unpriced);
        assert_eq!(estimated.model_id, "combo/auto");

        let mut reported = message();
        reported.cost = 2.5;
        reported.mark_provider_reported_cost();
        mark_attribution_conflict(&mut reported);
        assert_eq!(reported.cost, 2.5);
        assert_eq!(reported.cost_source, CostSource::ProviderReported);
    }
}
