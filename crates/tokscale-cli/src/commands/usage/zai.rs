use anyhow::Result;
use chrono::{TimeZone, Utc};
use serde::Deserialize;

use super::helpers::capitalize;
use super::{UsageMetric, UsageOutput};

#[derive(Debug, Deserialize)]
struct QuotaResp {
    data: Option<QuotaData>,
}

#[derive(Debug, Deserialize)]
struct QuotaData {
    limits: Option<Vec<Limit>>,
    level: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Limit {
    #[serde(rename = "type")]
    limit_type: Option<String>,
    #[allow(dead_code)]
    usage: Option<f64>,
    remaining: Option<f64>,
    percentage: Option<f64>,
    #[allow(dead_code)]
    current_value: Option<f64>,
    number: Option<i64>,
    unit: Option<i64>,
    #[serde(alias = "next_reset_time", alias = "nextResetTime")]
    next_reset_time: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubResp {
    data: Option<Vec<Sub>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sub {
    #[serde(alias = "product_name", alias = "productName")]
    product_name: Option<String>,
    #[serde(alias = "next_renew_time", alias = "nextRenewTime")]
    next_renew_time: Option<String>,
}

fn parse_reset_time_str(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
        return Some(s.to_string());
    }
    if let Ok(ts) = s.parse::<i64>() {
        let ms = if ts.abs() > 10_000_000_000 {
            ts
        } else {
            ts * 1000
        };
        return Utc
            .timestamp_millis_opt(ms)
            .single()
            .map(|dt| dt.to_rfc3339());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        if let Some(dt) = d.and_hms_opt(0, 0, 0) {
            return Some(chrono::DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc).to_rfc3339());
        }
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(chrono::DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc).to_rfc3339());
    }
    Some(s.to_string())
}

fn parse_reset_time(val: Option<&serde_json::Value>) -> Option<String> {
    let val = val?;
    match val {
        serde_json::Value::Number(n) => {
            let ts = n.as_i64()?;
            let ms = if ts.abs() > 10_000_000_000 {
                ts
            } else {
                ts * 1000
            };
            Utc.timestamp_millis_opt(ms)
                .single()
                .map(|dt| dt.to_rfc3339())
        }
        serde_json::Value::String(s) => parse_reset_time_str(s),
        _ => None,
    }
}

async fn fetch_quota(client: &reqwest::Client, key: &str) -> Result<QuotaResp> {
    let resp = client
        .get("https://api.z.ai/api/monitor/usage/quota/limit")
        .header("Authorization", format!("Bearer {key}"))
        .header("Accept", "application/json")
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Z.ai quota request failed (HTTP {})", resp.status());
    }
    Ok(resp.json().await?)
}

async fn fetch_sub(client: &reqwest::Client, key: &str) -> Result<SubResp> {
    let resp = client
        .get("https://api.z.ai/api/biz/subscription/list")
        .header("Authorization", format!("Bearer {key}"))
        .header("Accept", "application/json")
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Z.ai subscription request failed (HTTP {})", resp.status());
    }
    Ok(resp.json().await?)
}

pub fn has_credentials() -> bool {
    std::env::var("ZAI_API_KEY")
        .or_else(|_| std::env::var("GLM_API_KEY"))
        .is_ok()
}

