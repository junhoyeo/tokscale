use super::{
    UsageFetchDiagnostic, UsageFetchDiagnosticKind, UsageFetchDiagnosticSeverity, UsageFetchReport,
    UsageFreshness, UsageMetric, UsageOutput,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::Path;

const PROVIDER: &str = "Warp/Oz";

/// Only a manual `tokscale warp sync` refreshes the cache, so a minutes-old
/// sync is the normal state; past a day the card is shown as cached and warns.
const STALE_AFTER_SECONDS: i64 = 24 * 60 * 60;

/// Admit on the file alone so a malformed cache reaches `fetch_report` and is
/// reported, instead of the provider silently disappearing.
pub fn has_credentials() -> bool {
    usage_cache_path().is_file()
}

fn usage_cache_path() -> std::path::PathBuf {
    crate::warp::get_warp_cache_dir().join("usage.json")
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

/// The cache is written only by `tokscale warp sync`, so an old sync still
/// shows its numbers, marked as cached with its sync time and a warning that
/// says how old they are.
pub fn fetch_report() -> UsageFetchReport {
    report_at(&usage_cache_path(), Utc::now())
}

fn report_at(path: &Path, now: DateTime<Utc>) -> UsageFetchReport {
    read_report_at(path, now)
        .unwrap_or_else(|error| UsageFetchReport::from_error(PROVIDER, None, error))
}

fn read_report_at(path: &Path, now: DateTime<Utc>) -> Result<UsageFetchReport> {
    let content = std::fs::read_to_string(path).context("Warp aggregate usage cache not found")?;
    let cache: crate::warp::WarpUsageCache =
        serde_json::from_str(&content).context("Invalid Warp aggregate usage cache")?;
    anyhow::ensure!(
        cache.version == 1,
        "Unsupported Warp aggregate usage cache version {}",
        cache.version
    );
    let synced_at = DateTime::parse_from_rfc3339(&cache.synced_at)
        .context("Invalid Warp aggregate usage cache sync timestamp")?;
    let age = now.signed_duration_since(synced_at);
    anyhow::ensure!(
        age >= chrono::Duration::zero(),
        "Warp aggregate usage cache sync timestamp is in the future"
    );
    let stale = age > chrono::Duration::seconds(STALE_AFTER_SECONDS);

    let output = UsageOutput {
        freshness: Some(UsageFreshness {
            fetched_at: u64::try_from(synced_at.timestamp())
                .context("Warp aggregate usage cache sync timestamp is before 1970")?,
            stale,
        }),
        provider: PROVIDER.to_string(),
        account: None,
        credential_source: None,
        plan: None,
        email: None,
        metrics: build_metrics(&cache.usage),
        reset_credits: None,
        credit_status: None,
        spend_control: None,
    };
    let mut diagnostics = Vec::new();
    if stale {
        diagnostics.push(UsageFetchDiagnostic::with_kind(
            PROVIDER,
            None,
            UsageFetchDiagnosticKind::FetchFailed,
            UsageFetchDiagnosticSeverity::Warning,
            format!(
                "Warp usage was last synced {} ago; run `tokscale warp sync` to refresh it",
                describe_age(age)
            ),
        ));
    }
    Ok(UsageFetchReport {
        outputs: vec![output],
        diagnostics,
    })
}

fn describe_age(age: chrono::Duration) -> String {
    let minutes = age.num_minutes();
    if minutes < 60 {
        format!("{minutes} min")
    } else if minutes < 48 * 60 {
        format!("{} h", minutes / 60)
    } else {
        format!("{} days", minutes / (24 * 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::warp::WarpAggregateUsage;

    fn now() -> DateTime<Utc> {
        "2026-10-17T12:00:00Z".parse().unwrap()
    }

    /// Writes a v1 cache reporting a known zero: 0 of 100 requests, $0 spend.
    fn write_cache(path: &Path, synced_at: &str) {
        let cache = crate::warp::WarpUsageCache {
            version: 1,
            synced_at: synced_at.to_string(),
            usage: WarpAggregateUsage {
                requests_used: Some(0),
                request_limit: Some(100),
                spend_cents: Some(0),
                ..Default::default()
            },
            workspaces: Vec::new(),
        };
        std::fs::write(path, serde_json::to_string(&cache).unwrap()).unwrap();
    }

    #[test]
    fn cache_age_selects_updated_or_cached_with_sync_time() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("usage.json");
        for (synced_at, stale, age) in [
            ("2026-10-17T12:00:00Z", false, None),
            ("2026-10-17T09:00:00Z", false, None),
            ("2026-10-16T12:00:00Z", false, None),
            ("2026-10-16T11:59:59Z", true, Some("24 h")),
            ("2026-10-15T12:00:00Z", true, Some("2 days")),
            ("2026-10-10T12:00:00Z", true, Some("7 days")),
        ] {
            write_cache(&path, synced_at);
            let report = report_at(&path, now());
            assert_eq!(report.outputs.len(), 1, "{synced_at}: the card is kept");
            let output = &report.outputs[0];
            let synced = DateTime::parse_from_rfc3339(synced_at).unwrap().timestamp();
            assert_eq!(
                output.freshness,
                Some(UsageFreshness {
                    fetched_at: synced as u64,
                    stale,
                }),
                "{synced_at}"
            );
            // The known zero is still reported as 0% used, stale or not.
            assert_eq!(output.metrics[0].used_percent, 0.0);
            assert_eq!(output.metrics[1].remaining_label.as_deref(), Some("$0.00"));
            match age {
                None => assert!(report.diagnostics.is_empty(), "{synced_at}"),
                Some(age) => {
                    assert_eq!(report.diagnostics.len(), 1);
                    let diagnostic = &report.diagnostics[0];
                    assert_eq!(diagnostic.provider, PROVIDER);
                    assert_eq!(diagnostic.severity, UsageFetchDiagnosticSeverity::Warning);
                    assert!(diagnostic.message.contains(age), "{}", diagnostic.message);
                    assert!(diagnostic.message.contains("tokscale warp sync"));
                }
            }
        }
    }

    #[test]
    fn untrustworthy_cache_fails_closed_with_a_diagnostic() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("usage.json");
        let missing = report_at(&path, now());
        assert!(missing.outputs.is_empty());
        assert!(missing.diagnostics[0].message.contains("not found"));

        for (content, reason) in [
            ("malformed".to_string(), "Invalid"),
            (
                r#"{"version":1,"usage":{},"workspaces":[]}"#.to_string(),
                "Invalid",
            ),
            (
                r#"{"version":2,"syncedAt":"2026-10-17T12:00:00Z","usage":{},"workspaces":[]}"#
                    .to_string(),
                "version",
            ),
        ] {
            std::fs::write(&path, content).unwrap();
            let report = report_at(&path, now());
            assert!(report.outputs.is_empty());
            assert_eq!(report.diagnostics.len(), 1);
            assert_eq!(report.diagnostics[0].provider, PROVIDER);
            assert!(
                report.diagnostics[0].message.contains(reason),
                "{}",
                report.diagnostics[0].message
            );
        }

        for (synced_at, reason) in [
            ("2026-10-17T12:00:01Z", "future"),
            ("2026-10-17T12:00:00.001Z", "future"),
            ("", "timestamp"),
            ("not-a-time", "timestamp"),
        ] {
            write_cache(&path, synced_at);
            let report = report_at(&path, now());
            assert!(report.outputs.is_empty(), "{synced_at}");
            assert!(
                report.diagnostics[0].message.contains(reason),
                "{}",
                report.diagnostics[0].message
            );
        }
    }

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
