use std::{
    collections::HashMap,
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use anyhow::Result;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::helpers::capitalize;
use super::{UsageFetchReport, UsageFreshness, UsageMetric, UsageOutput};
#[path = "claude_state.rs"]
mod state;

const BETA_HEADER: &str = "oauth-2025-04-20";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

const COOLDOWN_DIR: &str = "claude-cooldowns";
const COOLDOWN_VERSION: u8 = 1;
static IN_MEMORY_COOLDOWNS: OnceLock<Mutex<HashMap<PathBuf, PersistedCooldown>>> = OnceLock::new();

/// Longest cooldown a `Retry-After` header can impose. A misbehaving proxy
/// that emits milliseconds instead of seconds (or a far-future date) must not
/// park the fetcher for hours.
const MAX_RETRY_AFTER_SECS: u64 = 3600;

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PersistedCooldown {
    version: u8,
    issued_at: u64,
    expires_at: u64,
}

fn cooldown_key(token: &str) -> String {
    // The token itself is never persisted; this only namespaces a cache file
    // so one Claude account cannot apply its Retry-After to another account.
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn cooldown_path_in(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join(COOLDOWN_DIR).join(format!("{key}.json"))
}

fn in_memory_cooldowns() -> &'static Mutex<HashMap<PathBuf, PersistedCooldown>> {
    IN_MEMORY_COOLDOWNS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn valid_cooldown(record: &PersistedCooldown, now: u64) -> Option<u64> {
    // Issuance time lets a clock moved backwards fail open instead of turning
    // an old record into a newly extended lockout. The duration cap also makes
    // manually corrupted or far-future records harmless.
    if record.version != COOLDOWN_VERSION
        || record.issued_at > now
        || record.expires_at <= now
        || record.expires_at < record.issued_at
        || record.expires_at - record.issued_at > MAX_RETRY_AFTER_SECS
    {
        return None;
    }
    Some(record.expires_at - now)
}

fn read_cooldown(path: &Path, now: u64) -> Option<u64> {
    let contents = fs::read_to_string(path).ok()?;
    let record = serde_json::from_str::<PersistedCooldown>(&contents).ok()?;
    valid_cooldown(&record, now)
}

fn staging_path(path: &Path) -> PathBuf {
    path.with_extension("tmp")
}

fn remove_own_staging_files(path: &Path) {
    let temporary = staging_path(path);
    let _ = fs::remove_file(temporary);

    // Older versions used a random suffix. Only reclaim files whose name is
    // derived from this exact account's committed filename, while holding the
    // same per-account lock, so another account's active staging file survives.
    let Some(parent) = path.parent() else {
        return;
    };
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return;
    };
    let prefix = format!("{stem}.");
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        let Some(entry_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let is_legacy_staging = entry_name
            .strip_prefix(&prefix)
            .and_then(|suffix| suffix.strip_suffix(".tmp"))
            .is_some_and(|suffix| uuid::Uuid::parse_str(suffix).is_ok());
        if is_legacy_staging {
            let _ = fs::remove_file(entry_path);
        }
    }
}

/// Keep lock retries bounded without tying tests to scheduler or fsync latency.
fn try_lock_cooldown<F>(lock: &fs::File, mut wait: F) -> bool
where
    F: FnMut(std::time::Duration),
{
    for _ in 0..10 {
        if lock.try_lock_exclusive().is_ok() {
            return true;
        }
        wait(std::time::Duration::from_millis(5));
    }
    false
}

fn write_cooldown_with_now<F, W>(
    path: &Path,
    record: &PersistedCooldown,
    now: F,
    wait: W,
) -> std::io::Result<()>
where
    F: FnOnce() -> u64,
    W: FnMut(std::time::Duration),
{
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("cooldown path has no parent"))?;
    fs::create_dir_all(parent)?;
    let lock_path = path.with_extension("lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    // Contention is bounded: preserve the longer cross-process cooldown when
    // possible, but never let a stuck cache lock hang an interactive refresh.
    if !try_lock_cooldown(&lock, wait) {
        return Ok(());
    }
    let lock_now = now();

    // A later 429 wins. Validate against the time the lock was obtained rather
    // than the handler's earlier timestamp: a delayed older handler must not
    // discard a newer record which was issued after that handler began.
    let existing = fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<PersistedCooldown>(&s).ok());
    if valid_cooldown(record, lock_now).is_none() {
        let _ = lock.unlock();
        return Ok(());
    }
    let selected = existing
        .filter(|current| {
            valid_cooldown(current, lock_now).is_some() && current.expires_at > record.expires_at
        })
        .unwrap_or(PersistedCooldown {
            version: record.version,
            issued_at: record.issued_at,
            expires_at: record.expires_at,
        });
    let data = serde_json::to_vec(&selected).map_err(io::Error::other)?;
    remove_own_staging_files(path);
    let temporary = staging_path(path);
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&data)?;
        file.sync_all()?;
        tokscale_core::fs_atomic::replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    let _ = lock.unlock();
    result
}

#[cfg(test)]
fn write_cooldown_at(
    path: &Path,
    record: &PersistedCooldown,
    lock_now: u64,
) -> std::io::Result<()> {
    write_cooldown_with_now(path, record, || lock_now, std::thread::sleep)
}

#[cfg(test)]
fn set_cooldown(path: &Path, retry_after_secs: u64, now: u64) {
    set_cooldown_with_now(path, retry_after_secs, now, now_epoch_secs);
}

fn set_cooldown_with_now<F>(path: &Path, retry_after_secs: u64, now: u64, current_now: F)
where
    F: Fn() -> u64,
{
    let deadline = now.saturating_add(retry_after_secs.min(MAX_RETRY_AFTER_SECS));
    let record = PersistedCooldown {
        version: COOLDOWN_VERSION,
        issued_at: now,
        expires_at: deadline,
    };
    // Keep the current TUI process protected even when the cache root is
    // inaccessible. This map is scoped by the hashed account file path.
    if let Ok(mut cooldowns) = in_memory_cooldowns().lock() {
        let memory_now = current_now();
        cooldowns.retain(|_, current| valid_cooldown(current, memory_now).is_some());
        if valid_cooldown(&record, memory_now).is_none() {
            return;
        }
        cooldowns
            .entry(path.to_path_buf())
            .and_modify(|current| {
                if current.expires_at < record.expires_at {
                    *current = record.clone();
                }
            })
            .or_insert_with(|| record.clone());
    }
    let _ = write_cooldown_with_now(path, &record, current_now, std::thread::sleep);
}

#[cfg(test)]
fn set_cooldown_at(path: &Path, retry_after_secs: u64, now: u64, current_now: u64) {
    set_cooldown_with_now(path, retry_after_secs, now, || current_now);
}

/// RFC 9110 allows `Retry-After` as delta-seconds or as an HTTP-date; a date
/// already in the past counts as zero. Both forms are clamped to
/// [`MAX_RETRY_AFTER_SECS`].
#[cfg(test)]
fn parse_retry_after(value: &str) -> Option<u64> {
    parse_retry_after_at(value, now_epoch_secs())
}

fn parse_retry_after_at(value: &str, now: u64) -> Option<u64> {
    let secs = value.parse::<u64>().ok().or_else(|| {
        chrono::DateTime::parse_from_rfc2822(value)
            .ok()
            .map(|d| (d.timestamp().max(0) as u64).saturating_sub(now))
    })?;
    Some(secs.min(MAX_RETRY_AFTER_SECS))
}

fn cooldown_remaining(path: &Path, now: u64) -> Option<u64> {
    let disk = read_cooldown(path, now);
    if let Ok(mut cooldowns) = in_memory_cooldowns().lock() {
        cooldowns.retain(|_, current| valid_cooldown(current, now).is_some());
        let memory = cooldowns
            .get(path)
            .and_then(|record| valid_cooldown(record, now));
        if let Some(remaining) = memory.into_iter().chain(disk).max() {
            return Some(remaining);
        }
    }
    disk
}

#[cfg(test)]
fn clear_cooldown() {
    in_memory_cooldowns().lock().unwrap().clear();
}

// `~/.claude/.credentials.json` and the macOS Keychain item
// `Claude Code-credentials` belong to Claude Code. This module is a quota
// viewer: it reads them and never writes them. Tokscale used to exchange Claude
// Code's refresh token on 401/403 and write the result back, but the write
// reconstructed the document from the few fields below, dropping every field
// tokscale does not model -- `scopes` among them -- which left Claude Code
// reporting "Not logged in" (#1001). An expired access token is Claude Code's
// to refresh on its next run, so a rejected token is reported as unavailable
// usage instead.
//
// On macOS, Claude Code keeps the durable login in Keychain and may leave a
// stale `~/.claude/.credentials.json` behind. Prefer a usable Keychain login:
// `expiresAt` describes token lifetime, not when the account last logged in.

#[derive(Debug, Clone, Deserialize)]
struct Credentials {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<Oauth>,
}

/// Deliberately does not model `refreshToken`: tokscale has no use for a
/// credential it must not spend. `expiresAt` is read only to detect a known
/// expired Keychain token when a usable file token is available.
#[derive(Debug, Clone, Deserialize)]
struct Oauth {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "expiresAt", default, deserialize_with = "lenient_expiry_ms")]
    expires_at: Option<i64>,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
}

fn lenient_expiry_ms<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Optional metadata must not make an otherwise usable login unreadable.
    Ok(serde_json::Value::deserialize(deserializer)?.as_i64())
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    five_hour: Option<Window>,
    seven_day: Option<Window>,
    seven_day_opus: Option<Window>,
    #[serde(default, deserialize_with = "lenient_limits")]
    limits: Vec<PlanLimit>,
}

/// A limits entry tokscale cannot parse must not cost the whole response: the
/// legacy windows and every other entry still report. The array is a moving
/// server-side schema, and this module only reads a handful of its fields.
///
/// The field is read as an untyped value rather than as a sequence for the same
/// reason: if `limits` ever stops being an array, the strongly-typed form fails
/// the whole document ("invalid type: map, expected a sequence") and takes the
/// legacy Session/Weekly windows down with it. A shape this module cannot walk
/// degrades to no scoped rows.
fn lenient_limits<'de, D>(deserializer: D) -> Result<Vec<PlanLimit>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| PlanLimit::deserialize(entry).ok())
                .collect()
        })
        .unwrap_or_default())
}

#[derive(Debug, Deserialize)]
struct Window {
    utilization: f64,
    resets_at: Option<String>,
}

/// One entry of the `limits` array Anthropic returns alongside the legacy
/// `five_hour`/`seven_day` windows. Model-scoped weekly caps arrive only here,
/// as `kind: "weekly_scoped"` with the model under `scope.model`.
#[derive(Debug, Deserialize)]
struct PlanLimit {
    kind: Option<String>,
    group: Option<String>,
    percent: Option<f64>,
    resets_at: Option<String>,
    scope: Option<LimitScope>,
    /// Marks the single window that is currently binding for the account, not
    /// whether a cap exists: in captured responses exactly one entry across all
    /// kinds is active, and it is the most-consumed one. Gating the scoped rows
    /// on it hides a model quota whenever another window happens to be the
    /// hottest, so it is only used as a tiebreak between duplicate entries.
    /// Optional because Claude Code's own schema does not model the field.
    is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct LimitScope {
    model: Option<ScopedModel>,
}

#[derive(Debug, Deserialize)]
struct ScopedModel {
    id: Option<String>,
    display_name: Option<String>,
}

/// A `weekly_scoped` entry that survived selection, kept with the identity
/// needed to dedup it before it reaches the metric list.
struct ScopedLimit {
    /// `scope.model.id`, lowercased. Authoritative when both sides have one.
    id: Option<String>,
    is_active: bool,
    metric: UsageMetric,
}

fn credentials_path() -> std::path::PathBuf {
    let home = crate::paths::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".claude").join(".credentials.json")
}

fn read_keychain() -> Result<String> {
    super::helpers::read_keychain("Claude Code-credentials")
}

pub fn has_credentials() -> bool {
    credentials_path().exists() || read_keychain().is_ok()
}

