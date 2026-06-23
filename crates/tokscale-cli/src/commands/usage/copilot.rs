use anyhow::Result;
use serde::Deserialize;

use super::helpers::{capitalize, read_keychain};
use super::{UsageMetric, UsageOutput};

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct PaidQuotaSnapshot {
    percent_remaining: Option<i64>,
    remaining: Option<i64>,
    entitlement: Option<i64>,
    #[allow(dead_code)]
    quota_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct PaidResponse {
    copilot_plan: Option<String>,
    quota_reset_date: Option<String>,
    quota_snapshots: Option<std::collections::HashMap<String, PaidQuotaSnapshot>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FreeResponse {
    copilot_plan: Option<String>,
    limited_user_quotas: Option<std::collections::HashMap<String, i64>>,
    monthly_quotas: Option<std::collections::HashMap<String, i64>>,
    limited_user_reset_date: Option<String>,
}

fn read_token_from_keychain() -> Result<String> {
    let raw = read_keychain("gh:github.com")?;
    // go-keyring may base64-encode the value
    if let Some(encoded) = raw.strip_prefix("go-keyring-base64:") {
        let decoded = base64_decode(encoded)?;
        Ok(decoded)
    } else {
        Ok(raw)
    }
}

fn gh_config_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("GH_CONFIG_DIR") {
        return std::path::PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        return std::path::PathBuf::from(dir).join("gh");
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")))
            .join("GitHub CLI");
    }
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".config").join("gh")
}

fn parse_token_from_hosts() -> Result<String> {
    let path = gh_config_dir().join("hosts.yml");
    if !path.exists() {
        anyhow::bail!("No gh hosts file");
    }
    let content = std::fs::read_to_string(&path)?;
    // Parse YAML-like: look for "oauth_token: <token>" under "github.com:"
    let mut in_github = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "github.com:" {
            in_github = true;
            continue;
        }
        // A non-indented, non-empty, non-comment line starts a new section
        if in_github
            && !line.starts_with(' ')
            && !line.starts_with('\t')
            && !trimmed.is_empty()
            && !trimmed.starts_with('#')
        {
            in_github = false;
        }
        if in_github && trimmed.starts_with("oauth_token:") {
            let token = trimmed.trim_start_matches("oauth_token:").trim();
            if !token.is_empty() {
                return Ok(token.to_string());
            }
        }
    }
    anyhow::bail!("No oauth_token found in hosts.yml")
}

fn read_token_from_hosts() -> Result<String> {
    parse_token_from_hosts()
}

fn read_credentials() -> Result<String> {
    read_token_from_keychain()
        .or_else(|_| read_token_from_hosts())
        .map_err(|_| {
            anyhow::anyhow!(
                "No GitHub Copilot credentials found. Run 'gh auth login' to authenticate."
            )
        })
}

fn base64_decode(input: &str) -> Result<String> {
    // Minimal base64 decode without adding a dependency
    const TABLE: &[Option<u8>; 128] = &{
        let mut table = [None; 128];
        let mut i = 0u8;
        while i < 26 {
            table[(b'A' + i) as usize] = Some(i);
            i += 1;
        }
        let mut i = 0u8;
        while i < 26 {
            table[(b'a' + i) as usize] = Some(26 + i);
            i += 1;
        }
        let mut i = 0u8;
        while i < 10 {
            table[(b'0' + i) as usize] = Some(52 + i);
            i += 1;
        }
        table[b'+' as usize] = Some(62);
        table[b'/' as usize] = Some(63);
        table
    };

    let bytes = input.as_bytes();
    let mut result = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in bytes {
        if b == b'=' {
            break;
        }
        if (b as usize) >= TABLE.len() {
            continue;
        }
        if let Some(v) = TABLE[b as usize] {
            buf = (buf << 6) | v as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                result.push((buf >> bits) as u8);
            }
        }
    }
    Ok(String::from_utf8(result)?)
}

fn pretty_category(key: &str) -> String {
    match key {
        "premium_interactions" => "Premium".into(),
        "chat" => "Chat".into(),
        "completions" => "Completions".into(),
        other => capitalize(other.replace('_', " ").as_str()),
    }
}

async fn fetch_api(client: &reqwest::Client, token: &str) -> Result<serde_json::Value> {
    let resp = client
        .get("https://api.github.com/copilot_internal/user")
        .header("Authorization", format!("token {token}"))
        .header("Accept", "application/json")
        .header("Editor-Version", "vscode/1.96.2")
        .header("Editor-Plugin-Version", "copilot-chat/0.26.7")
        .header("User-Agent", "GitHubCopilotChat/0.26.7")
        .header("X-Github-Api-Version", "2025-04-01")
        .send()
        .await?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("NEEDS_AUTH");
    }
    if !status.is_success() {
        anyhow::bail!("Copilot usage request failed (HTTP {status})");
    }
    Ok(resp.json().await?)
}

pub fn has_credentials() -> bool {
    if super::helpers::read_keychain("gh:github.com").is_ok() {
        return true;
    }
    parse_token_from_hosts().is_ok()
}

pub fn fetch() -> Result<UsageOutput> {
    let token = read_credentials()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::new();
        let resp = fetch_api(&client, &token).await?;

        let plan = resp
            .get("copilot_plan")
            .and_then(|v| v.as_str())
            .map(capitalize);

        let mut metrics = Vec::new();

        // Try paid tier response (quota_snapshots)
        if let Some(snapshots) = resp.get("quota_snapshots").and_then(|v| v.as_object()) {
            let reset_date = resp
                .get("quota_reset_date")
                .and_then(|v| v.as_str())
                .map(String::from);

            for (key, value) in snapshots {
                let remaining = value.get("remaining").and_then(|v| v.as_i64());
                let entitlement = value.get("entitlement").and_then(|v| v.as_i64());
                let pct_remaining = value
                    .get("percent_remaining")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| match (remaining, entitlement) {
                        (Some(r), Some(e)) if e > 0 => {
                            (r as f64 / e as f64 * 100.0).clamp(0.0, 100.0)
                        }
                        _ => 100.0,
                    })
                    .clamp(0.0, 100.0);

                let used_pct = 100.0 - pct_remaining;
                let remaining_pct = pct_remaining;

                let remaining_label = match (remaining, entitlement) {
                    (Some(r), Some(e)) => Some(format!("{r}/{e} left")),
                    _ => None,
                };

                metrics.push(UsageMetric {
                    label: pretty_category(key),
                    used_percent: used_pct,
                    remaining_percent: remaining_pct,
                    remaining_label,
                    resets_at: reset_date.clone(),
                });
            }
        }

        // Try free tier response (limited_user_quotas)
        if metrics.is_empty() {
            if let Some(quotas) = resp.get("limited_user_quotas").and_then(|v| v.as_object()) {
                let monthly = resp.get("monthly_quotas").and_then(|v| v.as_object());
                let reset_date = resp
                    .get("limited_user_reset_date")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                for (key, value) in quotas {
                    let remaining = value.as_i64().unwrap_or(0);
                    let total = monthly
                        .and_then(|m| m.get(key))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(remaining);

                    if total > 0 {
                        let used = (total - remaining).max(0);
                        let used_pct = (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0);
                        metrics.push(UsageMetric {
                            label: pretty_category(key),
                            used_percent: used_pct,
                            remaining_percent: 100.0 - used_pct,
                            remaining_label: Some(format!("{remaining}/{total} left")),
                            resets_at: reset_date.clone(),
                        });
                    }
                }
            }
        }

        Ok(UsageOutput {
            provider: "Copilot".into(),
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
