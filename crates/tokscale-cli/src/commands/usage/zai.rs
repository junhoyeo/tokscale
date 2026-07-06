use anyhow::Result;
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
}

#[derive(Debug, Deserialize)]
struct SubResp {
    data: Option<Vec<Sub>>,
}

#[derive(Debug, Deserialize)]
struct Sub {
    product_name: Option<String>,
    next_renew_time: Option<String>,
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

pub fn fetch() -> Result<UsageOutput> {
    let api_key = std::env::var("ZAI_API_KEY")
        .or_else(|_| std::env::var("GLM_API_KEY"))
        .map_err(|_| anyhow::anyhow!("No ZAI_API_KEY or GLM_API_KEY set."))?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::new();
        let quota = fetch_quota(&client, &api_key).await?;
        let sub = fetch_sub(&client, &api_key).await.ok();

        let plan = sub
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

        let mut session_metric = None;
        let mut weekly_metric = None;
        let mut search_metric = None;

        if let Some(limits) = quota.data.as_ref().and_then(|d| d.limits.as_ref()) {
            for limit in limits.iter() {
                // Skip limits with no percentage rather than fabricating
                // "0% used / 100% left" from a missing field.
                let pct = match limit.percentage {
                    Some(p) => p.clamp(0.0, 100.0),
                    None => continue,
                };

                match limit.limit_type.as_deref() {
                    Some("TOKENS_LIMIT") => {
                        let metric = UsageMetric {
                            label: String::new(),
                            used_percent: pct,
                            remaining_percent: 100.0 - pct,
                            remaining_label: None,
                            resets_at: None,
                        };
                        match (limit.unit, limit.number) {
                            (Some(3), Some(5)) => {
                                session_metric = Some(UsageMetric {
                                    label: "Session".into(),
                                    ..metric
                                });
                            }
                            (Some(6), Some(1)) => {
                                weekly_metric = Some(UsageMetric {
                                    label: "Weekly".into(),
                                    ..metric
                                });
                            }
                            _ => {}
                        }
                    }
                    Some("TIME_LIMIT") => {
                        let remaining_label = limit.remaining.map(|r| format!("{:.0} left", r));
                        search_metric = Some(UsageMetric {
                            label: "Web Search".into(),
                            used_percent: pct,
                            remaining_percent: 100.0 - pct,
                            remaining_label,
                            resets_at: sub
                                .as_ref()
                                .and_then(|s| s.data.as_ref())
                                .and_then(|d| d.first())
                                .and_then(|s| s.next_renew_time.clone()),
                        });
                    }
                    _ => {}
                }
            }
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
            plan,
            email: None,
            metrics,
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        })
    })
}
