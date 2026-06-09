use anyhow::Result;
use serde::Deserialize;

use super::helpers::capitalize;
use super::{UsageMetric, UsageOutput};

const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";

#[derive(Debug, Deserialize)]
struct Credentials {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_at: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    usage: Option<QuotaDetail>,
    limits: Option<Vec<LimitEntry>>,
    user: Option<UserInfo>,
}

#[derive(Debug, Deserialize)]
struct QuotaDetail {
    limit: Option<String>,
    remaining: Option<String>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LimitEntry {
    window: Option<LimitWindow>,
    detail: Option<QuotaDetail>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct LimitWindow {
    duration: Option<i64>,
    time_unit: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    membership: Option<Membership>,
}

#[derive(Debug, Deserialize)]
struct Membership {
    level: Option<String>,
}

fn credentials_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();

    // 1) kimi-code (supports KIMI_CODE_HOME override)
    let kimi_code_home = std::env::var("KIMI_CODE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .map(|h| h.join(".kimi-code"))
                .unwrap_or_else(|| std::path::PathBuf::from("."))
        });
    paths.push(kimi_code_home.join("credentials").join("kimi-code.json"));

    // 2) kimi-cli
    if let Some(home) = dirs::home_dir() {
        paths.push(
            home.join(".kimi")
                .join("credentials")
                .join("kimi-code.json"),
        );
    }

    paths
}

fn read_credentials() -> Result<(Credentials, std::path::PathBuf)> {
    for path in credentials_paths() {
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            return Ok((serde_json::from_str(&content)?, path));
        }
    }
    anyhow::bail!("No Kimi credentials found. Run 'kimi' to log in.")
}

fn save_credentials(
    path: &std::path::Path,
    access_token: &str,
    refresh_token: &str,
    expires_in: i64,
) {
    let expires_at = chrono::Utc::now().timestamp() as f64 + expires_in as f64;
    let json = serde_json::json!({
        "access_token": access_token,
        "refresh_token": refresh_token,
        "expires_at": expires_at,
        "scope": "kimi-code",
        "token_type": "Bearer"
    });
    let content = match serde_json::to_string_pretty(&json) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("warning: failed to serialize Kimi credentials: {e}");
            return;
        }
    };
    if let Err(e) = super::helpers::atomic_write_secret(path, content.as_bytes()) {
        eprintln!("warning: failed to save Kimi credentials: {e}");
    }
}

fn needs_refresh(expires_at: Option<f64>) -> bool {
    if let Some(expires_at) = expires_at {
        let now = chrono::Utc::now().timestamp() as f64;
        now + 300.0 > expires_at // 5 min buffer
    } else {
        false
    }
}

async fn refresh_token(client: &reqwest::Client, rt: &str) -> Result<RefreshResponse> {
    let resp = client
        .post("https://auth.kimi.com/api/oauth/token")
        .form(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", rt),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("Kimi token refresh failed (HTTP {})", resp.status());
    }
    Ok(resp.json().await?)
}

async fn fetch_usage(client: &reqwest::Client, token: &str) -> Result<UsageResponse> {
    let resp = client
        .get("https://api.kimi.com/coding/v1/usages")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .header("User-Agent", "OpenUsage")
        .send()
        .await?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("NEEDS_AUTH");
    }
    if !status.is_success() {
        anyhow::bail!("Kimi usage request failed (HTTP {status})");
    }
    Ok(resp.json().await?)
}

fn parse_quota_detail(label: &str, detail: &QuotaDetail) -> Option<UsageMetric> {
    let limit: i64 = detail.limit.as_ref()?.parse().ok()?;
    let remaining: i64 = detail.remaining.as_ref()?.parse().ok()?;
    if limit <= 0 {
        return None;
    }
    let used = (limit - remaining).max(0);
    let used_pct = (used as f64 / limit as f64 * 100.0).clamp(0.0, 100.0);
    Some(UsageMetric {
        label: label.into(),
        used_percent: used_pct,
        remaining_percent: 100.0 - used_pct,
        remaining_label: Some(format!("{remaining}/{limit} left")),
        resets_at: detail.reset_time.clone(),
    })
}

pub fn has_credentials() -> bool {
    credentials_paths().iter().any(|p| p.exists())
}

pub fn fetch() -> Result<UsageOutput> {
    let (creds, creds_path) = read_credentials()?;
    let mut access_token = creds
        .access_token
        .clone()
        .ok_or_else(|| anyhow::anyhow!("No Kimi access token."))?;
    let mut stored_refresh_token = creds.refresh_token.clone();
    let expires_at = creds.expires_at;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::new();

        // Proactive refresh if token is about to expire
        if needs_refresh(expires_at) {
            if let Some(ref rt_str) = stored_refresh_token {
                if let Ok(refreshed) = refresh_token(&client, rt_str).await {
                    if let Some(new_token) = refreshed.access_token.clone() {
                        access_token = new_token;
                        if let (Some(new_rt), Some(expires_in)) =
                            (&refreshed.refresh_token, refreshed.expires_in)
                        {
                            stored_refresh_token = Some(new_rt.clone());
                            save_credentials(&creds_path, &access_token, new_rt, expires_in);
                        }
                    }
                }
            }
        }

        let resp = match fetch_usage(&client, &access_token).await {
            Ok(r) => r,
            Err(e) if e.to_string().contains("NEEDS_AUTH") => {
                let rt_str = stored_refresh_token
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("No refresh token."))?;
                let refreshed = refresh_token(&client, rt_str).await?;
                let new = refreshed
                    .access_token
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Refresh returned no token."))?;
                if let (Some(new_rt), Some(expires_in)) =
                    (&refreshed.refresh_token, refreshed.expires_in)
                {
                    save_credentials(&creds_path, &new, new_rt, expires_in);
                }
                fetch_usage(&client, &new).await?
            }
            Err(e) => return Err(e),
        };

        let plan = resp
            .user
            .as_ref()
            .and_then(|u| u.membership.as_ref())
            .and_then(|m| m.level.as_ref())
            .map(|l| capitalize(l.trim_start_matches("LEVEL_").replace('_', " ").as_str()));

        let mut metrics = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Parse limits[] — use window duration to determine label
        if let Some(ref limits) = resp.limits {
            for entry in limits.iter() {
                if let Some(ref detail) = entry.detail {
                    let label = match entry.window.as_ref().and_then(|w| w.duration) {
                        Some(d) if d <= 3600 => "Session",
                        _ => "Weekly",
                    };
                    if let Some(metric) = parse_quota_detail(label, detail) {
                        let key = format!(
                            "{}:{}:{}",
                            label,
                            metric.used_percent,
                            metric.remaining_label.as_deref().unwrap_or("")
                        );
                        if seen.insert(key) {
                            metrics.push(metric);
                        }
                    }
                }
            }
        }

        // Parse top-level usage as "Weekly" (deduplicate against session)
        if let Some(ref usage) = resp.usage {
            if let Some(metric) = parse_quota_detail("Weekly", usage) {
                let key = format!(
                    "{}:{}:{}",
                    "Weekly",
                    metric.used_percent,
                    metric.remaining_label.as_deref().unwrap_or("")
                );
                if seen.insert(key) {
                    metrics.push(metric);
                }
            }
        }

        Ok(UsageOutput {
            provider: "Kimi".into(),
            account: None,
            plan,
            email: None,
            metrics,
        })
    })
}
