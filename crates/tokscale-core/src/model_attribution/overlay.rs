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
//!
//! Candidate competition is resolved over complete candidate sets, never over
//! a filtered remainder. For each adapter, every feasible record is computed
//! for every parked row first; a record claimed by more than one row is then
//! removed from *every* row's set. A row that still has two feasible records
//! from one adapter is ambiguous for that adapter, and a row whose only
//! candidate was removed has no evidence. Neither case may hand the request to
//! a surviving candidate, because the removed candidate was removed for being
//! contested, not for being wrong.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use crate::model_attribution::apply::{apply_target, mark_attribution_conflict};
use crate::model_attribution::diagnostics::{AttributionDiagnostics, AttributionOutcome, Decision};
use crate::model_attribution::matcher::{Candidate, GatewayIndex, MatchTier};
use crate::model_attribution::resolver::{self, Observation};
use crate::model_attribution::{magpie, park_cap};
use crate::pricing::PricingService;
use crate::sessions::opencodex;
use crate::{EstimateSource, ModelEvidence, UnifiedMessage};

struct Parked {
    message: UnifiedMessage,
    estimate_source: Option<EstimateSource>,
}

/// One loaded adapter ledger.
///
/// There is deliberately no chain position here. Nothing in a ledger proves
/// that one adapter is upstream of another, so adapter identity is used only
/// to keep two readings of the same ledger from counting as independent
/// observations.
struct AdapterIndex {
    adapter: &'static str,
    index: Arc<GatewayIndex>,
}

pub(crate) struct GatewayOverlay {
    indexes: Vec<AdapterIndex>,
    parked: RefCell<Vec<Parked>>,
    /// Set when the park cap was reached. The scan then attributes nothing at
    /// all: attributing only the rows that happened to fit would make the
    /// result depend on lane order, which is exactly what the one-to-one rules
    /// exist to prevent.
    exhausted: std::cell::Cell<bool>,
    /// Decisions for this scan only. See [`AttributionDiagnostics`].
    diagnostics: RefCell<AttributionDiagnostics>,
}

impl GatewayOverlay {
    /// Load every supported gateway ledger for this machine. Returns `None`
    /// when none exists, so a scan without gateways pays nothing.
    pub(crate) fn load(home_dir: &str, use_env_roots: bool) -> Option<Self> {
        let mut indexes = Vec::new();
        for (adapter, loaded) in [
            ("opencodex", opencodex::load_index(home_dir, use_env_roots)),
            ("magpie", magpie::load_index(home_dir, use_env_roots)),
        ] {
            if let Some(index) = loaded {
                if !index.is_empty() {
                    indexes.push(AdapterIndex { adapter, index });
                }
            }
        }
        (!indexes.is_empty()).then(|| Self {
            indexes,
            parked: RefCell::new(Vec::new()),
            exhausted: std::cell::Cell::new(false),
            diagnostics: RefCell::new(AttributionDiagnostics::default()),
        })
    }

    /// Whether any ledger contains evidence that could claim this message.
    ///
    /// A row a parser resolved from the provider's own response field already
    /// carries evidence stronger than any gateway selection, so it is never a
    /// candidate for re-identification. Otherwise this is a cheap membership
    /// test: exact correlation, and the one-to-one claim rules, still run over
    /// the parked set.
    pub(crate) fn could_claim(&self, message: &UnifiedMessage) -> bool {
        message.model_evidence == ModelEvidence::Requested
            && self
                .indexes
                .iter()
                .any(|adapter| adapter.index.could_claim(message))
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
        if parked.len() >= park_cap() {
            self.exhausted.set(true);
            return Err(Box::new(message));
        }
        parked.push(Parked {
            message,
            estimate_source,
        });
        Ok(())
    }

