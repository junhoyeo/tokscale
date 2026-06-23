//! Server-side aggregated multi-device stats for the TUI.
//!
//! Fetches `GET /api/me/stats` (see
//! `packages/frontend/src/app/api/me/stats/route.ts`) with the stored CLI
//! token and caches the response on disk with a ~1h TTL so the TUI can render
//! cache-first and refresh in the background. Everything here degrades
//! silently: callers treat any error as "no remote data" and fall back to
//! local-only display.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Wire schema version this CLI understands (`schemaVersion` in the JSON).
const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Cache freshness window. Server-side totals only change on `tokscale
/// submit`, so an hour keeps the footer indicator timely without hammering
/// the API on every TUI launch.
const CACHE_TTL_SECS: u64 = 3600;

const CACHE_FILE_NAME: &str = "remote-stats-cache.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDayStat {
    pub date: String,
    pub tokens: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDeviceStat {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub last_submitted_at: Option<String>,
}

/// Aggregated stats across all of the user's devices, as returned by
/// `GET /api/me/stats` plus local cache metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStats {
    pub schema_version: u32,
    pub total_tokens: u64,
    pub total_cost: f64,
    pub device_count: u64,
    #[serde(default)]
    pub last_submitted_at: Option<String>,
    #[serde(default)]
    pub days: Vec<RemoteDayStat>,
    #[serde(default)]
    pub devices: Vec<RemoteDeviceStat>,

    // ── Cache metadata (not sent by the server) ────────────────────────
    /// Unix seconds when this payload was fetched.
    #[serde(default)]
    pub fetched_at_secs: u64,
    /// Username the cache was fetched for; invalidates on account switch.
    #[serde(default)]
    pub cached_for_user: String,
    /// API base URL the cache was fetched from; invalidates on server switch.
    #[serde(default)]
    pub cached_for_api_url: String,
}

impl RemoteStats {
    /// Whether this payload is older than the cache TTL and should be
    /// refreshed in the background.
    pub fn is_stale(&self) -> bool {
        self.fetched_at_secs.saturating_add(CACHE_TTL_SECS) <= now_secs()
    }
}

