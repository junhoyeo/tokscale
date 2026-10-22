//! One-to-one request correlation between parsed client usage and gateway
//! ledgers.
//!
//! A gateway adapter decodes its own persisted format into [`GatewayRecord`]s;
//! nothing here knows how a record was stored. Correlation is deliberately
//! conservative:
//!
//! 1. a stable request/response identifier held by both sides;
//! 2. a native session identifier plus a verified token signature;
//! 3. the requested selector plus a verified token signature and timing.
//!
//! A timestamp is never sufficient on its own, an unverified token signature
//! cannot satisfy 2 or 3, and every record may attribute at most one request.

use std::collections::HashMap;

use crate::sessions::UnifiedMessage;
use crate::TokenBreakdown;

/// One gateway ledger may hold at most this many usable records. Beyond it the
/// oldest entries are dropped, so attribution fails closed rather than letting
/// a corrupt log consume unbounded memory.
pub(crate) const MAX_GATEWAY_RECORDS: usize = 1_000_000;

/// Timing tolerances a gateway adapter declares for its own ledger semantics.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MatchPolicy {
    pub(crate) max_start_skew_ms: i64,
    pub(crate) max_end_skew_ms: i64,
    pub(crate) max_response_duration_ms: i64,
}

impl MatchPolicy {
    /// A start-anchored request with a tight end anchor: the OpenCodex
    /// adapter's historical policy.
    pub(crate) const fn opencodex() -> Self {
        Self {
            max_start_skew_ms: 120_000,
            max_end_skew_ms: 2_000,
            max_response_duration_ms: 600_000,
        }
    }

    /// Magpie records the final attempt's own span, which excludes the gateway
    /// time and any failed tries that preceded it, so a client's recorded end
    /// can sit further past the row's end.
    pub(crate) const fn magpie() -> Self {
        Self {
            max_start_skew_ms: 120_000,
            max_end_skew_ms: 120_000,
            max_response_duration_ms: 600_000,
        }
    }
}

/// Token counters normalized to two additive totals plus the unambiguous
/// per-bucket counters.
///
/// Client tokens and gateway counters use different accounting conventions:
/// Tokscale's buckets are additive (the prompt total includes cache reads and
/// writes, the completion total includes reasoning), while gateways may report
/// an input total that already contains cache traffic and an output total that
/// already contains reasoning. Both constructors normalize into the same space
/// explicitly, so a comparison never adds a cache or reasoning bucket twice.
///
/// The one-hour cache-write split is a pricing dimension, not an accounting
/// difference: it never exceeds `cache_write`, so comparing that total already
/// binds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TokenSignature {
    pub(crate) prompt_total: i64,
    pub(crate) completion_total: i64,
    pub(crate) cache_read: i64,
    pub(crate) cache_write: i64,
    pub(crate) reasoning: i64,
}

impl TokenSignature {
    pub(crate) fn from_message(tokens: &TokenBreakdown) -> Self {
        Self {
            prompt_total: tokens
                .input
                .saturating_add(tokens.cache_read)
                .saturating_add(tokens.cache_write),
            completion_total: tokens.output.saturating_add(tokens.reasoning),
            cache_read: tokens.cache_read,
            cache_write: tokens.cache_write,
            reasoning: tokens.reasoning,
        }
    }

    /// Build a signature from gateway counters whose input excludes cache
    /// traffic and whose output already includes reasoning tokens.
    pub(crate) fn from_gateway(
        input: i64,
        output_including_reasoning: i64,
        cache_read: i64,
        cache_write: i64,
        reasoning: i64,
    ) -> Self {
        Self {
            prompt_total: input.saturating_add(cache_read).saturating_add(cache_write),
            completion_total: output_including_reasoning,
            cache_read,
            cache_write,
            reasoning,
        }
    }

