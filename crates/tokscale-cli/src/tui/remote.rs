use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const CACHE_TTL_SECS: u64 = 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteModelStat {
    pub model: String,
    pub cost: f64,
    pub tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteClientStat {
    pub client: String,
    pub cost: f64,
    pub tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDayStat {
    pub date: String,
    pub cost: f64,
    pub tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDeviceStat {
    pub id: String,
    pub cost: f64,
    #[serde(default)]
    pub tokens: u64,
    pub last_seen_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStats {
    pub total_cost: f64,
    pub total_tokens: u64,
    pub by_model: Vec<RemoteModelStat>,
    pub by_client: Vec<RemoteClientStat>,
    pub by_day: Vec<RemoteDayStat>,
    #[serde(default)]
    pub devices: Vec<RemoteDeviceStat>,
    #[serde(default)]
    pub fetched_at_secs: u64,
    /// Username the cache was fetched for; used to invalidate on account switch.
    #[serde(default)]
    pub cached_for_user: String,
    /// API base URL the cache was fetched from; used to invalidate on server switch.
    #[serde(default)]
    pub cached_for_api_url: String,
}

pub async fn fetch_remote_stats(token: &str, username: &str, api_base_url: &str) -> Result<RemoteStats> {
    let url = format!("{}/api/me/stats", api_base_url.trim_end_matches('/'));

    let response = reqwest::Client::new()
        .get(url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .context("Failed to fetch remote stats")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "Remote stats request failed with {}{}",
            status,
            if body.is_empty() {
                String::new()
            } else {
                format!(": {}", body)
            }
        );
    }

    let mut stats: RemoteStats = response
        .json()
        .await
        .context("Failed to parse remote stats response")?;
    stats.fetched_at_secs = now_secs();
    stats.cached_for_user = username.to_string();
    stats.cached_for_api_url = api_base_url.to_string();
    let _ = save_remote_stats_cache(&stats);
    Ok(stats)
}

pub fn load_cached_remote_stats(expected_user: Option<&str>, expected_api_url: Option<&str>) -> Option<RemoteStats> {
    let cache_path = get_cache_path().ok()?;
    if !cache_path.exists() {
        return None;
    }

    let file = File::open(cache_path).ok()?;
    let reader = BufReader::new(file);
    let stats: RemoteStats = serde_json::from_reader(reader).ok()?;

    let now = now_secs();
    if stats.fetched_at_secs.saturating_add(CACHE_TTL_SECS) <= now {
        return None;
    }

    // Reject cache if it belongs to a different account.
    if let Some(user) = expected_user {
        if stats.cached_for_user.is_empty() || stats.cached_for_user != user {
            return None;
        }
    }

    // Reject cache if it was fetched from a different API server.
    if let Some(api_url) = expected_api_url {
        if stats.cached_for_api_url.is_empty() || stats.cached_for_api_url != api_url {
            return None;
        }
    }

    Some(stats)
}

fn save_remote_stats_cache(stats: &RemoteStats) -> Result<()> {
    let cache_path = get_cache_path()?;
    if let Some(dir) = cache_path.parent() {
        fs::create_dir_all(dir).context("Failed to create remote stats cache directory")?;
    }

    let temp_path = cache_path.with_extension("json.tmp");
    let file = File::create(&temp_path).context("Failed to create remote stats temp cache file")?;
    let writer = BufWriter::new(file);
    serde_json::to_writer(writer, stats).context("Failed to write remote stats cache")?;

    if fs::rename(&temp_path, &cache_path).is_err() {
        fs::copy(&temp_path, &cache_path)
            .context("Failed to copy remote stats cache into place")?;
        let _ = fs::remove_file(&temp_path);
    }

    Ok(())
}

fn get_cache_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home
        .join(".cache")
        .join("tokscale")
        .join("remote-stats-cache.json"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
