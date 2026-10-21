//! Cross-gateway attribution overlay for the streaming parse.
//!
//! Lanes flush incrementally, but one-to-one claim resolution must not depend
//! on which adapter or lane ran first: if two parsed rows claim the same
//! gateway record, neither may win. A message that gateway evidence could
//! claim is therefore parked instead of released, and the whole set is
//! resolved once, after every lane has produced its rows.
//!
//! Parking is bounded twice over. A message is parked only when a loaded
//! ledger actually contains its selector, native session id, or response id,
//! so an untouched scan parks nothing and the full history is never retained;
//! and the park list itself has a cap past which further messages are released
//! unattributed, which is the same fail-closed outcome as missing evidence.
//!
//! Messages are still the accounting authority. The overlay corrects identity
//! and re-derives pricing; it never creates, merges, or splits a row, so token
//! totals and dedup identities are untouched by construction.

use std::cell::RefCell;
use std::sync::Arc;

use crate::model_attribution::apply::{apply_target, mark_attribution_conflict};
use crate::model_attribution::diagnostics::{self, AttributionOutcome, Decision};
use crate::model_attribution::matcher::{Candidate, GatewayIndex, ResolvedTarget};
use crate::model_attribution::resolver::{resolve, Observation, Resolution};
use crate::model_attribution::{magpie, MAX_PARKED_MESSAGES};
use crate::pricing::PricingService;
use crate::sessions::opencodex;
use crate::{EstimateSource, UnifiedMessage};

/// Position of an adapter in the gateway chain. A row downstream of another
/// gateway sees the upstream gateway's reply, so a larger value is closer to
/// the provider. Provider/model disagreement is still a conflict at any
/// precedence.
const PRECEDENCE_OPENCODEX: u8 = 1;
const PRECEDENCE_MAGPIE: u8 = 2;

struct Parked {
    message: UnifiedMessage,
    estimate_source: Option<EstimateSource>,
}

pub(crate) struct GatewayOverlay {
    indexes: Vec<(u8, Arc<GatewayIndex>)>,
    parked: RefCell<Vec<Parked>>,
    /// Set when the park cap was reached. The scan then attributes nothing at
    /// all: attributing only the rows that happened to fit would make the
    /// result depend on lane order, which is exactly what the one-to-one rules
    /// exist to prevent.
    exhausted: std::cell::Cell<bool>,
}

impl GatewayOverlay {
    /// Load every supported gateway ledger for this machine. Returns `None`
    /// when none exists, so a scan without gateways pays nothing.
    pub(crate) fn load(home_dir: &str, use_env_roots: bool) -> Option<Self> {
        let mut indexes = Vec::new();
        if let Some(index) = opencodex::load_index(home_dir, use_env_roots) {
            if !index.is_empty() {
                indexes.push((PRECEDENCE_OPENCODEX, index));
            }
        }
        if let Some(index) = magpie::load_index(home_dir, use_env_roots) {
            if !index.is_empty() {
                indexes.push((PRECEDENCE_MAGPIE, index));
            }
        }
        (!indexes.is_empty()).then(|| Self {
            indexes,
            parked: RefCell::new(Vec::new()),
            exhausted: std::cell::Cell::new(false),
        })
    }

    /// Whether any ledger contains evidence that could claim this message.
    /// Deliberately a cheap membership test: the exact correlation still runs
    /// over the parked set, where one-to-one claims can be enforced.
    pub(crate) fn could_claim(&self, message: &UnifiedMessage) -> bool {
        self.indexes
            .iter()
            .any(|(_, index)| index.could_claim(message))
    }

    /// Park a message whose identity gateway evidence may change. Returns the
    /// message back when the park list is full, so the caller releases it
    /// unattributed rather than growing memory without bound.
    #[allow(clippy::result_large_err)] // the caller must get its row back intact
    pub(crate) fn park(
        &self,
        message: UnifiedMessage,
        estimate_source: Option<EstimateSource>,
    ) -> Result<(), Box<UnifiedMessage>> {
        let mut parked = self.parked.borrow_mut();
        if parked.len() >= MAX_PARKED_MESSAGES {
            self.exhausted.set(true);
            return Err(Box::new(message));
        }
        parked.push(Parked {
            message,
            estimate_source,
        });
        Ok(())
    }