/// How [`fetch_policy`] obtains Claude Code's credential document. Injected
/// so tests can also drive the Keychain-sourced shape -- credentials present,
/// no file on disk -- which no test can produce through [`read_credentials`]
/// because the Keychain is a real macOS service.
type CredentialReader = fn() -> Result<Credentials>;

fn access_token(creds: &Credentials) -> Option<&str> {
    creds
        .claude_ai_oauth
        .as_ref()
        .and_then(|oauth| oauth.access_token.as_deref())
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

fn expiry_ms(creds: &Credentials) -> Option<i64> {
    creds
        .claude_ai_oauth
        .as_ref()
        .and_then(|oauth| oauth.expires_at)
}

/// Choose between the on-disk and Keychain copies of Claude Code's login.
///
/// Claude Code's durable store on macOS is Keychain. A leftover
/// `~/.claude/.credentials.json` must not win just because its token lives
/// longer, since it may belong to a previous account. Only a known expired
/// Keychain token allows a usable file credential to take precedence.
fn select_claude_credentials(
    file: Option<Credentials>,
    keychain: Option<Credentials>,
) -> Result<Credentials> {
    select_claude_credentials_at(file, keychain, chrono::Utc::now().timestamp_millis())
}

fn select_claude_credentials_at(
    file: Option<Credentials>,
    keychain: Option<Credentials>,
    now_ms: i64,
) -> Result<Credentials> {
    let file = file.filter(|creds| access_token(creds).is_some());
    let keychain = keychain.filter(|creds| access_token(creds).is_some());
    match (file, keychain) {
        (Some(file), Some(keychain)) => {
            let keychain_expired = expiry_ms(&keychain).is_some_and(|expiry| expiry <= now_ms);
            let file_expired = expiry_ms(&file).is_some_and(|expiry| expiry <= now_ms);
            if keychain_expired && !file_expired {
                Ok(file)
            } else {
                Ok(keychain)
            }
        }
        (Some(file), None) => Ok(file),
        (None, Some(keychain)) => Ok(keychain),
        (None, None) => {
            anyhow::bail!("No Claude credentials found. Run 'claude' to log in.")
        }
    }
}

fn parse_credentials_json(content: &str) -> Result<Credentials> {
    let creds: Credentials = serde_json::from_str(content)?;
    if access_token(&creds).is_none() {
        anyhow::bail!("Claude credentials have no access token");
    }
    Ok(creds)
}

fn read_file_credentials() -> Result<Credentials> {
    let content = std::fs::read_to_string(credentials_path())?;
    parse_credentials_json(&content)
}

fn read_keychain_credentials() -> Result<Credentials> {
    parse_credentials_json(&read_keychain()?)
}

fn read_credentials() -> Result<Credentials> {
    select_claude_credentials(
        read_file_credentials().ok(),
        read_keychain_credentials().ok(),
    )
}

#[derive(Debug)]
enum FetchError {
    Limited(Option<String>),
    Other(anyhow::Error),
}
impl From<reqwest::Error> for FetchError {
    fn from(error: reqwest::Error) -> Self {
        Self::Other(error.into())
    }
}
#[cfg(test)]
fn usage_client(timeout: std::time::Duration) -> anyhow::Result<reqwest::Client> {
    // Bound the complete request, including response-body reads, while retaining
    // the shared proxy and TLS configuration.
    Ok(tokscale_core::http::client_builder()
        .timeout(timeout)
        .build()?)
}

async fn fetch_usage(
    client: &reqwest::Client,
    usage_url: &str,
    token: &str,
) -> std::result::Result<UsageResponse, FetchError> {
    let resp = client
        .get(usage_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", BETA_HEADER)
        .send()
        .await?;
    let status = resp.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(FetchError::Limited(
            resp.headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned),
        ));
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(FetchError::Other(anyhow::anyhow!("Claude usage unavailable: stored access token was rejected (HTTP {status}). Run 'claude' so Claude Code can refresh its own login, then retry.")));
    }
    if !status.is_success() {
        return Err(FetchError::Other(anyhow::anyhow!(
            "Claude usage request failed (HTTP {status})"
        )));
    }
    Ok(resp.json().await?)
}

fn window_metric(label: &str, w: &Window) -> UsageMetric {
    let used = w.utilization.clamp(0.0, 100.0);
    UsageMetric {
        label: label.into(),
        used_percent: used,
        remaining_percent: 100.0 - used,
        remaining_label: None,
        resets_at: w.resets_at.clone(),
    }
}

/// The account-wide weekly cap is also reported as a scope in some responses;
/// `seven_day` already renders it, so a scope that names every model is not a
/// separate row. `all-models`, `all_models` and `All models` are the spellings
/// observed.
fn is_all_models_scope(value: &str) -> bool {
    let normalized = value
        .trim()
        .to_ascii_lowercase()
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    normalized == "all models"
}

/// Selects on what identifies a model-scoped weekly cap -- `kind`, the weekly
/// group, and a named model -- which is what Claude Code itself selects on.
/// Anything the entry does not carry (a percent, a display name, a reset time)
/// makes it unrenderable rather than fatal, so a partial entry is skipped and
/// the rest of the response still reports.
fn scoped_limit_metric(limit: &PlanLimit) -> Option<ScopedLimit> {
    if limit.kind.as_deref() != Some("weekly_scoped") {
        return None;
    }
    // `group` is absent from some entries; only an explicitly non-weekly group
    // disqualifies one, so a response that omits the field still reports.
    if limit
        .group
        .as_deref()
        .is_some_and(|group| group != "weekly")
    {
        return None;
    }

    let model = limit.scope.as_ref()?.model.as_ref()?;
    let id = model
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let label = model.display_name.as_deref()?.trim();
    if label.is_empty() {
        return None;
    }
    if is_all_models_scope(label) || id.is_some_and(is_all_models_scope) {
        return None;
    }

    let used = limit.percent?.clamp(0.0, 100.0);
    Some(ScopedLimit {
        id: id.map(str::to_ascii_lowercase),
        is_active: limit.is_active.unwrap_or(false),
        metric: UsageMetric {
            label: label.to_string(),
            used_percent: used,
            remaining_percent: 100.0 - used,
            remaining_label: None,
            resets_at: limit.resets_at.clone(),
        },
    })
}

/// Two entries describe the same quota when their model ids match; the display
/// name is the fallback for entries the server sends without an id. Name
/// matching is only reachable while one side is still id-less, because folding
/// sharpens a bucket's id (see [`usage_metrics`]), so an entry that arrives
/// without an id joins the first bucket wearing its label and every later entry
/// is then compared against a known id.
fn same_scoped_model(a: &ScopedLimit, b: &ScopedLimit) -> bool {
    match (a.id.as_deref(), b.id.as_deref()) {
        (Some(a_id), Some(b_id)) => a_id == b_id,
        _ => a.metric.label.eq_ignore_ascii_case(&b.metric.label),
    }
}

fn usage_metrics(resp: &UsageResponse) -> Vec<UsageMetric> {
    let mut metrics = Vec::new();
    if let Some(ref w) = resp.five_hour {
        metrics.push(window_metric("Session", w));
    }
    if let Some(ref w) = resp.seven_day {
        metrics.push(window_metric("Weekly", w));
    }
    if let Some(ref w) = resp.seven_day_opus {
        metrics.push(window_metric("Opus", w));
    }

    let mut scoped: Vec<ScopedLimit> = Vec::new();
    for candidate in resp.limits.iter().filter_map(scoped_limit_metric) {
        match scoped
            .iter_mut()
            .find(|existing| same_scoped_model(existing, &candidate))
        {
            Some(existing) => {
                // Every fold sharpens the bucket's identity, including the one
                // that replaces the reading: an id-less bucket adopts the first
                // id folded into it, and a bucket that has an id keeps it when
                // an id-less entry folds in. Without this, `same_scoped_model`
                // is not transitive -- an id-less "Opus" bucket would swallow
                // both `claude-opus-4-5` and `claude-opus-4-6` by name, dropping
                // a distinct quota, and which of the two survived depended on
                // whichever entry happened to carry `is_active`.
                let id = existing.id.take().or_else(|| candidate.id.clone());
                // The binding window is the authoritative reading when the same
                // model is reported twice; otherwise the first entry wins, so
                // the order the server sent is what renders.
                if candidate.is_active && !existing.is_active {
                    *existing = candidate;
                }
                existing.id = id;
            }
            None => scoped.push(candidate),
        }
    }

    // Only the legacy windows are merge targets: a scoped entry that names the
    // same model as `seven_day_opus` is the current reading of that quota, not a
    // second row. Identity between two scoped entries is already settled above,
    // so they are never folded into each other here.
    let legacy_count = metrics.len();
    let mut replaced = vec![false; legacy_count];
    for candidate in scoped {
        let mut metric = candidate.metric;
        let legacy_slot = metrics[..legacy_count]
            .iter()
            .enumerate()
            .find(|(index, existing)| {
                !replaced[*index] && existing.label.eq_ignore_ascii_case(&metric.label)
            })
            .map(|(index, _)| index);
        match legacy_slot {
            Some(index) => {
                // `resets_at` is nullable in the scoped entry; the legacy
                // window's reset time is better than a blank reset column.
                if metric.resets_at.is_none() {
                    metric.resets_at = metrics[index].resets_at.clone();
                }
                metrics[index] = metric;
                replaced[index] = true;
            }
            None => metrics.push(metric),
        }
    }

    metrics
}

/// The whole Claude usage path, with the only two things a test cannot supply
/// for real -- the usage endpoint and the credential source -- as parameters.
/// The destructive refresh-and-write this module was fixed for lived in exactly
/// this orchestration, so it is the layer the tests must enter; [`fetch_report`] is one
/// call with the production values and holds no logic that could diverge.
#[cfg(test)]
fn fetch_blocking(usage_url: &str, read: CredentialReader) -> Result<UsageOutput> {
    // Isolated test state prevents credential fixtures from touching real caches.
    let temp = tempfile::tempdir()?;
    fetch_blocking_in(usage_url, read, temp.path())
}
#[cfg(test)]
fn fetch_blocking_in(
    usage_url: &str,
    read: CredentialReader,
    cache_dir: &Path,
) -> Result<UsageOutput> {
    let report = fetch_report_at(usage_url, read, cache_dir, now_epoch_secs());
    report.outputs.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(report
            .diagnostics
            .first()
            .map(|d| d.message.clone())
            .unwrap_or_else(|| "Claude unavailable".into()))
    })
}

