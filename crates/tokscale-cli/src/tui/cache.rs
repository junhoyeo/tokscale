//! TUI data caching for instant startup.
//!
//! This module provides disk-based caching for TUI data to enable instant UI display
//! while fresh data loads in the background (matching TypeScript implementation behavior).

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokscale_core::GroupBy;

use crate::ClientFilter;

use super::data::{
    AgentUsage, ContributionDay, DailyModelInfo, DailySourceInfo, DailyUsage, GraphData,
    HourlyModelInfo, HourlyUsage, ModelUsage, TokenBreakdown, UsageData,
};

/// Cache staleness threshold: 5 minutes (matches TS implementation)
const CACHE_STALE_THRESHOLD_MS: u64 = 5 * 60 * 1000;
const CACHE_SCHEMA_VERSION: u32 = 6;

/// Get the cache directory path
/// Uses `~/.cache/tokscale/` to match TypeScript implementation for cache sharing
fn cache_dir() -> Option<PathBuf> {
    Some(crate::paths::get_cache_dir())
}

/// Get the cache file path
fn cache_file() -> Option<PathBuf> {
    cache_dir().map(|d| d.join("tui-data-cache.json"))
}

fn legacy_cache_files() -> Vec<PathBuf> {
    if crate::paths::is_config_dir_overridden() {
        return Vec::new();
    }

    crate::paths::legacy_dot_cache_tokscale_dir()
        .map(|dir| vec![dir.join("tui-data-cache.json")])
        .unwrap_or_default()
}