    /// Resolve and release every parked message.
    ///
    /// `release` applies the ordinary parse tail; the overlay only decides
    /// identity and pricing first.
    pub(crate) fn finish(
        &self,
        pricing: Option<&PricingService>,
        mut release: impl FnMut(UnifiedMessage, Option<EstimateSource>),
    ) {
        let parked = self.parked.take();
        if parked.is_empty() {
            return;
        }
        if self.exhausted.get() {
            // Fail closed over the whole scan rather than over a prefix of it.
            for parked in parked {
                release(parked.message, parked.estimate_source);
            }
            return;
        }

        // Every feasible claim is collected before any row changes, exactly as
        // the single-adapter path did: a record claimed by two rows blocks
        // both, and a row with two candidate records stays unresolved.
        let candidates: Vec<Vec<(u8, Candidate)>> = parked
            .iter()
            .map(|parked| {
                self.indexes
                    .iter()
                    .flat_map(|(precedence, index)| {
                        index
                            .candidates(&parked.message)
                            .into_iter()
                            .map(move |candidate| (*precedence, candidate))
                    })
                    .collect()
            })
            .collect();

        let mut claims: std::collections::HashMap<(u8, usize), usize> =
            std::collections::HashMap::new();
        for matches in &candidates {
            for (precedence, candidate) in matches {
                *claims.entry((*precedence, candidate.record)).or_default() += 1;
            }
        }

        for (parked, matches) in parked.into_iter().zip(candidates) {
            let mut parked = parked;
            let requested_model = parked.message.model_id.clone();
            let requested_provider = parked.message.provider_id.clone();

            // A record claimed by more than one row is ambiguous evidence for
            // all of them, so it contributes no observation at all.
            let matched_by = matches
                .iter()
                .map(|(_, candidate)| candidate.tier)
                .min()
                .map(|tier| tier.label());
            let unambiguous: Vec<Observation> = matches
                .iter()
                .filter(|(precedence, candidate)| {
                    claims.get(&(*precedence, candidate.record)) == Some(&1)
                })
                .filter_map(|(precedence, candidate)| {
                    self.target(precedence, candidate)
                        .map(|target| Observation {
                            precedence: *precedence,
                            target,
                        })
                })
                .collect();
            // Two records inside one adapter are two readings of the same
            // ledger, which that source cannot disambiguate; that is not two
            // gateways agreeing, so it must not resolve by picking either.
            let distinct_adapters = unambiguous
                .iter()
                .map(|observation| observation.precedence)
                .collect::<std::collections::HashSet<_>>()
                .len();
            let observations = if distinct_adapters == unambiguous.len() {
                unambiguous
            } else {
                Vec::new()
            };

            match resolve(observations) {
                Resolution::Unresolved => {
                    if !matches.is_empty() {
                        diagnostics::record(
                            &parked.message,
                            Decision {
                                requested_model,
                                requested_provider,
                                matched_by,
                                outcome: AttributionOutcome::Unresolved,
                            },
                        );
                    }
                }
                Resolution::Conflict => {
                    mark_attribution_conflict(&mut parked.message);
                    diagnostics::record(
                        &parked.message,
                        Decision {
                            requested_model,
                            requested_provider,
                            matched_by,
                            outcome: AttributionOutcome::Conflicted,
                        },
                    );
                }
                Resolution::Resolved(target) => {
                    let changed = apply_target(&mut parked.message, &target, pricing);
                    if changed {
                        diagnostics::record(
                            &parked.message,
                            Decision {
                                requested_model,
                                requested_provider,
                                matched_by,
                                outcome: AttributionOutcome::Resolved {
                                    adapter: target.adapter,
                                    strength: target.strength,
                                    provider: target.provider.clone(),
                                    model: target.model.clone(),
                                },
                            },
                        );
                    }
                }
            }

            // Repricing records a fresh origin under the final identity. The
            // pre-attribution origin was drained when the message was parked,
            // so prefer the fresh one and fall back to the parked one only
            // when identity did not change.
            let final_source = pricing
                .and_then(|pricing| pricing.take_cost_estimate_source(&parked.message))
                .or(parked.estimate_source);
            release(parked.message, final_source);
        }
    }

    fn target(&self, precedence: &u8, candidate: &Candidate) -> Option<ResolvedTarget> {
        self.indexes
            .iter()
            .find(|(value, _)| value == precedence)
            .and_then(|(_, index)| index.record(candidate.record).target())
    }
}
