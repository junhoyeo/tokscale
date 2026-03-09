//! TUI data caching for instant startup.
//!
//! This module provides disk-based caching for TUI data to enable instant UI display
//! while fresh data loads in the background (matching TypeScript implementation behavior).

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokscale_core::ClientId;

use super::data::{
    AgentUsage, ContributionDay, DailyModelInfo, DailyUsage, GraphData, ModelUsage, TokenBreakdown,
    UsageData,
};

/// Cache staleness threshold: 5 minutes (matches TS implementation)
const CACHE_STALE_THRESHOLD_MS: u64 = 5 * 60 * 1000;
const CACHE_SCHEMA_VERSION: u32 = 2;

/// Get the cache directory path
/// Uses `~/.cache/tokscale/` to match TypeScript implementation for cache sharing
fn cache_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".cache").join("tokscale"))
}

/// Get the cache file path
fn cache_file() -> Option<PathBuf> {
    cache_dir().map(|d| d.join("tui-data-cache.json"))
}

/// Cached TUI data structure (serializable)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedTUIData {
    schema_version: u32,
    timestamp: u64,
    enabled_clients: Vec<String>,
    #[serde(default)]
    include_synthetic: bool,
    data: CachedUsageData,
}

/// Serializable version of UsageData
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedUsageData {
    models: Vec<CachedModelUsage>,
    #[serde(default)]
    agents: Vec<CachedAgentUsage>,
    daily: Vec<CachedDailyUsage>,
    graph: Option<CachedGraphData>,
    total_tokens: u64,
    total_cost: f64,
    current_streak: u32,
    longest_streak: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedTokenBreakdown {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    reasoning: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedModelUsage {
    model: String,
    provider: String,
    client: String,
    tokens: CachedTokenBreakdown,
    cost: f64,
    session_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedAgentUsage {
    agent: String,
    clients: String,
    tokens: CachedTokenBreakdown,
    cost: f64,
    message_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedDailyModelInfo {
    client: String,
    tokens: CachedTokenBreakdown,
    cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedDailyUsage {
    date: String, // NaiveDate serialized as string
    tokens: CachedTokenBreakdown,
    cost: f64,
    models: Vec<(String, CachedDailyModelInfo)>, // HashMap as vec of tuples
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedContributionDay {
    date: String,
    tokens: u64,
    cost: f64,
    intensity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedGraphData {
    weeks: Vec<Vec<Option<CachedContributionDay>>>,
}

// Conversion implementations

impl From<&TokenBreakdown> for CachedTokenBreakdown {
    fn from(t: &TokenBreakdown) -> Self {
        Self {
            input: t.input,
            output: t.output,
            cache_read: t.cache_read,
            cache_write: t.cache_write,
            reasoning: t.reasoning,
        }
    }
}

impl From<CachedTokenBreakdown> for TokenBreakdown {
    fn from(t: CachedTokenBreakdown) -> Self {
        Self {
            input: t.input,
            output: t.output,
            cache_read: t.cache_read,
            cache_write: t.cache_write,
            reasoning: t.reasoning,
        }
    }
}

impl From<&ModelUsage> for CachedModelUsage {
    fn from(m: &ModelUsage) -> Self {
        Self {
            model: m.model.clone(),
            provider: m.provider.clone(),
            client: m.client.clone(),
            tokens: (&m.tokens).into(),
            cost: m.cost,
            session_count: m.session_count,
        }
    }
}

impl From<CachedModelUsage> for ModelUsage {
    fn from(m: CachedModelUsage) -> Self {
        Self {
            model: m.model,
            provider: m.provider,
            client: m.client,
            tokens: m.tokens.into(),
            cost: m.cost,
            session_count: m.session_count,
        }
    }
}

impl From<&AgentUsage> for CachedAgentUsage {
    fn from(a: &AgentUsage) -> Self {
        Self {
            agent: a.agent.clone(),
            clients: a.clients.clone(),
            tokens: (&a.tokens).into(),
            cost: a.cost,
            message_count: a.message_count,
        }
    }
}

impl From<CachedAgentUsage> for AgentUsage {
    fn from(a: CachedAgentUsage) -> Self {
        Self {
            agent: a.agent,
            clients: a.clients,
            tokens: a.tokens.into(),
            cost: a.cost,
            message_count: a.message_count,
        }
    }
}

impl From<&DailyModelInfo> for CachedDailyModelInfo {
    fn from(d: &DailyModelInfo) -> Self {
        Self {
            client: d.client.clone(),
            tokens: (&d.tokens).into(),
            cost: d.cost,
        }
    }
}

impl From<CachedDailyModelInfo> for DailyModelInfo {
    fn from(d: CachedDailyModelInfo) -> Self {
        Self {
            client: d.client,
            tokens: d.tokens.into(),
            cost: d.cost,
        }
    }
}

impl From<&DailyUsage> for CachedDailyUsage {
    fn from(d: &DailyUsage) -> Self {
        Self {
            date: d.date.to_string(),
            tokens: (&d.tokens).into(),
            cost: d.cost,
            models: d
                .models
                .iter()
                .map(|(k, v)| (k.clone(), v.into()))
                .collect(),
        }
    }
}

impl TryFrom<CachedDailyUsage> for DailyUsage {
    type Error = chrono::ParseError;

    fn try_from(d: CachedDailyUsage) -> Result<Self, Self::Error> {
        use chrono::NaiveDate;
        Ok(Self {
            date: NaiveDate::parse_from_str(&d.date, "%Y-%m-%d")?,
            tokens: d.tokens.into(),
            cost: d.cost,
            models: d.models.into_iter().map(|(k, v)| (k, v.into())).collect(),
        })
    }
}

impl From<&ContributionDay> for CachedContributionDay {
    fn from(c: &ContributionDay) -> Self {
        Self {
            date: c.date.to_string(),
            tokens: c.tokens,
            cost: c.cost,
            intensity: c.intensity,
        }
    }
}

impl TryFrom<CachedContributionDay> for ContributionDay {
    type Error = chrono::ParseError;

    fn try_from(c: CachedContributionDay) -> Result<Self, Self::Error> {
        use chrono::NaiveDate;
        Ok(Self {
            date: NaiveDate::parse_from_str(&c.date, "%Y-%m-%d")?,
            tokens: c.tokens,
            cost: c.cost,
            intensity: c.intensity,
        })
    }
}

impl From<&GraphData> for CachedGraphData {
    fn from(g: &GraphData) -> Self {
        Self {
            weeks: g
                .weeks
                .iter()
                .map(|week| {
                    week.iter()
                        .map(|day| day.as_ref().map(|d| d.into()))
                        .collect()
                })
                .collect(),
        }
    }
}

impl TryFrom<CachedGraphData> for GraphData {
    type Error = chrono::ParseError;

    fn try_from(g: CachedGraphData) -> Result<Self, Self::Error> {
        let weeks: Result<Vec<Vec<Option<ContributionDay>>>, _> = g
            .weeks
            .into_iter()
            .map(|week| {
                week.into_iter()
                    .map(|day| day.map(|d| d.try_into()).transpose())
                    .collect()
            })
            .collect();
        Ok(Self { weeks: weeks? })
    }
}

impl From<&UsageData> for CachedUsageData {
    fn from(u: &UsageData) -> Self {
        Self {
            models: u.models.iter().map(|m| m.into()).collect(),
            agents: u.agents.iter().map(|a| a.into()).collect(),
            daily: u.daily.iter().map(|d| d.into()).collect(),
            graph: u.graph.as_ref().map(|g| g.into()),
            total_tokens: u.total_tokens,
            total_cost: u.total_cost,
            current_streak: u.current_streak,
            longest_streak: u.longest_streak,
        }
    }
}

impl TryFrom<CachedUsageData> for UsageData {
    type Error = chrono::ParseError;

    fn try_from(u: CachedUsageData) -> Result<Self, Self::Error> {
        let daily: Result<Vec<DailyUsage>, _> = u.daily.into_iter().map(|d| d.try_into()).collect();
        let graph: Option<Result<GraphData, _>> = u.graph.map(|g| g.try_into());

        Ok(Self {
            models: u.models.into_iter().map(|m| m.into()).collect(),
            agents: u.agents.into_iter().map(|a| a.into()).collect(),
            daily: daily?,
            graph: graph.transpose()?,
            total_tokens: u.total_tokens,
            total_cost: u.total_cost,
            loading: false,
            error: None,
            current_streak: u.current_streak,
            longest_streak: u.longest_streak,
        })
    }
}

/// Result of loading the TUI cache — combines staleness check with data loading
/// to avoid double file I/O (previously is_cache_stale + load_cached_data both parsed the file).
pub enum CacheResult {
    /// Cache exists, is fresh (within TTL), and clients match exactly
    Fresh(UsageData),
    /// Cache exists and clients match (exact or subset), but needs background refresh
    Stale(UsageData),
    /// Cache missing, unreadable, unparseable, or clients don't match
    Miss,
}

/// How the cached client set relates to the currently enabled client set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientMatch {
    /// Cached clients are exactly the currently enabled clients
    Exact,
    /// Cached clients are a strict subset of the currently enabled clients.
    /// The cached data is still valid — it just doesn't cover the new clients yet.
    Subset,
    /// No usable overlap (superset, disjoint, or synthetic flag mismatch)
    Mismatch,
}
/// Load cached TUI data from disk with a single read/parse.
/// Returns Fresh/Stale/Miss so the caller can decide whether to
/// display cached data immediately and/or trigger a background refresh.
pub fn load_cache(enabled_clients: &HashSet<ClientId>, include_synthetic: bool) -> CacheResult {
    let Some(cache_path) = cache_file() else {
        return CacheResult::Miss;
    };
    if !cache_path.exists() {
        return CacheResult::Miss;
    }
    let file = match File::open(&cache_path) {
        Ok(f) => f,
        Err(_) => return CacheResult::Miss,
    };
    let reader = BufReader::new(file);
    let cached: CachedTUIData = match serde_json::from_reader(reader) {
        Ok(c) => c,
        Err(_) => return CacheResult::Miss,
    };
    if cached.schema_version != CACHE_SCHEMA_VERSION {
        return CacheResult::Miss;
    }

    // Check how cached clients relate to enabled clients
    let client_match = check_client_match(
        enabled_clients,
        include_synthetic,
        &cached.enabled_clients,
        cached.include_synthetic,
    );

    if client_match == ClientMatch::Mismatch {
        return CacheResult::Miss;
    }
    // Convert cached data to UsageData
    let data = match cached.data.try_into() {
        Ok(d) => d,
        Err(_) => return CacheResult::Miss,
    };

    // Subset match → always Stale so background refresh fetches the full set
    if client_match == ClientMatch::Subset {
        return CacheResult::Stale(data);
    }

    // Exact match → check staleness by TTL
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let cache_age = now.saturating_sub(cached.timestamp);
    if cache_age > CACHE_STALE_THRESHOLD_MS {
        CacheResult::Stale(data)
    } else {
        CacheResult::Fresh(data)
    }
}

/// Determine how the cached client set relates to the currently enabled set.
///
/// - `Exact`    — same clients, same synthetic flag
/// - `Subset`   — cached clients ⊆ enabled clients (e.g. update added a new client),
///   and cached doesn't carry data the user doesn't want
/// - `Mismatch` — anything else (superset, disjoint, unwanted synthetic data)
fn check_client_match(
    enabled_clients: &HashSet<ClientId>,
    include_synthetic: bool,
    cached_clients: &[String],
    cached_include_synthetic: bool,
) -> ClientMatch {
    // If cache has synthetic data but user doesn't want it → mismatch
    // (showing unwanted data is worse than a cache miss)
    if cached_include_synthetic && !include_synthetic {
        return ClientMatch::Mismatch;
    }

    // Every cached client must exist in the enabled set
    for cached_client_str in cached_clients {
        let in_enabled = enabled_clients
            .iter()
            .any(|c| c.as_str() == cached_client_str);
        if !in_enabled {
            return ClientMatch::Mismatch;
        }
    }

    // Exact match: same size + same synthetic flag + all cached ∈ enabled (checked above)
    let same_size = enabled_clients.len() == cached_clients.len();
    let same_synthetic = include_synthetic == cached_include_synthetic;

    if same_size && same_synthetic {
        ClientMatch::Exact
    } else {
        ClientMatch::Subset
    }
}

/// Save TUI data to disk cache
pub fn save_cached_data(
    data: &UsageData,
    enabled_clients: &HashSet<ClientId>,
    include_synthetic: bool,
) {
    let Some(cache_path) = cache_file() else {
        return;
    };

    // Ensure cache directory exists
    if let Some(dir) = cache_path.parent() {
        if fs::create_dir_all(dir).is_err() {
            return;
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let cached = CachedTUIData {
        schema_version: CACHE_SCHEMA_VERSION,
        timestamp,
        enabled_clients: enabled_clients
            .iter()
            .map(|s| s.as_str().to_string())
            .collect(),
        include_synthetic,
        data: data.into(),
    };

    // Write to temp file first, then rename (atomic)
    let temp_path = cache_path.with_extension("json.tmp");
    let file = match File::create(&temp_path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let writer = BufWriter::new(file);

    if serde_json::to_writer(writer, &cached).is_ok() {
        if fs::rename(&temp_path, &cache_path).is_err() {
            // Windows: rename can't overwrite; copy then cleanup so destination is never removed first.
            if fs::copy(&temp_path, &cache_path).is_ok() {
                let _ = fs::remove_file(&temp_path);
            }
        }
    } else {
        let _ = fs::remove_file(&temp_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::{env, fs};
    use tempfile::TempDir;

    fn make_clients(ids: &[ClientId]) -> HashSet<ClientId> {
        ids.iter().copied().collect()
    }

    // ── check_client_match ──────────────────────────────────────────

    #[test]
    fn test_exact_match() {
        let enabled = make_clients(&[ClientId::Claude, ClientId::OpenCode]);
        let cached = vec!["claude".to_string(), "opencode".to_string()];
        assert_eq!(
            check_client_match(&enabled, false, &cached, false),
            ClientMatch::Exact,
        );
    }

    #[test]
    fn test_subset_new_client_added() {
        // Simulates: update added Qwen, cache only has Claude + OpenCode
        let enabled = make_clients(&[ClientId::Claude, ClientId::OpenCode, ClientId::Qwen]);
        let cached = vec!["claude".to_string(), "opencode".to_string()];
        assert_eq!(
            check_client_match(&enabled, false, &cached, false),
            ClientMatch::Subset,
        );
    }

    #[test]
    fn test_subset_synthetic_added() {
        // Cache was saved without synthetic, now user enables it
        let enabled = make_clients(&[ClientId::Claude]);
        let cached = vec!["claude".to_string()];
        assert_eq!(
            check_client_match(&enabled, true, &cached, false),
            ClientMatch::Subset,
        );
    }

    #[test]
    fn test_mismatch_superset() {
        // Cache has more clients than enabled (user narrowed filter)
        let enabled = make_clients(&[ClientId::Claude]);
        let cached = vec!["claude".to_string(), "opencode".to_string()];
        assert_eq!(
            check_client_match(&enabled, false, &cached, false),
            ClientMatch::Mismatch,
        );
    }

    #[test]
    fn test_mismatch_disjoint() {
        let enabled = make_clients(&[ClientId::Claude]);
        let cached = vec!["opencode".to_string()];
        assert_eq!(
            check_client_match(&enabled, false, &cached, false),
            ClientMatch::Mismatch,
        );
    }

    #[test]
    fn test_mismatch_unwanted_synthetic() {
        // Cache has synthetic data but user doesn't want it
        let enabled = make_clients(&[ClientId::Claude]);
        let cached = vec!["claude".to_string()];
        assert_eq!(
            check_client_match(&enabled, false, &cached, true),
            ClientMatch::Mismatch,
        );
    }

    #[test]
    fn test_exact_with_synthetic() {
        let enabled = make_clients(&[ClientId::Claude]);
        let cached = vec!["claude".to_string()];
        assert_eq!(
            check_client_match(&enabled, true, &cached, true),
            ClientMatch::Exact,
        );
    }

    #[test]
    fn test_subset_both_new_client_and_synthetic() {
        // Update added new client AND user also enabled synthetic
        let enabled = make_clients(&[ClientId::Claude, ClientId::Qwen]);
        let cached = vec!["claude".to_string()];
        assert_eq!(
            check_client_match(&enabled, true, &cached, false),
            ClientMatch::Subset,
        );
    }

    #[test]
    fn test_empty_cache_is_subset() {
        let enabled = make_clients(&[ClientId::Claude]);
        let cached: Vec<String> = vec![];
        assert_eq!(
            check_client_match(&enabled, false, &cached, false),
            ClientMatch::Subset,
        );
    }

    #[test]
    #[serial]
    fn test_load_cache_misses_legacy_schema_without_version() {
        let temp_dir = TempDir::new().unwrap();
        let previous_home = env::var_os("HOME");
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let cache_path = cache_file().unwrap();
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(
            &cache_path,
            r#"{
  "timestamp": 9999999999999,
  "enabledClients": ["claude"],
  "includeSynthetic": false,
  "data": {
    "models": [],
    "daily": [],
    "graph": null,
    "totalTokens": 0,
    "totalCost": 0.0,
    "currentStreak": 0,
    "longestStreak": 0
  }
}"#,
        )
        .unwrap();

        let clients = make_clients(&[ClientId::Claude]);
        assert!(matches!(load_cache(&clients, false), CacheResult::Miss));

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
    }
}