/// Translate Z.ai's limit windows into the session/weekly/web-search metrics
/// tokscale surfaces.
///
/// Z.ai encodes each window as an opaque `(unit, number)` code pair rather than
/// a name: `(3, 5)` is the 5-hour rolling session window and `(6, 1)` is the
/// 1-week window. Unrecognized codes are skipped rather than guessed at.
fn build_metrics(
    limits: &[Limit],
    search_reset: Option<String>,
    session_metric: &mut Option<UsageMetric>,
    weekly_metric: &mut Option<UsageMetric>,
    search_metric: &mut Option<UsageMetric>,
) {
    for limit in limits.iter() {
        // Skip limits with no percentage rather than fabricating
        // "0% used / 100% left" from a missing field.
        let pct = match limit.percentage {
            Some(p) => p.clamp(0.0, 100.0),
            None => continue,
        };

        match limit.limit_type.as_deref() {
            // V3 GLM Coding plans report the same (unit, number)
            // windows as CREDIT_LIMIT instead of TOKENS_LIMIT. Prefer
            // CREDIT_LIMIT so a plan that ever emits both for one
            // window cannot silently last-write-wins.
            Some("TOKENS_LIMIT") | Some("CREDIT_LIMIT") => {
                let prefer = limit.limit_type.as_deref() == Some("CREDIT_LIMIT");
                let (target, label) = match (limit.unit, limit.number) {
                    (Some(3), Some(5)) => (&mut *session_metric, "Session"),
                    (Some(6), Some(1)) => (&mut *weekly_metric, "Weekly"),
                    _ => continue,
                };
                let resets_at = parse_reset_time(limit.next_reset_time.as_ref());
                if target.is_none() || prefer {
                    let existing_resets_at = target.as_ref().and_then(|m| m.resets_at.clone());
                    *target = Some(UsageMetric {
                        label: label.to_string(),
                        used_percent: pct,
                        remaining_percent: 100.0 - pct,
                        remaining_label: None,
                        resets_at: resets_at.or(existing_resets_at),
                    });
                } else if let Some(ref mut metric) = target {
                    if metric.resets_at.is_none() && resets_at.is_some() {
                        metric.resets_at = resets_at;
                    }
                }
            }
            Some("TIME_LIMIT") => {
                let remaining_label = limit.remaining.map(|r| format!("{:.0} left", r));
                let resets_at = parse_reset_time(limit.next_reset_time.as_ref())
                    .or_else(|| search_reset.clone());
                *search_metric = Some(UsageMetric {
                    label: "Web Search".into(),
                    used_percent: pct,
                    remaining_percent: 100.0 - pct,
                    remaining_label,
                    resets_at,
                });
            }
            _ => {}
        }
    }
}

