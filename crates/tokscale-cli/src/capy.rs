//! Capy (capy.ai) usage integration.
//!
//! Capy runs its agent loop on Capy's servers, so a thread on a connected
//! subscription such as Codex leaves no local session file. The usage API is
//! the only record: `GET /api/v1/usage/series` returns token totals per time
//! bucket, user, thread and model, and counts every route, charged or not.
//! `tokscale capy sync` caches those rows; `tokscale-core` parses the cache.

use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokscale_core::sessions::capy::{credentials_identity, CapyCredentials};

const CAPY_API_BASE: &str = "https://api.capy.ai/api/v1";
const CAPY_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const CAPY_API_KEY_ENV: &str = "CAPY_API_KEY";
const PAGE_LIMIT: &str = "1000";
/// Capy serves `hour` buckets for at most 31 days back and `day` buckets for a
/// year. Hourly rows keep local-day attribution exact for recent usage; older
/// usage falls back to UTC day rows.
const HOURLY_WINDOW_DAYS: i64 = 30;
const DAILY_WINDOW_DAYS: i64 = 364;
const MAX_PAGES_PER_WINDOW: usize = 500;
const MAX_ATTEMPTS: u32 = 3;
/// Implicit syncs before reports are skipped while the cache is younger than
/// this, so scripted runs do not call Capy on every invocation.
pub const CAPY_AUTO_SYNC_FRESHNESS: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapyUsageCache {
    version: i32,
    synced_at: String,
    /// Fingerprint of the API key and user filter that produced `items`, so a
    /// cache written under other credentials is never treated as current.
    /// Keep this before variable-size fields: core admits cache scope from a
    /// bounded JSON prefix and stops before `userIds` and `items`.
    #[serde(default)]
    identity: Option<String>,
    #[serde(default)]
    user_ids: Vec<String>,
    items: Vec<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCapyResult {
    pub synced: bool,
    pub rows: usize,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapyStatus {
    cache_dir: String,
    credentials_path: String,
    usage_path: String,
    has_credentials: bool,
    credentials_source: Option<&'static str>,
    user_ids: Vec<String>,
    has_cache: bool,
    cache_eligible: Option<bool>,
    synced_at: Option<String>,
    rows: usize,
    diagnostics: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CapyUser {
    id: String,
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CapyUserList {
    items: Vec<CapyUser>,
}

pub fn get_capy_cache_dir() -> PathBuf {
    crate::paths::get_config_dir().join("capy-cache")
}

fn credentials_path() -> PathBuf {
    get_capy_cache_dir().join("credentials.json")
}

fn usage_path() -> PathBuf {
    get_capy_cache_dir().join("usage.json")
}

fn ensure_cache_dir() -> Result<()> {
    let dir = get_capy_cache_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

fn write_secret_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    ensure_cache_dir()?;
    let json = serde_json::to_vec_pretty(value)?;
    crate::commands::usage::helpers::atomic_write_secret(path, &json)?;
    Ok(())
}

fn env_api_key() -> Option<String> {
    std::env::var(CAPY_API_KEY_ENV)
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

/// Saved credentials win over `CAPY_API_KEY` so a `--user` restriction chosen
/// at login is not silently dropped when the variable is also set.
fn load_credentials() -> Option<(CapyCredentials, &'static str)> {
    tokscale_core::sessions::capy::load_credentials(&get_capy_cache_dir())
}

pub fn has_credentials() -> bool {
    load_credentials().is_some()
}

pub fn has_usage_cache() -> bool {
    usage_path().is_file()
}

pub fn has_reportable_usage_cache() -> bool {
    has_usage_cache() && tokscale_core::sessions::capy::usage_cache_is_eligible(&usage_path())
}

fn load_usage_cache() -> Option<CapyUsageCache> {
    let content = fs::read_to_string(usage_path()).ok()?;
    serde_json::from_str(&content).ok()
}

fn usage_cache_age() -> Option<Duration> {
    fs::metadata(usage_path())
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
}

/// A cache is current only when it is younger than `max_age` and was written
/// for the credentials in use now. A login with another key or `--user`, or a
/// different `CAPY_API_KEY`, makes it due immediately.
fn cache_is_current(
    cache_identity: Option<&str>,
    age: Option<Duration>,
    credentials: &CapyCredentials,
    max_age: Duration,
) -> bool {
    cache_identity == Some(credentials_identity(credentials).as_str())
        && age.is_some_and(|age| age < max_age)
}

fn http_client() -> Result<reqwest::Client> {
    tokscale_core::http::client_builder()
        .timeout(CAPY_HTTP_TIMEOUT)
        .build()
        .context("Failed to build Capy HTTP client")
}

async fn get_json(
    client: &reqwest::Client,
    api_key: &str,
    url: &str,
    query: &[(&str, String)],
) -> Result<Value> {
    let mut last_error = None;
    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
        }
        let response = client
            .get(url)
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .query(query)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(anyhow::anyhow!("Capy request failed: {error}"));
                continue;
            }
        };
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!(
                "Capy rejected the API key (missing, revoked or expired). Mint a new key in Capy under Settings → API and run `tokscale capy login`."
            );
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            anyhow::bail!("Capy API key is not allowed to read usage (403).");
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
        {
            last_error = Some(anyhow::anyhow!("Capy usage API returned {status}"));
            continue;
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let snippet: String = body.chars().take(300).collect();
            anyhow::bail!("Capy usage API returned {status}: {snippet}");
        }
        return response
            .json::<Value>()
            .await
            .context("Capy usage API returned invalid JSON");
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Capy request failed")))
}

async fn fetch_users(client: &reqwest::Client, api_key: &str) -> Result<Vec<CapyUser>> {
    let value = get_json(client, api_key, &format!("{CAPY_API_BASE}/users"), &[]).await?;
    let list: CapyUserList =
        serde_json::from_value(value).context("Unexpected Capy /users response")?;
    Ok(list.items)
}

fn series_query(
    granularity: &str,
    from: &str,
    to: &str,
    user_ids: &[String],
    cursor: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut query = vec![
        ("from", from.to_string()),
        ("to", to.to_string()),
        ("granularity", granularity.to_string()),
        ("groupBy", "user".to_string()),
        ("groupBy", "thread".to_string()),
        ("groupBy", "item".to_string()),
        ("limit", PAGE_LIMIT.to_string()),
    ];
    for user_id in user_ids {
        query.push(("userIds", user_id.clone()));
    }
    if let Some(cursor) = cursor {
        query.push(("cursor", cursor.to_string()));
    }
    query
}

async fn fetch_series_window(
    client: &reqwest::Client,
    api_key: &str,
    granularity: &str,
    from: &str,
    to: &str,
    user_ids: &[String],
) -> Result<Vec<Value>> {
    let url = format!("{CAPY_API_BASE}/usage/series");
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES_PER_WINDOW {
        let query = series_query(granularity, from, to, user_ids, cursor.as_deref());
        let page = get_json(client, api_key, &url, &query).await?;
        let page_items = page
            .get("items")
            .and_then(Value::as_array)
            .context("Capy usage series response has no `items` array")?;
        items.extend(page_items.iter().cloned());
        cursor = page
            .get("cursor")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        if cursor.is_none() {
            return Ok(items);
        }
    }
    anyhow::bail!("Capy usage series did not finish after {MAX_PAGES_PER_WINDOW} pages")
}

/// Returns `(daily_from, hourly_from, now)` as RFC 3339 strings. Both window
/// starts sit on a UTC midnight so a day bucket never overlaps an hour bucket.
fn sync_windows(now: chrono::DateTime<chrono::Utc>) -> (String, String, String) {
    let midnight = |days_back: i64| {
        (now - chrono::Duration::days(days_back))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("midnight is a valid time")
            .and_utc()
    };
    let fmt =
        |dt: chrono::DateTime<chrono::Utc>| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    (
        fmt(midnight(DAILY_WINDOW_DAYS)),
        fmt(midnight(HOURLY_WINDOW_DAYS)),
        fmt(now),
    )
}

fn is_llm_row(item: &Value) -> bool {
    item.pointer("/dimensions/itemType").and_then(Value::as_str) == Some("llm")
}

const TOKEN_FIELDS: [&str; 4] = [
    "inputTokens",
    "outputTokens",
    "cacheReadTokens",
    "cacheWriteTokens",
];

fn row_start(item: &Value) -> Option<chrono::DateTime<chrono::Utc>> {
    let start = item.get("start")?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(start)
        .ok()
        .map(|start| start.with_timezone(&chrono::Utc))
}

fn is_hour_row(item: &Value) -> bool {
    let end = item
        .get("end")
        .and_then(Value::as_str)
        .and_then(|end| chrono::DateTime::parse_from_rfc3339(end).ok());
    match (row_start(item), end) {
        (Some(start), Some(end)) => {
            end.with_timezone(&chrono::Utc) - start <= chrono::Duration::hours(1)
        }
        _ => false,
    }
}

/// The UTC day and dimensions a row belongs to, so hour rows can be matched
/// with the day row that later replaces them.
fn day_key(item: &Value) -> Option<String> {
    let day = row_start(item)?.date_naive();
    let dim = |name: &str| {
        item.pointer(&format!("/dimensions/{name}"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    Some(format!(
        "{day}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        dim("userId"),
        dim("threadId"),
        dim("itemType"),
        dim("itemId"),
        dim("model")
    ))
}

fn token_count(item: &Value, field: &str) -> f64 {
    item.pointer(&format!("/tokens/{field}"))
        .and_then(Value::as_f64)
        .filter(|count| count.is_finite() && *count > 0.0)
        .unwrap_or(0.0)
}

/// Capy only serves hour buckets for the last month, so usage that ages out
/// comes back as a UTC day row. Stamped at the day's midpoint, that row can
/// fall on another local date than the hours it replaces, and a resubmission
/// would then count the moved usage on both dates. Hour rows cached by an
/// earlier sync are therefore kept, and each day row is cut down to the usage
/// those hours do not already cover; a day row with nothing left is dropped.
fn carry_hourly_attribution(
    previous: &[Value],
    daily: Vec<Value>,
    daily_from: chrono::DateTime<chrono::Utc>,
    hourly_from: chrono::DateTime<chrono::Utc>,
) -> Vec<Value> {
    let carried: Vec<Value> = previous
        .iter()
        .filter(|item| is_llm_row(item) && is_hour_row(item))
        .filter(|item| {
            row_start(item).is_some_and(|start| start >= daily_from && start < hourly_from)
        })
        .cloned()
        .collect();
    let mut covered: std::collections::HashMap<String, [f64; 4]> = std::collections::HashMap::new();
    for item in &carried {
        let Some(key) = day_key(item) else { continue };
        let sums = covered.entry(key).or_default();
        for (sum, field) in sums.iter_mut().zip(TOKEN_FIELDS) {
            *sum += token_count(item, field);
        }
    }

    let mut items = carried;
    for mut item in daily {
        let Some(sums) = day_key(&item).and_then(|key| covered.get(&key).copied()) else {
            items.push(item);
            continue;
        };
        let remaining: Vec<f64> = TOKEN_FIELDS
            .iter()
            .zip(sums)
            .map(|(field, sum)| (token_count(&item, field) - sum).max(0.0))
            .collect();
        if remaining.iter().all(|count| *count == 0.0) {
            continue;
        }
        if let Some(tokens) = item.get_mut("tokens").and_then(Value::as_object_mut) {
            for (field, count) in TOKEN_FIELDS.iter().zip(remaining) {
                tokens.insert((*field).to_string(), serde_json::json!(count as i64));
            }
        }
        items.push(item);
    }
    items
}

async fn fetch_usage(credentials: &CapyCredentials) -> Result<CapyUsageCache> {
    let client = http_client()?;
    let now = chrono::Utc::now();
    let (daily_from, hourly_from, to) = sync_windows(now);
    let key = credentials.api_key.as_str();
    let users = credentials.user_ids.as_slice();

    let mut daily =
        fetch_series_window(&client, key, "day", &daily_from, &hourly_from, users).await?;
    let mut hourly = fetch_series_window(&client, key, "hour", &hourly_from, &to, users).await?;
    // Only model usage carries tokens; machine time, reviews and images are
    // not something Tokscale reports.
    daily.retain(is_llm_row);
    hourly.retain(is_llm_row);

    // Hour rows from a cache written under other credentials describe other
    // usage and are not carried.
    let identity = credentials_identity(credentials);
    let previous = load_usage_cache()
        .filter(|cache| cache.identity.as_deref() == Some(identity.as_str()))
        .map(|cache| cache.items)
        .unwrap_or_default();
    let parse = |value: &str| {
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .context("Capy sync window is not RFC 3339")
    };
    let mut items =
        carry_hourly_attribution(&previous, daily, parse(&daily_from)?, parse(&hourly_from)?);
    items.extend(hourly);

    Ok(CapyUsageCache {
        version: 1,
        synced_at: now.to_rfc3339(),
        user_ids: credentials.user_ids.clone(),
        identity: Some(identity),
        items,
    })
}

pub async fn sync_capy_cache() -> SyncCapyResult {
    let Some((credentials, _)) = load_credentials() else {
        return SyncCapyResult {
            synced: false,
            rows: 0,
            error: Some(
                "Not authenticated. Run `tokscale capy login` (or set CAPY_API_KEY), then `tokscale capy sync`."
                    .to_string(),
            ),
        };
    };
    match fetch_usage(&credentials).await {
        Ok(cache) => {
            let rows = cache.items.len();
            if let Err(error) = write_secret_json(&usage_path(), &cache) {
                return SyncCapyResult {
                    synced: false,
                    rows,
                    error: Some(format!("Failed to write Capy cache: {error}")),
                };
            }
            SyncCapyResult {
                synced: true,
                rows,
                error: None,
            }
        }
        Err(error) => SyncCapyResult {
            synced: false,
            rows: 0,
            error: Some(error.to_string()),
        },
    }
}

/// The warning for a failed sync. It only claims a fallback when there is a
/// cache to fall back to; otherwise Capy usage is simply missing.
pub fn sync_failure_warning(error: &str, had_cache: bool) -> String {
    if had_cache {
        format!("  Capy sync failed; using cached data: {error}")
    } else {
        format!("  Capy sync failed; Capy usage is not included: {error}")
    }
}

/// Best-effort refresh before a local report. Returns a warning when a sync
/// was attempted and failed; `None` when it succeeded or was not attempted
/// (no credentials, offline, or the cache is current).
pub fn auto_sync_for_local_report() -> Option<String> {
    if tokscale_core::network::is_offline() {
        return None;
    }
    let (credentials, _) = load_credentials()?;
    let cache_identity = tokscale_core::sessions::capy::usage_cache_identity(&usage_path());
    if cache_is_current(
        cache_identity.as_deref(),
        usage_cache_age(),
        &credentials,
        CAPY_AUTO_SYNC_FRESHNESS,
    ) {
        return None;
    }
    let had_cache = has_usage_cache()
        && tokscale_core::sessions::capy::cache_identity_is_eligible(
            cache_identity.as_deref(),
            Some(&credentials),
        );
    let result = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime.block_on(sync_capy_cache()),
        Err(error) => SyncCapyResult {
            synced: false,
            rows: 0,
            error: Some(format!("Failed to initialize Capy sync runtime: {error}")),
        },
    };
    if result.synced {
        return None;
    }
    let error = result.error.as_deref().unwrap_or("unknown error");
    Some(sync_failure_warning(error, had_cache))
}

fn resolve_user_filters(users: &[CapyUser], wanted: &[String]) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    for want in wanted {
        let want = want.trim();
        let found = users.iter().find(|user| {
            user.id == want
                || user
                    .email
                    .as_deref()
                    .is_some_and(|email| email.eq_ignore_ascii_case(want))
        });
        match found {
            Some(user) => ids.push(user.id.clone()),
            None => anyhow::bail!("No member of this Capy organization matches `{want}`"),
        }
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub fn run_capy_login(key: Option<String>, users: Vec<String>) -> Result<()> {
    println!("\n  {}\n", "Capy - Login".cyan());
    let api_key = match key.or_else(env_api_key) {
        Some(key) => key,
        None => {
            println!(
                "{}",
                "  Mint an API key in Capy under Settings → API (it starts with `capy_`)."
                    .bright_black()
            );
            print!("  Enter Capy API key: ");
            std::io::stdout().flush()?;
            rpassword::read_password().context("Failed to read Capy API key")?
        }
    };
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        anyhow::bail!("Capy API key must not be empty");
    }

    let runtime = tokio::runtime::Runtime::new()?;
    let client = http_client()?;
    let members = runtime.block_on(fetch_users(&client, &api_key));
    let user_ids = match (members, users.is_empty()) {
        (Ok(members), false) => resolve_user_filters(&members, &users)?,
        (Ok(members), true) => {
            if members.len() > 1 {
                println!(
                    "{}",
                    format!(
                        "  This organization has {} members; sync will include every member's usage this key can read. Re-run with `--user <your email>` to count only your own.",
                        members.len()
                    )
                    .yellow()
                );
            }
            Vec::new()
        }
        (Err(error), false) => {
            anyhow::bail!("Could not list Capy users to resolve --user: {error}")
        }
        (Err(error), true) => {
            // An expired or revoked key fails here too; refuse to save it.
            if error.to_string().contains("rejected the API key") {
                return Err(error);
            }
            println!(
                "{}",
                format!("  Could not verify organization members: {error}").yellow()
            );
            Vec::new()
        }
    };

    let credentials = CapyCredentials {
        api_key,
        user_ids,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    write_secret_json(&credentials_path(), &credentials)?;
    println!("{}", "  Capy API key saved.".green());
    // A cache from another scope stays on disk but the shared scope gate keeps
    // reports from consuming it until a successful sync replaces it.
    println!(
        "{}",
        "  Run `tokscale capy sync` to cache usage (reports also refresh it automatically)."
            .bright_black()
    );
    Ok(())
}

pub fn run_capy_logout(purge_cache: bool) -> Result<()> {
    let credentials = credentials_path();
    if credentials.exists() {
        fs::remove_file(credentials)?;
    }
    if purge_cache {
        let usage = usage_path();
        if usage.exists() {
            fs::remove_file(usage)?;
        }
    }
    println!("{}", "  Capy credentials removed.".green());
    Ok(())
}

fn status_diagnostics(
    credentials: Option<&CapyCredentials>,
    cache: Option<&CapyUsageCache>,
) -> (Option<bool>, Vec<String>) {
    let cache_eligible = cache.map(|cache| {
        tokscale_core::sessions::capy::cache_identity_is_eligible(
            cache.identity.as_deref(),
            credentials,
        )
    });
    let mut diagnostics = Vec::new();
    if credentials.is_none() {
        diagnostics.push("missing API key; run `tokscale capy login`".to_string());
    }
    match cache_eligible {
        None => diagnostics.push("missing usage cache; run `tokscale capy sync`".to_string()),
        Some(false) => diagnostics.push(
            "stored usage cache belongs to different credentials; run `tokscale capy sync` to replace it"
                .to_string(),
        ),
        Some(true) => {}
    }
    (cache_eligible, diagnostics)
}

fn build_status() -> CapyStatus {
    let credentials = load_credentials();
    let cache = load_usage_cache();
    let (cache_eligible, diagnostics) = status_diagnostics(
        credentials.as_ref().map(|(credentials, _)| credentials),
        cache.as_ref(),
    );
    CapyStatus {
        cache_dir: get_capy_cache_dir().to_string_lossy().to_string(),
        credentials_path: credentials_path().to_string_lossy().to_string(),
        usage_path: usage_path().to_string_lossy().to_string(),
        has_credentials: credentials.is_some(),
        credentials_source: credentials.as_ref().map(|(_, source)| *source),
        user_ids: credentials
            .map(|(credentials, _)| credentials.user_ids)
            .unwrap_or_default(),
        has_cache: cache.is_some(),
        cache_eligible,
        synced_at: cache.as_ref().map(|cache| cache.synced_at.clone()),
        rows: cache.map(|cache| cache.items.len()).unwrap_or(0),
        diagnostics,
    }
}

pub fn run_capy_status(json: bool) -> Result<()> {
    let status = build_status();
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }
    println!("\n  {}", "Capy usage".cyan());
    let credentials = match status.credentials_source {
        Some("env") => "CAPY_API_KEY".green(),
        Some(_) => status.credentials_path.green(),
        None => "missing".yellow(),
    };
    println!("  {} {}", "API key:".bright_black(), credentials);
    if !status.user_ids.is_empty() {
        println!(
            "  {} {}",
            "Users:".bright_black(),
            status.user_ids.join(", ")
        );
    }
    println!(
        "  {} {}",
        "Cache:".bright_black(),
        match status.cache_eligible {
            Some(true) => status.usage_path.green(),
            Some(false) => format!(
                "{} (stored, unavailable for current credentials)",
                status.usage_path
            )
            .yellow(),
            None => status.usage_path.yellow(),
        }
    );
    if let Some(synced_at) = &status.synced_at {
        let stored = status.cache_eligible == Some(false);
        println!(
            "  {} {}",
            if stored {
                "Stored last sync:".bright_black()
            } else {
                "Last sync:".bright_black()
            },
            synced_at
        );
        println!(
            "  {} {}",
            if stored {
                "Stored model usage rows:".bright_black()
            } else {
                "Model usage rows:".bright_black()
            },
            status.rows
        );
    }
    for diagnostic in &status.diagnostics {
        println!("{}", format!("  Warning: {diagnostic}").yellow());
    }
    println!();
    Ok(())
}

pub fn run_capy_sync(json: bool) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let result = runtime.block_on(sync_capy_cache());
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if result.synced {
        println!(
            "{}",
            format!("  Capy: synced {} model usage rows", result.rows).green()
        );
    }
    manual_sync_completion(&result)
}

/// Scripts rely on the exit status, so a failed sync is an error even though
/// the JSON result is printed first.
fn manual_sync_completion(result: &SyncCapyResult) -> Result<()> {
    if result.synced {
        return Ok(());
    }
    anyhow::bail!(
        "Capy sync failed: {}",
        result.error.as_deref().unwrap_or("unknown error")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn creds(key: &str, users: &[&str]) -> CapyCredentials {
        CapyCredentials {
            api_key: key.to_string(),
            user_ids: users.iter().map(ToString::to_string).collect(),
            created_at: String::new(),
        }
    }

    #[test]
    fn identity_tracks_key_and_user_filter_but_not_filter_order() {
        let base = credentials_identity(&creds("capy_a", &["u1", "u2"]));
        assert_eq!(base, credentials_identity(&creds("capy_a", &["u2", "u1"])));
        assert_ne!(base, credentials_identity(&creds("capy_b", &["u1", "u2"])));
        assert_ne!(base, credentials_identity(&creds("capy_a", &["u1"])));
        assert_ne!(base, credentials_identity(&creds("capy_a", &[])));
        assert!(!base.contains("capy_a"));
    }

    #[test]
    fn usage_cache_writer_places_identity_before_variable_fields() {
        let cache = CapyUsageCache {
            version: 1,
            synced_at: "synthetic".to_string(),
            user_ids: vec!["u".repeat(70 * 1024)],
            identity: Some("scope-a".to_string()),
            items: Vec::new(),
        };
        let serialized = serde_json::to_string(&cache).unwrap();
        let identity = serialized.find(r#""identity""#).unwrap();
        let user_ids = serialized.find(r#""userIds""#).unwrap();
        let items = serialized.find(r#""items""#).unwrap();
        assert!(identity < user_ids);
        assert!(identity < items);
    }

    fn status_cache(identity: Option<String>) -> CapyUsageCache {
        CapyUsageCache {
            version: 1,
            synced_at: "synthetic".to_string(),
            user_ids: Vec::new(),
            identity,
            items: Vec::new(),
        }
    }

    #[test]
    fn status_distinguishes_eligible_and_foreign_scope_caches() {
        let credentials = creds("capy_scope_a", &[]);
        let matching = status_cache(Some(credentials_identity(&credentials)));
        let (eligible, diagnostics) = status_diagnostics(Some(&credentials), Some(&matching));
        assert_eq!(eligible, Some(true));
        assert!(diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.contains("different credentials")));

        let foreign = status_cache(Some(credentials_identity(&creds("capy_scope_b", &[]))));
        let (eligible, diagnostics) = status_diagnostics(Some(&credentials), Some(&foreign));
        assert_eq!(eligible, Some(false));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("different credentials")));
    }

    #[cfg(unix)]
    #[test]
    fn credentials_serialization_remains_0600_file_compatible() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("credentials.json");
        let credentials = creds("capy_synthetic_secret", &["user_1"]);
        let json = serde_json::to_vec_pretty(&credentials).unwrap();
        crate::commands::usage::helpers::atomic_write_secret(&path, &json).unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let restored: CapyCredentials = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored.api_key, credentials.api_key);
        assert_eq!(restored.user_ids, credentials.user_ids);
    }

    #[test]
    fn cache_is_current_only_for_matching_identity_and_age() {
        let credentials = creds("capy_a", &[]);
        let identity = credentials_identity(&credentials);
        let young = Some(Duration::from_secs(60));
        let old = Some(Duration::from_secs(3600));
        let max = CAPY_AUTO_SYNC_FRESHNESS;
        assert!(cache_is_current(Some(&identity), young, &credentials, max));
        assert!(!cache_is_current(Some(&identity), old, &credentials, max));
        assert!(!cache_is_current(Some(&identity), None, &credentials, max));
        // A cache from another key, or from before identities were recorded,
        // is due even when young.
        assert!(!cache_is_current(Some("0000"), young, &credentials, max));
        assert!(!cache_is_current(None, young, &credentials, max));
    }

    #[test]
    fn failure_warning_only_claims_a_fallback_when_a_cache_exists() {
        assert!(sync_failure_warning("boom", true).contains("using cached data"));
        let missing = sync_failure_warning("boom", false);
        assert!(!missing.contains("cached data"));
        assert!(missing.contains("not included"));
    }

    #[test]
    fn manual_sync_failure_is_an_error() {
        let failed = SyncCapyResult {
            synced: false,
            rows: 0,
            error: Some("Not authenticated".to_string()),
        };
        assert!(manual_sync_completion(&failed).is_err());
        let ok = SyncCapyResult {
            synced: true,
            rows: 3,
            error: None,
        };
        assert!(manual_sync_completion(&ok).is_ok());
    }

    fn row(start: &str, end: &str, input: i64, output: i64) -> Value {
        serde_json::json!({
            "start": start, "end": end,
            "dimensions": {"userId": "u", "threadId": "t", "itemType": "llm",
                "itemId": "openai/gpt-5.5", "model": "openai/gpt-5.5"},
            "tokens": {"inputTokens": input, "outputTokens": output,
                "cacheReadTokens": 0, "cacheWriteTokens": 0}
        })
    }

    fn utc(value: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn aged_out_hours_keep_their_buckets_and_day_rows_keep_only_the_rest() {
        let daily_from = utc("2025-11-15T00:00:00Z");
        let hourly_from = utc("2026-10-15T00:00:00Z");
        let previous = vec![
            // 01:00Z is the previous evening in America/New_York.
            row(
                "2026-10-14T01:00:00.000Z",
                "2026-10-14T02:00:00.000Z",
                1000,
                10,
            ),
            // Still inside the hourly window: the new hour fetch covers it.
            row("2026-10-20T01:00:00.000Z", "2026-10-20T02:00:00.000Z", 5, 5),
            // Older than the daily window.
            row("2025-11-01T01:00:00.000Z", "2025-11-01T02:00:00.000Z", 5, 5),
        ];
        let daily = vec![
            row(
                "2026-10-14T00:00:00.000Z",
                "2026-10-15T00:00:00.000Z",
                1200,
                10,
            ),
            row("2026-10-13T00:00:00.000Z", "2026-10-14T00:00:00.000Z", 7, 7),
        ];
        let items = carry_hourly_attribution(&previous, daily, daily_from, hourly_from);
        let starts: Vec<&str> = items.iter().map(|i| i["start"].as_str().unwrap()).collect();
        assert_eq!(
            starts,
            [
                "2026-10-14T01:00:00.000Z",
                "2026-10-14T00:00:00.000Z",
                "2026-10-13T00:00:00.000Z"
            ]
        );
        assert_eq!(items[0]["tokens"]["inputTokens"], 1000);
        assert_eq!(items[1]["tokens"]["inputTokens"], 200);
        assert_eq!(items[1]["tokens"]["outputTokens"], 0);
        assert_eq!(items[2]["tokens"]["inputTokens"], 7);
    }

    #[test]
    fn a_day_row_fully_covered_by_cached_hours_is_dropped() {
        let previous = vec![row(
            "2026-10-14T01:00:00.000Z",
            "2026-10-14T02:00:00.000Z",
            1000,
            10,
        )];
        let daily = vec![row(
            "2026-10-14T00:00:00.000Z",
            "2026-10-15T00:00:00.000Z",
            1000,
            10,
        )];
        let items = carry_hourly_attribution(
            &previous,
            daily,
            utc("2025-11-15T00:00:00Z"),
            utc("2026-10-15T00:00:00Z"),
        );
        assert_eq!(items, previous);
    }

    #[test]
    fn sync_windows_meet_on_a_utc_midnight() {
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 11, 14, 15, 30, 0)
            .unwrap();
        let (daily_from, hourly_from, to) = sync_windows(now);
        assert_eq!(daily_from, "2025-11-15T00:00:00.000Z");
        assert_eq!(hourly_from, "2026-10-15T00:00:00.000Z");
        assert_eq!(to, "2026-11-14T15:30:00.000Z");
    }

    #[test]
    fn series_query_repeats_group_by_and_user_filters() {
        let query = series_query(
            "hour",
            "a",
            "b",
            &["u1".to_string(), "u2".to_string()],
            Some("12:abc"),
        );
        let group_by: Vec<_> = query
            .iter()
            .filter(|(key, _)| *key == "groupBy")
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(group_by, ["user", "thread", "item"]);
        assert_eq!(query.iter().filter(|(key, _)| *key == "userIds").count(), 2);
        assert!(query.contains(&("cursor", "12:abc".to_string())));
    }

    #[test]
    fn resolve_user_filters_matches_id_or_email() {
        let users = vec![
            CapyUser {
                id: "user_a".to_string(),
                email: Some("A@Example.com".to_string()),
            },
            CapyUser {
                id: "user_b".to_string(),
                email: None,
            },
        ];
        let ids =
            resolve_user_filters(&users, &["a@example.com".to_string(), "user_b".to_string()])
                .unwrap();
        assert_eq!(ids, ["user_a", "user_b"]);
        assert!(resolve_user_filters(&users, &["nobody".to_string()]).is_err());
    }

    #[test]
    fn only_llm_rows_are_kept() {
        assert!(is_llm_row(
            &serde_json::json!({"dimensions": {"itemType": "llm"}})
        ));
        assert!(!is_llm_row(
            &serde_json::json!({"dimensions": {"itemType": "machine"}})
        ));
    }
}