    /// Build a signature from OpenAI-shaped counters whose input total already
    /// contains the cache read (so adding it again would double count) and
    /// whose output already includes reasoning.
    pub(crate) fn from_wire_totals(
        input_including_cache_read: i64,
        output_including_reasoning: i64,
        cache_read: i64,
        cache_write: i64,
        reasoning: i64,
    ) -> Self {
        Self {
            prompt_total: input_including_cache_read.saturating_add(cache_write),
            completion_total: output_including_reasoning,
            cache_read,
            cache_write,
            reasoning,
        }
    }

    /// Compare against a gateway signature.
    ///
    /// Totals and cache buckets must agree exactly. `reasoning` is compared
    /// only when this side reports a nonzero bucket: several parsers keep the
    /// whole completion count in `output` and leave `reasoning` at zero
    /// because their source does not separate the two, so a zero here means
    /// "not split", not "no reasoning tokens". The completion total already
    /// binds the sum, so an unsplit client cannot smuggle extra tokens past
    /// the comparison.
    fn matches(&self, other: &Self) -> bool {
        self.cache_read == other.cache_read
            && self.cache_write == other.cache_write
            && (self.reasoning == 0 || self.reasoning == other.reasoning)
            && self.prompt_total == other.prompt_total
            && self.completion_total == other.completion_total
    }
}

/// Which correlation rule established a match, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MatchTier {
    /// A stable request or response identifier held by both sides.
    Identifier,
    /// A native session identifier plus a verified token signature.
    Session,
    /// The requested selector plus a verified token signature and timing.
    Selector,
}

impl MatchTier {
    /// Local diagnostics label. Never part of a submitted payload.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Identifier => "identifier",
            Self::Session => "session",
            Self::Selector => "selector",
        }
    }
}

/// How strongly a record's own evidence establishes a physical model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EvidenceStrength {
    /// The selected upstream member, with no usable response-side model field.
    ConfirmedSelection,
    /// The model the completed provider response reported serving.
    ResponseReported,
}

/// One normalized gateway record: only the evidence the adapter can establish.
#[derive(Debug, Clone)]
pub(crate) struct GatewayRecord {
    /// Stable adapter name, exposed only in local diagnostics.
    pub(crate) adapter: &'static str,
    /// The model the client asked the gateway for.
    pub(crate) selector: Option<String>,
    /// The model the gateway chose to contact.
    pub(crate) selected_model: Option<String>,
    /// The model the upstream response reported serving.
    pub(crate) served_model: Option<String>,
    /// The provider that served the completed request.
    pub(crate) provider: Option<String>,
    /// Service tier the response reported, when it was authoritative.
    pub(crate) service_tier: Option<String>,
    /// Native client session identifiers the gateway recorded.
    pub(crate) session_ids: Vec<String>,
    /// Stable identifiers the record and the client can both name: the
    /// response id the gateway returned, and the vendor request id when the
    /// ledger kept one. A client parser exposes the same strings in its dedup
    /// key, so a match proves the two describe one call.
    pub(crate) stable_ids: Vec<String>,
    /// Request start, epoch milliseconds.
    pub(crate) start_ms: i64,
    /// Recorded request duration, when the source establishes one.
    pub(crate) duration_ms: Option<i64>,
    /// Token signatures of the successful, reported attempt(s). Empty means
    /// the record cannot verify a token match.
    pub(crate) signatures: Vec<TokenSignature>,
    /// True only for a completed, non-aborted request that reached an upstream.
    pub(crate) completed: bool,
    /// False when more than one successful attempt could claim the row, so the
    /// row may still reserve its request but cannot name a physical target.
    pub(crate) target_eligible: bool,
    /// Whether a confirmed selected model may stand in for a missing served
    /// model. Adapters whose contract requires a response-reported identity
    /// keep this false.
    pub(crate) allow_selected_fallback: bool,
}

/// A physical identity established by gateway evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTarget {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) service_tier: Option<String>,
    pub(crate) strength: EvidenceStrength,
    /// Adapter that produced this evidence, for local diagnostics only.
    pub(crate) adapter: &'static str,
}