fn limited_report(state: &state::State, now: u64, message: String) -> UsageFetchReport {
    let mut report = UsageFetchReport::from_error("Claude", None, anyhow::anyhow!(message));
    report.outputs.extend(state.sample(now, true));
    report
}
#[cfg(test)]
fn fetch_report_at(
    usage_url: &str,
    read: CredentialReader,
    cache_dir: &Path,
    now: u64,
) -> UsageFetchReport {
    fetch_report_with_clock(usage_url, read, cache_dir, || now)
}
fn fetch_report_with_clock(
    usage_url: &str,
    read: CredentialReader,
    cache_dir: &Path,
    clock: impl Fn() -> u64,
) -> UsageFetchReport {
    match fetch_policy(usage_url, read, cache_dir, clock(), clock) {
        Ok(report) => report,
        Err(error) => UsageFetchReport::from_error("Claude", None, error),
    }
}
fn fetch_policy(
    usage_url: &str,
    read: CredentialReader,
    cache_dir: &Path,
    now: u64,
    clock: impl Fn() -> u64,
) -> Result<UsageFetchReport> {
    let creds = read()?;
    let token = access_token(&creds).ok_or_else(|| anyhow::anyhow!("No Claude access token."))?;
    let key = cooldown_key(token);
    let path = state::path(cache_dir, &key);
    let cached = state::load(&path, now);
    let local_lock = state::memory_lock(&path);
    let Ok(_guard) = local_lock.try_lock() else {
        return Ok(limited_report(
            &cached,
            clock(),
            "Claude usage request already in progress; retry shortly".into(),
        ));
    };
    let _disk_guard = match state::disk_lock(&path) {
        Ok(lock) => Some(lock),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            return Ok(limited_report(
                &cached,
                clock(),
                "Claude usage request already in progress; retry shortly".into(),
            ))
        }
        Err(_) => {
            let now = clock();
            let wait = cached.until.saturating_sub(now).max(
                cooldown_remaining(&cooldown_path_in(cache_dir, &key), now).unwrap_or_default(),
            );
            let message = if wait > 0 {
                format!("rate-limited; retry in {wait}s; restore cache directory access")
            } else {
                "Claude usage request coordination unavailable; restore cache directory access and retry".into()
            };
            return Ok(limited_report(&cached, now, message));
        }
    };
    let now = clock();
    let mut state = state::load(&path, now);
    let cooldown_file = cooldown_path_in(cache_dir, &key);
    let wait = cooldown_remaining(&cooldown_file, now)
        .unwrap_or_default()
        .max(state.until.saturating_sub(now));
    if wait > 0 {
        return Ok(limited_report(
            &state,
            now,
            format!("rate-limited; retry in {wait}s"),
        ));
    }
    if let Some(sample) = state.fresh(now) {
        return Ok(UsageFetchReport::from_outputs(vec![sample]));
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let response = rt.block_on(fetch_usage(
        &super::helpers::http_client()?,
        usage_url,
        token,
    ));
    let now = clock();
    match response {
        Err(FetchError::Limited(header)) => {
            let wait = state.limited(
                header
                    .as_deref()
                    .and_then(|value| parse_retry_after_at(value, now)),
                now,
            );
            state::save(&path, &mut state, now);
            set_cooldown_with_now(&cooldown_file, wait, now, || now);
            Ok(limited_report(
                &state,
                now,
                format!("HTTP 429; retry in {wait}s"),
            ))
        }
        Err(FetchError::Other(error)) => {
            // Rejected or unavailable credentials must not remain eligible on startup.
            state.sample = None;
            state::save(&path, &mut state, now);
            Err(error)
        }
        Ok(resp) => {
            let oauth = creds.claude_ai_oauth.as_ref().unwrap();
            let plan = oauth.subscription_type.as_ref().map(|s| {
                match oauth
                    .rate_limit_tier
                    .as_deref()
                    .and_then(|t| t.rsplit('_').next())
                {
                    Some(mult) => format!("{} {}", capitalize(s), mult),
                    None => capitalize(s),
                }
            });
            let output = UsageOutput {
                provider: "Claude".into(),
                account: None,
                credential_source: None,
                freshness: Some(UsageFreshness {
                    fetched_at: now,
                    stale: false,
                }),
                plan,
                email: None,
                metrics: usage_metrics(&resp),
                reset_credits: None,
                credit_status: None,
                spend_control: None,
            };
            state.sample = Some(output.clone());
            state.failures = 0;
            state.until = 0;
            state::save(&path, &mut state, now);
            Ok(UsageFetchReport::from_outputs(vec![output]))
        }
    }
}
pub fn fetch_report() -> UsageFetchReport {
    fetch_report_with_clock(
        USAGE_URL,
        read_credentials,
        &crate::paths::get_cache_dir(),
        now_epoch_secs,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::commands::usage::test_server::{spawn_server, Seen};

    fn assert_request_timeout(partial_body: bool) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (release, released) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "client never connected");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            };
            // macOS can inherit the listener's nonblocking flag on accept.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            if partial_body {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nContent-Type: application/json\r\n\r\n{").unwrap();
            }
            let _ = released.recv_timeout(Duration::from_secs(3));
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(1),
                fetch_usage(
                    &usage_client(Duration::from_millis(100)).unwrap(),
                    &url,
                    "test-token",
                ),
            )
            .await
        });
        let _ = release.send(());
        server.join().unwrap();
        let response = result.expect("HTTP client must time out before the watchdog");
        let Err(FetchError::Other(error)) = response else {
            panic!("expected an HTTP timeout, got {response:?}");
        };
        assert!(error.downcast_ref::<reqwest::Error>().unwrap().is_timeout());
    }

    #[test]
    fn request_times_out_waiting_for_headers() {
        assert_request_timeout(false);
    }

    #[test]
    fn request_times_out_reading_response_body() {
        assert_request_timeout(true);
    }

    #[test]
    fn policy_reuses_success_then_recovers_from_rate_limit() {
        let temp = tempfile::tempdir().unwrap();
        let (url, log) = spawn_server(|_, n| {
            if n == 1 {
                (429, "{}".into())
            } else {
                (200, USAGE_BODY.into())
            }
        });
        let read = |now| fetch_report_at(&url, keychain_credentials, temp.path(), now);
        let first = read(1000);
        assert_eq!(first.outputs.len(), 1);
        assert_eq!(
            read(1030).outputs[0].freshness.as_ref().unwrap().fetched_at,
            1000
        );
        assert_eq!(log.lock().unwrap().len(), 1);
        let limited = read(1060);
        assert!(limited.outputs[0].freshness.as_ref().unwrap().stale);
        assert_eq!(
            limited.outputs[0].freshness.as_ref().unwrap().fetched_at,
            1000
        );
        assert!(limited.diagnostics[0].message.contains("60s"));
        assert_eq!(read(1100).outputs.len(), 1);
        assert_eq!(log.lock().unwrap().len(), 2);
        let recovered = read(1120);
        assert!(recovered.diagnostics.is_empty());
        assert_eq!(
            recovered.outputs[0].freshness.as_ref().unwrap().fetched_at,
            1120
        );
        assert_eq!(log.lock().unwrap().len(), 3);
    }

    #[test]
    fn policy_rechecks_clock_after_request_lock() {
        let temp = tempfile::tempdir().unwrap();
        let (url, log) = spawn_server(|_, _| (200, USAGE_BODY.into()));
        fetch_report_at(&url, keychain_credentials, temp.path(), 1000);
        let calls = std::sync::atomic::AtomicU64::new(0);
        let report = fetch_report_with_clock(&url, keychain_credentials, temp.path(), || {
            if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                1030
            } else {
                4601
            }
        });
        assert_eq!(log.lock().unwrap().len(), 2);
        assert_eq!(
            report.outputs[0].freshness.as_ref().unwrap().fetched_at,
            4601
        );
    }

    #[test]
    fn policy_timestamps_sample_and_cooldown_at_response_completion() {
        for status in [200, 429] {
            let temp = tempfile::tempdir().unwrap();
            let (url, _) = spawn_server(move |_, _| {
                (
                    status,
                    if status == 200 {
                        USAGE_BODY.into()
                    } else {
                        "{}".into()
                    },
                )
            });
            let clock = std::sync::atomic::AtomicU64::new(0);
            let report = fetch_report_with_clock(&url, keychain_credentials, temp.path(), || {
                if clock.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
                    1000
                } else {
                    1060
                }
            });
            if status == 200 {
                assert_eq!(
                    report.outputs[0].freshness.as_ref().unwrap().fetched_at,
                    1060
                );
            } else {
                let path = state::path(temp.path(), &cooldown_key("stale-access-token"));
                assert_eq!(state::load(&path, 1060).until, 1120);
            }
        }
    }

    #[test]
    fn policy_http_date_uses_injected_clock() {
        let temp = tempfile::tempdir().unwrap();
        let url = super::super::test_server::spawn_raw_server(|_| {
            b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: Thu, 01 Jan 1970 00:17:40 GMT\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec()
        });
        let result = fetch_report_at(&url, keychain_credentials, temp.path(), 1000);
        assert!(
            result.diagnostics[0].message.contains("60s"),
            "{:?}",
            result.diagnostics
        );
    }

    #[test]
    fn policy_progressive_backoff_persists_and_success_resets() {
        let temp = tempfile::tempdir().unwrap();
        let (url, log) = spawn_server(|_, n| {
            if n == 3 {
                (200, USAGE_BODY.into())
            } else {
                (429, "{}".into())
            }
        });
        for (now, wait) in [(1000, 60), (1060, 120), (1180, 240)] {
            let r = fetch_report_at(&url, keychain_credentials, temp.path(), now);
            assert!(r.diagnostics[0].message.contains(&format!("{wait}s")));
            let _ = fetch_report_at(&url, keychain_credentials, temp.path(), now + 1);
        }
        assert_eq!(log.lock().unwrap().len(), 3);
        assert_eq!(
            fetch_report_at(&url, keychain_credentials, temp.path(), 1420)
                .outputs
                .len(),
            1
        );
        let limited = fetch_report_at(&url, keychain_credentials, temp.path(), 1480);
        assert!(limited.diagnostics[0].message.contains("60s"));
    }

    #[test]
    fn policy_excludes_expired_future_other_tokens_and_missing_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let (url, _) = spawn_server(|_, n| {
            if n == 0 {
                (200, USAGE_BODY.into())
            } else {
                (429, "{}".into())
            }
        });
        assert_eq!(
            fetch_report_at(&url, keychain_credentials, temp.path(), 1000)
                .outputs
                .len(),
            1
        );
        fn other() -> Result<Credentials> {
            parse_credentials_json(r#"{"claudeAiOauth":{"accessToken":"another"}}"#)
        }
        fn missing() -> Result<Credentials> {
            anyhow::bail!("missing")
        }
        assert!(fetch_report_at(&url, other, temp.path(), 1010)
            .outputs
            .is_empty());
        assert!(fetch_report_at(&url, missing, temp.path(), 1010)
            .outputs
            .is_empty());
        assert!(
            fetch_report_at(&url, keychain_credentials, temp.path(), 999)
                .outputs
                .is_empty()
        );
        assert!(
            fetch_report_at(&url, keychain_credentials, temp.path(), 4601)
                .outputs
                .is_empty()
        );
    }

    #[test]
    fn policy_invalid_retry_after_persists_progressive_one_hour_cap() {
        let temp = tempfile::tempdir().unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let seen = calls.clone();
        let url = super::super::test_server::spawn_raw_server(move |_| {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: invalid\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec()
        });
        let path = state::path(temp.path(), &cooldown_key("stale-access-token"));
        let mut now = 1000;
        for (index, wait) in [60, 120, 240, 480, 960, 1920, 3600, 3600]
            .into_iter()
            .enumerate()
        {
            state::forget_memory(&path);
            let report = fetch_report_at(&url, keychain_credentials, temp.path(), now);
            assert!(report.diagnostics[0].message.contains(&format!("{wait}s")));
            let during = fetch_report_at(&url, keychain_credentials, temp.path(), now + 1);
            assert!(during.diagnostics[0]
                .message
                .contains(&format!("{}s", wait - 1)));
            assert_eq!(
                calls.load(std::sync::atomic::Ordering::SeqCst),
                index as u64 + 1
            );
            now += wait;
        }
    }

    #[test]
    fn policy_never_falls_back_on_auth_or_transport_failures() {
        for status in [401, 403, 500] {
            let temp = tempfile::tempdir().unwrap();
            let (url, _) = spawn_server(move |_, n| {
                if n == 0 {
                    (200, USAGE_BODY.into())
                } else {
                    (status, "{}".into())
                }
            });
            assert_eq!(
                fetch_report_at(&url, keychain_credentials, temp.path(), 1000)
                    .outputs
                    .len(),
                1
            );
            assert!(
                fetch_report_at(&url, keychain_credentials, temp.path(), 1060)
                    .outputs
                    .is_empty()
            );
            let path = state::path(temp.path(), &cooldown_key("stale-access-token"));
            assert!(state::load(&path, 1060).sample(1060, true).is_none());
        }
    }

    #[test]
    fn policy_rejects_semantically_corrupt_samples() {
        for invalid_provider in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let (url, log) = spawn_server(|_, _| (200, USAGE_BODY.into()));
            fetch_report_at(&url, keychain_credentials, temp.path(), 1000);
            let path = state::path(temp.path(), &cooldown_key("stale-access-token"));
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            if invalid_provider {
                value["sample"]["provider"] = serde_json::json!("Other account");
            } else {
                value["sample"]["freshness"]["fetched_at"] = serde_json::json!(1001);
            }
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            state::forget_memory(&path);
            let result = fetch_report_at(&url, keychain_credentials, temp.path(), 1030);
            assert_eq!(result.outputs[0].provider, "Claude");
            assert_eq!(log.lock().unwrap().len(), 2);
        }
    }

    #[test]
    fn policy_corrupt_state_fails_open_and_durable_sample_survives_restart() {
        let temp = tempfile::tempdir().unwrap();
        let (url, log) = spawn_server(|_, _| (200, USAGE_BODY.into()));
        let path = state::path(temp.path(), &cooldown_key("stale-access-token"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "invalid json").unwrap();
        assert_eq!(
            fetch_report_at(&url, keychain_credentials, temp.path(), 1000)
                .outputs
                .len(),
            1
        );
        state::forget_memory(&path);
        let cached = fetch_report_at(&url, keychain_credentials, temp.path(), 1030);
        assert_eq!(
            cached.outputs[0].freshness.as_ref().unwrap().fetched_at,
            1000
        );
        assert_eq!(log.lock().unwrap().len(), 1);
        state::forget_memory(&path);
        assert_eq!(
            fetch_report_at(&url, keychain_credentials, temp.path(), 1060)
                .outputs
                .len(),
            1
        );
        assert_eq!(log.lock().unwrap().len(), 2);
        let raw = fs::read_to_string(path).unwrap();
        assert!(!raw.contains("stale-access-token"));
        assert!(!raw.contains("refreshToken"));
    }

    #[test]
    fn policy_concurrent_callers_issue_one_request() {
        let temp = tempfile::tempdir().unwrap();
        let (url, log) = spawn_server(|_, _| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            (200, USAGE_BODY.into())
        });
        let barrier = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    barrier.wait();
                    fetch_report_at(&url, keychain_credentials, temp.path(), 1000)
                });
            }
            barrier.wait();
        });
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[test]
    fn policy_bounded_request_contention_and_memory_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let (url, log) = spawn_server(|_, _| (200, USAGE_BODY.into()));
        let path = state::path(temp.path(), &cooldown_key("stale-access-token"));
        let lock = state::disk_lock(&path).unwrap();
        let started = std::time::Instant::now();
        let report = fetch_report_at(&url, keychain_credentials, temp.path(), 1000);
        assert!(report.outputs.is_empty());
        assert!(report.diagnostics[0].message.contains("in progress"));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(log.lock().unwrap().is_empty());
        drop(lock);
        let blocked = temp.path().join("not-a-directory");
        fs::write(&blocked, "blocked").unwrap();
        let first = fetch_report_at(&url, keychain_credentials, &blocked, 1000);
        assert!(first.outputs.is_empty());
        assert!(first.diagnostics[0].message.contains("cache"));
        let second = fetch_report_at(&url, keychain_credentials, &blocked, 1030);
        assert!(second.outputs.is_empty());
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn policy_same_second_auth_rejection_survives_failed_state_write() {
        for status in [401, 403] {
            let temp = tempfile::tempdir().unwrap();
            let path = state::path(temp.path(), &cooldown_key("stale-access-token"));
            let server_path = path.clone();
            let backup = path.with_extension("backup");
            let server_backup = backup.clone();
            let calls = std::sync::atomic::AtomicU64::new(0);
            let url = super::super::test_server::spawn_raw_server(move |_| {
                let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let (code, header, body) = match n {
                    0 => (200, "", USAGE_BODY),
                    1 => (429, "Retry-After: 0\r\n", "{}"),
                    _ => {
                        fs::rename(&server_path, &server_backup).unwrap();
                        fs::create_dir(&server_path).unwrap();
                        (status, "", "{}")
                    }
                };
                format!("HTTP/1.1 {code} Test\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).into_bytes()
            });
            assert_eq!(
                fetch_report_at(&url, keychain_credentials, temp.path(), 1000)
                    .outputs
                    .len(),
                1
            );
            assert_eq!(
                fetch_report_at(&url, keychain_credentials, temp.path(), 1060)
                    .outputs
                    .len(),
                1
            );
            assert!(
                fetch_report_at(&url, keychain_credentials, temp.path(), 1060)
                    .outputs
                    .is_empty()
            );
            fs::remove_dir(&path).unwrap();
            fs::rename(&backup, &path).unwrap();
            assert!(
                state::load(&path, 1060).sample(1060, true).is_none(),
                "rejected quota resurrected from equal-time disk state"
            );
            // A counter collision must also prefer the invalidation over quota.
            let mut disk: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            disk["revision"] = serde_json::json!(disk["revision"].as_u64().unwrap() + 1);
            fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();
            assert!(state::load(&path, 1060).sample(1060, true).is_none());
        }
    }

    #[test]
    fn policy_newer_disk_invalidation_beats_same_second_memory_sample() {
        let temp = tempfile::tempdir().unwrap();
        let (url, _) = spawn_server(|_, _| (200, USAGE_BODY.into()));
        fetch_report_at(&url, keychain_credentials, temp.path(), 1000);
        let path = state::path(temp.path(), &cooldown_key("stale-access-token"));
        let mut disk: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        disk["revision"] = serde_json::json!(disk["revision"].as_u64().unwrap_or(0) + 1);
        disk["sample"] = serde_json::Value::Null;
        fs::write(&path, serde_json::to_vec(&disk).unwrap()).unwrap();
        assert!(state::load(&path, 1000).sample(1000, true).is_none());
    }

    #[test]
    fn policy_lock_io_failure_blocks_http_across_processes() {
        const CACHE_ENV: &str = "TOKSCALE_TEST_CLAUDE_LOCK_CACHE";
        const URL_ENV: &str = "TOKSCALE_TEST_CLAUDE_LOCK_URL";
        if let Some(path) = std::env::var_os(CACHE_ENV) {
            let report = fetch_report_at(
                &std::env::var(URL_ENV).unwrap(),
                keychain_credentials,
                Path::new(&path),
                1000,
            );
            assert!(report.outputs.is_empty());
            assert!(report.diagnostics[0]
                .message
                .contains("cache directory access"));
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let blocked = temp.path().join("cache");
        fs::write(&blocked, "blocked").unwrap();
        let (url, log) = spawn_server(|_, _| (200, USAGE_BODY.into()));
        let mut children = Vec::new();
        for _ in 0..2 {
            children.push(std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact","commands::usage::claude::tests::policy_lock_io_failure_blocks_http_across_processes"])
                .env(CACHE_ENV,&blocked).env(URL_ENV,&url)
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
                .spawn().unwrap());
        }
        for child in children {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn policy_retains_success_when_storage_fails_after_lock_acquisition() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        let sample_path = state::path(&cache, &cooldown_key("stale-access-token"));
        let server_sample_path = sample_path.clone();
        let (url, log) = spawn_server(move |_, _| {
            // A directory at the JSON destination prevents atomic replacement
            // without moving the directory containing the open request lock.
            fs::create_dir(&server_sample_path).unwrap();
            (200, USAGE_BODY.into())
        });
        assert_eq!(
            fetch_report_at(&url, keychain_credentials, &cache, 1000)
                .outputs
                .len(),
            1
        );
        assert!(sample_path.is_dir(), "the sample write must have failed");
        let fresh = fetch_report_at(&url, keychain_credentials, &cache, 1030);
        assert_eq!(fresh.outputs.len(), 1);
        assert!(!fresh.outputs[0].freshness.as_ref().unwrap().stale);
        assert!(fresh.diagnostics.is_empty());
        // The request has completed, so its lock file can now be replaced on
        // Windows too. This separately simulates unavailable coordination.
        let lock_path = sample_path.with_extension("request.lock");
        fs::remove_file(&lock_path).unwrap();
        fs::create_dir(&lock_path).unwrap();
        let cached = fetch_report_at(&url, keychain_credentials, &cache, 1030);
        assert_eq!(
            cached.outputs[0].freshness.as_ref().unwrap().fetched_at,
            1000
        );
        assert!(cached.outputs[0].freshness.as_ref().unwrap().stale);
        assert!(cached.diagnostics[0]
            .message
            .contains("cache directory access"));
        assert_eq!(log.lock().unwrap().len(), 1);
        assert!(fetch_report_at(&url, keychain_credentials, &cache, 4601)
            .outputs
            .is_empty());
        // Fixing access restores coordinated requests instead of permanent lockout.
        fs::remove_dir(&lock_path).unwrap();
        fs::remove_dir(&sample_path).unwrap();
        let (recovered_url, recovered_log) = spawn_server(|_, _| (200, USAGE_BODY.into()));
        assert_eq!(
            fetch_report_at(&recovered_url, keychain_credentials, &cache, 4601)
                .outputs
                .len(),
            1
        );
        assert_eq!(recovered_log.lock().unwrap().len(), 1);
    }

    /// A Claude Code credential document with the fields tokscale models
    /// (`accessToken`, `expiresAt`, `subscriptionType`, `rateLimitTier`), the
    /// fields it does not (`refreshToken`, `scopes`), and a key that does not
    /// exist today -- Claude Code owns the schema and may add more.
    const FIXTURE: &str = r#"{
  "claudeAiOauth": {
    "accessToken": "stale-access-token",
    "refreshToken": "claude-code-owned-refresh-token",
    "expiresAt": 1757000000000,
    "scopes": ["user:inference", "user:profile"],
    "subscriptionType": "max",
    "rateLimitTier": "default_max_20x"
  },
  "someKeyTokscaleDoesNotModel": { "keep": true }
}"#;

    const USAGE_BODY: &str =
        r#"{"five_hour":{"utilization":12.5,"resets_at":"2026-08-03T12:00:00Z"}}"#;

    /// A response carrying both the legacy windows and the `limits` array, with
    /// the entry shapes that must not reach the metric list: a non-weekly kind,
    /// the account-wide weekly entry `seven_day` already reports, and a scoped
    /// entry with no model to name.
    const SCOPED_USAGE_BODY: &str = r#"{
  "five_hour": { "utilization": 0, "resets_at": "2026-08-12T05:30:00Z" },
  "seven_day": { "utilization": 30, "resets_at": "2026-08-17T04:00:00Z" },
  "seven_day_opus": null,
  "limits": [
    { "kind": "session", "group": "session", "percent": 0, "is_active": false },
    { "kind": "weekly_all", "group": "weekly", "percent": 30, "is_active": false },
    {
      "kind": "weekly_scoped",
      "group": "weekly",
      "percent": 47,
      "resets_at": "2026-08-17T04:00:00Z",
      "scope": { "model": { "id": null, "display_name": "Fable" } },
      "is_active": true
    },
    {
      "kind": "weekly_scoped",
      "group": "weekly",
      "percent": 90,
      "scope": { "model": { "display_name": "Sonnet" } },
      "is_active": false
    },
    { "kind": "weekly_scoped", "percent": 99, "is_active": true }
  ]
}"#;

    /// Trimmed from a live Anthropic response captured on 2026-08-07 (published
    /// verbatim in egoist/waku `src/usage.rs`). It is the ordinary case this
    /// module has to handle: the session window is the binding one, so the Fable
    /// weekly cap the user is being metered against carries `is_active: false`.
    const LIVE_USAGE_BODY: &str = r#"{
  "five_hour": {"utilization": 41.0, "resets_at": "2026-08-07T14:59:59.729061+00:00"},
  "seven_day": {"utilization": 20.0, "resets_at": "2026-08-13T11:59:59.729091+00:00"},
  "seven_day_opus": null,
  "limits": [
    {"kind": "session", "group": "session", "percent": 41, "severity": "normal",
     "resets_at": "2026-08-07T14:59:59.729061+00:00", "scope": null, "is_active": true},
    {"kind": "weekly_all", "group": "weekly", "percent": 20, "severity": "normal",
     "resets_at": "2026-08-13T11:59:59.729091+00:00", "scope": null, "is_active": false},
    {"kind": "weekly_scoped", "group": "weekly", "percent": 38, "severity": "normal",
     "resets_at": "2026-08-13T11:59:59.729307+00:00",
     "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null},
     "is_active": false}
  ]
}"#;

    fn labels(metrics: &[UsageMetric]) -> Vec<&str> {
        metrics.iter().map(|m| m.label.as_str()).collect()
    }

    fn parse(body: &str) -> UsageResponse {
        serde_json::from_str(body).expect("usage response parses")
    }

    /// Mirrors the path component of [`USAGE_URL`] so the recorded request line
    /// is the same string production would produce.
    const USAGE_PATH: &str = "/api/oauth/usage";

    fn spawn_usage_server(usage_status: u16) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let (base, log) = spawn_server(move |path, _| {
            if path == USAGE_PATH {
                (usage_status, USAGE_BODY.to_string())
            } else {
                (404, "{}".to_string())
            }
        });
        (format!("{base}{USAGE_PATH}"), log)
    }

    /// The request log is asserted alongside the credential bytes because a
    /// refresh-and-retry regression shows up here as a second request even when
    /// it writes nothing to disk. It only sees traffic aimed at the injected
    /// URL: the refresh this change removed POSTed to an absolute
    /// `platform.claude.com` URL that no local server can intercept, so byte
    /// equality is still what actually pins #1001.
    fn assert_only_usage_request(log: &Arc<Mutex<Vec<Seen>>>) {
        let seen = log.lock().expect("request log");
        assert_eq!(
            *seen,
            vec![Seen {
                request: format!("GET {USAGE_PATH}"),
                // The fixture's token, so this also proves the credential came
                // from the injected reader and not from the real `$HOME`.
                bearer: Some("stale-access-token".to_string()),
            }],
            "tokscale made a request other than the single usage GET"
        );
    }

    struct HomeGuard {
        previous: Option<std::ffi::OsString>,
        dir: std::path::PathBuf,
    }

    impl HomeGuard {
        fn new(name: &str) -> Self {
            let previous = std::env::var_os("HOME");
            let dir = std::env::temp_dir().join(format!(
                "tokscale-claude-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join(".claude")).expect("create fake home");
            unsafe {
                std::env::set_var("HOME", &dir);
            }
            Self { previous, dir }
        }

        fn credentials(&self) -> std::path::PathBuf {
            self.dir.join(".claude").join(".credentials.json")
        }

        fn write_fixture(&self) {
            std::fs::write(self.credentials(), FIXTURE).expect("write fixture credentials");
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var("HOME", value),
                    None => std::env::remove_var("HOME"),
                }
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Stands in for a Keychain-sourced credential: the same document, with
    /// nothing on disk. `read_credentials()` cannot be made to produce this
    /// shape in a test because the Keychain is a real macOS service.
    fn keychain_credentials() -> Result<Credentials> {
        Ok(serde_json::from_str(FIXTURE)?)
    }

    /// File-only reader for HomeGuard tests. Production [`read_credentials`]
    /// also consults the real Keychain, which on a developer Mac can outrank
    /// the fixture as Claude Code's durable login store.
    fn file_only_credentials() -> Result<Credentials> {
        read_file_credentials()
    }

    fn creds_with(access: &str, expires_at: Option<i64>) -> Credentials {
        Credentials {
            claude_ai_oauth: Some(Oauth {
                access_token: Some(access.to_string()),
                expires_at,
                subscription_type: None,
                rate_limit_tier: None,
            }),
        }
    }

    #[test]
    fn select_credentials_prefers_live_keychain_over_expired_file() {
        let file = creds_with("file-token", Some(1_000));
        let keychain = creds_with("keychain-token", Some(2_000));
        let selected = select_claude_credentials_at(Some(file), Some(keychain), 1_500).unwrap();
        assert_eq!(access_token(&selected), Some("keychain-token"));
    }

    #[test]
    fn select_credentials_prefers_keychain_when_expiries_tie() {
        let selected = select_claude_credentials_at(
            Some(creds_with("file-token", Some(2_000))),
            Some(creds_with("keychain-token", Some(2_000))),
            1_500,
        )
        .unwrap();
        assert_eq!(access_token(&selected), Some("keychain-token"));
    }

    #[test]
    fn select_credentials_prefers_keychain_when_file_has_no_expiry() {
        let selected = select_claude_credentials_at(
            Some(creds_with("file-token", None)),
            Some(creds_with("keychain-token", Some(2_000))),
            1_500,
        )
        .unwrap();
        assert_eq!(access_token(&selected), Some("keychain-token"));
    }

    #[test]
    fn select_credentials_uses_whichever_side_has_an_access_token() {
        let from_keychain =
            select_claude_credentials(None, Some(creds_with("keychain-token", None))).unwrap();
        assert_eq!(access_token(&from_keychain), Some("keychain-token"));

        let from_file =
            select_claude_credentials(Some(creds_with("file-token", None)), None).unwrap();
        assert_eq!(access_token(&from_file), Some("file-token"));
    }

    #[test]
    fn select_credentials_does_not_treat_longer_token_lifetime_as_newer_login() {
        let selected = select_claude_credentials(
            Some(creds_with("previous-account-token", Some(i64::MAX))),
            Some(creds_with("current-account-token", Some(i64::MAX - 1))),
        )
        .unwrap();
        assert_eq!(access_token(&selected), Some("current-account-token"));
    }

    #[test]
    fn select_credentials_prefers_keychain_without_expiry_over_live_file() {
        let selected = select_claude_credentials(
            Some(creds_with("previous-account-token", Some(i64::MAX))),
            Some(creds_with("current-account-token", None)),
        )
        .unwrap();
        assert_eq!(access_token(&selected), Some("current-account-token"));
    }

    #[test]
    fn select_credentials_uses_file_when_keychain_is_expired() {
        for file_expiry in [Some(i64::MAX), None] {
            let selected = select_claude_credentials(
                Some(creds_with("file-token", file_expiry)),
                Some(creds_with("expired-keychain-token", Some(0))),
            )
            .unwrap();
            assert_eq!(access_token(&selected), Some("file-token"));
        }
    }

    #[test]
    fn select_credentials_treats_expiry_at_current_time_as_expired() {
        let selected = select_claude_credentials_at(
            Some(creds_with("file-token", Some(2_000))),
            Some(creds_with("expired-keychain-token", Some(1_500))),
            1_500,
        )
        .unwrap();
        assert_eq!(access_token(&selected), Some("file-token"));
    }

    #[test]
    fn select_credentials_keeps_durable_account_when_both_tokens_are_expired() {
        let selected = select_claude_credentials_at(
            Some(creds_with("previous-account-token", Some(2_000))),
            Some(creds_with("current-account-token", Some(1_000))),
            3_000,
        )
        .unwrap();
        assert_eq!(access_token(&selected), Some("current-account-token"));
    }

    #[test]
    fn credential_parsing_accepts_missing_or_null_expiry() {
        for document in [
            serde_json::json!({"claudeAiOauth": {"accessToken": "valid-access-token"}}),
            serde_json::json!({
                "claudeAiOauth": {"accessToken": "valid-access-token", "expiresAt": null}
            }),
        ] {
            let credentials = parse_credentials_json(&document.to_string()).unwrap();
            assert_eq!(access_token(&credentials), Some("valid-access-token"));
            assert_eq!(expiry_ms(&credentials), None);
        }
    }

    #[test]
    fn credential_parsing_ignores_noninteger_expiry() {
        for expiry in [
            serde_json::json!("1757000000000"),
            serde_json::json!(1.5),
            serde_json::json!(true),
            serde_json::json!({"value": 1757000000000_i64}),
            serde_json::json!([1757000000000_i64]),
            serde_json::json!(u64::MAX),
        ] {
            let document = serde_json::json!({
                "claudeAiOauth": {
                    "accessToken": "valid-access-token",
                    "expiresAt": expiry,
                    "subscriptionType": "max"
                }
            });
            let credentials = parse_credentials_json(&document.to_string())
                .expect("unrecognized expiry must not discard a valid access token");
            assert_eq!(access_token(&credentials), Some("valid-access-token"));
            assert_eq!(expiry_ms(&credentials), None);
            assert_eq!(
                credentials
                    .claude_ai_oauth
                    .unwrap()
                    .subscription_type
                    .as_deref(),
                Some("max")
            );
        }
    }

    /// #1001: a rejected access token must not make tokscale rewrite Claude
    /// Code's credential file. Byte equality is the assertion that matters --
    /// any reconstruction of the document fails it, whatever fields it keeps.
    /// Entry is through `fetch_blocking` with a file-only reader so the HomeGuard
    /// fixture is the credential under test, not a live Keychain login.
    #[test]
    #[serial_test::serial]
    fn rejected_token_leaves_claude_credentials_untouched() {
        clear_cooldown();
        let home = HomeGuard::new("401");
        home.write_fixture();
        let before = std::fs::read(home.credentials()).expect("read before");
        let (usage_url, log) = spawn_usage_server(401);

        let result = fetch_blocking(&usage_url, file_only_credentials);

        let err = result.expect_err("401 must surface as an error, not a refresh");
        assert!(
            err.to_string().contains("Run 'claude'"),
            "error should point at Claude Code's own login, got: {err}"
        );

        let after = std::fs::read(home.credentials()).expect("credentials must still exist");
        assert_eq!(
            String::from_utf8_lossy(&after),
            String::from_utf8_lossy(&before),
            "tokscale rewrote Claude Code's credential file"
        );
        assert_only_usage_request(&log);
    }

    /// A 403 takes the same branch as a 401 and must be just as inert.
    #[test]
    #[serial_test::serial]
    fn forbidden_response_leaves_claude_credentials_untouched() {
        clear_cooldown();
        let home = HomeGuard::new("403");
        home.write_fixture();
        let before = std::fs::read(home.credentials()).expect("read before");
        let (usage_url, log) = spawn_usage_server(403);

        let result = fetch_blocking(&usage_url, file_only_credentials);

        assert!(result.is_err(), "403 must surface as an error");
        let after = std::fs::read(home.credentials()).expect("credentials must still exist");
        assert_eq!(
            String::from_utf8_lossy(&after),
            String::from_utf8_lossy(&before),
            "tokscale rewrote Claude Code's credential file"
        );
        assert_only_usage_request(&log);
    }

    /// On macOS the credentials live in the Keychain and the file does not
    /// exist. Tokscale must not conjure a partial one -- the old code did,
    /// because it read from the Keychain but wrote to the file unconditionally.
    #[test]
    #[serial_test::serial]
    fn rejected_token_does_not_create_a_credential_file() {
        clear_cooldown();
        let home = HomeGuard::new("nofile");
        let (usage_url, log) = spawn_usage_server(401);

        let result = fetch_blocking(&usage_url, keychain_credentials);

        assert!(result.is_err(), "401 must surface as an error");
        assert!(
            !home.credentials().exists(),
            "tokscale created a credential file Claude Code did not have"
        );
        assert_only_usage_request(&log);
    }

    #[test]
    #[serial_test::serial]
    fn successful_usage_fetch_leaves_claude_credentials_untouched() {
        clear_cooldown();
        let home = HomeGuard::new("200");
        home.write_fixture();
        let before = std::fs::read(home.credentials()).expect("read before");
        let (usage_url, log) = spawn_usage_server(200);

        let output = fetch_blocking(&usage_url, file_only_credentials)
            .expect("200 usage response should parse");

        assert_eq!(output.plan.as_deref(), Some("Max 20x"));
        assert_eq!(output.metrics.len(), 1);
        assert_eq!(output.metrics[0].label, "Session");
        assert!((output.metrics[0].used_percent - 12.5).abs() < f64::EPSILON);

        let after = std::fs::read(home.credentials()).expect("credentials must still exist");
        assert_eq!(
            String::from_utf8_lossy(&after),
            String::from_utf8_lossy(&before),
            "tokscale rewrote Claude Code's credential file"
        );
        assert_only_usage_request(&log);
    }

    #[test]
    fn exposes_model_scoped_weekly_limits() {
        let metrics = usage_metrics(&parse(SCOPED_USAGE_BODY));

        assert_eq!(labels(&metrics), ["Session", "Weekly", "Fable", "Sonnet"]);
        let fable = &metrics[2];
        assert!((fable.used_percent - 47.0).abs() < f64::EPSILON);
        assert!((fable.remaining_percent - 53.0).abs() < f64::EPSILON);
        assert_eq!(fable.resets_at.as_deref(), Some("2026-08-17T04:00:00Z"));
    }

    /// The regression this module was carrying: `is_active` marks the single
    /// binding window, so gating the scoped row on it hid the model quota
    /// whenever the session window happened to be the hottest -- which is what
    /// this captured response shows.
    #[test]
    fn inactive_scoped_limit_is_still_reported() {
        let metrics = usage_metrics(&parse(LIVE_USAGE_BODY));

        assert_eq!(labels(&metrics), ["Session", "Weekly", "Fable"]);
        let fable = &metrics[2];
        assert!((fable.used_percent - 38.0).abs() < f64::EPSILON);
        assert!((fable.remaining_percent - 62.0).abs() < f64::EPSILON);
        assert_eq!(
            fable.resets_at.as_deref(),
            Some("2026-08-13T11:59:59.729307+00:00")
        );
    }

    /// `is_active` is absent from Claude Code's own schema for this array, so a
    /// response that omits it must not lose its scoped rows.
    #[test]
    fn scoped_limit_without_is_active_is_reported() {
        let metrics = usage_metrics(&parse(
            r#"{
  "limits": [{
    "kind": "weekly_scoped",
    "group": "weekly",
    "percent": 23,
    "resets_at": "2026-08-17T04:00:00Z",
    "scope": { "model": { "display_name": "Fable" } }
  }]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Fable"]);
        assert!((metrics[0].used_percent - 23.0).abs() < f64::EPSILON);
    }

    /// A fresh week reports the cap at 0%; it is still a cap, and it is exactly
    /// the reading a user refreshes to see.
    #[test]
    fn scoped_limit_at_zero_percent_is_reported() {
        let metrics = usage_metrics(&parse(
            r#"{
  "limits": [{
    "kind": "weekly_scoped",
    "group": "weekly",
    "percent": 0,
    "scope": { "model": { "display_name": "Fable" } },
    "is_active": false
  }]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Fable"]);
        assert_eq!(metrics[0].used_percent, 0.0);
        assert_eq!(metrics[0].remaining_percent, 100.0);
    }

    #[test]
    fn every_scoped_model_gets_its_own_row() {
        let metrics = usage_metrics(&parse(
            r#"{
  "five_hour": { "utilization": 10, "resets_at": "session-reset" },
  "seven_day": { "utilization": 20, "resets_at": "weekly-reset" },
  "limits": [
    { "kind": "weekly_scoped", "group": "weekly", "percent": 38,
      "scope": { "model": { "id": "claude-fable-1", "display_name": "Fable" } },
      "is_active": false },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 61,
      "scope": { "model": { "id": "claude-opus-4-6", "display_name": "Opus" } },
      "is_active": true },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 4,
      "scope": { "model": { "id": "claude-sonnet-4-6", "display_name": "Sonnet" } },
      "is_active": false }
  ]
}"#,
        ));

        assert_eq!(
            labels(&metrics),
            ["Session", "Weekly", "Fable", "Opus", "Sonnet"]
        );
        assert!((metrics[3].used_percent - 61.0).abs() < f64::EPSILON);
    }

    #[test]
    fn scoped_limit_replaces_matching_legacy_model_window() {
        let metrics = usage_metrics(&parse(
            r#"{
  "seven_day_opus": { "utilization": 60, "resets_at": "legacy" },
  "limits": [{
    "kind": "weekly_scoped",
    "percent": 25,
    "resets_at": "current",
    "scope": { "model": { "display_name": "Opus" } },
    "is_active": true
  }]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Opus"]);
        assert!((metrics[0].used_percent - 25.0).abs() < f64::EPSILON);
        assert_eq!(metrics[0].resets_at.as_deref(), Some("current"));
    }

    /// `resets_at` is nullable on a scoped entry. Replacing the legacy window
    /// wholesale would blank the reset column the legacy window did carry.
    #[test]
    fn replacing_a_legacy_window_keeps_its_reset_time() {
        let metrics = usage_metrics(&parse(
            r#"{
  "seven_day_opus": { "utilization": 60, "resets_at": "2026-08-17T04:00:00Z" },
  "limits": [{
    "kind": "weekly_scoped",
    "group": "weekly",
    "percent": 25,
    "resets_at": null,
    "scope": { "model": { "display_name": "Opus" } },
    "is_active": false
  }]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Opus"]);
        assert!((metrics[0].used_percent - 25.0).abs() < f64::EPSILON);
        assert_eq!(
            metrics[0].resets_at.as_deref(),
            Some("2026-08-17T04:00:00Z")
        );
    }

    /// The account-wide weekly cap is `seven_day`; a scope naming every model is
    /// the same number under another label.
    #[test]
    fn all_models_scope_does_not_duplicate_the_weekly_row() {
        let metrics = usage_metrics(&parse(
            r#"{
  "seven_day": { "utilization": 20, "resets_at": "weekly-reset" },
  "limits": [
    { "kind": "weekly_scoped", "group": "weekly", "percent": 20,
      "scope": { "model": { "id": "all-models", "display_name": "All models" } },
      "is_active": true },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 20,
      "scope": { "model": { "display_name": "all_models" } },
      "is_active": false }
  ]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Weekly"]);
    }

    /// Same model reported twice: one row, and the binding entry is the reading
    /// that survives. The ids match, so the differing display names do not make
    /// them two models.
    #[test]
    fn duplicate_scoped_models_dedupe_on_model_id() {
        let metrics = usage_metrics(&parse(
            r#"{
  "limits": [
    { "kind": "weekly_scoped", "group": "weekly", "percent": 12,
      "scope": { "model": { "id": "claude-fable-1", "display_name": "Fable 1" } },
      "is_active": false },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 38,
      "resets_at": "binding-reset",
      "scope": { "model": { "id": "claude-fable-1", "display_name": "Fable" } },
      "is_active": true }
  ]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Fable"]);
        assert!((metrics[0].used_percent - 38.0).abs() < f64::EPSILON);
        assert_eq!(metrics[0].resets_at.as_deref(), Some("binding-reset"));
    }

    /// Distinct ids are distinct quotas even when the server sends the same
    /// display name for both.
    #[test]
    fn distinct_model_ids_are_not_deduped_by_display_name() {
        let metrics = usage_metrics(&parse(
            r#"{
  "limits": [
    { "kind": "weekly_scoped", "group": "weekly", "percent": 12,
      "scope": { "model": { "id": "claude-opus-4-5", "display_name": "Opus" } },
      "is_active": false },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 38,
      "scope": { "model": { "id": "claude-opus-4-6", "display_name": "Opus" } },
      "is_active": false }
  ]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Opus", "Opus"]);
        assert!((metrics[0].used_percent - 12.0).abs() < f64::EPSILON);
        assert!((metrics[1].used_percent - 38.0).abs() < f64::EPSILON);
    }

    /// An entry the server sent without an id must not swallow the models that
    /// do carry one. Name matching is the only identity an id-less entry has, so
    /// it folds into the first bucket wearing its label -- and that fold has to
    /// sharpen the bucket's id, or the bucket keeps matching every later id by
    /// name and the third entry's distinct quota is silently dropped.
    #[test]
    fn id_less_entry_does_not_swallow_distinct_model_ids() {
        let metrics = usage_metrics(&parse(
            r#"{
  "limits": [
    { "kind": "weekly_scoped", "group": "weekly", "percent": 10,
      "scope": { "model": { "id": null, "display_name": "Opus" } },
      "is_active": false },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 20,
      "scope": { "model": { "id": "claude-opus-4-5", "display_name": "Opus" } },
      "is_active": false },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 30,
      "scope": { "model": { "id": "claude-opus-4-6", "display_name": "Opus" } },
      "is_active": false }
  ]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Opus", "Opus"]);
        assert!((metrics[0].used_percent - 10.0).abs() < f64::EPSILON);
        assert!((metrics[1].used_percent - 30.0).abs() < f64::EPSILON);
    }

    /// The same three entries with the binding flag on the middle one. Which
    /// entry happens to be active decides which reading survives the fold, and
    /// it must not decide how many quotas the account has.
    #[test]
    fn active_entry_does_not_change_how_many_scoped_rows_survive() {
        let metrics = usage_metrics(&parse(
            r#"{
  "limits": [
    { "kind": "weekly_scoped", "group": "weekly", "percent": 10,
      "scope": { "model": { "id": null, "display_name": "Opus" } },
      "is_active": false },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 20,
      "scope": { "model": { "id": "claude-opus-4-5", "display_name": "Opus" } },
      "is_active": true },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 30,
      "scope": { "model": { "id": "claude-opus-4-6", "display_name": "Opus" } },
      "is_active": false }
  ]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Opus", "Opus"]);
        assert!((metrics[0].used_percent - 20.0).abs() < f64::EPSILON);
        assert!((metrics[1].used_percent - 30.0).abs() < f64::EPSILON);
    }

    /// Taking the binding entry's reading must not cost the bucket the id it
    /// already knew: the active entry here has none, and a bucket reset to
    /// id-less would go on to swallow the distinct model that follows.
    #[test]
    fn active_id_less_entry_keeps_the_bucket_model_id() {
        let metrics = usage_metrics(&parse(
            r#"{
  "limits": [
    { "kind": "weekly_scoped", "group": "weekly", "percent": 20,
      "scope": { "model": { "id": "claude-opus-4-5", "display_name": "Opus" } },
      "is_active": false },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 40,
      "scope": { "model": { "display_name": "Opus" } },
      "is_active": true },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 30,
      "scope": { "model": { "id": "claude-opus-4-6", "display_name": "Opus" } },
      "is_active": false }
  ]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Opus", "Opus"]);
        assert!((metrics[0].used_percent - 40.0).abs() < f64::EPSILON);
        assert!((metrics[1].used_percent - 30.0).abs() < f64::EPSILON);
    }

    /// Sharpening the bucket id must not hand the same model two rows: an entry
    /// that arrives without an id still folds into the bucket its label names.
    #[test]
    fn id_less_entries_still_fold_into_an_identified_bucket() {
        let metrics = usage_metrics(&parse(
            r#"{
  "limits": [
    { "kind": "weekly_scoped", "group": "weekly", "percent": 12,
      "scope": { "model": { "id": null, "display_name": "Opus" } },
      "is_active": false },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 38,
      "resets_at": "binding-reset",
      "scope": { "model": { "id": "claude-opus-4-6", "display_name": "Opus" } },
      "is_active": true },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 99,
      "scope": { "model": { "display_name": "opus" } },
      "is_active": false }
  ]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Opus"]);
        assert!((metrics[0].used_percent - 38.0).abs() < f64::EPSILON);
        assert_eq!(metrics[0].resets_at.as_deref(), Some("binding-reset"));
    }

    /// `limits` is a server-side schema, and its container type is as much a
    /// moving part as its entries. A `limits` that stops being an array must
    /// cost the scoped rows only -- a strongly-typed sequence fails the whole
    /// document ("invalid type: map, expected a sequence") and the user sees
    /// "Claude usage unavailable" instead of Session and Weekly.
    #[test]
    fn non_array_limits_does_not_sink_the_response() {
        for limits in [
            r#"{ "weekly_scoped": [] }"#,
            r#""weekly_scoped""#,
            "7",
            "true",
        ] {
            let body = format!(
                r#"{{
  "five_hour": {{ "utilization": 41, "resets_at": "session-reset" }},
  "seven_day": {{ "utilization": 20, "resets_at": "weekly-reset" }},
  "limits": {limits}
}}"#
            );

            let metrics = usage_metrics(&parse(&body));

            assert_eq!(
                labels(&metrics),
                ["Session", "Weekly"],
                "legacy windows lost to a `limits` of {limits}"
            );
        }
    }

    /// A scoped entry outside the weekly group is not a weekly quota row.
    #[test]
    fn non_weekly_group_scoped_entry_is_ignored() {
        let metrics = usage_metrics(&parse(
            r#"{
  "seven_day": { "utilization": 20, "resets_at": "weekly-reset" },
  "limits": [{
    "kind": "weekly_scoped",
    "group": "session",
    "percent": 99,
    "scope": { "model": { "display_name": "Fable" } },
    "is_active": true
  }]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Weekly"]);
    }

    /// Entries the server sends without the parts a row needs are skipped one by
    /// one; they must not take the rest of the response down with them, and must
    /// not overwrite a legacy window with a blank reading.
    #[test]
    fn unrenderable_scoped_entries_are_skipped_individually() {
        let metrics = usage_metrics(&parse(
            r#"{
  "five_hour": { "utilization": 10, "resets_at": "session-reset" },
  "seven_day_opus": { "utilization": 60, "resets_at": "legacy-reset" },
  "limits": [
    { "kind": "weekly_scoped", "group": "weekly", "percent": 50, "is_active": true },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 50, "scope": {}, "is_active": true },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 50,
      "scope": { "model": { "id": "claude-opus-4-6" } }, "is_active": true },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 50,
      "scope": { "model": { "display_name": "   " } }, "is_active": true },
    { "kind": "weekly_scoped", "group": "weekly",
      "scope": { "model": { "display_name": "Opus" } }, "is_active": true },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 38,
      "resets_at": "fable-reset",
      "scope": { "model": { "display_name": "Fable" } }, "is_active": false }
  ]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Session", "Opus", "Fable"]);
        assert!((metrics[1].used_percent - 60.0).abs() < f64::EPSILON);
        assert_eq!(metrics[1].resets_at.as_deref(), Some("legacy-reset"));
        assert!((metrics[2].used_percent - 38.0).abs() < f64::EPSILON);
    }

    /// The `limits` array is a server-side schema this module reads a few fields
    /// of. An entry it cannot parse is dropped on its own; the legacy windows
    /// and the entries it can parse still report.
    #[test]
    fn unparseable_limit_entry_does_not_sink_the_response() {
        let metrics = usage_metrics(&parse(
            r#"{
  "five_hour": { "utilization": 41, "resets_at": "session-reset" },
  "limits": [
    { "kind": 7, "group": "weekly", "percent": 50 },
    { "kind": "weekly_scoped", "group": "weekly", "percent": "38",
      "scope": { "model": { "display_name": "Sonnet" } } },
    { "kind": "weekly_scoped", "group": "weekly", "percent": 38,
      "scope": { "model": { "display_name": "Fable" } }, "is_active": false }
  ]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Session", "Fable"]);
        assert!((metrics[1].used_percent - 38.0).abs() < f64::EPSILON);
    }

    /// A response with no `limits` array at all is the old shape, and still has
    /// to report its legacy windows.
    #[test]
    fn response_without_limits_reports_legacy_windows() {
        let metrics = usage_metrics(&parse(
            r#"{
  "five_hour": { "utilization": 12.5, "resets_at": "session-reset" },
  "seven_day": { "utilization": 30, "resets_at": "weekly-reset" },
  "seven_day_opus": { "utilization": 60, "resets_at": "opus-reset" }
}"#,
        ));

        assert_eq!(labels(&metrics), ["Session", "Weekly", "Opus"]);
    }

    /// Out-of-range percents are clamped to the same 0-100 scale the legacy
    /// `utilization` windows use, so the remaining half of the bar stays sane.
    #[test]
    fn scoped_percent_is_clamped_to_the_utilization_scale() {
        let metrics = usage_metrics(&parse(
            r#"{
  "limits": [
    { "kind": "weekly_scoped", "group": "weekly", "percent": 140,
      "scope": { "model": { "display_name": "Fable" } }, "is_active": true },
    { "kind": "weekly_scoped", "group": "weekly", "percent": -3,
      "scope": { "model": { "display_name": "Sonnet" } }, "is_active": false }
  ]
}"#,
        ));

        assert_eq!(labels(&metrics), ["Fable", "Sonnet"]);
        assert_eq!(metrics[0].used_percent, 100.0);
        assert_eq!(metrics[0].remaining_percent, 0.0);
        assert_eq!(metrics[1].used_percent, 0.0);
        assert_eq!(metrics[1].remaining_percent, 100.0);
    }

    /// File parsing is the source of truth for what tokscale models. The refresh
    /// token must not survive the round trip: the fix is only real if the field
    /// is absent from the model, not merely unused by the caller.
    #[test]
    #[serial_test::serial]
    fn parsed_credentials_do_not_carry_a_refresh_token() {
        let home = HomeGuard::new("parse");
        home.write_fixture();

        let oauth = file_only_credentials()
            .expect("fixture credentials parse")
            .claude_ai_oauth
            .expect("fixture has claudeAiOauth");

        assert_eq!(oauth.access_token.as_deref(), Some("stale-access-token"));
        assert_eq!(oauth.expires_at, Some(1_757_000_000_000));
        let debug = format!("{oauth:?}");
        assert!(
            !debug.contains("claude-code-owned-refresh-token"),
            "Oauth still models a refresh token: {debug}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn cooldown_is_inactive_by_default() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        assert!(cooldown_remaining(&temp.path().join("missing.json"), 1_000).is_none());
    }

    #[test]
    #[serial_test::serial]
    fn persisted_cooldown_activates_and_expires_without_sleeping() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        set_cooldown_at(&path, 60, 1_000, 1_000);
        assert_eq!(cooldown_remaining(&path, 1_010), Some(50));
        clear_cooldown(); // simulates a new tokscale process
        assert_eq!(cooldown_remaining(&path, 1_010), Some(50));
        assert!(cooldown_remaining(&path, 1_060).is_none());
    }

    #[test]
    #[serial_test::serial]
    fn cooldown_blocks_fetch_blocking() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let path = cooldown_path_in(temp.path(), &cooldown_key("stale-access-token"));
        set_cooldown(&path, 60, now_epoch_secs());
        let (usage_url, log) = spawn_usage_server(200);
        let result = fetch_blocking_in(&usage_url, keychain_credentials, temp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("rate-limited"));
        assert!(log.lock().unwrap().is_empty(), "cooldown must prevent HTTP");
        clear_cooldown();
    }

    #[test]
    #[serial_test::serial]
    fn rate_limit_persists_across_process_state() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let (usage_url, log) = spawn_usage_server(429);
        let first = fetch_blocking_in(&usage_url, keychain_credentials, temp.path());
        assert!(first.unwrap_err().to_string().contains("HTTP 429"));
        clear_cooldown(); // Simulates an independent tokscale invocation.
        let second = fetch_blocking_in(&usage_url, keychain_credentials, temp.path());
        assert!(second.unwrap_err().to_string().contains("rate-limited"));
        assert_eq!(
            log.lock().unwrap().len(),
            1,
            "persistent cooldown made no second request"
        );
        clear_cooldown();
    }

    #[test]
    #[serial_test::serial]
    fn persisted_cooldown_is_visible_to_child_process() {
        const CHILD_PATH: &str = "TOKSCALE_CLAUDE_COOLDOWN_CHILD_PATH";
        if let Some(path) = std::env::var_os(CHILD_PATH) {
            assert!(cooldown_remaining(Path::new(&path), now_epoch_secs()).is_some());
            return;
        }

        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        set_cooldown(&path, 60, now_epoch_secs());
        clear_cooldown();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::usage::claude::tests::persisted_cooldown_is_visible_to_child_process",
                "--nocapture",
            ])
            .env(CHILD_PATH, &path)
            .status()
            .expect("launch child test process");
        assert!(status.success(), "child process did not observe cooldown");
    }

    /// A proxy that emits `Retry-After` in milliseconds -- or any absurdly
    /// large value -- must not impose an hours-long cooldown.
    #[test]
    fn huge_retry_after_clamps_to_the_max() {
        assert_eq!(
            parse_retry_after("999999999999"),
            Some(MAX_RETRY_AFTER_SECS)
        );
        assert_eq!(
            parse_retry_after(&u64::MAX.to_string()),
            Some(MAX_RETRY_AFTER_SECS)
        );
    }

    /// The deadline arithmetic must not overflow even if a huge wait reaches
    /// `set_cooldown` directly.
    #[test]
    #[serial_test::serial]
    fn set_cooldown_saturates_instead_of_overflowing() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        set_cooldown_at(&path, u64::MAX, u64::MAX - 10, u64::MAX - 10);
        assert_eq!(cooldown_remaining(&path, u64::MAX - 1), Some(1));
        clear_cooldown();
    }

    #[test]
    #[serial_test::serial]
    fn malformed_expired_future_and_backward_clock_records_fail_open() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        fs::write(&path, "not json").unwrap();
        assert!(cooldown_remaining(&path, 1_000).is_none());

        for record in [
            PersistedCooldown {
                version: COOLDOWN_VERSION,
                issued_at: 900,
                expires_at: 999,
            },
            PersistedCooldown {
                version: COOLDOWN_VERSION,
                issued_at: 1_001,
                expires_at: 1_020,
            },
            PersistedCooldown {
                version: COOLDOWN_VERSION,
                issued_at: 1_000,
                expires_at: 1_000 + MAX_RETRY_AFTER_SECS + 1,
            },
        ] {
            fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
            clear_cooldown();
            assert!(cooldown_remaining(&path, 1_000).is_none(), "{record:?}");
        }
    }

    #[test]
    #[serial_test::serial]
    fn in_memory_cooldown_fails_open_after_clock_rollback() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        set_cooldown_at(&path, 60, 1_000, 1_000);
        assert!(cooldown_remaining(&path, 999).is_none());
        clear_cooldown();
    }

    #[test]
    #[serial_test::serial]
    fn longer_disk_cooldown_wins_over_shorter_memory_cooldown() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        set_cooldown_at(&path, 30, 1_000, 1_000);
        fs::write(
            &path,
            serde_json::to_vec(&PersistedCooldown {
                version: COOLDOWN_VERSION,
                issued_at: 1_000,
                expires_at: 1_090,
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(cooldown_remaining(&path, 1_000), Some(90));
        clear_cooldown();
    }

    #[test]
    #[serial_test::serial]
    fn cooldown_still_blocks_http_when_cache_cannot_be_written() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let blocked_cache = temp.path().join("cache");
        fs::create_dir_all(&blocked_cache).unwrap();
        let sample_path = state::path(&blocked_cache, &cooldown_key("stale-access-token"));
        let cooldown_file = cooldown_path_in(&blocked_cache, &cooldown_key("stale-access-token"));
        let server_sample_path = sample_path.clone();
        let server_cooldown_file = cooldown_file.clone();
        let (usage_url, log) = spawn_server(move |_, _| {
            // Block both JSON destinations after acquiring the request lock.
            // Renaming its open parent directory would fail on Windows.
            fs::create_dir(&server_sample_path).unwrap();
            fs::create_dir_all(&server_cooldown_file).unwrap();
            (429, "{}".into())
        });
        let first = fetch_blocking_in(&usage_url, keychain_credentials, &blocked_cache);
        assert!(first.unwrap_err().to_string().contains("HTTP 429"));
        assert!(sample_path.is_dir());
        assert!(cooldown_file.is_dir());
        let second = fetch_blocking_in(&usage_url, keychain_credentials, &blocked_cache);
        assert!(second.unwrap_err().to_string().contains("retry in"));
        assert_eq!(log.lock().unwrap().len(), 1);
        clear_cooldown();
    }

    #[test]
    #[serial_test::serial]
    fn cooldown_is_scoped_to_access_token_and_cache_root() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let other_root = tempfile::tempdir().unwrap();
        let path = cooldown_path_in(temp.path(), &cooldown_key("token-one"));
        let other_token = cooldown_path_in(temp.path(), &cooldown_key("token-two"));
        let other_profile = cooldown_path_in(other_root.path(), &cooldown_key("token-one"));
        set_cooldown_at(&path, 60, 1_000, 1_000);
        assert_eq!(cooldown_remaining(&path, 1_000), Some(60));
        assert!(cooldown_remaining(&other_token, 1_000).is_none());
        assert!(cooldown_remaining(&other_profile, 1_000).is_none());
        let serialized = fs::read_to_string(&path).unwrap();
        assert!(!serialized.contains("token-one"));
        assert!(!path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("token-one"));
        clear_cooldown();
    }

    #[test]
    #[serial_test::serial]
    fn valid_retry_after_replaces_malformed_and_future_records() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        for contents in [
            "not json".as_bytes(),
            &serde_json::to_vec(&PersistedCooldown {
                version: COOLDOWN_VERSION,
                issued_at: 1_000,
                expires_at: 1_000 + MAX_RETRY_AFTER_SECS + 1,
            })
            .unwrap(),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            set_cooldown_at(&path, 60, 1_000, 1_000);
            clear_cooldown();
            assert_eq!(cooldown_remaining(&path, 1_000), Some(60));
        }
    }

    #[test]
    #[serial_test::serial]
    fn shorter_update_does_not_shorten_active_cooldown() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        // Each uncontended write must persist the maximum, in either order.
        // Clear memory after every write so it cannot hide a shortened disk record.
        for waits in [[30, 90, 10], [90, 30, 10]] {
            clear_cooldown();
            let _ = fs::remove_file(&path);
            let mut longest = 0;
            for wait in waits {
                set_cooldown_at(&path, wait, 1_000, 1_000);
                longest = longest.max(wait);
                clear_cooldown();
                assert_eq!(cooldown_remaining(&path, 1_000), Some(longest));
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn delayed_older_writer_preserves_newer_active_cooldown() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        let newer = PersistedCooldown {
            version: COOLDOWN_VERSION,
            issued_at: 1_010,
            expires_at: 1_100,
        };
        let older = PersistedCooldown {
            version: COOLDOWN_VERSION,
            issued_at: 1_000,
            expires_at: 1_030,
        };

        write_cooldown_at(&path, &newer, 1_010).unwrap();
        write_cooldown_with_now(
            &path,
            &older,
            || {
                let lock = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path.with_extension("lock"))
                    .unwrap();
                assert!(
                    lock.try_lock_exclusive().is_err(),
                    "clock sampled after lock acquisition"
                );
                1_020
            },
            std::thread::sleep,
        )
        .unwrap();
        assert_eq!(read_cooldown(&path, 1_020), Some(80));

        set_cooldown_at(&path, 90, 1_010, 1_010);
        set_cooldown_at(&path, 30, 1_000, 1_020);
        clear_cooldown();
        assert_eq!(cooldown_remaining(&path, 1_020), Some(80));
    }

    #[test]
    #[serial_test::serial]
    fn writer_replaces_malformed_and_future_records_at_lock_time() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        let replacement = PersistedCooldown {
            version: COOLDOWN_VERSION,
            issued_at: 1_020,
            expires_at: 1_080,
        };

        for contents in [
            "not json".as_bytes(),
            &serde_json::to_vec(&PersistedCooldown {
                version: COOLDOWN_VERSION,
                issued_at: 1_021,
                expires_at: 1_080,
            })
            .unwrap(),
        ] {
            fs::write(&path, contents).unwrap();
            write_cooldown_at(&path, &replacement, 1_020).unwrap();
            assert_eq!(read_cooldown(&path, 1_020), Some(60));
        }
    }

    #[test]
    #[serial_test::serial]
    fn delayed_writer_does_not_persist_record_invalid_at_lock_time() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        let active = PersistedCooldown {
            version: COOLDOWN_VERSION,
            issued_at: 1_020,
            expires_at: 1_080,
        };
        let stale = PersistedCooldown {
            version: COOLDOWN_VERSION,
            issued_at: 1_000,
            expires_at: 1_030,
        };
        write_cooldown_at(&path, &active, 1_020).unwrap();
        write_cooldown_at(&path, &stale, 1_040).unwrap();
        assert_eq!(read_cooldown(&path, 1_040), Some(40));

        set_cooldown_at(&path, 60, 1_050, 1_040);
        set_cooldown_at(&path, 30, 1_000, 1_040);
        clear_cooldown();
        assert_eq!(cooldown_remaining(&path, 1_040), Some(40));
    }

    #[test]
    #[serial_test::serial]
    fn writer_reclaims_own_orphaned_staging_files_without_touching_other_accounts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("account-one.json");
        let other_path = temp.path().join("account-two.json");
        let own_orphan = staging_path(&path);
        let legacy_own_orphan = path.with_extension("00000000-0000-0000-0000-000000000001.tmp");
        let own_non_staging = path.with_extension("manual.tmp");
        let other_active = other_path.with_extension("00000000-0000-0000-0000-000000000002.tmp");
        fs::write(&own_orphan, "interrupted writer").unwrap();
        fs::write(&legacy_own_orphan, "interrupted legacy writer").unwrap();
        fs::write(&own_non_staging, "not our staging format").unwrap();
        fs::write(&other_active, "another account is writing").unwrap();

        let record = PersistedCooldown {
            version: COOLDOWN_VERSION,
            issued_at: 1_000,
            expires_at: 1_060,
        };
        write_cooldown_at(&path, &record, 1_000).unwrap();

        assert_eq!(read_cooldown(&path, 1_000), Some(60));
        assert!(!own_orphan.exists());
        assert!(!legacy_own_orphan.exists());
        assert_eq!(
            fs::read_to_string(own_non_staging).unwrap(),
            "not our staging format"
        );
        assert_eq!(
            fs::read_to_string(other_active).unwrap(),
            "another account is writing"
        );
    }

    #[test]
    fn cooldown_lock_stops_after_its_retry_budget() {
        // Use separate opens: cloned handles can share flock locks on Unix.
        let temp = tempfile::NamedTempFile::new().unwrap();
        let holder = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(temp.path())
            .unwrap();
        let contender = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(temp.path())
            .unwrap();
        holder.lock_exclusive().unwrap();
        let mut waits = Vec::new();
        assert!(!try_lock_cooldown(&contender, |duration| {
            waits.push(duration);
            assert!(waits.len() <= 10, "lock acquisition exceeded retry budget");
        }));
        assert_eq!(waits, vec![std::time::Duration::from_millis(5); 10]);
        holder.unlock().unwrap();
    }

    #[test]
    fn cooldown_lock_can_succeed_on_its_last_attempt() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let holder = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(temp.path())
            .unwrap();
        let contender = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(temp.path())
            .unwrap();
        holder.lock_exclusive().unwrap();
        let mut waits = 0;
        assert!(try_lock_cooldown(&contender, |_| {
            waits += 1;
            if waits == 9 {
                holder.unlock().unwrap();
            }
        }));
        assert_eq!(waits, 9);
        assert!(holder.try_lock_exclusive().is_err());
        contender.unlock().unwrap();
        assert!(try_lock_cooldown(&holder, |_| panic!(
            "free lock must not wait"
        )));
        holder.unlock().unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn contended_writer_leaves_committed_and_staging_files_untouched() {
        clear_cooldown();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        let committed = PersistedCooldown {
            version: COOLDOWN_VERSION,
            issued_at: 1_000,
            expires_at: 1_060,
        };
        fs::write(&path, serde_json::to_vec(&committed).unwrap()).unwrap();
        let staging = staging_path(&path);
        let legacy = path.with_extension("00000000-0000-0000-0000-000000000003.tmp");
        fs::write(&staging, "active writer").unwrap();
        fs::write(&legacy, "active legacy writer").unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))
            .unwrap();
        lock.lock_exclusive().unwrap();

        // The held lock forces persistence to exhaust its bounded retry budget.
        // The process must still enforce the longer cooldown in memory.
        set_cooldown_at(&path, 90, 1_000, 1_000);
        assert_eq!(cooldown_remaining(&path, 1_000), Some(90));
        set_cooldown_at(&path, 30, 1_000, 1_000);
        assert_eq!(cooldown_remaining(&path, 1_000), Some(90));

        assert_eq!(fs::read_to_string(&staging).unwrap(), "active writer");
        assert_eq!(fs::read_to_string(&legacy).unwrap(), "active legacy writer");
        assert_eq!(
            fs::read(&path).unwrap(),
            serde_json::to_vec(&committed).unwrap()
        );
        clear_cooldown();
        assert_eq!(cooldown_remaining(&path, 1_000), Some(60));
        lock.unlock().unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn concurrent_writers_preserve_the_later_cooldown_in_memory_when_disk_is_locked() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cooldown.json");
        clear_cooldown();
        // Hold the lock until both workers finish. Neither can persist, regardless
        // of thread scheduling or coverage overhead, so this tests the memory max.
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))
            .unwrap();
        lock.lock_exclusive().unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for wait in [30, 90] {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                set_cooldown_at(&path, wait, 1_000, 1_000);
            }));
        }
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(cooldown_remaining(&path, 1_000), Some(90));
        assert!(
            !path.exists(),
            "contended writers must not commit without a lock"
        );
        clear_cooldown();
        assert_eq!(cooldown_remaining(&path, 1_000), None);
        lock.unlock().unwrap();
    }

    /// Two writers contend for the disk lock and both commit. The first holds
    /// the lock, paused in its `now` hook (sampled after acquisition); the
    /// second loses its first attempt and is parked in its injected wait until
    /// the first has committed. Channels drive every step, so neither thread
    /// scheduling nor fsync latency decides what reaches the disk -- unlike a
    /// real 5 ms sleep, the parked wait cannot run out the retry budget.
    #[test]
    fn contending_writers_both_persist_and_keep_the_longer_cooldown() {
        use std::sync::mpsc;
        for (first, second) in [(30, 90), (90, 30)] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("cooldown.json");
            let record = |wait: u64| PersistedCooldown {
                version: COOLDOWN_VERSION,
                issued_at: 1_000,
                expires_at: 1_000 + wait,
            };

            let (first_locked_tx, first_locked) = mpsc::channel();
            let (release_first_tx, release_first) = mpsc::channel::<()>();
            let first_writer = {
                let path = path.clone();
                let record = record(first);
                std::thread::spawn(move || {
                    write_cooldown_with_now(
                        &path,
                        &record,
                        move || {
                            first_locked_tx.send(()).unwrap();
                            release_first.recv().unwrap();
                            1_000
                        },
                        |_| panic!("the first writer must find the lock free"),
                    )
                })
            };
            first_locked.recv().unwrap();

            let (second_waiting_tx, second_waiting) = mpsc::channel();
            let (resume_second_tx, resume_second) = mpsc::channel::<()>();
            let second_writer = {
                let path = path.clone();
                let record = record(second);
                std::thread::spawn(move || {
                    let mut waits = 0;
                    write_cooldown_with_now(
                        &path,
                        &record,
                        || 1_000,
                        move |_| {
                            waits += 1;
                            assert_eq!(waits, 1, "the second writer must retry exactly once");
                            second_waiting_tx.send(()).unwrap();
                            resume_second.recv().unwrap();
                        },
                    )
                })
            };
            // The second writer has tried the lock and lost.
            second_waiting.recv().unwrap();
            release_first_tx.send(()).unwrap();
            first_writer.join().unwrap().unwrap();
            assert_eq!(read_cooldown(&path, 1_000), Some(first));

            resume_second_tx.send(()).unwrap();
            second_writer.join().unwrap().unwrap();
            assert_eq!(
                read_cooldown(&path, 1_000),
                Some(first.max(second)),
                "{first}s then {second}s"
            );
        }
    }

    /// RFC 9110 also allows `Retry-After` as an HTTP-date. That form must
    /// yield the actual wait, not fall through to the 5s default and re-hammer
    /// the endpoint; a date in the past waits for nothing, and a far-future
    /// date clamps like a huge delta-seconds value does.
    #[test]
    fn http_date_retry_after_is_honored() {
        let http_date =
            |d: chrono::DateTime<chrono::Utc>| d.format("%a, %d %b %Y %H:%M:%S GMT").to_string();

        let soon = http_date(chrono::Utc::now() + chrono::Duration::seconds(120));
        let wait = parse_retry_after(&soon).expect("HTTP-date Retry-After parses");
        assert!(
            (110..=120).contains(&wait),
            "expected ~120s from {soon}, got {wait}"
        );

        let past = http_date(chrono::Utc::now() - chrono::Duration::seconds(120));
        assert_eq!(parse_retry_after(&past), Some(0));

        let far = http_date(chrono::Utc::now() + chrono::Duration::days(30));
        assert_eq!(parse_retry_after(&far), Some(MAX_RETRY_AFTER_SECS));

        assert_eq!(parse_retry_after("not-a-date"), None);
    }
}
