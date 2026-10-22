//! Evidence precedence and conflict detection.
//!
//! Resolution never scores confidence and never reads an adapter's name as
//! proof of where it sits in a gateway chain. Nothing in a ledger establishes
//! that one adapter is upstream of another: an adapter records a call, not the
//! route that reached it. When several adapters observe one request, they must
//! therefore agree on the model, the provider that answered, and any
//! authoritative service tier they report. A disagreement is a conflict, not
//! an invitation to prefer whichever adapter happens to be closer to the
//! provider.
//!
//! Evidence strength still orders observations by what they prove. A
//! response-reported identity proves the weights the vendor answered with; a
//! confirmed selection proves the member the gateway contacted, which a vendor
//! may still redirect. The stronger tier therefore supplies the identity — and
//! its provider — instead of inheriting one adapter's provider and another
//! adapter's strength.
//!
//! The service tier is evidence about the same completed call, not a field of
//! the winning model claim. Every observation that reports an authoritative
//! tier must agree with every other one, regardless of which observation
//! supplied the model identity: a silent observation is not a competing claim,
//! but two different reported tiers describe two different tariffs for one
//! request. Choosing between them by adapter name would price the call at an
//! arbitrary rate, so a tier disagreement fails closed like a model or
//! provider disagreement.

use crate::model_attribution::matcher::{EvidenceStrength, ResolvedTarget};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// No adapter established a physical identity.
    Unresolved,
    /// Exactly one identity is established by every adapter that resolved.
    Resolved(ResolvedTarget),
    /// Independent observations disagree about the served identity or the
    /// authoritative service tier that prices it.
    Conflict,
}

/// One adapter's resolved opinion about a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Observation {
    pub(crate) target: ResolvedTarget,
}

pub(crate) fn resolve(observations: Vec<Observation>) -> Resolution {
    let Some(first) = observations.first() else {
        return Resolution::Unresolved;
    };

    // Different model identities cannot both be the served model.
    if observations
        .iter()
        .any(|observation| !same_model(&observation.target.model, &first.target.model))
    {
        return Resolution::Conflict;
    }

    // Provider spelling is part of the identity here. Without a verifiable
    // chain relation there is no evidence that one adapter's spelling is the
    // upstream of another's, so choosing between them would be a guess about
    // topology. This holds regardless of tier: a weaker observation that
    // disagrees about who answered is still a contradiction.
    if observations
        .iter()
        .any(|observation| observation.target.provider != first.target.provider)
    {
        return Resolution::Conflict;
    }

    // A tier is authoritative response evidence about this call, independent
    // of which observation established the model identity. Collect every
    // reported tier first: silence is not a competing claim, but two
    // different reported tiers describe two different tariffs for one
    // request. Sorting by adapter keeps the spelling deterministic without
    // letting registration order choose a price.
    let mut tier_claims: Vec<(&str, &str)> = observations
        .iter()
        .filter_map(|observation| {
            let tier = observation.target.service_tier.as_deref()?.trim();
            (!tier.is_empty()).then_some((observation.target.adapter, tier))
        })
        .collect();
    tier_claims.sort_unstable_by_key(|(adapter, _)| *adapter);
    let service_tier = match tier_claims.first() {
        None => None,
        Some((_, agreed)) => {
            if tier_claims
                .iter()
                .any(|(_, tier)| !tier.eq_ignore_ascii_case(agreed))
            {
                return Resolution::Conflict;
            }
            Some((*agreed).to_string())
        }
    };

    // Only the strongest tier may name the target. A weaker observation that
    // agrees on the model and provider reinforces it; one observation's model
    // identity is never combined with another's strength. The tier merged
    // above is a separate claim about the same call, not an identity field.
    let strongest = observations
        .iter()
        .map(|observation| observation.target.strength)
        .max()
        .unwrap_or(EvidenceStrength::ConfirmedSelection);
    let mut winning: Vec<&Observation> = observations
        .iter()
        .filter(|observation| observation.target.strength == strongest)
        .collect();

    // Deterministic reporting adapter once every identity field agrees, so
    // index registration order can never choose which adapter is named.
    winning.sort_unstable_by(|left, right| left.target.adapter.cmp(right.target.adapter));

    let mut target = winning[0].target.clone();
    target.strength = strongest;
    target.service_tier = service_tier;
    Resolution::Resolved(target)
}

