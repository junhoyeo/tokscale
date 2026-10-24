//! Scan-scoped attribution diagnostics.
//!
//! `tokscale pricing explain` re-parses local sources and then explains each
//! resulting message, but attribution evidence is not part of the message
//! layout: adding serialized provenance fields to every `UnifiedMessage` would
//! change the cache format and the submission DTO for one local view. Instead
//! the overlay returns what it decided beside the parsed rows, and `explain`
//! looks a row up in that value.
//!
//! The table belongs to one scan. A process-global table would make two
//! concurrent parses clear or read each other's decisions, so the overlay owns
//! its own diagnostics and the parse API hands them to the caller. Nothing
//! persists beyond the scan that produced it.
//!
//! The table is local-only. No entry names a credential, a request body, or a
//! remote host; it carries model/provider identities and the adapter that
//! supplied them.

use std::collections::HashMap;

use crate::model_attribution::matcher::EvidenceStrength;
use crate::sessions::UnifiedMessage;

/// Bounded so a scan over a huge history cannot grow the side table without
/// limit. Diagnostics are best-effort: past the cap, newer decisions are
/// dropped rather than the process holding every row of a scan.
const MAX_DECISIONS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RowKey {
    client: String,
    session: String,
    timestamp: i64,
    tokens: [i64; 6],
}

impl RowKey {
    fn of(message: &UnifiedMessage) -> Self {
        Self {
            client: message.client.clone(),
            session: message.session_id.clone(),
            timestamp: message.timestamp,
            tokens: [
                message.tokens.input,
                message.tokens.output,
                message.tokens.cache_read,
                message.tokens.cache_write,
                message.tokens.cache_write_1h,
                message.tokens.reasoning,
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttributionOutcome {
    /// One adapter established the served model.
    Resolved {
        adapter: &'static str,
        strength: EvidenceStrength,
        provider: String,
        model: String,
    },
    /// Independent adapters disagreed about the served model or provider.
    Conflicted,
    /// The row matched gateway records but they did not establish one model.
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decision {
    pub(crate) requested_model: String,
    pub(crate) requested_provider: String,
    /// Strongest correlation rule that matched, when a record did match.
    pub(crate) matched_by: Option<&'static str>,
    /// Every adapter whose ledger produced at least one feasible candidate for
    /// this row, sorted and deduplicated. Local diagnostics only; it exists so
    /// a nested-gateway test can assert that both ledgers participated instead
    /// of only the one that happened to supply the winning observation.
    pub(crate) participating_adapters: Vec<&'static str>,
    pub(crate) outcome: AttributionOutcome,
}

impl Decision {
    /// One of `resolved`, `unresolved`, or `conflict`.
    pub(crate) fn outcome_label(&self) -> &'static str {
        match self.outcome {
            AttributionOutcome::Resolved { .. } => "resolved",
            AttributionOutcome::Conflicted => "conflict",
            AttributionOutcome::Unresolved => "unresolved",
        }
    }

    pub(crate) fn resolved_target(&self) -> Option<(&'static str, &str, &str, EvidenceStrength)> {
        match &self.outcome {
            AttributionOutcome::Resolved {
                adapter,
                strength,
                provider,
                model,
            } => Some((adapter, provider, model, *strength)),
            _ => None,
        }
    }
}

impl EvidenceStrength {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::ResponseReported => "response-reported",
            Self::ConfirmedSelection => "confirmed-selection",
        }
    }
}

/// Attribution decisions produced by one parse.
///
/// Returned beside the messages it describes. Lookups are read-only, so a
/// concurrent scan with its own diagnostics can never clear or contaminate
/// this one.
#[derive(Debug, Clone, Default)]
pub struct AttributionDiagnostics {
    decisions: HashMap<RowKey, Decision>,
    /// Decisions dropped because the table hit its cap. Surfaced so
    /// `pricing explain` can say the block is incomplete instead of silently
    /// omitting rows.
    truncated: usize,
}

impl AttributionDiagnostics {
    pub(crate) fn record(&mut self, message: &UnifiedMessage, decision: Decision) {
        if self.decisions.len() >= MAX_DECISIONS {
            self.truncated += 1;
            return;
        }
        self.decisions.insert(RowKey::of(message), decision);
    }

    pub(crate) fn lookup(&self, message: &UnifiedMessage) -> Option<Decision> {
        self.decisions.get(&RowKey::of(message)).cloned()
    }

    /// How many decisions were dropped after the table hit its cap.
    pub fn truncated_decisions(&self) -> usize {
        self.truncated
    }
}
