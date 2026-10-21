//! Evidence precedence and conflict detection.
//!
//! Resolution never scores confidence. It either proves one physical identity,
//! finds no usable evidence, or finds contradictory identities that must stay
//! unresolved. Adapter execution order cannot decide the winner: when several
//! adapters match the same call, agreement is required on the model, and only
//! the provider spelling may differ between them (gateways name the same
//! upstream differently).
//!
//! A target resolved only by a confirmed gateway selection is still weaker
//! evidence than one a completed response reported, so the former never
//! displaces the latter.

use crate::model_attribution::matcher::{EvidenceStrength, ResolvedTarget};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// No adapter established a physical identity.
    Unresolved,
    /// Exactly one identity is established by every adapter that resolved.
    Resolved(ResolvedTarget),
    /// Independent observations disagree about the served model.
    Conflict,
}

/// One adapter's resolved opinion about a call, with the adapter's position in
/// the gateway chain. A larger number is closer to the upstream provider, so a
/// downstream observation wins when two adapters agree on the model but spell
/// the provider differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Observation {
    pub(crate) precedence: u8,
    pub(crate) target: ResolvedTarget,
}

pub(crate) fn resolve(observations: Vec<Observation>) -> Resolution {
    let Some(first) = observations.first() else {
        return Resolution::Unresolved;
    };

    // Different model identities cannot both be the served model. Provider
    // spelling is not part of the identity check: two gateways may name the
    // same upstream differently while describing the same weights.
    if observations
        .iter()
        .any(|observation| observation.target.model != first.target.model)
    {
        return Resolution::Conflict;
    }

    let winner = observations
        .iter()
        .max_by(|left, right| {
            left.precedence
                .cmp(&right.precedence)
                .then_with(|| left.target.strength.cmp(&right.target.strength))
                // Final tie-break on a value both adapters supplied, so the
                // result never depends on which adapter ran first.
                .then_with(|| right.target.provider.cmp(&left.target.provider))
        })
        .expect("a non-empty observation list has a maximum");

    // A response-reported identity outranks a confirmed selection regardless
    // of which gateway produced it.
    let strongest = observations
        .iter()
        .map(|observation| observation.target.strength)
        .max()
        .unwrap_or(EvidenceStrength::ConfirmedSelection);
    let mut target = winner.target.clone();
    target.strength = strongest;
    Resolution::Resolved(target)
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

    fn observation(precedence: u8, provider: &str, model: &str) -> Observation {
        Observation {
            precedence,
            target: target(provider, model, EvidenceStrength::ResponseReported),
        }
    }

    #[test]
    fn no_observation_is_unresolved() {
        assert_eq!(resolve(Vec::new()), Resolution::Unresolved);
    }

    #[test]
    fn one_observation_resolves() {
        let resolution = resolve(vec![observation(1, "provider", "model")]);
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
    fn agreeing_gateways_reinforce_one_identity() {
        let resolution = resolve(vec![
            observation(1, "spelling-b", "model"),
            observation(2, "spelling-a", "model"),
        ]);
        assert_eq!(
            resolution,
            Resolution::Resolved(target(
                "spelling-a",
                "model",
                EvidenceStrength::ResponseReported
            ))
        );
    }

    #[test]
    fn disagreeing_served_models_are_a_conflict_in_either_order() {
        let left = observation(1, "provider-a", "model-a");
        let right = observation(1, "provider-b", "model-b");
        assert_eq!(
            resolve(vec![left.clone(), right.clone()]),
            Resolution::Conflict
        );
        assert_eq!(resolve(vec![right, left]), Resolution::Conflict);
    }

    #[test]
    fn response_reported_evidence_outranks_a_confirmed_selection() {
        let selected = Observation {
            precedence: 9,
            target: target("provider", "model", EvidenceStrength::ConfirmedSelection),
        };
        let reported = observation(0, "provider", "model");
        let Resolution::Resolved(target) = resolve(vec![selected, reported]) else {
            panic!("agreeing model identities must resolve");
        };
        assert_eq!(target.strength, EvidenceStrength::ResponseReported);
    }
}