impl GatewayRecord {
    /// The record's own physical target, if its evidence establishes one.
    pub(crate) fn target(&self) -> Option<ResolvedTarget> {
        if !self.completed || !self.target_eligible {
            return None;
        }
        let provider = non_empty(self.provider.as_deref())?;
        if let Some(served) = self.reported_served_model() {
            return Some(ResolvedTarget {
                provider: provider.to_string(),
                model: served.to_string(),
                service_tier: self.service_tier.clone(),
                strength: EvidenceStrength::ResponseReported,
                adapter: self.adapter,
            });
        }
        if !self.allow_selected_fallback {
            return None;
        }
        let selected = non_empty(self.selected_model.as_deref())?;
        // A gateway may report that it "selected" the same virtual label the
        // client asked for, a routing group, or a router marker such as
        // `provider/auto`. None of those names weights, so promoting one would
        // turn a router label into a confirmed physical identity. The record
        // still reserves its request: an unprovable selection is not evidence
        // for any other candidate.
        if self
            .selector
            .as_deref()
            .and_then(|selector| non_empty(Some(selector)))
            .is_some_and(|selector| selector.eq_ignore_ascii_case(selected))
            || is_virtual_model(selected)
        {
            return None;
        }
        Some(ResolvedTarget {
            provider: provider.to_string(),
            model: selected.to_string(),
            service_tier: self.service_tier.clone(),
            strength: EvidenceStrength::ConfirmedSelection,
            adapter: self.adapter,
        })
    }
}

impl GatewayRecord {
    /// The response-reported model, unless it names no physical model.
    ///
    /// A router can answer with the virtual selector it was asked for, or with
    /// another routing label, instead of the weights that produced the reply.
    /// Such a string is not response evidence; the record falls back to its
    /// confirmed selection instead. A served value equal to the *selected*
    /// model is real evidence: the gateway contacted exactly that model and
    /// the response confirmed it.
    fn reported_served_model(&self) -> Option<&str> {
        let served = non_empty(self.served_model.as_deref())?;
        let echoes_selector = self
            .selector
            .as_deref()
            .and_then(|selector| non_empty(Some(selector)))
            .is_some_and(|selector| selector.eq_ignore_ascii_case(served));
        (!echoes_selector && !is_virtual_model(served)).then_some(served)
    }
}

/// Whether a model string names a router rather than weights.
///
/// This is field evidence, not a guess about how specific a name looks: the
/// shared routing-label predicate is the same one pricing refuses, and a
/// `<router>/auto` spelling is that same label as the model segment. The
/// caller additionally rejects a value equal to the requested selector.
pub(crate) fn is_virtual_model(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || crate::pricing::lookup::is_routing_label(value) {
        return true;
    }
    value
        .rsplit_once('/')
        .is_some_and(|(_, model)| crate::pricing::lookup::is_routing_label(model))
}

pub(crate) fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// A candidate record for one message, tagged with the rule that matched it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Candidate {
    pub(crate) record: usize,
    pub(crate) tier: MatchTier,
}

/// Indexed gateway records, queried one message at a time.
#[derive(Debug)]
pub(crate) struct GatewayIndex {
    policy: MatchPolicy,
    records: Vec<GatewayRecord>,
    by_selector: HashMap<String, Vec<usize>>,
    by_session: HashMap<String, Vec<usize>>,
    by_id: HashMap<String, Vec<usize>>,
}

impl GatewayIndex {
    pub(crate) fn new(policy: MatchPolicy, mut records: Vec<GatewayRecord>) -> Self {
        if records.len() > MAX_GATEWAY_RECORDS {
            records.drain(..records.len() - MAX_GATEWAY_RECORDS);
        }
        let mut index = Self {
            policy,
            records,
            by_selector: HashMap::new(),
            by_session: HashMap::new(),
            by_id: HashMap::new(),
        };
        index.build();
        index
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.records.len()
    }