    /// Resolve and release every parked message, returning this scan's
    /// diagnostics.
    ///
    /// `release` applies the ordinary parse tail; the overlay only decides
    /// identity and pricing first.
    pub(crate) fn finish(
        &self,
        pricing: Option<&PricingService>,
        mut release: impl FnMut(UnifiedMessage, Option<EstimateSource>),
    ) -> AttributionDiagnostics {
        let parked = self.parked.take();
        if parked.is_empty() || self.exhausted.get() {
            // Fail closed over the whole scan rather than over a prefix of it.
            for parked in parked {
                release(parked.message, parked.estimate_source);
            }
            return self.diagnostics.borrow().clone();
        }

        // Every feasible claim is collected for every parked row before any
        // row changes. `candidates[i][q]` lists the records adapter `q` could
        // attribute to row `i`.
        let candidates: Vec<Vec<Vec<Candidate>>> = parked
            .iter()
            .map(|parked| {
                self.indexes
                    .iter()
                    .map(|adapter| adapter.index.candidates(&parked.message))
                    .collect()
            })
            .collect();

        // A record claimed by two rows is ambiguous evidence for both, so it
        // contributes no observation to either. The full candidate set stays
        // visible: removing a contested record never promotes its sibling.
        let mut claims: HashMap<(usize, usize), usize> = HashMap::new();
        for row in &candidates {
            for (adapter, matches) in row.iter().enumerate() {
                for candidate in matches {
                    *claims.entry((adapter, candidate.record)).or_default() += 1;
                }
            }
        }

        for (parked, per_adapter) in parked.into_iter().zip(candidates) {
            let released = self.resolve(parked, &per_adapter, &claims, pricing);
            release(released.message, released.estimate_source);
        }

        self.diagnostics.borrow().clone()
    }

    fn resolve(
        &self,
        parked: Parked,
        per_adapter: &[Vec<Candidate>],
        claims: &HashMap<(usize, usize), usize>,
        pricing: Option<&PricingService>,
    ) -> Released {
        let Parked {
            mut message,
            estimate_source,
        } = parked;
        let requested_model = message.model_id.clone();
        let requested_provider = message.provider_id.clone();

        // One adapter must establish exactly one uncontested record. Two
        // records from one ledger are two readings that source cannot
        // disambiguate; that is not two gateways agreeing, and it is not a
        // reason to pick one of them.
        let mut observations: Vec<Observation> = Vec::new();
        let mut matched_by: Option<MatchTier> = None;
        let mut ambiguous = false;
        let mut participating_adapters = Vec::new();
        for (adapter, matches) in per_adapter.iter().enumerate() {
            if matches.is_empty() {
                continue;
            }
            participating_adapters.push(self.indexes[adapter].adapter);
            let tier = matches[0].tier;
            matched_by = Some(matched_by.map_or(tier, |current| current.min(tier)));
            if matches.len() > 1 {
                ambiguous = true;
                continue;
            }
            let candidate = matches[0];
            if claims.get(&(adapter, candidate.record)) != Some(&1) {
                // Contested with another row: no usable observation here.
                continue;
            }
            if let Some(target) = self.indexes[adapter]
                .index
                .record(candidate.record)
                .target()
            {
                observations.push(Observation { target });
            }
        }
        participating_adapters.sort_unstable();
        participating_adapters.dedup();

        if ambiguous {
            observations.clear();
        }

        let Some(matched_by) = matched_by else {
            // The index matched only on a coarse membership key; no record
            // actually correlated with this row.
            return Released {
                message,
                estimate_source,
            };
        };

        match resolver::resolve(observations) {
            resolver::Resolution::Unresolved => {
                self.record(
                    &message,
                    Decision {
                        requested_model,
                        requested_provider,
                        matched_by: Some(matched_by.label()),
                        participating_adapters,
                        outcome: AttributionOutcome::Unresolved,
                    },
                );
                Released {
                    message,
                    estimate_source,
                }
            }
            resolver::Resolution::Conflict => {
                mark_attribution_conflict(&mut message);
                self.record(
                    &message,
                    Decision {
                        requested_model,
                        requested_provider,
                        matched_by: Some(matched_by.label()),
                        participating_adapters,
                        outcome: AttributionOutcome::Conflicted,
                    },
                );
                Released {
                    message,
                    // The old estimate priced an identity now in doubt.
                    estimate_source: None,
                }
            }
            resolver::Resolution::Resolved(target) => {
                let changed = apply_target(&mut message, &target, pricing);
                let final_source = if changed {
                    // Repricing records a fresh origin under the final
                    // identity. The previous origin describes the identity
                    // that was replaced, so it must not survive: carrying it
                    // would label a price computed for a different
                    // model/provider/tier.
                    self.record(
                        &message,
                        Decision {
                            requested_model,
                            requested_provider,
                            matched_by: Some(matched_by.label()),
                            participating_adapters,
                            outcome: AttributionOutcome::Resolved {
                                adapter: target.adapter,
                                strength: target.strength,
                                provider: target.provider.clone(),
                                model: target.model.clone(),
                            },
                        },
                    );
                    pricing.and_then(|pricing| pricing.take_cost_estimate_source(&message))
                } else {
                    estimate_source
                };
                Released {
                    message,
                    estimate_source: final_source,
                }
            }
        }
    }

