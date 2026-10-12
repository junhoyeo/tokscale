//! Scan-scoped side metadata; the public and cached message layout stays stable.

use crate::{sessions::UnifiedMessage, CostSource, EstimateSource};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(PartialEq, Eq, Hash)]
struct AppliedEstimate {
    client: String,
    model: String,
    provider: String,
    session: String,
    timestamp: i64,
    service_tier: Option<String>,
    tokens: [i64; 6],
    cost_bits: u64,
}

impl From<&UnifiedMessage> for AppliedEstimate {
    fn from(message: &UnifiedMessage) -> Self {
        Self {
            client: message.client.clone(),
            model: message.model_id.clone(),
            provider: message.provider_id.clone(),
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

impl CostEstimateCapture {
    pub(crate) fn record(&self, message: &UnifiedMessage, source: EstimateSource) {
        if message.cost_source != CostSource::Estimated {
            return;
        }
        let mut estimates = self.estimates.lock().unwrap();
        let entry = estimates.entry(message.into()).or_insert((source, 0));
        if entry.0 != source {
            entry.0 = EstimateSource::Mixed;
        }
        entry.1 += 1;
    }

    pub(crate) fn take(&self, message: &UnifiedMessage) -> Option<EstimateSource> {
        if message.cost_source != CostSource::Estimated {
            return None;
        }
        let mut estimates = self.estimates.lock().unwrap();
        let key = AppliedEstimate::from(message);
        let (source, count) = estimates.get_mut(&key)?;
        let result = *source;
        *count -= 1;
        if *count == 0 {
            estimates.remove(&key);
        }
        Some(result)
    }

    pub(crate) fn clear(&self) {
        self.estimates.lock().unwrap().clear();
    }
}