    fn build(&mut self) {
        for (slot, record) in self.records.iter().enumerate() {
            if let Some(selector) = non_empty(record.selector.as_deref()) {
                self.by_selector
                    .entry(selector.to_string())
                    .or_default()
                    .push(slot);
            }
            for session in &record.session_ids {
                if !session.is_empty() {
                    self.by_session
                        .entry(session.clone())
                        .or_default()
                        .push(slot);
                }
            }
            for id in &record.stable_ids {
                if !id.is_empty() {
                    self.by_id.entry(id.clone()).or_default().push(slot);
                }
            }
        }
        // Every index is consumed in time order, so a bounded window is a
        // binary search instead of a scan.
        let records = &self.records;
        for slots in self
            .by_selector
            .values_mut()
            .chain(self.by_session.values_mut())
            .chain(self.by_id.values_mut())
        {
            slots.sort_unstable_by_key(|slot| records[*slot].start_ms);
            slots.dedup();
        }
    }

    pub(crate) fn record(&self, slot: usize) -> &GatewayRecord {
        &self.records[slot]
    }

    /// Whether any record could conceivably claim this message. Callers use
    /// this to avoid retaining messages that have no gateway evidence.
    pub(crate) fn could_claim(&self, message: &UnifiedMessage) -> bool {
        if self.is_empty() || message.timestamp <= 0 || message.message_count != 1 {
            return false;
        }
        // A parser that already observed contradictory authoritative evidence
        // resolved the row to "unknown" on purpose; a gateway log cannot
        // override that, and matching its placeholder name would be a guess.
        if message.model_attribution_conflicted {
            return false;
        }
        // A row whose parser read the provider's own response model already
        // carries the strongest evidence there is. A gateway selection may
        // confirm it, but the overlay never needs to park it.
        if message.model_evidence == crate::sessions::ModelEvidence::ResponseReported {
            return false;
        }
        if self.by_selector.contains_key(&message.model_id) {
            return true;
        }
        if !message.session_id.is_empty() && self.by_session.contains_key(&message.session_id) {
            return true;
        }
        response_ids(&message.dedup_key).any(|id| self.by_id.contains_key(id))
    }

    /// Candidate records for one message, using only the strongest rule that
    /// produced any match. A weaker rule never widens a stronger one.
    pub(crate) fn candidates(&self, message: &UnifiedMessage) -> Vec<Candidate> {
        if self.is_empty() || message.timestamp <= 0 {
            return Vec::new();
        }
        // A row that sums more than one request (a turn aggregate, or a
        // parser's combined counters) cannot be re-identified by one gateway
        // record: there is no per-model decomposition to apply, and choosing
        // the record's model would attribute every call in the turn to it.
        // Zero marks a supplemental row that is not counted as a call.
        if message.message_count != 1
            || message.model_attribution_conflicted
            || message.model_evidence == crate::sessions::ModelEvidence::ResponseReported
        {
            return Vec::new();
        }

        let mut identifier = Vec::new();
        for id in response_ids(&message.dedup_key) {
            if let Some(slots) = self.by_id.get(id) {
                self.collect(
                    slots,
                    message,
                    false,
                    MatchTier::Identifier,
                    &mut identifier,
                );
            }
        }
        if !identifier.is_empty() {
            return identifier;
        }

        let mut session = Vec::new();
        if !message.session_id.is_empty() {
            if let Some(slots) = self.by_session.get(&message.session_id) {
                self.collect(slots, message, true, MatchTier::Session, &mut session);
            }
        }
        if !session.is_empty() {
            return session;
        }

        let mut selector = Vec::new();
        if let Some(slots) = self.by_selector.get(&message.model_id) {
            self.collect(slots, message, true, MatchTier::Selector, &mut selector);
        }
        selector
    }

