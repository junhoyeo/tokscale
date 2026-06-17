use super::{UsageMetric, UsageOutput};
use anyhow::Result;

pub fn has_credentials() -> bool {
    crate::warp::load_usage_cache().is_some()
}

pub fn fetch() -> Result<UsageOutput> {
    let cache = crate::warp::load_usage_cache()
        .ok_or_else(|| anyhow::anyhow!("Warp aggregate usage cache not found"))?;
    let mut metrics = Vec::new();

    if let Some(used) = cache.usage.requests_used {
        let (used_percent, remaining_percent, remaining_label) =
            if let Some(limit) = cache.usage.request_limit.filter(|limit| *limit > 0) {
                let used_percent = (used as f64 / limit as f64 * 100.0).clamp(0.0, 100.0);
                let remaining = limit.saturating_sub(used);
                (
                    used_percent,
                    100.0 - used_percent,
                    Some(format!("{remaining} requests left")),
                )
            } else {
                (0.0, 0.0, Some(format!("{used} requests used")))
            };
        metrics.push(UsageMetric {
            label: "Requests".to_string(),
            used_percent,
            remaining_percent,
            remaining_label,
            resets_at: cache.usage.next_refresh_time.clone(),
        });
    }

    if let Some(spend_cents) = cache.usage.spend_cents {
        metrics.push(UsageMetric {
            label: "Spend".to_string(),
            used_percent: 0.0,
            remaining_percent: 0.0,
            remaining_label: Some(format!("${:.2}", spend_cents as f64 / 100.0)),
            resets_at: cache.usage.next_refresh_time.clone(),
        });
    }

    Ok(UsageOutput {
        provider: "Warp/Oz".to_string(),
        account: None,
        plan: Some("Aggregate API cache".to_string()),
        email: None,
        metrics,
    })
}