/// Fetch `GET /api/me/stats` with the given bearer token and persist the
/// result to the on-disk cache. Built for use from a background thread:
/// spins up its own current-thread runtime, mirroring
/// `crate::commands::usage` provider fetchers.
// Only referenced from App's cfg(not(test)) fetch path — tests never hit the network.
#[cfg_attr(test, allow(dead_code))]
pub fn fetch_remote_stats(token: &str, username: &str, api_base_url: &str) -> Result<RemoteStats> {
    let url = format!("{}/api/me/stats", api_base_url.trim_end_matches('/'));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let mut stats: RemoteStats = rt.block_on(async {
        let response = reqwest::Client::new()
            .get(url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .context("Failed to fetch remote stats")?;

        if !response.status().is_success() {
            anyhow::bail!("Remote stats request failed with {}", response.status());
        }

        response
            .json::<RemoteStats>()
            .await
            .context("Failed to parse remote stats response")
    })?;

    if stats.schema_version != SUPPORTED_SCHEMA_VERSION {
        anyhow::bail!(
            "Unsupported remote stats schema version {}",
            stats.schema_version
        );
    }

    stats.fetched_at_secs = now_secs();
    stats.cached_for_user = username.to_string();
    stats.cached_for_api_url = api_base_url.to_string();
    let _ = save_remote_stats_cache(&stats);
    Ok(stats)
}

/// Load the cached stats if they are fresh (within TTL) and were fetched for
/// the same account and API server. Returns `None` otherwise.
pub fn load_cached_remote_stats(
    expected_user: &str,
    expected_api_url: &str,
) -> Option<RemoteStats> {
    if expected_user.is_empty() {
        return None;
    }

    let cache_path = get_cache_path()?;
    let content = fs::read_to_string(cache_path).ok()?;
    let stats: RemoteStats = serde_json::from_str(&content).ok()?;

    if stats.schema_version != SUPPORTED_SCHEMA_VERSION {
        return None;
    }

    if stats.fetched_at_secs.saturating_add(CACHE_TTL_SECS) <= now_secs() {
        return None;
    }

    // Reject cache belonging to a different account or API server.
    if stats.cached_for_user.is_empty() || stats.cached_for_user != expected_user {
        return None;
    }
    let cached_url = stats.cached_for_api_url.trim_end_matches('/');
    if cached_url.is_empty() || cached_url != expected_api_url.trim_end_matches('/') {
        return None;
    }

    Some(stats)
}

fn save_remote_stats_cache(stats: &RemoteStats) -> Result<()> {
    let cache_path = get_cache_path().context("Could not resolve remote stats cache directory")?;
    let json = serde_json::to_string(stats).context("Failed to serialize remote stats cache")?;
    let temp_path = cache_path.with_extension("json.tmp");
    fs::write(&temp_path, json).context("Failed to write remote stats temp cache file")?;
    if tokscale_core::fs_atomic::replace_file(&temp_path, &cache_path).is_err() {
        let _ = fs::remove_file(&temp_path);
        anyhow::bail!("Failed to move remote stats cache into place");
    }
    Ok(())
}

fn get_cache_path() -> Option<PathBuf> {
    let dir = crate::paths::get_cache_dir();
    if fs::create_dir_all(&dir).is_err() {
        return None;
    }
    Some(dir.join(CACHE_FILE_NAME))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;

    fn sample_stats(fetched_at_secs: u64) -> RemoteStats {
        RemoteStats {
            schema_version: SUPPORTED_SCHEMA_VERSION,
            total_tokens: 1250,
            total_cost: 1.75,
            device_count: 2,
            last_submitted_at: Some("2026-06-02T12:00:00.000Z".to_string()),
            days: vec![RemoteDayStat {
                date: "2026-06-01".to_string(),
                tokens: 1000,
                input_tokens: 600,
                output_tokens: 400,
                cost: 1.5,
            }],
            devices: vec![RemoteDeviceStat {
                id: "device-1".to_string(),
                display_name: Some("Work laptop".to_string()),
                last_submitted_at: Some("2026-06-02T12:00:00.000Z".to_string()),
            }],
            fetched_at_secs,
            cached_for_user: "alice".to_string(),
            cached_for_api_url: "https://tokscale.ai".to_string(),
        }
    }

    fn with_temp_config_dir(test: impl FnOnce()) {
        let temp = tempfile::tempdir().expect("tempdir");
        let prev = env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            env::set_var("TOKSCALE_CONFIG_DIR", temp.path());
        }
        test();
        unsafe {
            match prev {
                Some(v) => env::set_var("TOKSCALE_CONFIG_DIR", v),
                None => env::remove_var("TOKSCALE_CONFIG_DIR"),
            }
        }
    }

    #[test]
    #[serial]
    fn cache_round_trips_for_matching_account_and_server() {
        with_temp_config_dir(|| {
            let stats = sample_stats(now_secs());
            save_remote_stats_cache(&stats).expect("save cache");

            let loaded = load_cached_remote_stats("alice", "https://tokscale.ai")
                .expect("fresh cache should load");
            assert_eq!(loaded.total_tokens, 1250);
            assert_eq!(loaded.device_count, 2);
            assert_eq!(loaded.days.len(), 1);
            assert_eq!(
                loaded.devices[0].display_name.as_deref(),
                Some("Work laptop")
            );
        });
    }

    #[test]
    #[serial]
    fn cache_normalizes_trailing_slash_in_api_url() {
        with_temp_config_dir(|| {
            save_remote_stats_cache(&sample_stats(now_secs())).expect("save cache");
            assert!(load_cached_remote_stats("alice", "https://tokscale.ai/").is_some());
        });
    }

    #[test]
    #[serial]
    fn cache_rejects_stale_entries() {
        with_temp_config_dir(|| {
            let stats = sample_stats(now_secs().saturating_sub(CACHE_TTL_SECS + 1));
            save_remote_stats_cache(&stats).expect("save cache");
            assert!(load_cached_remote_stats("alice", "https://tokscale.ai").is_none());
        });
    }

    #[test]
    #[serial]
    fn cache_rejects_other_accounts_servers_and_anonymous_lookups() {
        with_temp_config_dir(|| {
            save_remote_stats_cache(&sample_stats(now_secs())).expect("save cache");
            assert!(load_cached_remote_stats("bob", "https://tokscale.ai").is_none());
            assert!(load_cached_remote_stats("alice", "https://staging.tokscale.ai").is_none());
            // Env-token sessions have no username; they must never trust cache.
            assert!(load_cached_remote_stats("", "https://tokscale.ai").is_none());
        });
    }

    #[test]
    #[serial]
    fn cache_rejects_unknown_schema_versions() {
        with_temp_config_dir(|| {
            let mut stats = sample_stats(now_secs());
            stats.schema_version = SUPPORTED_SCHEMA_VERSION + 1;
            save_remote_stats_cache(&stats).expect("save cache");
            assert!(load_cached_remote_stats("alice", "https://tokscale.ai").is_none());
        });
    }

    #[test]
    fn staleness_follows_ttl() {
        assert!(!sample_stats(now_secs()).is_stale());
        assert!(sample_stats(now_secs().saturating_sub(CACHE_TTL_SECS + 1)).is_stale());
    }
}