/// Model identity comparison. Vendors are case-insensitive about model ids in
/// practice, and a case-only difference cannot describe different weights.
fn same_model(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(provider: &str, model: &str, strength: EvidenceStrength) -> ResolvedTarget {
        ResolvedTarget {
            provider: provider.into(),
            model: model.into(),
            service_tier: None,
            strength,
            adapter: "fixture",
        }
    }

    fn observation(provider: &str, model: &str) -> Observation {
        Observation {
            target: target(provider, model, EvidenceStrength::ResponseReported),
        }
    }

    #[test]
    fn no_observation_is_unresolved() {
        assert_eq!(resolve(Vec::new()), Resolution::Unresolved);
    }

    #[test]
    fn one_observation_resolves() {
        let resolution = resolve(vec![observation("provider", "model")]);
        assert_eq!(
            resolution,
            Resolution::Resolved(target(
                "provider",
                "model",
                EvidenceStrength::ResponseReported
            ))
        );
    }

    /// Unchanged from the shipped implementation: agreeing adapters reinforce
    /// one identity without reading either adapter's name as topology.
    #[test]
    fn agreeing_gateways_reinforce_one_identity() {
        let resolution = resolve(vec![
            observation("provider", "model"),
            observation("provider", "model"),
        ]);
        assert_eq!(
            resolution,
            Resolution::Resolved(target(
                "provider",
                "model",
                EvidenceStrength::ResponseReported
            ))
        );
    }

    #[test]
    fn disagreeing_served_models_are_a_conflict_in_either_order() {
        let left = observation("provider-a", "model-a");
        let right = observation("provider-b", "model-b");
        assert_eq!(
            resolve(vec![left.clone(), right.clone()]),
            Resolution::Conflict
        );
        assert_eq!(resolve(vec![right, left]), Resolution::Conflict);
    }

    /// A provider mismatch is not reconciled by evidence strength: neither
    /// adapter can prove it is upstream of the other.
    #[test]
    fn disagreeing_providers_are_a_conflict_in_either_order() {
        let left = observation("provider-a", "model");
        let right = observation("provider-b", "model");
        assert_eq!(
            resolve(vec![left.clone(), right.clone()]),
            Resolution::Conflict
        );
        assert_eq!(resolve(vec![right, left]), Resolution::Conflict);
    }

    #[test]
    fn disagreeing_providers_are_a_conflict_across_strengths() {
        let selected = Observation {
            target: target("provider-a", "model", EvidenceStrength::ConfirmedSelection),
        };
        let reported = observation("provider-b", "model");
        assert_eq!(
            resolve(vec![selected.clone(), reported.clone()]),
            Resolution::Conflict
        );
        assert_eq!(resolve(vec![reported, selected]), Resolution::Conflict);
    }

    #[test]
    fn response_reported_evidence_outranks_a_confirmed_selection() {
        let selected = Observation {
            target: target("provider", "model", EvidenceStrength::ConfirmedSelection),
        };
        let reported = observation("provider", "model");
        let Resolution::Resolved(target) = resolve(vec![selected, reported]) else {
            panic!("agreeing model identities must resolve");
        };
        assert_eq!(target.strength, EvidenceStrength::ResponseReported);
    }

    /// The identity fields come from the strongest observations; a weaker
    /// observation never lends its strength or its adapter name.
    #[test]
    fn the_winning_strength_supplies_the_identity() {
        let mut selected = Observation {
            target: target("provider", "model", EvidenceStrength::ConfirmedSelection),
        };
        selected.target.adapter = "a";
        let mut reported = observation("provider", "model");
        reported.target.adapter = "z";
        let Resolution::Resolved(target) = resolve(vec![selected, reported]) else {
            panic!("agreeing model identities must resolve");
        };
        assert_eq!(target.adapter, "z");
        assert_eq!(target.strength, EvidenceStrength::ResponseReported);
    }

    fn tiered(adapter: &'static str, tier: Option<&str>) -> Observation {
        Observation {
            target: ResolvedTarget {
                provider: "provider".into(),
                model: "model".into(),
                service_tier: tier.map(str::to_string),
                strength: EvidenceStrength::ResponseReported,
                adapter,
            },
        }
    }

    /// One gateway reporting the authoritative tier and the other staying
    /// silent is not a disagreement. The adapter whose name sorts first must
    /// not be able to discard the evidence the other one proved.
    #[test]
    fn a_tier_reported_by_one_agreeing_gateway_is_not_discarded() {
        let silent = tiered("magpie", None);
        let reported = tiered("opencodex", Some("priority"));
        for observations in [
            vec![silent.clone(), reported.clone()],
            vec![reported.clone(), silent.clone()],
        ] {
            let Resolution::Resolved(target) = resolve(observations) else {
                panic!("a silent observation is not a competing tier claim");
            };
            assert_eq!(target.service_tier.as_deref(), Some("priority"));
        }
    }

    /// The agreed tier also survives when the observation that reported it is
    /// not the one that established the model identity. The tier is a separate
    /// claim about the same call, not an identity field of the winner.
    #[test]
    fn a_tier_reported_only_by_the_weaker_observation_is_kept() {
        let mut silent = observation("provider", "model");
        silent.target.adapter = "opencodex";
        let mut reported = tiered("magpie", Some("priority"));
        reported.target.strength = EvidenceStrength::ConfirmedSelection;

        let Resolution::Resolved(target) = resolve(vec![silent, reported]) else {
            panic!("agreeing model identities must resolve");
        };
        assert_eq!(target.strength, EvidenceStrength::ResponseReported);
        assert_eq!(target.service_tier.as_deref(), Some("priority"));
    }

    #[test]
    fn agreeing_tiers_resolve_with_that_tier() {
        let left = tiered("magpie", Some("priority"));
        let right = tiered("opencodex", Some("priority"));
        let Resolution::Resolved(target) = resolve(vec![left, right]) else {
            panic!("identical tiers must resolve");
        };
        assert_eq!(target.service_tier.as_deref(), Some("priority"));
    }

    /// Two explicit served tiers describe two different tariffs for one call.
    /// Neither adapter can prove the other wrong, and picking one would price
    /// the row at a rate the evidence never established.
    #[test]
    fn disagreeing_service_tiers_are_a_conflict_in_either_order() {
        let default = tiered("magpie", Some("default"));
        let priority = tiered("opencodex", Some("priority"));
        assert_eq!(
            resolve(vec![default.clone(), priority.clone()]),
            Resolution::Conflict
        );
        assert_eq!(resolve(vec![priority, default]), Resolution::Conflict);
    }

    /// The contradiction is about the tier, not about the model-identity
    /// strength: both observations reported the served tier, so differing
    /// strengths cannot break the tie.
    #[test]
    fn disagreeing_service_tiers_are_a_conflict_across_strengths() {
        let mut default = tiered("magpie", Some("default"));
        default.target.strength = EvidenceStrength::ConfirmedSelection;
        let priority = tiered("opencodex", Some("priority"));
        assert_eq!(
            resolve(vec![default.clone(), priority.clone()]),
            Resolution::Conflict
        );
        assert_eq!(resolve(vec![priority, default]), Resolution::Conflict);
    }

    /// Tier spellings are compared the way pricing compares them. A case-only
    /// difference names one tier, and the reported spelling is chosen by
    /// adapter name rather than observation order.
    #[test]
    fn tier_agreement_ignores_case_and_picks_a_deterministic_spelling() {
        let magpie = tiered("magpie", Some("Priority"));
        let opencodex = tiered("opencodex", Some("priority"));
        for observations in [
            vec![magpie.clone(), opencodex.clone()],
            vec![opencodex.clone(), magpie.clone()],
        ] {
            let Resolution::Resolved(target) = resolve(observations) else {
                panic!("a case-only difference is one tier");
            };
            assert_eq!(target.service_tier.as_deref(), Some("Priority"));
        }
    }
}