    fn record(&self, message: &UnifiedMessage, decision: Decision) {
        self.diagnostics.borrow_mut().record(message, decision);
    }

    #[cfg(test)]
    pub(crate) fn set_park_cap_for_test(cap: Option<usize>) {
        crate::model_attribution::PARK_CAP_OVERRIDE.with(|value| value.set(cap));
    }

    #[cfg(test)]
    pub(crate) fn diagnostics(&self) -> AttributionDiagnostics {
        self.diagnostics.borrow().clone()
    }
}

struct Released {
    message: UnifiedMessage,
    estimate_source: Option<EstimateSource>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_attribution::matcher::{
        EvidenceStrength, GatewayRecord, MatchPolicy, TokenSignature,
    };
    use crate::pricing::custom::CustomPricing;
    use crate::pricing::{ModelPricing, PricingService};
    use crate::{CostSource, TokenBreakdown};
    use std::collections::HashMap;

    const START: i64 = 1_700_000_000_000;

    fn tokens() -> TokenBreakdown {
        TokenBreakdown {
            input: 100,
            output: 20,
            cache_read: 400,
            ..Default::default()
        }
    }

    fn record(selector: &str) -> GatewayRecord {
        let tokens = tokens();
        GatewayRecord {
            adapter: "fixture",
            selector: Some(selector.into()),
            selected_model: None,
            served_model: Some("served-model".into()),
            provider: Some("provider".into()),
            service_tier: None,
            session_ids: Vec::new(),
            stable_ids: Vec::new(),
            start_ms: START,
            duration_ms: Some(1_000),
            signatures: vec![TokenSignature::from_gateway(
                tokens.input,
                tokens.output + tokens.reasoning,
                tokens.cache_read,
                tokens.cache_write,
                tokens.reasoning,
            )],
            completed: true,
            target_eligible: true,
            allow_selected_fallback: false,
        }
    }

    fn named_record(adapter: &'static str, selector: &str, served: &str) -> GatewayRecord {
        let mut record = record(selector);
        record.adapter = adapter;
        record.served_model = Some(served.into());
        record
    }

    fn message(selector: &str) -> UnifiedMessage {
        let mut message = UnifiedMessage::new(
            "claude",
            selector,
            "anthropic",
            "session",
            START,
            tokens(),
            5.0,
        );
        message.duration_ms = Some(1_000);
        message.cost_source = CostSource::Estimated;
        message
    }

    fn overlay(records: Vec<GatewayRecord>) -> GatewayOverlay {
        overlay_of(vec![("fixture", records)])
    }

