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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapyCredentials {
    api_key: String,
    /// Restrict sync to these Capy user ids. Empty means every user the key
    /// can read, which for a personal organization is just its owner.
    #[serde(default)]
    user_ids: Vec<String>,
    created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapyUsageCache {
    version: i32,
    synced_at: String,
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

fn load_saved_credentials() -> Option<CapyCredentials> {
    let content = fs::read_to_string(credentials_path()).ok()?;
    serde_json::from_str(&content).ok()
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
    if let Some(credentials) = load_saved_credentials() {
        return Some((credentials, "saved"));
    }
    env_api_key().map(|api_key| {
        (
            CapyCredentials {
                api_key,
                user_ids: Vec::new(),
                created_at: String::new(),
            },
            "env",
        )
    })
}

pub fn has_credentials() -> bool {
    load_credentials().is_some()
}

fn load_usage_cache() -> Option<CapyUsageCache> {
    let content = fs::read_to_string(usage_path()).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn capy_usage_cache_is_fresh(max_age: Duration) -> bool {
    fs::metadata(usage_path())
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < max_age)
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

async fn fetch_usage(credentials: &CapyCredentials) -> Result<CapyUsageCache> {
    let client = http_client()?;
    let now = chrono::Utc::now();
    let (daily_from, hourly_from, to) = sync_windows(now);
    let key = credentials.api_key.as_str();
    let users = credentials.user_ids.as_slice();

    let mut items =
        fetch_series_window(&client, key, "day", &daily_from, &hourly_from, users).await?;
    items.extend(fetch_series_window(&client, key, "hour", &hourly_from, &to, users).await?);
    // Only model usage carries tokens; machine time, reviews and images are
    // not something Tokscale reports.
    items.retain(is_llm_row);

    Ok(CapyUsageCache {
        version: 1,
        synced_at: now.to_rfc3339(),
        user_ids: credentials.user_ids.clone(),
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

/// Best-effort refresh before a local report. `None` means no sync was
/// attempted (no credentials, offline, or the cache is fresh).
pub fn auto_sync_for_local_report() -> Option<SyncCapyResult> {
    if tokscale_core::network::is_offline()
        || !has_credentials()
        || capy_usage_cache_is_fresh(CAPY_AUTO_SYNC_FRESHNESS)
    {
        return None;
    }
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            return Some(SyncCapyResult {
                synced: false,
                rows: 0,
                error: Some(format!("Failed to initialize Capy sync runtime: {error}")),
            })
        }
    };
    Some(runtime.block_on(sync_capy_cache()))
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

    write_secret_json(
        &credentials_path(),
        &CapyCredentials {
            api_key,
            user_ids,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )?;
    println!("{}", "  Capy API key saved.".green());
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

fn build_status() -> CapyStatus {
    let credentials = load_credentials();
    let cache = load_usage_cache();
    let mut diagnostics = Vec::new();
    if credentials.is_none() {
        diagnostics.push("missing API key; run `tokscale capy login`".to_string());
    }
    if cache.is_none() {
        diagnostics.push("missing usage cache; run `tokscale capy sync`".to_string());
    }
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
        if status.has_cache {
            status.usage_path.green()
        } else {
            status.usage_path.yellow()
        }
    );
    if let Some(synced_at) = &status.synced_at {
        println!("  {} {}", "Last sync:".bright_black(), synced_at);
        println!("  {} {}", "Model usage rows:".bright_black(), status.rows);
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
        return Ok(());
    }
    if result.synced {
        println!(
            "{}",
            format!("  Capy: synced {} model usage rows", result.rows).green()
        );
    } else if let Some(error) = &result.error {
        eprintln!("{}", format!("  Capy sync failed: {error}").yellow());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
