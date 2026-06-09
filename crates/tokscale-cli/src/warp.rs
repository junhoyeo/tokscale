use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const WARP_GRAPHQL_ENDPOINT: &str = "https://app.warp.dev/graphql/v2";
const WARP_HTTP_TIMEOUT: Duration = Duration::from_secs(8);

const REQUEST_LIMIT_QUERY: &str = r#"query GetRequestLimitInfo { requestLimitInfo { requestLimit requestsUsedSinceLastRefresh nextRefreshTime bonusGrantsInfo { spendingInfo { currentMonthSpendCents currentMonthCreditsPurchased } } } }"#;
const WORKSPACES_QUERY: &str = r#"query GetWorkspacesMetadataForUser { workspacesMetadataForUser { id name totalRequestsUsedSinceLastRefresh aiOverages { currentMonthlyRequestCostCents currentMonthlyRequestsUsed } usageInfo { requestsUsedSinceLastRefresh } members { userId usageInfo { requestsUsedSinceLastRefresh } } } }"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WarpCredentials {
    auth_value: String,
    auth_kind: WarpAuthKind,
    created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum WarpAuthKind {
    Bearer,
    Cookie,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WarpAggregateUsage {
    pub requests_used: Option<i64>,
    pub request_limit: Option<i64>,
    pub spend_cents: Option<i64>,
    pub credits_purchased_cents: Option<i64>,
    pub next_refresh_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WarpWorkspaceUsage {
    pub id: Option<String>,
    pub name: Option<String>,
    pub requests_used: Option<i64>,
    pub spend_cents: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WarpUsageCache {
    pub version: i32,
    pub synced_at: String,
    pub usage: WarpAggregateUsage,
    pub workspaces: Vec<WarpWorkspaceUsage>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WarpStatus {
    cache_dir: String,
    credentials_path: String,
    usage_path: String,
    has_credentials: bool,
    has_cache: bool,
    requests_used: Option<i64>,
    request_limit: Option<i64>,
    spend_cents: Option<i64>,
    workspace_count: usize,
    diagnostics: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncWarpResult {
    synced: bool,
    requests_used: Option<i64>,
    spend_cents: Option<i64>,
    workspace_count: usize,
    error: Option<String>,
}

pub fn get_warp_cache_dir() -> PathBuf {
    crate::paths::get_config_dir().join("warp-cache")
}

fn credentials_path() -> PathBuf {
    get_warp_cache_dir().join("credentials.json")
}

fn usage_path() -> PathBuf {
    get_warp_cache_dir().join("usage.json")
}

fn ensure_cache_dir() -> Result<()> {
    let dir = get_warp_cache_dir();
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

fn atomic_write_secret(path: &Path, data: &[u8]) -> Result<()> {
    crate::commands::usage::helpers::atomic_write_secret(path, data)?;
    Ok(())
}

fn save_credentials(creds: &WarpCredentials) -> Result<()> {
    ensure_cache_dir()?;
    let json = serde_json::to_vec_pretty(creds)?;
    atomic_write_secret(&credentials_path(), &json)?;
    Ok(())
}

fn load_credentials() -> Option<WarpCredentials> {
    let content = fs::read_to_string(credentials_path()).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn has_credentials() -> bool {
    load_credentials().is_some()
}

pub(crate) fn load_usage_cache() -> Option<WarpUsageCache> {
    let content = fs::read_to_string(usage_path()).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn has_usage_cache_in_home(home_dir: &Path) -> bool {
    home_dir
        .join(".config/tokscale/warp-cache/usage.json")
        .exists()
}

pub fn run_warp_login(token: Option<String>, cookie: bool) -> Result<()> {
    println!("\n  {}\n", "Warp/Oz - Login".cyan());
    let auth_value = match token {
        Some(token) => token,
        None => {
            print!("  Enter Warp bearer token or Cookie header value: ");
            std::io::stdout().flush()?;
            rpassword::read_password().context("Failed to read Warp credential")?
        }
    };
    let auth_value = auth_value.trim().to_string();
    if auth_value.is_empty() {
        anyhow::bail!("Warp credential must not be empty");
    }

    save_credentials(&WarpCredentials {
        auth_value,
        auth_kind: if cookie {
            WarpAuthKind::Cookie
        } else {
            WarpAuthKind::Bearer
        },
        created_at: chrono::Utc::now().to_rfc3339(),
    })?;

    println!("{}", "  Warp credentials saved.".green());
    println!(
        "{}",
        "  Run `tokscale warp sync` to cache aggregate requests and spend.".bright_black()
    );
    Ok(())
}

pub fn run_warp_logout(purge_cache: bool) -> Result<()> {
    let creds = credentials_path();
    if creds.exists() {
        fs::remove_file(creds)?;
    }
    if purge_cache {
        let usage = usage_path();
        if usage.exists() {
            fs::remove_file(usage)?;
        }
    }
    println!("{}", "  Warp credentials removed.".green());
    Ok(())
}

pub fn run_warp_status(json: bool) -> Result<()> {
    let status = build_status();
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    println!("\n  {}", "Warp/Oz aggregate usage".cyan());
    println!(
        "  {} {}",
        "Credentials:".bright_black(),
        if status.has_credentials {
            "found".green()
        } else {
            "missing".yellow()
        }
    );
    println!(
        "  {} {}",
        "Cache:".bright_black(),
        if status.has_cache {
            status.usage_path.green()
        } else {
            status.usage_path.yellow()
        }
    );
    if let Some(requests) = status.requests_used {
        println!("  {} {}", "Requests:".bright_black(), requests);
    }
    if let Some(limit) = status.request_limit {
        println!("  {} {}", "Request limit:".bright_black(), limit);
    }
    if let Some(cents) = status.spend_cents {
        println!(
            "  {} ${:.2}",
            "Current spend:".bright_black(),
            cents as f64 / 100.0
        );
    }
    for diagnostic in status.diagnostics {
        println!("{}", format!("  Warning: {diagnostic}").yellow());
    }
    println!();
    Ok(())
}

pub fn run_warp_sync(json: bool) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(sync_warp_cache());
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if result.synced {
        println!(
            "{}",
            format!(
                "  Warp: synced aggregate usage (requests={}, spend=${:.2}, workspaces={})",
                result.requests_used.unwrap_or(0),
                result.spend_cents.unwrap_or(0) as f64 / 100.0,
                result.workspace_count
            )
            .green()
        );
    } else if let Some(error) = result.error {
        eprintln!("{}", format!("  Warp sync failed: {error}").yellow());
    }
    Ok(())
}

async fn sync_warp_cache() -> SyncWarpResult {
    let credentials = match load_credentials() {
        Some(credentials) => credentials,
        None => {
            return SyncWarpResult {
                synced: false,
                requests_used: None,
                spend_cents: None,
                workspace_count: 0,
                error: Some(
                    "Not authenticated. Run `tokscale warp login`, then `tokscale warp sync`."
                        .to_string(),
                ),
            };
        }
    };

    match fetch_warp_usage(&credentials).await {
        Ok(cache) => {
            if let Err(error) = write_usage_cache(&cache) {
                return SyncWarpResult {
                    synced: false,
                    requests_used: cache.usage.requests_used,
                    spend_cents: cache.usage.spend_cents,
                    workspace_count: cache.workspaces.len(),
                    error: Some(format!("Failed to write Warp cache: {error}")),
                };
            }
            SyncWarpResult {
                synced: true,
                requests_used: cache.usage.requests_used,
                spend_cents: cache.usage.spend_cents,
                workspace_count: cache.workspaces.len(),
                error: None,
            }
        }
        Err(error) => SyncWarpResult {
            synced: false,
            requests_used: None,
            spend_cents: None,
            workspace_count: 0,
            error: Some(error.to_string()),
        },
    }
}

async fn fetch_warp_usage(credentials: &WarpCredentials) -> Result<WarpUsageCache> {
    let client = reqwest::Client::builder()
        .timeout(WARP_HTTP_TIMEOUT)
        .build()
        .context("Failed to build Warp HTTP client")?;
    let responses = vec![
        send_graphql(
            &client,
            credentials,
            "GetRequestLimitInfo",
            REQUEST_LIMIT_QUERY,
        )
        .await?,
        send_graphql(
            &client,
            credentials,
            "GetWorkspacesMetadataForUser",
            WORKSPACES_QUERY,
        )
        .await?,
    ];
    normalize_graphql_usage(&responses)
}

async fn send_graphql(
    client: &reqwest::Client,
    credentials: &WarpCredentials,
    operation_name: &str,
    query: &str,
) -> Result<Value> {
    let mut request = client
        .post(WARP_GRAPHQL_ENDPOINT)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "operationName": operation_name,
            "query": query,
            "variables": {}
        }));
    request = match credentials.auth_kind {
        WarpAuthKind::Bearer => request.bearer_auth(&credentials.auth_value),
        WarpAuthKind::Cookie => request.header("Cookie", &credentials.auth_value),
    };

    let response = request.send().await?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        || response.status() == reqwest::StatusCode::FORBIDDEN
    {
        anyhow::bail!(
            "Warp authentication failed. Run `tokscale warp login` with a fresh credential."
        );
    }
    if !response.status().is_success() {
        anyhow::bail!("Warp GraphQL API returned status {}", response.status());
    }
    let value: Value = response.json().await?;
    ensure_graphql_response_ok(&value)?;
    Ok(value)
}

fn ensure_graphql_response_ok(response: &Value) -> Result<()> {
    let Some(errors) = response.get("errors").and_then(Value::as_array) else {
        return Ok(());
    };
    if errors.is_empty() {
        return Ok(());
    }

    let message = errors
        .iter()
        .filter_map(|error| error.get("message").and_then(Value::as_str))
        .take(3)
        .collect::<Vec<_>>()
        .join("; ");
    if message.is_empty() {
        anyhow::bail!("Warp GraphQL API returned errors");
    }
    anyhow::bail!("Warp GraphQL API returned errors: {message}");
}

fn write_usage_cache(cache: &WarpUsageCache) -> Result<()> {
    ensure_cache_dir()?;
    let json = serde_json::to_vec_pretty(cache)?;
    atomic_write_secret(&usage_path(), &json)?;
    Ok(())
}

fn build_status() -> WarpStatus {
    let cache = load_usage_cache();
    let has_credentials = has_credentials();
    let has_cache = cache.is_some();
    let cache_dir = get_warp_cache_dir();
    let credentials_path = credentials_path();
    let usage_path = usage_path();
    let mut diagnostics = Vec::new();
    if !has_credentials {
        diagnostics.push("missing credentials; run `tokscale warp login`".to_string());
    }
    if !has_cache {
        diagnostics.push("missing aggregate usage cache; run `tokscale warp sync`".to_string());
    }

    let usage = cache.as_ref().map(|cache| &cache.usage);
    WarpStatus {
        cache_dir: cache_dir.to_string_lossy().to_string(),
        credentials_path: credentials_path.to_string_lossy().to_string(),
        usage_path: usage_path.to_string_lossy().to_string(),
        has_credentials,
        has_cache,
        requests_used: usage.and_then(|usage| usage.requests_used),
        request_limit: usage.and_then(|usage| usage.request_limit),
        spend_cents: usage.and_then(|usage| usage.spend_cents),
        workspace_count: cache.map(|cache| cache.workspaces.len()).unwrap_or(0),
        diagnostics,
    }
}

fn normalize_graphql_usage(responses: &[Value]) -> Result<WarpUsageCache> {
    let mut usage = WarpAggregateUsage::default();
    let mut workspaces = Vec::new();

    for response in responses {
        usage.requests_used = usage.requests_used.or_else(|| {
            find_i64_by_keys(
                response,
                &[
                    "requestsUsedSinceLastRefresh",
                    "totalRequestsUsedSinceLastRefresh",
                    "currentMonthlyRequestsUsed",
                ],
            )
        });
        usage.request_limit = usage
            .request_limit
            .or_else(|| find_i64_by_keys(response, &["requestLimit"]));
        usage.spend_cents = usage.spend_cents.or_else(|| {
            find_i64_by_keys(
                response,
                &["currentMonthSpendCents", "currentMonthlyRequestCostCents"],
            )
        });
        usage.credits_purchased_cents = usage
            .credits_purchased_cents
            .or_else(|| find_i64_by_keys(response, &["currentMonthCreditsPurchased"]));
        usage.next_refresh_time = usage
            .next_refresh_time
            .or_else(|| find_string_by_key(response, "nextRefreshTime"));
        workspaces.extend(extract_workspace_usage(response));
    }

    if usage.requests_used.is_none() && usage.spend_cents.is_none() && workspaces.is_empty() {
        anyhow::bail!("Warp GraphQL response did not contain aggregate usage fields");
    }

    Ok(WarpUsageCache {
        version: 1,
        synced_at: chrono::Utc::now().to_rfc3339(),
        usage,
        workspaces,
    })
}

fn extract_workspace_usage(value: &Value) -> Vec<WarpWorkspaceUsage> {
    let mut out = Vec::new();
    collect_workspace_usage(value, &mut out);
    out
}

fn collect_workspace_usage(value: &Value, out: &mut Vec<WarpWorkspaceUsage>) {
    match value {
        Value::Object(map) => {
            let looks_like_workspace = map.contains_key("aiOverages")
                || map.contains_key("totalRequestsUsedSinceLastRefresh");
            if looks_like_workspace {
                let workspace = WarpWorkspaceUsage {
                    id: map
                        .get("id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    name: map
                        .get("name")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    requests_used: find_i64_by_keys(
                        value,
                        &[
                            "currentMonthlyRequestsUsed",
                            "totalRequestsUsedSinceLastRefresh",
                            "requestsUsedSinceLastRefresh",
                        ],
                    ),
                    spend_cents: find_i64_by_keys(
                        value,
                        &["currentMonthlyRequestCostCents", "currentMonthSpendCents"],
                    ),
                };
                if workspace.requests_used.is_some() || workspace.spend_cents.is_some() {
                    out.push(workspace);
                    return;
                }
            }
            for child in map.values() {
                collect_workspace_usage(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_workspace_usage(item, out);
            }
        }
        _ => {}
    }
}

fn find_i64_by_keys(value: &Value, keys: &[&str]) -> Option<i64> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(value_to_i64) {
                    return Some(found.max(0));
                }
            }
            for child in map.values() {
                if let Some(found) = find_i64_by_keys(child, keys) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(|item| find_i64_by_keys(item, keys)),
        _ => None,
    }
}

fn find_string_by_key(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(found) = map.get(key).and_then(Value::as_str) {
                return Some(found.to_string());
            }
            map.values()
                .find_map(|child| find_string_by_key(child, key))
        }
        Value::Array(items) => items.iter().find_map(|item| find_string_by_key(item, key)),
        _ => None,
    }
}

fn value_to_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_graphql_response_ok_rejects_semantic_errors() {
        let response = serde_json::json!({
            "data": {
                "requestLimitInfo": null
            },
            "errors": [
                { "message": "token expired" },
                { "message": "workspace unavailable" }
            ]
        });

        let err = ensure_graphql_response_ok(&response)
            .unwrap_err()
            .to_string();

        assert!(err.contains("Warp GraphQL API returned errors"));
        assert!(err.contains("token expired"));
        assert!(err.contains("workspace unavailable"));
    }

    #[test]
    fn normalize_graphql_usage_extracts_request_limit_and_workspace_overage() {
        let responses = vec![
            serde_json::json!({
                "data": {
                    "requestLimitInfo": {
                        "requestLimit": 100,
                        "requestsUsedSinceLastRefresh": 42,
                        "nextRefreshTime": "2026-06-01T00:00:00Z",
                        "bonusGrantsInfo": {
                            "spendingInfo": {
                                "currentMonthSpendCents": 1234,
                                "currentMonthCreditsPurchased": 500
                            }
                        }
                    }
                }
            }),
            serde_json::json!({
                "data": {
                    "workspacesMetadataForUser": [
                        {
                            "id": "workspace-1",
                            "name": "Personal",
                            "aiOverages": {
                                "currentMonthlyRequestCostCents": 345,
                                "currentMonthlyRequestsUsed": 12
                            }
                        }
                    ]
                }
            }),
        ];

        let cache = normalize_graphql_usage(&responses).unwrap();

        assert_eq!(cache.usage.requests_used, Some(42));
        assert_eq!(cache.usage.request_limit, Some(100));
        assert_eq!(cache.usage.spend_cents, Some(1234));
        assert_eq!(cache.usage.credits_purchased_cents, Some(500));
        assert_eq!(
            cache.usage.next_refresh_time.as_deref(),
            Some("2026-06-01T00:00:00Z")
        );
        assert_eq!(cache.workspaces.len(), 1);
        assert_eq!(cache.workspaces[0].requests_used, Some(12));
        assert_eq!(cache.workspaces[0].spend_cents, Some(345));
    }
}