    fn overlay_of(adapters: Vec<(&'static str, Vec<GatewayRecord>)>) -> GatewayOverlay {
        GatewayOverlay {
            indexes: adapters
                .into_iter()
                .map(|(adapter, records)| AdapterIndex {
                    adapter,
                    index: Arc::new(GatewayIndex::new(MatchPolicy::opencodex(), records)),
                })
                .collect(),
            parked: RefCell::new(Vec::new()),
            exhausted: std::cell::Cell::new(false),
            diagnostics: RefCell::new(AttributionDiagnostics::default()),
        }
    }

    fn finish(overlay: &GatewayOverlay) -> Vec<UnifiedMessage> {
        finish_with_pricing(overlay, None)
    }

    fn finish_with_pricing(
        overlay: &GatewayOverlay,
        pricing: Option<&PricingService>,
    ) -> Vec<UnifiedMessage> {
        let mut released = Vec::new();
        overlay.finish(pricing, |message, _| released.push(message));
        released
    }

    fn decision(overlay: &GatewayOverlay, message: &UnifiedMessage) -> Decision {
        overlay
            .diagnostics()
            .lookup(message)
            .expect("the row should carry a recorded decision")
    }

    struct ParkCapReset;

    impl Drop for ParkCapReset {
        fn drop(&mut self) {
            GatewayOverlay::set_park_cap_for_test(None);
        }
    }

    #[test]
    fn a_claimed_row_is_reidentified_and_repriced() {
        let overlay = overlay(vec![record("selector")]);
        overlay.park(message("selector"), None).unwrap();

        let released = finish(&overlay);
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].model_id, "served-model");
        assert_eq!(released[0].provider_id, "provider");
        assert_eq!(released[0].cost, 0.0);
        assert_eq!(released[0].cost_source, CostSource::Unknown);
    }

    /// One adapter offering two feasible records cannot pick between them.
    /// Dropping the contested one and attributing from the survivor would turn
    /// an ambiguity into a fabricated match.
    #[test]
    fn one_adapter_with_two_feasible_records_is_ambiguous() {
        let overlay = overlay(vec![record("selector"), record("selector")]);
        overlay.park(message("selector"), None).unwrap();

        let released = finish(&overlay);
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].model_id, "selector");
        assert_eq!(released[0].cost, 5.0);
        assert_eq!(
            decision(&overlay, &released[0]).outcome,
            AttributionOutcome::Unresolved
        );
    }

    /// A -> {R1, R2}, B -> {R1}: R1 is contested, so it is removed from both
    /// rows' feasible sets. A is left with R2, but A started with two feasible
    /// records and so stays ambiguous; B loses its only candidate. Neither row
    /// may fall back to the record that remains.
    ///
    /// This is the exact shape the previous implementation got wrong: it
    /// filtered contested records out first and then resolved whatever
    /// survived, so A was attributed to R2 even though R2 was never the only
    /// possibility.
    #[test]
    fn a_contested_candidate_never_becomes_the_only_match() {
        // R1 matches both rows by native session id; R2 matches only A.
        let mut shared = record("selector");
        shared.session_ids = vec!["session-a".into(), "session-b".into()];
        let mut only_a = record("selector");
        only_a.session_ids = vec!["session-a".into()];
        only_a.served_model = Some("other-served-model".into());

        let overlay = overlay(vec![shared, only_a]);
        let mut first = message("selector");
        first.session_id = "session-a".into();
        let mut second = message("selector");
        second.session_id = "session-b".into();
        overlay.park(first, None).unwrap();
        overlay.park(second, None).unwrap();

        let released = finish(&overlay);
        assert_eq!(released.len(), 2);
        assert!(
            released
                .iter()
                .all(|message| message.model_id == "selector"),
            "no row may be attributed from a partial candidate set"
        );
        for message in &released {
            assert_eq!(
                decision(&overlay, message).outcome,
                AttributionOutcome::Unresolved
            );
        }
    }

    /// A row that only ever had one feasible record keeps failing closed when
    /// another row contests it, even if the competing row's other candidate is
    /// a different record.
    #[test]
    fn a_contested_only_candidate_leaves_no_usable_observation() {
        let overlay = overlay(vec![record("selector"), record("selector")]);
        let mut second = message("selector");
        second.session_id = "second-session".into();
        overlay.park(message("selector"), None).unwrap();
        overlay.park(second, None).unwrap();

        let released = finish(&overlay);
        assert!(released
            .iter()
            .all(|message| message.model_id == "selector"));
    }

    #[test]
    fn two_rows_claiming_one_record_both_stay_unattributed() {
        let overlay = overlay(vec![record("selector")]);
        overlay.park(message("selector"), None).unwrap();
        overlay.park(message("selector"), None).unwrap();

        let released = finish(&overlay);
        assert_eq!(released.len(), 2);
        assert!(released
            .iter()
            .all(|message| message.model_id == "selector"));
    }

    #[test]
    fn exceeding_the_park_cap_attributes_nothing_in_this_scan() {
        let _reset = ParkCapReset;
        GatewayOverlay::set_park_cap_for_test(Some(1));
        let overlay = overlay(vec![record("selector")]);

        overlay.park(message("selector"), None).unwrap();
        let returned = overlay.park(message("selector"), None);
        assert!(returned.is_err(), "the cap must return the row");
        let returned = *returned.unwrap_err();
        assert_eq!(returned.model_id, "selector");

        let released = finish(&overlay);
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].model_id, "selector", "no partial attribution");
    }

    #[test]
    fn a_row_no_ledger_can_claim_is_never_parked() {
        let overlay = overlay(vec![record("other-selector")]);
        assert!(!overlay.could_claim(&message("selector")));
        assert!(overlay.could_claim(&message("other-selector")));
    }

    /// Native response evidence outranks any gateway observation, so such a
    /// row is never even a parking candidate.
    #[test]
    fn a_response_reported_row_is_never_parked() {
        let overlay = overlay(vec![record("selector")]);
        let mut message = message("selector");
        message.model_evidence = ModelEvidence::ResponseReported;
        assert!(!overlay.could_claim(&message));
    }

    #[test]
    fn a_row_a_parser_already_marked_conflicted_is_never_reidentified() {
        let overlay = overlay(vec![record("selector")]);
        let mut message = message("selector");
        message.model_attribution_conflicted = true;
        message.cost = 1.0;
        overlay.park(message, None).unwrap();

        let released = finish(&overlay);
        assert_eq!(released[0].model_id, "selector");
        assert!(released[0].model_attribution_conflicted);
    }

    #[test]
    fn a_multi_call_aggregate_is_never_reidentified() {
        let overlay = overlay(vec![record("selector")]);
        let mut message = message("selector");
        message.message_count = 4;
        overlay.park(message, None).unwrap();

        let released = finish(&overlay);
        assert_eq!(released[0].model_id, "selector");
    }

    /// Both ledgers participate and agree. The result must not depend on the
    /// order the adapters were registered in.
    #[test]
    fn agreeing_adapters_resolve_identically_in_either_order() {
        let opencodex = vec![named_record("opencodex", "selector", "served-model")];
        let magpie = vec![named_record("magpie", "selector", "served-model")];

        for adapters in [
            vec![("opencodex", opencodex.clone()), ("magpie", magpie.clone())],
            vec![("magpie", magpie.clone()), ("opencodex", opencodex.clone())],
        ] {
            let overlay = overlay_of(adapters);
            overlay.park(message("selector"), None).unwrap();
            let released = finish(&overlay);

            assert_eq!(released[0].model_id, "served-model");
            assert_eq!(released[0].provider_id, "provider");
            let decision = decision(&overlay, &released[0]);
            assert_eq!(
                decision.participating_adapters,
                vec!["magpie", "opencodex"],
                "both adapters must contribute evidence"
            );
            assert_eq!(decision.outcome_label(), "resolved");
            assert_eq!(
                decision.resolved_target().unwrap().3,
                EvidenceStrength::ResponseReported
            );
        }
    }

    #[test]
    fn disagreeing_providers_are_a_conflict() {
        let mut other = named_record("magpie", "selector", "served-model");
        other.provider = Some("other-provider".into());
        let overlay = overlay_of(vec![
            (
                "opencodex",
                vec![named_record("opencodex", "selector", "served-model")],
            ),
            ("magpie", vec![other]),
        ]);
        overlay.park(message("selector"), None).unwrap();

        let released = finish(&overlay);
        assert_eq!(released[0].model_id, "selector");
        assert!(released[0].model_attribution_conflicted);
        assert_eq!(
            decision(&overlay, &released[0]).outcome,
            AttributionOutcome::Conflicted
        );
    }

    /// A gateway selection must never overwrite a model the client's own
    /// response field reported. The row keeps its identity and evidence tier.
    #[test]
    fn native_response_evidence_survives_a_gateway_selection() {
        let mut weaker = named_record("magpie", "selector", "gateway-guess");
        weaker.served_model = None;
        weaker.selected_model = Some("gateway-guess".into());
        weaker.allow_selected_fallback = true;
        let overlay = overlay_of(vec![("magpie", vec![weaker])]);

        let mut message = message("selector");
        message.model_id = "native-model".into();
        message.model_evidence = ModelEvidence::ResponseReported;
        message.cost = 0.0;
        overlay.park(message, None).unwrap();

        let released = finish(&overlay);
        assert_eq!(released[0].model_id, "native-model");
        assert_eq!(released[0].model_evidence, ModelEvidence::ResponseReported);
    }

    /// The resolved strength comes from the strongest observation. A
    /// response-reported record must supply its own tier, not inherit the
    /// weaker selection's identity.
    #[test]
    fn the_resolved_strength_comes_from_the_strongest_evidence() {
        let mut weaker = record("selector");
        weaker.served_model = None;
        weaker.selected_model = Some("served-model".into());
        weaker.allow_selected_fallback = true;
        let overlay = overlay_of(vec![
            ("opencodex", vec![weaker]),
            ("magpie", vec![record("selector")]),
        ]);
        overlay.park(message("selector"), None).unwrap();

        let released = finish(&overlay);
        assert_eq!(released[0].model_id, "served-model");
        assert_eq!(released[0].provider_id, "provider");
        let (_, _, _, strength) = decision(&overlay, &released[0])
            .resolved_target()
            .expect("the decision resolved");
        assert_eq!(
            strength,
            EvidenceStrength::ResponseReported,
            "the response-reported observation must supply the strength"
        );
    }

    /// A confirmed selection must not be able to smuggle a weaker provider or
    /// tier into a response-reported resolution.
    #[test]
    fn the_winning_tier_supplies_the_provider() {
        let mut selected = record("selector");
        selected.served_model = None;
        selected.selected_model = Some("served-model".into());
        selected.allow_selected_fallback = true;
        selected.provider = Some("selected-provider".into());
        let reported = record("selector");

        let overlay = overlay_of(vec![
            ("opencodex", vec![selected]),
            ("magpie", vec![reported]),
        ]);
        overlay.park(message("selector"), None).unwrap();

        let released = finish(&overlay);
        assert!(
            released[0].model_attribution_conflicted,
            "a weaker observation that names a different provider is a contradiction, never a silent field merge"
        );
        assert_eq!(
            decision(&overlay, &released[0]).outcome,
            AttributionOutcome::Conflicted
        );
    }

    /// Pricing provenance must not survive an identity change. The old origin
    /// described a price for a model this row no longer names.
    #[test]
    fn a_changed_identity_drops_the_previous_pricing_origin() {
        let overlay = overlay(vec![record("selector")]);
        overlay
            .park(message("selector"), Some(EstimateSource::Custom))
            .unwrap();

        let mut released = Vec::new();
        overlay.finish(None, |message, source| released.push((message, source)));
        assert_eq!(released[0].0.model_id, "served-model");
        assert_eq!(released[0].1, None);
    }

    #[test]
    fn an_unchanged_identity_keeps_the_previous_pricing_origin() {
        let overlay = overlay(vec![record("other-selector")]);
        overlay
            .park(message("selector"), Some(EstimateSource::Custom))
            .unwrap();

        // `could_claim` is a membership test, so parking is allowed here even
        // though correlation later finds nothing.
        let mut released = Vec::new();
        overlay.finish(None, |message, source| released.push((message, source)));
        assert_eq!(released[0].0.model_id, "selector");
        assert_eq!(released[0].1, Some(EstimateSource::Custom));
    }

    /// Repricing under a new identity records the new origin, never reusing
    /// the parked one.
    #[test]
    fn repricing_records_a_fresh_origin_for_the_new_identity() {
        let mut catalog = HashMap::new();
        catalog.insert(
            "served-model".to_string(),
            ModelPricing {
                input_cost_per_token: Some(1e-6),
                output_cost_per_token: Some(2e-6),
                ..Default::default()
            },
        );
        let pricing =
            PricingService::new_with_custom(CustomPricing::default(), catalog, HashMap::new())
                .with_cost_provenance_capture();

        let overlay = overlay(vec![record("selector")]);
        overlay
            .park(message("selector"), Some(EstimateSource::Custom))
            .unwrap();

        let mut released = Vec::new();
        overlay.finish(Some(&pricing), |message, source| {
            released.push((message, source))
        });
        assert_eq!(released[0].0.model_id, "served-model");
        assert!(released[0].0.cost > 0.0);
        assert_eq!(
            released[0].1,
            Some(EstimateSource::Catalog),
            "the origin must describe the price applied after attribution"
        );
    }

    /// A source estimate must never survive a conflict, since the identity it
    /// priced is now in doubt.
    #[test]
    fn a_conflict_drops_the_previous_pricing_origin() {
        let overlay = overlay(vec![record("selector")]);
        overlay
            .park(message("selector"), Some(EstimateSource::Catalog))
            .unwrap();
        // Replace the index with two contradictory records for one row.
        let overlay = GatewayOverlay {
            indexes: vec![AdapterIndex {
                adapter: "fixture",
                index: Arc::new(GatewayIndex::new(
                    MatchPolicy::opencodex(),
                    vec![
                        record("selector"),
                        named_record("fixture", "other-selector", "other-served-model"),
                    ],
                )),
            }],
            parked: overlay.parked,
            exhausted: std::cell::Cell::new(false),
            diagnostics: RefCell::new(AttributionDiagnostics::default()),
        };
        let mut second = message("other-selector");
        second.session_id = "second".into();
        overlay.park(second, None).unwrap();

        let mut released = Vec::new();
        overlay.finish(None, |message, source| released.push((message, source)));
        assert!(released
            .iter()
            .all(|(message, _)| !message.model_attribution_conflicted
                || message.cost_source != CostSource::Estimated));
    }
}
