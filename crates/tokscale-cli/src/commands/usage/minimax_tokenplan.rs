use anyhow::Result;
use chrono::{TimeZone, Utc};
use serde::Deserialize;

use super::{UsageAccount, UsageMetric, UsageOutput};

const TOKEN_PLAN_PATH: &str = "/v1/token_plan/remains";

// MiniMax runs separate token-plan backends for its domestic (minimaxi.com) and
// international (minimax.io) sites, each behind its own API key.
struct Site {
    label: &'static str,
    base_url: &'static str,
    key_env: &'static str,
}

const SITES: &[Site] = &[
    Site {
        label: "CN",
        base_url: "https://www.minimaxi.com",
        key_env: "MINIMAX_TOKEN_PLAN_CN_KEY",
    },
    Site {
        label: "Global",
        base_url: "https://www.minimax.io",
        key_env: "MINIMAX_TOKEN_PLAN_GLOBAL_KEY",
    },
];

#[derive(Debug, Deserialize)]
struct ApiResponse {
    base_resp: Option<BaseResp>,
    model_remains: Option<Vec<ModelRemains>>,
}

#[derive(Debug, Deserialize)]
struct BaseResp {
    status_code: Option<i64>,
    status_msg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelRemains {
    model_name: Option<String>,
    current_interval_remaining_percent: Option<i64>,
    end_time: Option<i64>,
    current_weekly_status: Option<i64>,
    current_weekly_remaining_percent: Option<i64>,
    weekly_end_time: Option<i64>,
}

fn read_key(site: &Site) -> Option<String> {
    let key = std::env::var(site.key_env).ok()?;
    let trimmed = key.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn has_credentials() -> bool {
    SITES.iter().any(|s| read_key(s).is_some())
}

fn epoch_ms_to_rfc3339(ts: i64) -> Option<String> {
    let ms = if ts.abs() > 10_000_000_000 {
        ts
    } else {
        ts * 1000
    };
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.to_rfc3339())
}

fn is_auth_error(resp: &ApiResponse) -> bool {
    matches!(
        resp.base_resp.as_ref().and_then(|b| b.status_code),
        Some(1004)
    )
}

fn is_api_error(resp: &ApiResponse) -> bool {
    resp.base_resp
        .as_ref()
        .and_then(|b| b.status_code)
        .map(|code| code != 0)
        .unwrap_or(false)
}

fn build_metrics(remains: &[ModelRemains]) -> Vec<UsageMetric> {
    let mut metrics = Vec::new();
    for m in remains {
        let name = m.model_name.as_deref().unwrap_or("model");

        if let Some(pct) = m.current_interval_remaining_percent {
            let remaining = pct.clamp(0, 100) as f64;
            metrics.push(UsageMetric {
                label: name.to_string(),
                used_percent: 100.0 - remaining,
                remaining_percent: remaining,
                remaining_label: None,
                resets_at: m.end_time.and_then(epoch_ms_to_rfc3339),
            });
        }

        // Skip when the plan has no weekly limit: an inactive status must not be
        // rendered as "0% left / 100% used".
        if m.current_weekly_status.is_some_and(|s| s != 0) {
            if let Some(pct) = m.current_weekly_remaining_percent {
                let remaining = pct.clamp(0, 100) as f64;
                metrics.push(UsageMetric {
                    label: format!("{name}·wk"),
                    used_percent: 100.0 - remaining,
                    remaining_percent: remaining,
                    remaining_label: None,
                    resets_at: m.weekly_end_time.and_then(epoch_ms_to_rfc3339),
                });
            }
        }
    }
    metrics
}

async fn fetch_site(client: &reqwest::Client, site: &Site, key: &str) -> Result<ApiResponse> {
    let url = format!("{}{TOKEN_PLAN_PATH}", site.base_url);
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .send()
        .await?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!(
            "MiniMax Token Plan ({}) session expired; check your API key",
            site.label
        );
    }
    if !status.is_success() {
        anyhow::bail!(
            "MiniMax Token Plan ({}) request failed (HTTP {status})",
            site.label
        );
    }
    Ok(resp.json().await?)
}