pub fn fetch() -> Result<UsageOutput> {
    let api_key = std::env::var("ZAI_API_KEY")
        .or_else(|_| std::env::var("GLM_API_KEY"))
        .map_err(|_| anyhow::anyhow!("No ZAI_API_KEY or GLM_API_KEY set."))?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = tokscale_core::http::client();
        let quota = fetch_quota(&client, &api_key).await?;
        let sub = fetch_sub(&client, &api_key).await.ok();

        let base_plan = sub
            .as_ref()
            .and_then(|s| s.data.as_ref())
            .and_then(|d| d.first())
            .and_then(|s| s.product_name.clone())
            .or_else(|| {
                quota
                    .data
                    .as_ref()
                    .and_then(|d| d.level.clone())
                    .map(|l| capitalize(&l))
            });

        let next_renew = sub
            .as_ref()
            .and_then(|s| s.data.as_ref())
            .and_then(|d| d.first())
            .and_then(|s| s.next_renew_time.clone());

        let plan = match (base_plan, next_renew.as_deref()) {
            (Some(tier), Some(renew)) if !renew.trim().is_empty() => {
                Some(format!("{tier} (renews {})", renew.trim()))
            }
            (Some(tier), _) => Some(tier),
            (None, Some(renew)) if !renew.trim().is_empty() => {
                Some(format!("Renews {}", renew.trim()))
            }
            (None, _) => None,
        };

        let mut session_metric = None;
        let mut weekly_metric = None;
        let mut search_metric = None;

        let search_reset = next_renew.as_deref().and_then(parse_reset_time_str);

        if let Some(limits) = quota.data.as_ref().and_then(|d| d.limits.as_ref()) {
            build_metrics(
                limits,
                search_reset,
                &mut session_metric,
                &mut weekly_metric,
                &mut search_metric,
            );
        }

        let mut metrics = Vec::new();
        if let Some(m) = session_metric {
            metrics.push(m);
        }
        if let Some(m) = weekly_metric {
            metrics.push(m);
        }
        if let Some(m) = search_metric {
            metrics.push(m);
        }

        Ok(UsageOutput {
            provider: "Z.ai".into(),
            account: None,
            credential_source: None,
            plan,
            email: None,
            metrics,
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(kind: &str, unit: i64, number: i64, percentage: f64) -> Limit {
        serde_json::from_value(serde_json::json!({
            "type": kind,
            "unit": unit,
            "number": number,
            "percentage": percentage,
        }))
        .expect("valid Limit fixture")
    }

    fn run(
        limits: &[Limit],
    ) -> (
        Option<UsageMetric>,
        Option<UsageMetric>,
        Option<UsageMetric>,
    ) {
        let mut session = None;
        let mut weekly = None;
        let mut search = None;
        build_metrics(limits, None, &mut session, &mut weekly, &mut search);
        (session, weekly, search)
    }

    #[test]
    fn credit_limit_session_window_maps_to_session_metric() {
        let (session, weekly, _) = run(&[limit("CREDIT_LIMIT", 3, 5, 40.0)]);
        let session = session.expect("session metric present");
        assert_eq!(session.label, "Session");
        assert_eq!(session.used_percent, 40.0);
        assert_eq!(session.remaining_percent, 60.0);
        assert!(weekly.is_none());
    }

    #[test]
    fn credit_limit_weekly_window_maps_to_weekly_metric() {
        let (session, weekly, _) = run(&[limit("CREDIT_LIMIT", 6, 1, 25.0)]);
        let weekly = weekly.expect("weekly metric present");
        assert_eq!(weekly.label, "Weekly");
        assert_eq!(weekly.used_percent, 25.0);
        assert!(session.is_none());
    }

    #[test]
    fn credit_limit_is_preferred_over_tokens_limit_for_same_window() {
        // TOKENS_LIMIT first, then CREDIT_LIMIT: credit must win.
        let (session, _, _) = run(&[
            limit("TOKENS_LIMIT", 3, 5, 10.0),
            limit("CREDIT_LIMIT", 3, 5, 90.0),
        ]);
        assert_eq!(session.expect("session metric present").used_percent, 90.0);

        // Reversed order: CREDIT_LIMIT must not be clobbered by a later
        // TOKENS_LIMIT for the same window.
        let (session, _, _) = run(&[
            limit("CREDIT_LIMIT", 3, 5, 90.0),
            limit("TOKENS_LIMIT", 3, 5, 10.0),
        ]);
        assert_eq!(session.expect("session metric present").used_percent, 90.0);
    }

    #[test]
    fn unrecognized_window_codes_are_skipped() {
        let (session, weekly, search) = run(&[limit("CREDIT_LIMIT", 9, 9, 50.0)]);
        assert!(session.is_none());
        assert!(weekly.is_none());
        assert!(search.is_none());
    }

    #[test]
    fn credit_limit_with_next_reset_time_sets_resets_at() {
        let lim: Limit = serde_json::from_value(serde_json::json!({
            "type": "CREDIT_LIMIT",
            "unit": 6,
            "number": 1,
            "percentage": 100.0,
            "nextResetTime": 1788701854998i64,
        }))
        .expect("valid limit with nextResetTime");

        let (_, weekly, _) = run(&[lim]);
        let weekly = weekly.expect("weekly metric present");
        assert_eq!(weekly.used_percent, 100.0);
        assert_eq!(weekly.remaining_percent, 0.0);
        assert_eq!(
            weekly.resets_at.as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );
    }

    #[test]
    fn credit_limit_preserves_resets_at_from_tokens_limit_fallback() {
        let tokens_lim: Limit = serde_json::from_value(serde_json::json!({
            "type": "TOKENS_LIMIT",
            "unit": 3,
            "number": 5,
            "percentage": 20.0,
            "nextResetTime": 1788701854998i64,
        }))
        .expect("tokens limit");
        let credit_lim: Limit = serde_json::from_value(serde_json::json!({
            "type": "CREDIT_LIMIT",
            "unit": 3,
            "number": 5,
            "percentage": 80.0,
        }))
        .expect("credit limit without reset");

        let (session, _, _) = run(&[tokens_lim, credit_lim]);
        let session = session.expect("session metric present");
        assert_eq!(session.used_percent, 80.0);
        assert_eq!(
            session.resets_at.as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );
    }

    #[test]
    fn sub_resp_deserializes_camel_case_fields() {
        let json = serde_json::json!({
            "data": [{
                "id": "845544",
                "productName": "GLM Coding Max",
                "nextRenewTime": "2026-11-30"
            }]
        });
        let sub_resp: SubResp = serde_json::from_value(json).expect("valid SubResp");
        let sub = sub_resp.data.as_ref().unwrap().first().unwrap();
        assert_eq!(sub.product_name.as_deref(), Some("GLM Coding Max"));
        assert_eq!(sub.next_renew_time.as_deref(), Some("2026-11-30"));
    }

    #[test]
    fn parse_reset_time_handles_dates_and_epoch() {
        let val_num = serde_json::json!(1788701854998i64);
        assert_eq!(
            parse_reset_time(Some(&val_num)).as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );

        let val_str = serde_json::json!("1788701854998");
        assert_eq!(
            parse_reset_time(Some(&val_str)).as_deref(),
            Some("2026-09-06T13:37:34.998+00:00")
        );

        let val_date = serde_json::json!("2026-11-30");
        assert_eq!(
            parse_reset_time(Some(&val_date)).as_deref(),
            Some("2026-11-30T00:00:00+00:00")
        );
    }
}