    fn collect(
        &self,
        slots: &[usize],
        message: &UnifiedMessage,
        require_tokens: bool,
        tier: MatchTier,
        out: &mut Vec<Candidate>,
    ) {
        for slot in self.window(slots, message) {
            if self.record_matches(slot, message, require_tokens) {
                out.push(Candidate { record: slot, tier });
            }
        }
        out.sort_unstable_by_key(|candidate| candidate.record);
        out.dedup_by_key(|candidate| candidate.record);
    }

    /// Slots whose recorded start can still reach this message. The window is
    /// the request-start skew plus any response span, bounded in both
    /// directions so a start-less row is anchored on its response end.
    fn window(&self, slots: &[usize], message: &UnifiedMessage) -> Vec<usize> {
        let span = message
            .duration_ms
            .filter(|duration| *duration >= 0)
            .unwrap_or(self.policy.max_response_duration_ms);
        let lower = message
            .timestamp
            .saturating_sub(self.policy.max_start_skew_ms)
            .saturating_sub(self.policy.max_response_duration_ms);
        let upper = message
            .timestamp
            .saturating_add(span)
            .saturating_add(self.policy.max_end_skew_ms);
        let begin = slots.partition_point(|slot| self.records[*slot].start_ms < lower);
        let end = slots.partition_point(|slot| self.records[*slot].start_ms <= upper);
        slots[begin..end].to_vec()
    }

    fn record_matches(&self, slot: usize, message: &UnifiedMessage, require_tokens: bool) -> bool {
        let record = &self.records[slot];
        if !record.completed {
            return false;
        }
        if !timing_matches(message, record, self.policy) {
            return false;
        }
        if record.signatures.is_empty() {
            // An identifier match establishes identity on its own; a weaker
            // rule cannot claim a token match it never verified.
            return !require_tokens;
        }
        let signature = TokenSignature::from_message(&message.tokens);
        record
            .signatures
            .iter()
            .any(|candidate| candidate.matches(&signature))
    }
}

fn timing_matches(message: &UnifiedMessage, record: &GatewayRecord, policy: MatchPolicy) -> bool {
    if message.timestamp <= 0 {
        return false;
    }
    let record_end = record
        .start_ms
        .saturating_add(record.duration_ms.unwrap_or(0).max(0));
    match message.duration_ms.filter(|duration| *duration >= 0) {
        Some(duration) => {
            if message.timestamp.abs_diff(record.start_ms) > policy.max_start_skew_ms as u64 {
                return false;
            }
            let message_end = message.timestamp.saturating_add(duration);
            message_end.abs_diff(record_end) <= policy.max_end_skew_ms as u64
        }
        // An unconfirmed usage record is stamped when the response lands, so
        // its clock anchors on the request's end rather than its start.
        None => message.timestamp.abs_diff(record_end) <= policy.max_end_skew_ms as u64,
    }
}

