//! Scan-scoped attribution diagnostics.
//!
//! `tokscale pricing explain` re-parses local sources and then explains each
//! resulting message, but attribution evidence is not part of the message
//! layout: adding serialized provenance fields to every `UnifiedMessage` would
//! change the cache format and the submission DTO for one local view. Instead
//! the overlay records what it decided in a bounded side table keyed by the
//! stable, post-attribution identity of the row, and `explain` looks that up.
//!
//! The table is local-only. No entry names a credential, a request body, or a
//! remote host; it carries model/provider identities and the adapter that
//! supplied them.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

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
    /// Independent adapters disagreed about the served model.
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

static DECISIONS: LazyLock<Mutex<HashMap<RowKey, Decision>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn lock() -> std::sync::MutexGuard<'static, HashMap<RowKey, Decision>> {
    // Diagnostics are side metadata. A poisoned lock degrades the explanation
    // to "no attribution detail"; making it fatal would turn one panicked
    // worker into a failed scan over a debug view.
    DECISIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Begin a new attribution pass. Callers clear before parsing so a lookup
/// reflects the scan being explained rather than a previous one.
pub(crate) fn clear() {
    lock().clear();
}

pub(crate) fn record(message: &UnifiedMessage, decision: Decision) {
    let mut decisions = lock();
    if decisions.len() >= MAX_DECISIONS {
        return;
    }
    decisions.insert(RowKey::of(message), decision);
}

pub(crate) fn lookup(message: &UnifiedMessage) -> Option<Decision> {
    lock().get(&RowKey::of(message)).cloned()
}
