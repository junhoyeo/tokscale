use anyhow::Result;
use chrono::{TimeZone, Utc};
use serde::Deserialize;

use super::helpers::capitalize;
use super::{UsageMetric, UsageOutput};

const MODEL_CALLS_PER_PROMPT: i64 = 15;

#[derive(Debug, Deserialize)]
struct ApiResponse {
    base_resp: Option<BaseResp>,
    model_remains: Option<Vec<ModelRemains>>,
    data: Option<ApiData>,
    current_subscribe_title: Option<String>,
    plan_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BaseResp {
    status_code: Option<i64>,
    status_msg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiData {
    model_remains: Option<Vec<ModelRemains>>,
    current_subscribe_title: Option<String>,
    plan_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelRemains {
    current_interval_total_count: Option<i64>,
    current_interval_usage_count: Option<i64>,
    current_interval_remaining_count: Option<i64>,
    current_interval_used_count: Option<i64>,
    current_subscribe_title: Option<String>,
    #[allow(dead_code)]
    start_time: Option<i64>,
    end_time: Option<i64>,
    remains_time: Option<i64>,
}

fn read_api_key() -> Result<String> {
    std::env::var("MINIMAX_API_KEY")
        .or_else(|_| std::env::var("MINIMAX_API_TOKEN"))
        .map_err(|_| anyhow::anyhow!("No MINIMAX_API_KEY or MINIMAX_API_TOKEN set."))
}

fn is_auth_error(resp: &ApiResponse) -> bool {
    if let Some(ref base) = resp.base_resp {
        if base.status_code == Some(1004) {
            return true;
        }
        if let Some(ref msg) = base.status_msg {
            let lower = msg.to_lowercase();
            if lower.contains("cookie") || lower.contains("log in") || lower.contains("login") {
                return true;
            }
        }
    }
    false
}

fn is_api_error(resp: &ApiResponse) -> bool {
    if let Some(ref base) = resp.base_resp {
        if base.status_code.unwrap_or(0) != 0 {
            return true;
        }
    }
    false
}

fn normalize_plan_name(raw: &str) -> String {
    let without_prefix = raw
        .trim_start_matches("MiniMax Coding Plan")
        .trim()
        .trim_start_matches(':')
        .trim_start_matches('-')
        .trim();
    if without_prefix.is_empty() {
        capitalize(raw.trim())
    } else {
        capitalize(without_prefix)
    }
}

fn infer_plan(total: i64) -> Option<String> {
    let prompt_limit = if total % MODEL_CALLS_PER_PROMPT == 0 {
        total / MODEL_CALLS_PER_PROMPT
    } else {
        total
    };
    Some(match prompt_limit {
        100 => "Starter".into(),
        300 => "Plus".into(),
        1000 => "Max".into(),
        2000 => "Ultra".into(),
        _ => return None,
    })
}

fn epoch_to_ms(ts: i64) -> i64 {
    if ts.abs() > 10_000_000_000 {
        ts
    } else {
        ts * 1000
    }
}

fn parse_end_time(ts: i64) -> String {
    let ms = epoch_to_ms(ts);
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| ts.to_string())
}

async fn fetch_api(client: &reqwest::Client, key: &str) -> Result<ApiResponse> {
    let resp = client
        .get("https://api.minimax.io/v1/api/openplatform/coding_plan/remains")
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .send()
        .await?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("Session expired. Check your MiniMax API key.");
    }
    if !status.is_success() {
        anyhow::bail!("MiniMax usage request failed (HTTP {status})");
    }
    Ok(resp.json().await?)
}

pub fn has_credentials() -> bool {
    std::env::var("MINIMAX_API_KEY")
        .or_else(|_| std::env::var("MINIMAX_API_TOKEN"))
        .is_ok()
}

pub fn fetch() -> Result<UsageOutput> {
    let api_key = read_api_key()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::new();
        let resp = fetch_api(&client, &api_key).await?;

        if is_auth_error(&resp) {
            anyhow::bail!("Session expired. Check your MiniMax API key.");
        }
        if is_api_error(&resp) {
            let msg = resp
                .base_resp
                .as_ref()
                .and_then(|b| b.status_msg.clone())
                .unwrap_or_else(|| "Unknown error".into());
            anyhow::bail!("MiniMax API error: {msg}");
        }

        // model_remains can be top-level or nested under "data"
        let remains = resp
            .model_remains
            .as_ref()
            .or_else(|| resp.data.as_ref().and_then(|d| d.model_remains.as_ref()))
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        // Pick the first entry with a valid total
        let chosen = remains
            .iter()
            .find(|m| m.current_interval_total_count.unwrap_or(0) > 0);

        let mut metrics = Vec::new();
        let mut plan: Option<String> = resp
            .data
            .as_ref()
            .and_then(|d| d.current_subscribe_title.as_ref().or(d.plan_name.as_ref()))
            .or_else(|| {
                resp.current_subscribe_title
                    .as_ref()
                    .or(resp.plan_name.as_ref())
            })
            .map(|s| normalize_plan_name(s));

        if let Some(model) = chosen {
            if plan.is_none() {
                plan = model
                    .current_subscribe_title
                    .as_ref()
                    .map(|s| normalize_plan_name(s));
            }

            let total = model.current_interval_total_count.unwrap_or(0);

            // Prefer explicit used_count, then compute from remaining
            let used = model
                .current_interval_used_count
                .map(|u| u.clamp(0, total))
                .unwrap_or_else(|| {
                    // Both remaining_count and usage_count represent remaining prompts
                    let remaining = model
                        .current_interval_remaining_count
                        .or(model.current_interval_usage_count)
                        .unwrap_or(0);
                    (total - remaining).max(0)
                });

            let used_pct = if total > 0 {
                (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };

            // Reset time: prefer end_time, fallback to remains_time
            let resets_at = model.end_time.map(parse_end_time).or_else(|| {
                model.remains_time.map(|rt| {
                    let ms = if rt > 1_000_000_000 { rt } else { rt * 1000 };
                    let dt = Utc::now() + chrono::Duration::milliseconds(ms);
                    dt.to_rfc3339()
                })
            });

            metrics.push(UsageMetric {
                label: "Session".into(),
                used_percent: used_pct,
                remaining_percent: 100.0 - used_pct,
                remaining_label: Some(format!("{}/{} prompts left", total - used, total)),
                resets_at,
            });

            if plan.is_none() {
                plan = infer_plan(total);
            }
        }

        Ok(UsageOutput {
            provider: "MiniMax".into(),
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