/// Response and request identifiers a client parser retained inside its dedup
/// key.
///
/// Parsers that key a row by the provider's own response id expose it here.
/// The returned values are raw provider identifiers, never synthetic keys.
pub(crate) fn response_ids(dedup_key: &Option<String>) -> impl Iterator<Item = &str> {
    let key = dedup_key.as_deref().unwrap_or("");
    let mut ids: Vec<&str> = Vec::new();
    // `<namespace>:response:<id>`
    if let Some((_, rest)) = key.split_once(":response:") {
        if !rest.is_empty() {
            ids.push(rest);
        }
    }
    // `codex:usage-record:<scope>:<response id>`
    if let Some(rest) = key.strip_prefix("codex:usage-record:") {
        if let Some((_, id)) = rest.rsplit_once(':') {
            if !id.is_empty() && id != rest {
                ids.push(id);
            }
        }
    }
    // `<namespace>:<session>:req:<request id>`
    if let Some((_, rest)) = key.split_once(":req:") {
        if !rest.is_empty() {
            ids.push(rest);
        }
    }
    ids.into_iter()
        .filter(|id| !id.is_empty() && !id.contains(':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: i64 = 1_700_000_000_000;

    fn message(model: &str, tokens: TokenBreakdown) -> UnifiedMessage {
        UnifiedMessage::new("codex", model, "openai", "session", START, tokens, 0.0)
    }

    fn tokens() -> TokenBreakdown {
        TokenBreakdown {
            input: 100,
            output: 20,
            cache_read: 400,
            cache_write: 0,
            ..TokenBreakdown::default()
        }
    }

    fn record(selector: &str, signature: TokenSignature) -> GatewayRecord {
        GatewayRecord {
            adapter: "fixture",
            selector: Some(selector.into()),
            selected_model: Some("wire-model".into()),
            served_model: Some("served-model".into()),
            provider: Some("provider".into()),
            service_tier: None,
            session_ids: Vec::new(),
            stable_ids: Vec::new(),
            start_ms: START,
            duration_ms: Some(1_000),
            signatures: vec![signature],
            completed: true,
            target_eligible: true,
            allow_selected_fallback: false,
        }
    }

    fn signature(tokens: &TokenBreakdown) -> TokenSignature {
        TokenSignature::from_gateway(
            tokens.input,
            tokens.output + tokens.reasoning,
            tokens.cache_read,
            tokens.cache_write,
            tokens.reasoning,
        )
    }

    #[test]
    fn verifies_an_exact_token_signature_on_the_selector_tier() {
        // given: the gateway reports input without cache traffic and output
        // including reasoning; the client stored the same buckets additively.
        let tokens = tokens();
        let index = GatewayIndex::new(
            MatchPolicy::opencodex(),
            vec![record("selector", signature(&tokens))],
        );
        let mut message = message("selector", tokens);
        message.duration_ms = Some(1_000);

        let candidates = index.candidates(&message);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].tier, MatchTier::Selector);
    }

    #[test]
    fn a_reasoning_mismatch_fails_closed() {
        let tokens = tokens();
        let mut gateway = signature(&tokens);
        gateway.reasoning = 4;
        let index = GatewayIndex::new(MatchPolicy::opencodex(), vec![record("selector", gateway)]);
        let mut message = message("selector", tokens);
        message.duration_ms = Some(1_000);

        assert!(index.candidates(&message).is_empty());
    }

    #[test]
    fn matching_usage_with_distant_timing_does_not_match() {
        let tokens = tokens();
        let mut row = record("selector", signature(&tokens));
        row.start_ms += 600_000;
        let index = GatewayIndex::new(MatchPolicy::opencodex(), vec![row]);
        let mut message = message("selector", tokens);
        message.duration_ms = Some(1_000);

        assert!(index.candidates(&message).is_empty());
    }

    #[test]
    fn identifier_matches_are_recovered_from_response_dedup_keys() {
        let tokens = tokens();
        let mut row = record("other-selector", signature(&tokens));
        row.stable_ids = vec!["resp-fixture".into()];
        let index = GatewayIndex::new(MatchPolicy::opencodex(), vec![row]);
        let mut message = message("selector", tokens);
        message.duration_ms = Some(1_000);
        message.dedup_key = Some("pi:response:resp-fixture".into());

        let candidates = index.candidates(&message);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].tier, MatchTier::Identifier);
    }

    #[test]
    fn a_session_match_requires_a_verified_token_signature() {
        let tokens = tokens();
        let mut row = record("other-selector", signature(&tokens));
        row.session_ids = vec!["session".into()];
        let index = GatewayIndex::new(MatchPolicy::opencodex(), vec![row]);
        let mut message = message("selector", TokenBreakdown::default());
        message.duration_ms = Some(1_000);

        assert!(index.candidates(&message).is_empty());
    }

    #[test]
    fn a_completed_record_may_use_the_selection_only_when_its_adapter_allows_it() {
        let tokens = tokens();
        let mut row = record("selector", signature(&tokens));
        row.served_model = None;
        row.allow_selected_fallback = true;
        let target = row.target().expect("the selected model is usable evidence");
        assert_eq!(target.model, "wire-model");
        assert_eq!(target.strength, EvidenceStrength::ConfirmedSelection);

        row.allow_selected_fallback = false;
        assert!(row.target().is_none());
    }

    /// A selected model that only repeats the virtual selector, or that is a
    /// routing label in its own right, names no weights. Promoting it would
    /// turn a router into a "confirmed" physical identity, so it stays
    /// unclaimed and reserves the request for nothing.
    #[test]
    fn a_virtual_selection_is_never_a_confirmed_identity() {
        for (selector, selected) in [
            ("group/auto", "group/auto"),
            ("combo/auto", "combo/auto"),
            ("some-selector", "auto"),
            ("some-selector", "AGENT_REVIEW"),
            ("some-selector", "openrouter/auto"),
            ("some-selector", "   "),
        ] {
            let tokens = tokens();
            let mut row = record(selector, signature(&tokens));
            row.served_model = None;
            row.selected_model = Some(selected.into());
            row.allow_selected_fallback = true;
            assert!(
                row.target().is_none(),
                "{selector} -> {selected} must not become a physical identity"
            );
        }
    }

    /// A served value equal to the *selected* model is real evidence: the
    /// gateway contacted exactly that model and the response confirmed it.
    #[test]
    fn a_served_value_matching_the_selection_is_response_evidence() {
        let tokens = tokens();
        let mut row = record("group/auto", signature(&tokens));
        row.selected_model = Some("glm-5.2".into());
        row.served_model = Some("glm-5.2".into());
        let target = row.target().expect("the response named the served model");
        assert_eq!(target.model, "glm-5.2");
        assert_eq!(target.strength, EvidenceStrength::ResponseReported);
    }

    /// A response that only echoes the requested selector is not response
    /// evidence, but it must not hide a usable selection either.
    #[test]
    fn a_served_echo_of_the_selector_falls_back_to_the_selection() {
        let tokens = tokens();
        let mut row = record("group/auto", signature(&tokens));
        row.selected_model = Some("glm-5.2".into());
        row.served_model = Some("group/auto".into());
        row.allow_selected_fallback = true;
        let target = row.target().expect("the selection is still usable");
        assert_eq!(target.model, "glm-5.2");
        assert_eq!(target.strength, EvidenceStrength::ConfirmedSelection);
    }

    /// A response-reported row is never a parking candidate: its evidence is
    /// stronger than anything a ledger can add.
    #[test]
    fn a_response_reported_row_is_never_a_claim() {
        let tokens = tokens();
        let index = GatewayIndex::new(
            MatchPolicy::opencodex(),
            vec![record("selector", signature(&tokens))],
        );
        let mut message = message("selector", tokens);
        message.duration_ms = Some(1_000);
        message.model_evidence = crate::sessions::ModelEvidence::ResponseReported;

        assert!(!index.could_claim(&message));
        assert!(index.candidates(&message).is_empty());
    }

    #[test]
    fn an_incomplete_record_never_produces_a_target() {
        let tokens = tokens();
        let mut row = record("selector", signature(&tokens));
        row.completed = false;
        assert!(row.target().is_none());
    }

    #[test]
    fn a_multi_call_aggregate_is_never_a_claim() {
        let tokens = tokens();
        let index = GatewayIndex::new(
            MatchPolicy::opencodex(),
            vec![record("selector", signature(&tokens))],
        );
        let mut message = message("selector", tokens);
        message.duration_ms = Some(1_000);
        message.message_count = 3;

        assert!(index.candidates(&message).is_empty());
        assert!(!index.could_claim(&message));
    }

    #[test]
    fn an_unknown_selector_is_never_a_claim() {
        let tokens = tokens();
        let index = GatewayIndex::new(
            MatchPolicy::opencodex(),
            vec![record("selector", signature(&tokens))],
        );
        let mut message = message("different-selector", tokens);
        message.duration_ms = Some(1_000);

        assert!(index.candidates(&message).is_empty());
        assert!(!index.could_claim(&message));
    }
}