/// Cached TUI data structure (serializable)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedTUIData {
    #[serde(default)]
    schema_version: u32,
    timestamp: u64,
    enabled_clients: Vec<String>,
    #[serde(default)]
    include_synthetic: bool,
    #[serde(default)]
    group_by: Option<String>,
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
    #[serde(default)]
    hourly: Vec<CachedHourlyUsage>,
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
    #[serde(default)]
    workspace_key: Option<String>,
    #[serde(default)]
    workspace_label: Option<String>,
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
    #[serde(default)]
    client: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    color_key: String,
    tokens: CachedTokenBreakdown,
    cost: f64,
    #[serde(default)]
    messages: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedDailySourceInfo {
    tokens: CachedTokenBreakdown,
    cost: f64,
    models: Vec<(String, CachedDailyModelInfo)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedDailyUsage {
    date: String, // NaiveDate serialized as string
    tokens: CachedTokenBreakdown,
    cost: f64,
    #[serde(default)]
    models: Vec<(String, CachedDailyModelInfo)>,
    #[serde(default)]
    source_breakdown: Vec<(String, CachedDailySourceInfo)>,
    #[serde(default)]
    message_count: u32,
    #[serde(default)]
    turn_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedHourlyModelInfo {
    #[serde(default)]
    provider: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    color_key: String,
    tokens: CachedTokenBreakdown,
    cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedHourlyUsage {
    datetime: String, // NaiveDateTime as "YYYY-MM-DD HH:MM:SS"
    tokens: CachedTokenBreakdown,
    cost: f64,
    clients: Vec<String>,
    models: Vec<(String, CachedHourlyModelInfo)>,
    #[serde(default)]
    message_count: u32,
    #[serde(default)]
    turn_count: u32,
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
            workspace_key: m.workspace_key.clone(),
            workspace_label: m.workspace_label.clone(),
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
            workspace_key: m.workspace_key,
            workspace_label: m.workspace_label,
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
            client: String::new(),
            provider: d.provider.clone(),
            display_name: d.display_name.clone(),
            color_key: d.color_key.clone(),
            tokens: (&d.tokens).into(),
            cost: d.cost,
            messages: d.messages,
        }
    }
}

fn daily_model_info_from_cached(key: &str, value: CachedDailyModelInfo) -> DailyModelInfo {
    let display_name = if value.display_name.is_empty() {
        key.to_string()
    } else {
        value.display_name
    };
    let color_key = if value.color_key.is_empty() {
        display_name
            .rsplit_once(" / ")
            .map(|(_, base_model)| base_model.to_string())
            .unwrap_or_else(|| display_name.clone())
    } else {
        value.color_key
    };

    DailyModelInfo {
        provider: value.provider,
        display_name,
        color_key,
        tokens: value.tokens.into(),
        cost: value.cost,
        messages: value.messages,
    }
}

impl From<&DailySourceInfo> for CachedDailySourceInfo {
    fn from(source: &DailySourceInfo) -> Self {
        Self {
            tokens: (&source.tokens).into(),
            cost: source.cost,
            models: source
                .models
                .iter()
                .map(|(key, value)| (key.clone(), value.into()))
                .collect(),
        }
    }
}

impl From<CachedDailySourceInfo> for DailySourceInfo {
    fn from(source: CachedDailySourceInfo) -> Self {
        Self {
            tokens: source.tokens.into(),
            cost: source.cost,
            models: source
                .models
                .into_iter()
                .map(|(key, value)| {
                    let model_info = daily_model_info_from_cached(&key, value);
                    (key, model_info)
                })
                .collect(),
        }
    }
}

impl From<&HourlyModelInfo> for CachedHourlyModelInfo {
    fn from(h: &HourlyModelInfo) -> Self {
        Self {
            provider: h.provider.clone(),
            display_name: h.display_name.clone(),
            color_key: h.color_key.clone(),
            tokens: (&h.tokens).into(),
            cost: h.cost,
        }
    }
}

fn hourly_model_info_from_cached(key: &str, value: CachedHourlyModelInfo) -> HourlyModelInfo {
    let display_name = if value.display_name.is_empty() {
        key.to_string()
    } else {
        value.display_name
    };
    let color_key = if value.color_key.is_empty() {
        display_name
            .rsplit_once(" / ")
            .map(|(_, base_model)| base_model.to_string())
            .unwrap_or_else(|| display_name.clone())
    } else {
        value.color_key
    };

    HourlyModelInfo {
        provider: value.provider,
        display_name,
        color_key,
        tokens: value.tokens.into(),
        cost: value.cost,
    }
}

impl From<&HourlyUsage> for CachedHourlyUsage {
    fn from(h: &HourlyUsage) -> Self {
        Self {
            datetime: h.datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
            tokens: (&h.tokens).into(),
            cost: h.cost,
            clients: h.clients.iter().cloned().collect(),
            models: h
                .models
                .iter()
                .map(|(k, v)| (k.clone(), v.into()))
                .collect(),
            message_count: h.message_count,
            turn_count: h.turn_count,
        }
    }
}

impl TryFrom<CachedHourlyUsage> for HourlyUsage {
    type Error = chrono::ParseError;

    fn try_from(h: CachedHourlyUsage) -> Result<Self, Self::Error> {
        use chrono::NaiveDateTime;
        Ok(Self {
            datetime: NaiveDateTime::parse_from_str(&h.datetime, "%Y-%m-%d %H:%M:%S")?,
            tokens: h.tokens.into(),
            cost: h.cost,
            clients: h.clients.into_iter().collect(),
            models: h
                .models
                .into_iter()
                .map(|(key, value)| {
                    let model_info = hourly_model_info_from_cached(&key, value);
                    (key, model_info)
                })
                .collect(),
            message_count: h.message_count,
            turn_count: h.turn_count,
        })
    }
}

impl From<&DailyUsage> for CachedDailyUsage {
    fn from(d: &DailyUsage) -> Self {
        Self {
            date: d.date.to_string(),
            tokens: (&d.tokens).into(),
            cost: d.cost,
            models: Vec::new(),
            source_breakdown: d
                .source_breakdown
                .iter()
                .map(|(key, value)| (key.clone(), value.into()))
                .collect(),
            message_count: d.message_count,
            turn_count: d.turn_count,
        }
    }
}

impl TryFrom<CachedDailyUsage> for DailyUsage {
    type Error = chrono::ParseError;

    fn try_from(d: CachedDailyUsage) -> Result<Self, Self::Error> {
        use chrono::NaiveDate;

        let source_breakdown = if d.source_breakdown.is_empty() {
            let mut legacy_sources: BTreeMap<String, DailySourceInfo> = BTreeMap::new();
            for (key, value) in d.models {
                let client = if value.client.is_empty() {
                    "unknown".to_string()
                } else {
                    value.client.clone()
                };
                let model_info = daily_model_info_from_cached(&key, value);
                let source = legacy_sources
                    .entry(client)
                    .or_insert_with(|| DailySourceInfo {
                        tokens: TokenBreakdown::default(),
                        cost: 0.0,
                        models: BTreeMap::new(),
                    });
                source.tokens.input = source.tokens.input.saturating_add(model_info.tokens.input);
                source.tokens.output = source
                    .tokens
                    .output
                    .saturating_add(model_info.tokens.output);
                source.tokens.cache_read = source
                    .tokens
                    .cache_read
                    .saturating_add(model_info.tokens.cache_read);
                source.tokens.cache_write = source
                    .tokens
                    .cache_write
                    .saturating_add(model_info.tokens.cache_write);
                source.tokens.reasoning = source
                    .tokens
                    .reasoning
                    .saturating_add(model_info.tokens.reasoning);
                source.cost += model_info.cost;
                source.models.insert(key, model_info);
            }
            legacy_sources
        } else {
            d.source_breakdown
                .into_iter()
                .map(|(key, value)| (key, value.into()))
                .collect()
        };

        Ok(Self {
            date: NaiveDate::parse_from_str(&d.date, "%Y-%m-%d")?,
            tokens: d.tokens.into(),
            cost: d.cost,
            source_breakdown,
            message_count: d.message_count,
            turn_count: d.turn_count,
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
            hourly: u.hourly.iter().map(|h| h.into()).collect(),
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
        let hourly: Result<Vec<HourlyUsage>, _> =
            u.hourly.into_iter().map(|h| h.try_into()).collect();
        let graph: Option<Result<GraphData, _>> = u.graph.map(|g| g.try_into());

        Ok(Self {
            models: u.models.into_iter().map(|m| m.into()).collect(),
            agents: u.agents.into_iter().map(|a| a.into()).collect(),
            daily: daily?,
            hourly: hourly?,
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
///
/// `enabled_clients` is the unified `HashSet<ClientFilter>` (Synthetic
/// included as a set member). The on-disk format keeps the legacy
/// `(enabled_clients: Vec<String>, include_synthetic: bool)` shape so
/// existing user caches keep working across upgrades — projection
/// happens here.
pub fn load_cache(enabled_clients: &HashSet<ClientFilter>, group_by: &GroupBy) -> CacheResult {
    let Some(cache_path) = cache_file() else {
        return CacheResult::Miss;
    };
    let cached: Option<CachedTUIData> = match File::open(&cache_path) {
        Ok(file) => {
            let reader = BufReader::new(file);
            serde_json::from_reader(reader).ok()
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            legacy_cache_files().into_iter().find_map(|path| {
                let file = File::open(path).ok()?;
                let reader = BufReader::new(file);
                serde_json::from_reader(reader).ok()
            })
        }
        Err(_) => None,
    };
    let Some(cached) = cached else {
        return CacheResult::Miss;
    };
    if cached.schema_version > CACHE_SCHEMA_VERSION {
        return CacheResult::Miss;
    }
    let schema_outdated = cached.schema_version < CACHE_SCHEMA_VERSION;
    let cached_group_by = cached
        .group_by
        .as_deref()
        .and_then(|value: &str| value.parse::<GroupBy>().ok());
    if schema_outdated && cached_group_by.is_none() {
        return CacheResult::Miss;
    }

    if cached_group_by.as_ref() != Some(group_by) {
        return CacheResult::Miss;
    }

    // Check how cached clients relate to enabled clients
    let client_match = check_client_match(
        enabled_clients,
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

    if schema_outdated || client_match == ClientMatch::Subset {
        return CacheResult::Stale(data);
    }

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
///
/// Cached side stays in the legacy `(Vec<String>, bool)` shape so we can
/// read pre-refactor cache files without a migration step. Enabled side
/// is the new unified `HashSet<ClientFilter>`.
fn check_client_match(
    enabled_clients: &HashSet<ClientFilter>,
    cached_clients: &[String],
    cached_include_synthetic: bool,
) -> ClientMatch {
    let include_synthetic = enabled_clients.contains(&ClientFilter::Synthetic);

    // If cache has synthetic data but user doesn't want it → mismatch
    // (showing unwanted data is worse than a cache miss)
    if cached_include_synthetic && !include_synthetic {
        return ClientMatch::Mismatch;
    }

    // Every cached client must exist in the enabled set. Compare on the
    // canonical lowercase id so we don't have to round-trip through
    // ClientId for clients that map 1:1.
    for cached_client_str in cached_clients {
        let in_enabled = enabled_clients
            .iter()
            .any(|f| f.as_filter_str() == cached_client_str);
        if !in_enabled {
            return ClientMatch::Mismatch;
        }
    }

    // Exact match requires same set membership on BOTH sides:
    //   |enabled non-synthetic| == |cached_clients|  AND
    //   include_synthetic == cached_include_synthetic
    let enabled_non_synthetic = enabled_clients.len() - usize::from(include_synthetic);
    let same_size = enabled_non_synthetic == cached_clients.len();
    let same_synthetic = include_synthetic == cached_include_synthetic;

    if same_size && same_synthetic {
        ClientMatch::Exact
    } else {
        ClientMatch::Subset
    }
}

/// Save TUI data to disk cache.
///
/// On-disk schema keeps the legacy `(Vec<String> enabled_clients, bool
/// include_synthetic)` pair so caches written by older releases remain
/// readable across upgrades. We project the unified
/// `HashSet<ClientFilter>` here.
pub fn save_cached_data(
    data: &UsageData,
    enabled_clients: &HashSet<ClientFilter>,
    group_by: &GroupBy,
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

    // Project unified set into the legacy on-disk shape.
    let include_synthetic = enabled_clients.contains(&ClientFilter::Synthetic);
    let mut clients_vec: Vec<String> = enabled_clients
        .iter()
        .filter(|f| !matches!(f, ClientFilter::Synthetic))
        .map(|f| f.as_filter_str().to_string())
        .collect();
    // Sort so the cache key is deterministic across runs / HashSet
    // iteration order — otherwise unrelated runs would invalidate each
    // other's caches just because the JSON ordering shuffled.
    clients_vec.sort();

    let cached = CachedTUIData {
        schema_version: CACHE_SCHEMA_VERSION,
        timestamp,
        enabled_clients: clients_vec,
        include_synthetic,
        group_by: Some(group_by.to_string()),
        data: data.into(),
    };

    // INVARIANT: All cache writes use atomic temp-file rename. NEVER delete
    // the canonical cache file before writing — a partial save or process
    // crash between delete and rename would lose the cache. The temp-file
    // pattern makes corruption-on-crash impossible.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let temp_path = cache_path.with_file_name(format!(
        ".{}.{}.{:x}.tmp",
        cache_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("tui-data-cache.json"),
        std::process::id(),
        nanos
    ));
    let file = match File::create(&temp_path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let writer = BufWriter::new(file);

    if serde_json::to_writer(writer, &cached).is_ok() {
        let _ = tokscale_core::fs_atomic::replace_file(&temp_path, &cache_path);
    } else {
        let _ = fs::remove_file(&temp_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::{env, fs};
    use tempfile::TempDir;

    /// Build a unified filter set. Pass `synthetic=true` to include
    /// `ClientFilter::Synthetic` as a set member (the new way to express
    /// "user has synthetic enabled").
    fn make_filters(filters: &[ClientFilter], synthetic: bool) -> HashSet<ClientFilter> {
        let mut set: HashSet<ClientFilter> = filters.iter().copied().collect();
        if synthetic {
            set.insert(ClientFilter::Synthetic);
        }
        set
    }

    // ── check_client_match ──────────────────────────────────────────

    #[test]
    fn test_exact_match() {
        let enabled = make_filters(&[ClientFilter::Claude, ClientFilter::Opencode], false);
        let cached = vec!["claude".to_string(), "opencode".to_string()];
        assert_eq!(
            check_client_match(&enabled, &cached, false),
            ClientMatch::Exact,
        );
    }

    #[test]
    fn test_subset_new_client_added() {
        // Simulates: update added Qwen, cache only has Claude + OpenCode
        let enabled = make_filters(
            &[
                ClientFilter::Claude,
                ClientFilter::Opencode,
                ClientFilter::Qwen,
            ],
            false,
        );
        let cached = vec!["claude".to_string(), "opencode".to_string()];
        assert_eq!(
            check_client_match(&enabled, &cached, false),
            ClientMatch::Subset,
        );
    }

    #[test]
    fn test_subset_synthetic_added() {
        // Cache was saved without synthetic, now user enables it
        let enabled = make_filters(&[ClientFilter::Claude], true);
        let cached = vec!["claude".to_string()];
        assert_eq!(
            check_client_match(&enabled, &cached, false),
            ClientMatch::Subset,
        );
    }

    #[test]
    fn test_mismatch_superset() {
        // Cache has more clients than enabled (user narrowed filter)
        let enabled = make_filters(&[ClientFilter::Claude], false);
        let cached = vec!["claude".to_string(), "opencode".to_string()];
        assert_eq!(
            check_client_match(&enabled, &cached, false),
            ClientMatch::Mismatch,
        );
    }

    #[test]
    fn test_mismatch_disjoint() {
        let enabled = make_filters(&[ClientFilter::Claude], false);
        let cached = vec!["opencode".to_string()];
        assert_eq!(
            check_client_match(&enabled, &cached, false),
            ClientMatch::Mismatch,
        );
    }

    #[test]
    fn test_mismatch_unwanted_synthetic() {
        // Cache has synthetic data but user doesn't want it
        let enabled = make_filters(&[ClientFilter::Claude], false);
        let cached = vec!["claude".to_string()];
        assert_eq!(
            check_client_match(&enabled, &cached, true),
            ClientMatch::Mismatch,
        );
    }

    #[test]
    fn test_exact_with_synthetic() {
        let enabled = make_filters(&[ClientFilter::Claude], true);
        let cached = vec!["claude".to_string()];
        assert_eq!(
            check_client_match(&enabled, &cached, true),
            ClientMatch::Exact,
        );
    }

    #[test]
    fn test_subset_both_new_client_and_synthetic() {
        // Update added new client AND user also enabled synthetic
        let enabled = make_filters(&[ClientFilter::Claude, ClientFilter::Qwen], true);
        let cached = vec!["claude".to_string()];
        assert_eq!(
            check_client_match(&enabled, &cached, false),
            ClientMatch::Subset,
        );
    }

    #[test]
    fn test_empty_cache_is_subset() {
        let enabled = make_filters(&[ClientFilter::Claude], false);
        let cached: Vec<String> = vec![];
        assert_eq!(
            check_client_match(&enabled, &cached, false),
            ClientMatch::Subset,
        );
    }

    #[test]
    #[serial]
    fn test_load_cache_misses_for_legacy_schema_without_group_by() {
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

        let clients = make_filters(&[ClientFilter::Claude], false);
        assert!(matches!(
            load_cache(&clients, &GroupBy::Model),
            CacheResult::Miss
        ));

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    #[test]
    #[serial]
    fn test_load_cache_misses_when_group_by_differs() {
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
  "schemaVersion": 4,
  "timestamp": 9999999999999,
  "enabledClients": ["claude"],
  "includeSynthetic": false,
  "groupBy": "model",
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

        let clients = make_filters(&[ClientFilter::Claude], false);
        assert!(matches!(
            load_cache(&clients, &GroupBy::WorkspaceModel),
            CacheResult::Miss
        ));

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    #[test]
    #[serial]
    fn test_load_cache_stale_legacy_daily_models_without_display_fields() {
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
  "schemaVersion": 3,
  "timestamp": 9999999999999,
  "enabledClients": ["claude"],
  "includeSynthetic": false,
  "groupBy": "model",
  "data": {
    "models": [],
    "agents": [],
    "daily": [{
      "date": "2026-03-18",
      "tokens": {
        "input": 10,
        "output": 5,
        "cacheRead": 0,
        "cacheWrite": 0,
        "reasoning": 0
      },
      "cost": 1.25,
      "models": [[
        "claude-sonnet-4-5",
        {
          "client": "claude",
          "tokens": {
            "input": 10,
            "output": 5,
            "cacheRead": 0,
            "cacheWrite": 0,
            "reasoning": 0
          },
          "cost": 1.25
        }
      ]]
    }],
    "graph": null,
    "totalTokens": 15,
    "totalCost": 1.25,
    "currentStreak": 1,
    "longestStreak": 1
  }
}"#,
        )
        .unwrap();

        let clients = make_filters(&[ClientFilter::Claude], false);
        match load_cache(&clients, &GroupBy::Model) {
            CacheResult::Stale(data) => {
                let source = data.daily[0].source_breakdown.get("claude").unwrap();
                let daily_model = source.models.get("claude-sonnet-4-5").unwrap();
                assert_eq!(daily_model.display_name, "claude-sonnet-4-5");
                assert_eq!(daily_model.color_key, "claude-sonnet-4-5");
            }
            other => panic!(
                "expected stale legacy cache, got {:?}",
                other_variant_name(&other)
            ),
        }

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    #[test]
    #[serial]
    fn test_load_cache_reads_source_breakdown_from_current_schema() {
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
  "schemaVersion": 6,
  "timestamp": 9999999999999,
  "enabledClients": ["claude", "cursor"],
  "includeSynthetic": false,
  "groupBy": "model",
  "data": {
    "models": [],
    "agents": [],
    "daily": [{
      "date": "2026-03-18",
      "tokens": {
        "input": 30,
        "output": 15,
        "cacheRead": 0,
        "cacheWrite": 0,
        "reasoning": 0
      },
      "cost": 3.25,
      "sourceBreakdown": [[
        "claude",
        {
          "tokens": {
            "input": 10,
            "output": 5,
            "cacheRead": 0,
            "cacheWrite": 0,
            "reasoning": 0
          },
          "cost": 1.25,
          "models": [[
            "claude-sonnet-4-5",
            {
              "provider": "anthropic",
              "displayName": "claude-sonnet-4-5",
              "colorKey": "claude-sonnet-4-5",
              "tokens": {
                "input": 10,
                "output": 5,
                "cacheRead": 0,
                "cacheWrite": 0,
                "reasoning": 0
              },
              "cost": 1.25
            }
          ]]
        }
      ], [
        "cursor",
        {
          "tokens": {
            "input": 20,
            "output": 10,
            "cacheRead": 0,
            "cacheWrite": 0,
            "reasoning": 0
          },
          "cost": 2.0,
          "models": [[
            "claude-sonnet-4-5",
            {
              "provider": "anthropic",
              "displayName": "claude-sonnet-4-5",
              "colorKey": "claude-sonnet-4-5",
              "tokens": {
                "input": 20,
                "output": 10,
                "cacheRead": 0,
                "cacheWrite": 0,
                "reasoning": 0
              },
              "cost": 2.0
            }
          ]]
        }
      ]]
    }],
    "graph": null,
    "totalTokens": 45,
    "totalCost": 3.25,
    "currentStreak": 1,
    "longestStreak": 1
  }
}"#,
        )
        .unwrap();

        let clients = make_filters(&[ClientFilter::Claude, ClientFilter::Cursor], false);
        match load_cache(&clients, &GroupBy::Model) {
            CacheResult::Fresh(data) => {
                assert_eq!(data.daily[0].source_breakdown.len(), 2);
                let cursor = data.daily[0].source_breakdown.get("cursor").unwrap();
                let model = cursor.models.get("claude-sonnet-4-5").unwrap();
                assert_eq!(model.provider, "anthropic");
                assert_eq!(model.tokens.total(), 30);
            }
            other => panic!(
                "expected fresh current-schema cache, got {:?}",
                other_variant_name(&other)
            ),
        }

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    #[test]
    #[serial]
    fn test_load_cache_stale_legacy_hourly_models_without_display_fields() {
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
  "schemaVersion": 5,
  "timestamp": 9999999999999,
  "enabledClients": ["claude"],
  "includeSynthetic": false,
  "groupBy": "model",
  "data": {
    "models": [],
    "agents": [],
    "daily": [],
    "hourly": [{
      "datetime": "2026-03-18 10:00:00",
      "tokens": {
        "input": 10,
        "output": 5,
        "cacheRead": 0,
        "cacheWrite": 0,
        "reasoning": 0
      },
      "cost": 1.25,
      "clients": ["claude"],
      "models": [[
        "claude-sonnet-4-5",
        {
          "tokens": {
            "input": 10,
            "output": 5,
            "cacheRead": 0,
            "cacheWrite": 0,
            "reasoning": 0
          },
          "cost": 1.25
        }
      ]],
      "messageCount": 1,
      "turnCount": 1
    }],
    "graph": null,
    "totalTokens": 15,
    "totalCost": 1.25,
    "currentStreak": 1,
    "longestStreak": 1
  }
}"#,
        )
        .unwrap();

        let clients = make_filters(&[ClientFilter::Claude], false);
        match load_cache(&clients, &GroupBy::Model) {
            CacheResult::Fresh(data) | CacheResult::Stale(data) => {
                let hourly_model = data.hourly[0].models.get("claude-sonnet-4-5").unwrap();
                assert_eq!(hourly_model.display_name, "claude-sonnet-4-5");
                assert_eq!(hourly_model.color_key, "claude-sonnet-4-5");
            }
            other => panic!("expected cache data, got {:?}", other_variant_name(&other)),
        }

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    #[test]
    #[serial]
    fn test_load_cache_legacy_empty_client_falls_back_to_unknown() {
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
  "schemaVersion": 3,
  "timestamp": 9999999999999,
  "enabledClients": ["claude"],
  "includeSynthetic": false,
  "groupBy": "model",
  "data": {
    "models": [],
    "agents": [],
    "daily": [{
      "date": "2026-03-18",
      "tokens": {
        "input": 10,
        "output": 5,
        "cacheRead": 0,
        "cacheWrite": 0,
        "reasoning": 0
      },
      "cost": 1.25,
      "models": [[
        "claude-sonnet-4-5",
        {
          "client": "",
          "tokens": {
            "input": 10,
            "output": 5,
            "cacheRead": 0,
            "cacheWrite": 0,
            "reasoning": 0
          },
          "cost": 1.25
        }
      ]]
    }],
    "graph": null,
    "totalTokens": 15,
    "totalCost": 1.25,
    "currentStreak": 1,
    "longestStreak": 1
  }
}"#,
        )
        .unwrap();

        let clients = make_filters(&[ClientFilter::Claude], false);
        match load_cache(&clients, &GroupBy::Model) {
            CacheResult::Stale(data) => {
                assert!(
                    data.daily[0].source_breakdown.contains_key("unknown"),
                    "empty client should fall back to 'unknown'"
                );
                let unknown = data.daily[0].source_breakdown.get("unknown").unwrap();
                assert_eq!(unknown.models.len(), 1);
                let model = unknown.models.get("claude-sonnet-4-5").unwrap();
                assert_eq!(model.tokens.total(), 15);
            }
            other => panic!(
                "expected stale legacy cache, got {:?}",
                other_variant_name(&other)
            ),
        }

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
    }

    #[test]
    #[serial]
    fn load_cache_falls_back_to_legacy_dot_cache_path() {
        let temp_dir = TempDir::new().unwrap();
        let previous_home = env::var_os("HOME");
        let previous_override = env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            env::set_var("HOME", temp_dir.path());
            env::remove_var("TOKSCALE_CONFIG_DIR");
        }

        let legacy_path = temp_dir.path().join(".cache/tokscale/tui-data-cache.json");
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::write(
            &legacy_path,
            r#"{
  "schemaVersion": 6,
  "timestamp": 9999999999999,
  "enabledClients": ["claude"],
  "includeSynthetic": false,
  "groupBy": "model",
  "data": {
    "models": [],
    "agents": [],
    "daily": [],
    "hourly": [],
    "graph": null,
    "totalTokens": 0,
    "totalCost": 0.0,
    "currentStreak": 0,
    "longestStreak": 0
  }
}"#,
        )
        .unwrap();

        let clients = make_filters(&[ClientFilter::Claude], false);
        assert!(matches!(
            load_cache(&clients, &GroupBy::Model),
            CacheResult::Fresh(_)
        ));

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
        match previous_override {
            Some(value) => unsafe { env::set_var("TOKSCALE_CONFIG_DIR", value) },
            None => unsafe { env::remove_var("TOKSCALE_CONFIG_DIR") },
        }
    }

    #[test]
    #[serial]
    fn load_cache_skips_legacy_when_overridden() {
        let temp_dir = TempDir::new().unwrap();
        let override_dir = TempDir::new().unwrap();
        let previous_home = env::var_os("HOME");
        let previous_override = env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            env::set_var("HOME", temp_dir.path());
            env::set_var("TOKSCALE_CONFIG_DIR", override_dir.path());
        }

        let legacy_path = temp_dir.path().join(".cache/tokscale/tui-data-cache.json");
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        fs::write(
            &legacy_path,
            r#"{
  "schemaVersion": 6,
  "timestamp": 9999999999999,
  "enabledClients": ["claude"],
  "includeSynthetic": false,
  "groupBy": "model",
  "data": {
    "models": [],
    "agents": [],
    "daily": [],
    "hourly": [],
    "graph": null,
    "totalTokens": 0,
    "totalCost": 0.0,
    "currentStreak": 0,
    "longestStreak": 0
  }
}"#,
        )
        .unwrap();

        let clients = make_filters(&[ClientFilter::Claude], false);
        assert!(matches!(
            load_cache(&clients, &GroupBy::Model),
            CacheResult::Miss
        ));

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
        match previous_override {
            Some(value) => unsafe { env::set_var("TOKSCALE_CONFIG_DIR", value) },
            None => unsafe { env::remove_var("TOKSCALE_CONFIG_DIR") },
        }
    }

    #[test]
    #[serial]
    fn save_cached_data_does_not_delete_destination() {
        let temp_dir = TempDir::new().unwrap();
        let previous_home = env::var_os("HOME");
        let previous_override = env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            env::set_var("HOME", temp_dir.path());
            env::remove_var("TOKSCALE_CONFIG_DIR");
        }

        let cache_path = cache_file().unwrap();
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        let old_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        fs::write(
            &cache_path,
            format!(
                r#"{{
  "schemaVersion": 6,
  "timestamp": {old_timestamp},
  "enabledClients": ["claude"],
  "includeSynthetic": false,
  "groupBy": "model",
  "data": {{
    "models": [],
    "agents": [],
    "daily": [],
    "hourly": [],
    "graph": null,
    "totalTokens": 0,
    "totalCost": 0.0,
    "currentStreak": 0,
    "longestStreak": 0
  }}
}}"#
            ),
        )
        .unwrap();
        assert!(fs::metadata(&cache_path).is_ok());

        let clients = make_filters(&[ClientFilter::Claude], false);
        save_cached_data(&UsageData::default(), &clients, &GroupBy::Model);

        let metadata = fs::metadata(&cache_path).unwrap();
        assert!(metadata.is_file());
        let saved: CachedTUIData = serde_json::from_slice(&fs::read(&cache_path).unwrap()).unwrap();
        assert!(saved.timestamp >= old_timestamp);

        match previous_home {
            Some(home) => unsafe { env::set_var("HOME", home) },
            None => unsafe { env::remove_var("HOME") },
        }
        match previous_override {
            Some(value) => unsafe { env::set_var("TOKSCALE_CONFIG_DIR", value) },
            None => unsafe { env::remove_var("TOKSCALE_CONFIG_DIR") },
        }
    }

    fn other_variant_name(result: &CacheResult) -> &'static str {
        match result {
            CacheResult::Fresh(_) => "Fresh",
            CacheResult::Stale(_) => "Stale",
            CacheResult::Miss => "Miss",
        }
    }
}
