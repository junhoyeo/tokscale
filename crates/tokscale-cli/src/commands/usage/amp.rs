use anyhow::Result;
use serde::Deserialize;

use super::{UsageMetric, UsageOutput};

#[derive(Debug, Deserialize)]
struct Secrets {
    #[serde(rename = "apiKey@https://ampcode.com/")]
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    #[allow(dead_code)]
    ok: Option<bool>,
    result: Option<ApiResult>,
}

#[derive(Debug, Deserialize)]
struct ApiResult {
    display_text: Option<String>,
}

fn read_credentials() -> Result<String> {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    let path = home
        .join(".local")
        .join("share")
        .join("amp")
        .join("secrets.json");
    if !path.exists() {
        anyhow::bail!("No Amp credentials found. Run 'amp' to log in.");
    }
    let content = std::fs::read_to_string(&path)?;
    let secrets: Secrets = serde_json::from_str(&content)?;
    secrets
        .api_key
        .ok_or_else(|| anyhow::anyhow!("No Amp API key in secrets.json"))
}

/// Parse a dollar amount like "$4.50" or "$1,200.00" from text starting at the given prefix.
fn parse_dollar_after(text: &str, prefix: &str) -> Option<f64> {
    let start = text.find(prefix)? + prefix.len();
    let rest = &text[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != ',')
        .unwrap_or(rest.len());
    let num_str = &rest[..end];
    num_str.replace(',', "").parse().ok()
}

fn parse_display_text(text: &str) -> Vec<UsageMetric> {
    let mut metrics = Vec::new();

    // Parse free tier: "$X/$Y remaining"
    // Look for pattern like "$4.50/$20.00 remaining"
    if let Some(slash_pos) = text.find("/$") {
        if let Some(dollar_before) = text[..slash_pos].rfind('$') {
            let before = &text[dollar_before + 1..slash_pos];
            if let Ok(remaining) = before.replace(',', "").parse::<f64>() {
                // Find the total after /$
                let after = &text[slash_pos + 2..];
                if let Some(space_pos) = after.find(|c: char| c.is_ascii_whitespace()) {
                    if let Ok(total) = after[..space_pos].replace(',', "").parse::<f64>() {
                        if total > 0.0 && total.is_finite() && remaining.is_finite() {
                            let used = (total - remaining).max(0.0);
                            let used_pct = if used.is_finite() {
                                (used / total * 100.0).clamp(0.0, 100.0)
                            } else {
                                0.0
                            };
                            let remaining_pct = (100.0 - used_pct).clamp(0.0, 100.0);
                            let mut resets_at = None;

                            // Estimate reset time from hourly replenish rate
                            if let Some(rate) = parse_dollar_after(text, "+$") {
                                if rate > 0.0 && used > 0.0 && rate.is_finite() {
                                    let secs = (used / rate * 3600.0) as i64;
                                    let resets =
                                        chrono::Utc::now() + chrono::Duration::seconds(secs);
                                    resets_at = Some(resets.to_rfc3339());
                                }
                            }

                            metrics.push(UsageMetric {
                                label: "Free".into(),
                                used_percent: used_pct,
                                remaining_percent: remaining_pct,
                                remaining_label: Some(format!("${remaining:.2}/${total:.2}")),
                                resets_at,
                            });
                        }
                    }
                }
            }
        }
    }

    // Parse credits: "Individual credits: $X remaining"
    if let Some(credits) = parse_dollar_after(text, "Individual credits: $") {
        metrics.push(UsageMetric {
            label: "Credits".into(),
            used_percent: 0.0,
            remaining_percent: 100.0,
            remaining_label: Some(format!("${credits:.2} left")),
            resets_at: None,
        });
    }

    metrics
}

fn detect_plan(metrics: &[UsageMetric]) -> Option<String> {
    let has_free = metrics.iter().any(|m| m.label == "Free");
    let has_credits = metrics.iter().any(|m| m.label == "Credits");
    match (has_free, has_credits) {
        (true, _) => Some("Free".into()),
        (false, true) => Some("Credits".into()),
        _ => None,
    }
}

pub fn has_credentials() -> bool {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".local")
        .join("share")
        .join("amp")
        .join("secrets.json")
        .exists()
}

pub fn fetch() -> Result<UsageOutput> {
    let api_key = read_credentials()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::new();
        let resp = client
            .post("https://ampcode.com/api/internal")
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "method": "userDisplayBalanceInfo",
                "params": {}
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            anyhow::bail!("Amp usage request failed (HTTP {})", resp.status());
        }

        let body: ApiResponse = resp.json().await?;
        if body.ok == Some(false) {
            let msg = body
                .result
                .as_ref()
                .and_then(|r| r.display_text.as_deref())
                .unwrap_or("unknown error");
            anyhow::bail!("Amp API returned an error: {msg}");
        }
        let display_text = body.result.and_then(|r| r.display_text).unwrap_or_default();

        let metrics = parse_display_text(&display_text);
        let plan = detect_plan(&metrics);

        Ok(UsageOutput {
            provider: "Amp".into(),
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
