//! Scan-scoped side metadata; the public and cached message layout stays stable.

use crate::sessions::{BillingIdentity, SharedStr, UnifiedMessage};
use crate::{CostSource, EstimateSource};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(PartialEq, Eq, Hash)]
struct AppliedEstimate {
    client: SharedStr,
    model: SharedStr,
    provider: SharedStr,
    billing_provider: Option<SharedStr>,
    pricing_model: Option<SharedStr>,
    session: SharedStr,
    timestamp: i64,
    service_tier: Option<SharedStr>,
    tokens: [i64; 6],
    cost_bits: u64,
}

impl From<&UnifiedMessage> for AppliedEstimate {
    fn from(message: &UnifiedMessage) -> Self {
        Self {
            client: message.client.clone(),
            model: message.model_id.clone(),
            provider: message.provider_id.clone(),
            billing_provider: match &message.billing_identity {
                BillingIdentity::Known { id, .. } => Some(id.clone()),
                BillingIdentity::Unknown => None,
            },
            pricing_model: message.pricing_model_id.clone(),
            session: message.session_id.clone(),
            timestamp: message.timestamp,
            service_tier: message.service_tier.clone(),
            tokens: [
                message.tokens.input,
                message.tokens.output,
                message.tokens.cache_read,
                message.tokens.cache_write,
                message.tokens.cache_write_1h,
                message.tokens.reasoning,
            ],
            cost_bits: message.cost.to_bits(),
        }
    }
}

/// Retain only origins of estimates actually applied in this scan. Counts
/// handle identical rows; lane flushing releases leftovers from deduplication.
#[derive(Default)]
pub(crate) struct CostEstimateCapture {
    estimates: Mutex<HashMap<AppliedEstimate, (EstimateSource, usize)>>,
}

type Estimates = HashMap<AppliedEstimate, (EstimateSource, usize)>;

impl CostEstimateCapture {
    /// Lock the map, recovering the data if another thread panicked while
    /// holding it.
    ///
    /// Captured origins are side metadata: a missing entry degrades one row to
    /// `estimated/unknown`, which the aggregator already handles. Propagating
    /// poison instead would turn one worker's panic into a panic in every
    /// other rayon worker that touches pricing, failing a whole scan over
    /// metadata that is allowed to be absent.
    fn lock(&self) -> std::sync::MutexGuard<'_, Estimates> {
        self.estimates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn record(&self, message: &UnifiedMessage, source: EstimateSource) {
        if message.cost_source != CostSource::Estimated {
            return;
        }
        let mut estimates = self.lock();
        let entry = estimates.entry(message.into()).or_insert((source, 0));
        if entry.0 != source {
            entry.0 = EstimateSource::Mixed;
        }
        entry.1 = entry.1.saturating_add(1);
    }

    pub(crate) fn take(&self, message: &UnifiedMessage) -> Option<EstimateSource> {
        if message.cost_source != CostSource::Estimated {
            return None;
        }
        let mut estimates = self.lock();
        let key = AppliedEstimate::from(message);
        let (source, count) = estimates.get_mut(&key)?;
        let result = *source;
        // One `take` per `record` is the intended protocol, so reaching zero
        // means this was the last identical row and the entry is dropped.
        // `checked_sub` states that invariant instead of relying on the
        // `get_mut` miss above to prevent an underflow panic in release mode.
        match count.checked_sub(1) {
            Some(0) | None => {
                estimates.remove(&key);
            }
            Some(remaining) => *count = remaining,
        }
        Some(result)
    }

    pub(crate) fn clear(&self) {
        self.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::BillingIdentityOrigin;
    use crate::TokenBreakdown;

    fn estimated_message(cost: f64) -> UnifiedMessage {
        let mut message = UnifiedMessage::new(
            "test",
            "model",
            "provider",
            "session",
            0,
            TokenBreakdown {
                input: 10,
                ..Default::default()
            },
            cost,
        );
        message.cost_source = CostSource::Estimated;
        message
    }

    #[test]
    fn identical_rows_are_released_once_each_and_then_report_nothing() {
        let capture = CostEstimateCapture::default();
        let message = estimated_message(1.0);
        capture.record(&message, EstimateSource::Catalog);
        capture.record(&message, EstimateSource::Catalog);

        assert_eq!(capture.take(&message), Some(EstimateSource::Catalog));
        assert_eq!(capture.take(&message), Some(EstimateSource::Catalog));
        // A third take has no recorded estimate behind it. It must report
        // nothing rather than underflow the refcount.
        assert_eq!(capture.take(&message), None);
        assert_eq!(capture.take(&message), None);
    }

    #[test]
    fn disagreeing_sources_for_one_row_collapse_to_mixed() {
        let capture = CostEstimateCapture::default();
        let message = estimated_message(1.0);
        capture.record(&message, EstimateSource::Catalog);
        capture.record(&message, EstimateSource::Custom);

        assert_eq!(capture.take(&message), Some(EstimateSource::Mixed));
    }

    #[test]
    fn a_poisoned_lock_still_serves_captured_origins() {
        use std::sync::Arc;

        let capture = Arc::new(CostEstimateCapture::default());
        let message = estimated_message(1.0);
        capture.record(&message, EstimateSource::Catalog);

        let poisoner = Arc::clone(&capture);
        let panicked = std::thread::spawn(move || {
            let _guard = poisoner.estimates.lock().unwrap();
            panic!("poison the capture mutex");
        })
        .join();
        assert!(panicked.is_err());
        assert!(capture.estimates.is_poisoned());

        // A panicking worker must not escalate into a failed scan for every
        // other worker that reads pricing metadata.
        assert_eq!(capture.take(&message), Some(EstimateSource::Catalog));
        capture.clear();
    }

    #[test]
    fn billing_provider_reuses_the_message_identity() {
        let mut message = estimated_message(1.0);
        message.billing_identity = BillingIdentity::from_explicit(
            Some("anthropic"),
            BillingIdentityOrigin::ExplicitRecord,
        );
        let BillingIdentity::Known { id, .. } = &message.billing_identity else {
            panic!("expected a known billing identity");
        };

        let estimate = AppliedEstimate::from(&message);
        let stored = estimate
            .billing_provider
            .as_ref()
            .expect("billing provider");
        assert!(stored.shares_allocation_with(id));
    }
}
