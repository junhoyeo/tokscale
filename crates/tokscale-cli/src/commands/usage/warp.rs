use super::{UsageMetric, UsageOutput};
use anyhow::Result;

pub fn has_credentials() -> bool {
    crate::warp::load_usage_cache().is_some()
}

fn build_metrics(usage: &crate::warp::WarpAggregateUsage) -> Vec<UsageMetric> {
    let mut metrics = Vec::new();

    if let Some(used) = usage.requests_used {
        let (used_percent, remaining_percent, remaining_label) =
            if let Some(limit) = usage.request_limit.filter(|limit| *limit > 0) {
                let used_percent = (used as f64 / limit as f64 * 100.0).clamp(0.0, 100.0);
                let remaining = limit.saturating_sub(used);
                (
                    used_percent,
                    100.0 - used_percent,
                    Some(format!("{remaining} requests left")),
                )
            } else {
                // No request limit: this is an informational counter, not a
                // capped quota. Render the bar as full (100% remaining) rather
                // than an empty "exhausted" bar.
                (0.0, 100.0, Some(format!("{used} requests used")))
            };
        metrics.push(UsageMetric {
            label: "Requests".to_string(),
            used_percent,
            remaining_percent,
            remaining_label,
            resets_at: usage.next_refresh_time.clone(),
        });
    }

    if let Some(spend_cents) = usage.spend_cents {
        metrics.push(UsageMetric {
            label: "Spend".to_string(),
            used_percent: 0.0,
            // Spend is an informational dollar figure, not a consumed quota, so
            // keep the bar full instead of rendering a false "exhausted" bar.
            remaining_percent: 100.0,
            remaining_label: Some(format!("${:.2}", spend_cents as f64 / 100.0)),
            resets_at: usage.next_refresh_time.clone(),
        });
    }

    metrics
}

pub fn fetch() -> Result<UsageOutput> {
    let cache = crate::warp::load_usage_cache()
        .ok_or_else(|| anyhow::anyhow!("Warp aggregate usage cache not found"))?;
    let metrics = build_metrics(&cache.usage);

    Ok(UsageOutput {
        provider: "Warp/Oz".to_string(),
        account: None,
        plan: None,
        email: None,
        metrics,
        reset_credits: None,
        credit_status: None,
        spend_control: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::warp::WarpAggregateUsage;

    #[test]
    fn spend_metric_reads_full_not_exhausted() {
        let usage = WarpAggregateUsage {
            spend_cents: Some(1234),
            ..Default::default()
        };
        let metrics = build_metrics(&usage);
        let spend = metrics.iter().find(|m| m.label == "Spend").unwrap();
        // Informational $ figure: bar must read full, not a false 0%-remaining.
        assert_eq!(spend.remaining_percent, 100.0);
        assert_eq!(spend.used_percent, 0.0);
        assert_eq!(spend.remaining_label.as_deref(), Some("$12.34"));
    }

    #[test]
    fn unlimited_requests_read_full_not_exhausted() {
        let usage = WarpAggregateUsage {
            requests_used: Some(42),
            request_limit: None,
            ..Default::default()
        };
        let metrics = build_metrics(&usage);
        let requests = metrics.iter().find(|m| m.label == "Requests").unwrap();
        assert_eq!(requests.remaining_percent, 100.0);
        assert_eq!(requests.used_percent, 0.0);
        assert_eq!(
            requests.remaining_label.as_deref(),
            Some("42 requests used")
        );
    }

    #[test]
    fn limited_requests_compute_usage() {
        let usage = WarpAggregateUsage {
            requests_used: Some(25),
            request_limit: Some(100),
            ..Default::default()
        };
        let metrics = build_metrics(&usage);
        let requests = metrics.iter().find(|m| m.label == "Requests").unwrap();
        assert_eq!(requests.used_percent, 25.0);
        assert_eq!(requests.remaining_percent, 75.0);
        assert_eq!(
            requests.remaining_label.as_deref(),
            Some("75 requests left")
        );
    }
}