pub fn fetch_all() -> Result<Vec<UsageOutput>> {
    let targets: Vec<&Site> = SITES.iter().filter(|s| read_key(s).is_some()).collect();
    if targets.is_empty() {
        return Ok(vec![]);
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::new();
        let mut outputs = Vec::new();

        for site in targets {
            let key = read_key(site).unwrap_or_default();
            let resp = match fetch_site(&client, site, &key).await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("MiniMax Token Plan ({}): {e}", site.label);
                    continue;
                }
            };

            if is_auth_error(&resp) {
                eprintln!(
                    "MiniMax Token Plan ({}): session expired; check your API key",
                    site.label
                );
                continue;
            }
            if is_api_error(&resp) {
                let msg = resp
                    .base_resp
                    .as_ref()
                    .and_then(|b| b.status_msg.clone())
                    .unwrap_or_else(|| "unknown error".into());
                eprintln!("MiniMax Token Plan ({}): {msg}", site.label);
                continue;
            }

            let remains = resp.model_remains.as_deref().unwrap_or(&[]);
            let metrics = build_metrics(remains);
            // Skip sites with no renderable windows so we don't emit a bare header row.
            if metrics.is_empty() {
                continue;
            }
            outputs.push(UsageOutput {
                provider: "MiniMax Token Plan".into(),
                account: Some(UsageAccount {
                    id: site.label.to_string(),
                    label: Some(site.label.to_string()),
                    is_active: true,
                }),
                plan: None,
                email: None,
                metrics,
                reset_credits: None,
                credit_status: None,
                spend_control: None,
            });
        }

        Ok(outputs)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real token_plan/remains payload (status_code 0 = success). Each model has
    // both an interval (short) and weekly window, measured by remaining_percent.
    const SAMPLE: &str = r#"{"model_remains":[{"start_time":1781834400000,"end_time":1781852400000,"remains_time":16640796,"current_interval_total_count":0,"current_interval_usage_count":0,"model_name":"general","current_weekly_total_count":0,"current_weekly_usage_count":0,"weekly_start_time":1781452800000,"weekly_end_time":1782057600000,"weekly_remains_time":221840796,"current_interval_status":1,"current_interval_remaining_percent":98,"current_weekly_status":1,"current_weekly_remaining_percent":67,"weekly_boost_permille":1500},{"start_time":1781798400000,"end_time":1781884800000,"remains_time":49040796,"current_interval_total_count":3,"current_interval_usage_count":0,"model_name":"video","current_weekly_total_count":21,"current_weekly_usage_count":0,"weekly_start_time":1781452800000,"weekly_end_time":1782057600000,"weekly_remains_time":221840796,"current_interval_status":1,"current_interval_remaining_percent":100,"current_weekly_status":1,"current_weekly_remaining_percent":100}],"base_resp":{"status_code":0,"status_msg":"success"}}"#;

    #[test]
    fn builds_interval_and_weekly_metrics_from_token_plan_response() {
        let resp: ApiResponse = serde_json::from_str(SAMPLE).unwrap();
        let metrics = build_metrics(resp.model_remains.as_deref().unwrap_or(&[]));

        // 2 models x 2 windows (interval + weekly)
        assert_eq!(metrics.len(), 4);

        // general interval: 98% remaining -> 2% used, resets at end_time (2026)
        assert_eq!(metrics[0].label, "general");
        assert_eq!(metrics[0].remaining_percent, 98.0);
        assert_eq!(metrics[0].used_percent, 2.0);
        assert!(metrics[0].resets_at.as_deref().unwrap().contains("2026"));

        // general weekly: 67% remaining -> 33% used
        assert_eq!(metrics[1].label, "general·wk");
        assert_eq!(metrics[1].remaining_percent, 67.0);
        assert_eq!(metrics[1].used_percent, 33.0);

        // video interval: 100% remaining
        assert_eq!(metrics[2].label, "video");
        assert_eq!(metrics[2].remaining_percent, 100.0);
        assert_eq!(metrics[2].used_percent, 0.0);

        // video weekly: 100% remaining
        assert_eq!(metrics[3].label, "video·wk");
        assert_eq!(metrics[3].remaining_percent, 100.0);
    }

    #[test]
    fn flags_non_zero_status_code_as_api_error() {
        let ok: ApiResponse =
            serde_json::from_str(r#"{"base_resp":{"status_code":0,"status_msg":"success"}}"#)
                .unwrap();
        assert!(!is_api_error(&ok));
        assert!(!is_auth_error(&ok));

        let unauthorized: ApiResponse = serde_json::from_str(
            r#"{"base_resp":{"status_code":1004,"status_msg":"unauthorized"}}"#,
        )
        .unwrap();
        assert!(is_api_error(&unauthorized));
        assert!(is_auth_error(&unauthorized));
    }

    #[test]
    fn omits_window_when_its_percent_is_absent() {
        let resp: ApiResponse = serde_json::from_str(
            r#"{"model_remains":[{"model_name":"general","current_interval_remaining_percent":50}],"base_resp":{"status_code":0}}"#,
        )
        .unwrap();
        let metrics = build_metrics(resp.model_remains.as_deref().unwrap_or(&[]));

        // Only the interval window is present -> a single metric, no weekly row.
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].label, "general");
        assert_eq!(metrics[0].remaining_percent, 50.0);
    }

    #[test]
    fn skips_weekly_window_when_its_status_is_inactive() {
        // Old plans without a weekly limit may still return weekly fields (e.g.
        // percent 0), but current_weekly_status signals the window is inactive.
        let resp: ApiResponse = serde_json::from_str(
            r#"{"model_remains":[{"model_name":"general","current_interval_remaining_percent":80,"current_weekly_status":0,"current_weekly_remaining_percent":0}],"base_resp":{"status_code":0}}"#,
        )
        .unwrap();
        let metrics = build_metrics(resp.model_remains.as_deref().unwrap_or(&[]));

        // Interval is active; weekly must be suppressed despite a percent being
        // present, so it never reads as "0% left / 100% used".
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].label, "general");
        assert_eq!(metrics[0].remaining_percent, 80.0);
    }

    #[test]
    fn treats_seconds_and_millis_epochs_equivalently() {
        // The seconds-vs-ms heuristic must scale a seconds-scale epoch up by
        // 1000 so it matches the same instant expressed in milliseconds.
        let seconds = epoch_ms_to_rfc3339(1_781_852_400).unwrap();
        let millis = epoch_ms_to_rfc3339(1_781_852_400_000).unwrap();
        assert_eq!(seconds, millis);
        assert!(seconds.contains("2026"));
    }
}
