//! Evidence precedence and conflict detection.
//!
//! Resolution never scores confidence and never reads an adapter's name as
//! proof of where it sits in a gateway chain. Nothing in a ledger establishes
//! that one adapter is upstream of another: an adapter records a call, not the
//! route that reached it. When several adapters observe one request, they must
//! therefore agree on the model *and* the provider that answered. A
//! disagreement is a conflict, not an invitation to prefer whichever adapter
//! happens to be closer to the provider.
//!
//! Evidence strength still orders observations by what they prove. A
//! response-reported identity proves the weights the vendor answered with; a
//! confirmed selection proves the member the gateway contacted, which a vendor
//! may still redirect. The stronger tier therefore supplies the identity — and
//! its provider — instead of inheriting one adapter's provider and another
//! adapter's strength.

use crate::model_attribution::matcher::{EvidenceStrength, ResolvedTarget};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// No adapter established a physical identity.
    Unresolved,
    /// Exactly one identity is established by every adapter that resolved.
    Resolved(ResolvedTarget),
    /// Independent observations disagree about the served identity.
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

    // Only the strongest tier may name the target. A weaker observation that
    // agrees on the model and provider reinforces it; both are already known
    // to agree, and one observation's fields are never combined with another's
    // strength.
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

    /// The provider must come from the observation whose strength is reported,
    /// never from a weaker one; otherwise the result would combine fields from
    /// two different claims.
    #[test]
    fn the_winning_tier_supplies_every_field() {
        let mut selected = Observation {
            target: target("provider", "model", EvidenceStrength::ConfirmedSelection),
        };
        selected.target.service_tier = Some("standard".into());
        let mut reported = observation("provider", "model");
        reported.target.service_tier = Some("priority".into());
        selected.target.adapter = "a";
        reported.target.adapter = "z";
        let Resolution::Resolved(target) = resolve(vec![selected, reported]) else {
            panic!("agreeing model identities must resolve");
        };
        assert_eq!(target.adapter, "z");
        assert_eq!(target.service_tier.as_deref(), Some("priority"));
    }
}
