use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("Could not determine home directory")
}

fn cursor_credentials_path(home_dir: &Path) -> PathBuf {
    home_dir.join(".config/tokscale/cursor-credentials.json")
}

fn old_cursor_credentials_path(home_dir: &Path) -> PathBuf {
    home_dir.join(".tokscale/cursor-credentials.json")
}

fn cursor_cache_dir(home_dir: &Path) -> PathBuf {
    home_dir.join(".config/tokscale/cursor-cache")
}

fn old_cursor_cache_dir(home_dir: &Path) -> PathBuf {
    home_dir.join(".tokscale/cursor-cache")
}

const USAGE_CSV_ENDPOINT: &str =
    "https://cursor.com/api/dashboard/export-usage-events-csv?strategy=tokens";
const USAGE_SUMMARY_ENDPOINT: &str = "https://cursor.com/api/usage-summary";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorCredentials {
    #[serde(rename = "sessionToken")]
    pub session_token: String,
    #[serde(rename = "userId", skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "expiresAt", skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CursorCredentialsStore {
    pub version: i32,
    #[serde(rename = "activeAccountId")]
    pub active_account_id: String,
    pub accounts: HashMap<String, CursorCredentials>,
}

#[derive(Debug, Serialize)]
pub struct AccountInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(rename = "userId", skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "isActive")]
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct SyncCursorResult {
    pub synced: bool,
    pub rows: usize,
    pub error: Option<String>,
}

pub fn get_cursor_credentials_path() -> Result<PathBuf> {
    Ok(cursor_credentials_path(&home_dir()?))
}

pub fn get_cursor_cache_dir() -> Result<PathBuf> {
    Ok(cursor_cache_dir(&home_dir()?))
}

fn migrate_cache_dir_from_old_path_in_home(home_dir: &Path) {
    let old_dir = old_cursor_cache_dir(home_dir);
    let new_dir = cursor_cache_dir(home_dir);
    if !new_dir.exists()
        && old_dir.exists()
        && fs::create_dir_all(&new_dir).is_ok()
        && copy_dir_recursive(&old_dir, &new_dir).is_ok()
    {
        let _ = fs::remove_dir_all(&old_dir);
    }
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            fs::create_dir_all(&target)?;
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn build_cursor_headers(session_token: &str) -> reqwest::header::HeaderMap {
    use reqwest::header::HeaderValue;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("Accept", HeaderValue::from_static("*/*"));
    headers.insert(
        "Accept-Language",
        HeaderValue::from_static("en-US,en;q=0.9"),
    );
    if let Ok(cookie) = format!("WorkosCursorSessionToken={}", session_token).parse() {
        headers.insert("Cookie", cookie);
    }
    headers.insert(
        "Referer",
        HeaderValue::from_static("https://www.cursor.com/settings"),
    );
    headers.insert(
        "User-Agent",
        HeaderValue::from_static("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"),
    );
    headers
}

fn count_cursor_csv_rows(csv_text: &str) -> usize {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(csv_text.as_bytes());
    reader.records().filter_map(|r| r.ok()).count()
}

fn atomic_write_file(path: &std::path::Path, contents: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid cache path"))?;
    if !parent.exists() {
        fs::create_dir_all(parent)?;
    }

    let temp_name = format!(
        ".tmp-{}-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("cursor"),
        std::process::id()
    );
    let temp_path = parent.join(temp_name);

    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp_path)?;
        file.write_all(contents.as_bytes())?;
    }

    #[cfg(not(unix))]
    {
        fs::write(&temp_path, contents)?;
    }

    if let Err(err) = fs::rename(&temp_path, path) {
        if path.exists() {
            match fs::copy(&temp_path, path) {
                Ok(_) => {
                    let _ = fs::remove_file(&temp_path);
                }
                Err(copy_err) => {
                    let _ = fs::remove_file(&temp_path);
                    return Err(anyhow::anyhow!(
                        "Failed to persist file with rename ({}) and copy fallback ({})",
                        err,
                        copy_err
                    ));
                }
            }
        } else {
            let _ = fs::remove_file(&temp_path);
            return Err(err.into());
        }
    }
    Ok(())
}

fn ensure_config_dir_in_home(home_dir: &Path) -> Result<()> {
    let config_dir = home_dir.join(".config/tokscale");

    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

fn extract_user_id_from_session_token(token: &str) -> Option<String> {
    let token = token.trim();
    if token.contains("%3A%3A") {
        let user_id = token.split("%3A%3A").next()?.trim();
        if user_id.is_empty() {
            return None;
        }
        return Some(user_id.to_string());
    }
    if token.contains("::") {
        let user_id = token.split("::").next()?.trim();
        if user_id.is_empty() {
            return None;
        }
        return Some(user_id.to_string());
    }
    None
}

fn derive_account_id(session_token: &str) -> String {
    if let Some(user_id) = extract_user_id_from_session_token(session_token) {
        return user_id;
    }
    let mut hasher = Sha256::new();
    hasher.update(session_token.as_bytes());
    let hash = hasher.finalize();
    let hex = format!("{:x}", hash);
    format!("anon-{}", &hex[..12])
}

fn sanitize_account_id_for_filename(account_id: &str) -> String {
    let sanitized: String = account_id
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('-');
    let result = if trimmed.len() > 80 {
        &trimmed[..80]
    } else {
        trimmed
    };
    if result.is_empty() {
        "account".to_string()
    } else {
        result.to_string()
    }
}

pub fn load_credentials_store() -> Option<CursorCredentialsStore> {
    let home_dir = home_dir().ok()?;
    load_credentials_store_from_home(&home_dir)
}

fn load_credentials_store_from_home(home_dir: &Path) -> Option<CursorCredentialsStore> {
    let path = cursor_credentials_path(home_dir);
    let old_path = old_cursor_credentials_path(home_dir);
    let read_path = if path.exists() {
        path.clone()
    } else if old_path.exists() {
        old_path
    } else {
        return None;
    };

    let content = fs::read_to_string(&read_path).ok()?;

    if let Ok(mut store) = serde_json::from_str::<CursorCredentialsStore>(&content) {
        if store.version == 1 && !store.accounts.is_empty() {
            let mut changed = false;
            if !store.accounts.contains_key(&store.active_account_id) {
                if let Some(first_id) = store.accounts.keys().next().cloned() {
                    store.active_account_id = first_id;
                    changed = true;
                }
            }
            if changed || read_path != path {
                let _ = save_credentials_store_in_home(home_dir, &store);
            }
            if read_path != path {
                let _ = fs::remove_file(old_cursor_credentials_path(home_dir));
            }
            return Some(store);
        }
    }

    if let Ok(single) = serde_json::from_str::<CursorCredentials>(&content) {
        let account_id = derive_account_id(&single.session_token);
        let mut accounts = HashMap::new();
        accounts.insert(account_id.clone(), single);
        let migrated = CursorCredentialsStore {
            version: 1,
            active_account_id: account_id,
            accounts,
        };

        let _ = save_credentials_store_in_home(home_dir, &migrated);
        if read_path != path {
            let _ = fs::remove_file(old_cursor_credentials_path(home_dir));
        }
        return Some(migrated);
    }

    None
}

pub fn save_credentials_store(store: &CursorCredentialsStore) -> Result<()> {
    save_credentials_store_in_home(&home_dir()?, store)
}

fn save_credentials_store_in_home(home_dir: &Path, store: &CursorCredentialsStore) -> Result<()> {
    ensure_config_dir_in_home(home_dir)?;
    let path = cursor_credentials_path(home_dir);
    let json = serde_json::to_string_pretty(store)?;
    atomic_write_file(&path, &json)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

fn resolve_account_id(store: &CursorCredentialsStore, name_or_id: &str) -> Option<String> {
    let needle = name_or_id.trim();
    if needle.is_empty() {
        return None;
    }

    if store.accounts.contains_key(needle) {
        return Some(needle.to_string());
    }

    let needle_lower = needle.to_lowercase();
    for (id, acct) in &store.accounts {
        if let Some(label) = &acct.label {
            if label.to_lowercase() == needle_lower {
                return Some(id.clone());
            }
        }
    }

    None
}

pub fn list_accounts() -> Vec<AccountInfo> {
    let store = match load_credentials_store() {
        Some(s) => s,
        None => return vec![],
    };

    let mut accounts: Vec<AccountInfo> = store
        .accounts
        .iter()
        .map(|(id, acct)| AccountInfo {
            id: id.clone(),
            label: acct.label.clone(),
            user_id: acct.user_id.clone(),
            created_at: acct.created_at.clone(),
            is_active: id == &store.active_account_id,
        })
        .collect();

    accounts.sort_by(|a, b| {
        if a.is_active != b.is_active {
            return if a.is_active {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        let la = a.label.as_deref().unwrap_or(&a.id).to_lowercase();
        let lb = b.label.as_deref().unwrap_or(&b.id).to_lowercase();
        la.cmp(&lb)
    });

    accounts
}

pub fn find_account(name_or_id: &str) -> Option<AccountInfo> {
    let store = load_credentials_store()?;
    let resolved = resolve_account_id(&store, name_or_id)?;
    let acct = store.accounts.get(&resolved)?;

    Some(AccountInfo {
        id: resolved.clone(),
        label: acct.label.clone(),
        user_id: acct.user_id.clone(),
        created_at: acct.created_at.clone(),
        is_active: resolved == store.active_account_id,
    })
}

pub fn save_credentials(token: &str, label: Option<&str>) -> Result<String> {
    let account_id = derive_account_id(token);
    let user_id = extract_user_id_from_session_token(token);

    let mut store = load_credentials_store().unwrap_or_else(|| CursorCredentialsStore {
        version: 1,
        active_account_id: account_id.clone(),
        accounts: HashMap::new(),
    });

    if let Some(lbl) = label {
        let needle = lbl.trim().to_lowercase();
        if !needle.is_empty() {
            for (id, acct) in &store.accounts {
                if id == &account_id {
                    continue;
                }
                if let Some(existing_label) = &acct.label {
                    if existing_label.trim().to_lowercase() == needle {
                        anyhow::bail!("Cursor account label already exists: {}", lbl);
                    }
                }
            }
        }
    }

    let credentials = CursorCredentials {
        session_token: token.to_string(),
        user_id,
        created_at: chrono::Utc::now().to_rfc3339(),
        expires_at: None,
        label: label.map(|s| s.to_string()),
    };

    store.accounts.insert(account_id.clone(), credentials);
    store.active_account_id = account_id.clone();

    save_credentials_store(&store)?;

    Ok(account_id)
}

pub fn remove_account(name_or_id: &str, purge_cache: bool) -> Result<()> {
    let mut store =
        load_credentials_store().ok_or_else(|| anyhow::anyhow!("No saved Cursor accounts"))?;

    let resolved = resolve_account_id(&store, name_or_id)
        .ok_or_else(|| anyhow::anyhow!("Account not found: {}", name_or_id))?;

    let was_active = resolved == store.active_account_id;

    let cache_dir = get_cursor_cache_dir()?;
    if cache_dir.exists() {
        let per_account = cache_dir.join(format!(
            "usage.{}.csv",
            sanitize_account_id_for_filename(&resolved)
        ));
        if per_account.exists() {
            if purge_cache {
                let _ = fs::remove_file(&per_account);
            } else {
                let _ = archive_cache_file(&per_account, &format!("usage.{}", resolved));
            }
        }
        if was_active {
            let active_file = cache_dir.join("usage.csv");
            if active_file.exists() {
                if purge_cache {
                    let _ = fs::remove_file(&active_file);
                } else {
                    let _ = archive_cache_file(&active_file, &format!("usage.active.{}", resolved));
                }
            }
        }
    }

    store.accounts.remove(&resolved);

    if store.accounts.is_empty() {
        let path = get_cursor_credentials_path()?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        return Ok(());
    }

    if was_active {
        if let Some(first_id) = store.accounts.keys().next().cloned() {
            let new_account_file = cache_dir.join(format!(
                "usage.{}.csv",
                sanitize_account_id_for_filename(&first_id)
            ));
            let active_file = cache_dir.join("usage.csv");
            if new_account_file.exists() {
                let _ = fs::rename(&new_account_file, &active_file);
            }
            store.active_account_id = first_id;
        }
    }

    save_credentials_store(&store)?;
    Ok(())
}

pub fn remove_all_accounts(purge_cache: bool) -> Result<()> {
    let cache_dir = get_cursor_cache_dir()?;
    if cache_dir.exists() {
        if let Ok(entries) = fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("usage") && name.ends_with(".csv") {
                    if purge_cache {
                        let _ = fs::remove_file(entry.path());
                    } else {
                        let _ = archive_cache_file(&entry.path(), &format!("usage.all.{}", name));
                    }
                }
            }
        }
    }

    let path = get_cursor_credentials_path()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn set_active_account(name_or_id: &str) -> Result<()> {
    let mut store =
        load_credentials_store().ok_or_else(|| anyhow::anyhow!("No saved Cursor accounts"))?;

    let resolved = resolve_account_id(&store, name_or_id)
        .ok_or_else(|| anyhow::anyhow!("Account not found: {}", name_or_id))?;

    let old_active_id = store.active_account_id.clone();

    if resolved != old_active_id {
        let _ = reconcile_cache_files(&old_active_id, &resolved);
    }

    store.active_account_id = resolved;
    save_credentials_store(&store)?;

    Ok(())
}

fn reconcile_cache_files(old_account_id: &str, new_account_id: &str) -> Result<()> {
    let cache_dir = get_cursor_cache_dir()?;
    if !cache_dir.exists() {
        return Ok(());
    }

    let active_file = cache_dir.join("usage.csv");
    let old_account_file = cache_dir.join(format!(
        "usage.{}.csv",
        sanitize_account_id_for_filename(old_account_id)
    ));
    let new_account_file = cache_dir.join(format!(
        "usage.{}.csv",
        sanitize_account_id_for_filename(new_account_id)
    ));

    if active_file.exists() {
        if old_account_file.exists() {
            let _ = archive_cache_file(&old_account_file, old_account_id);
        }
        fs::rename(&active_file, &old_account_file)?;
    }

    if new_account_file.exists() {
        if active_file.exists() {
            let _ = archive_cache_file(&active_file, "usage.active");
        }
        fs::rename(&new_account_file, &active_file)?;
    }

    Ok(())
}

pub fn load_active_credentials() -> Option<CursorCredentials> {
    let store = load_credentials_store()?;
    store.accounts.get(&store.active_account_id).cloned()
}

fn is_cursor_usage_csv_filename(name: &str) -> bool {
    if name == "usage.csv" {
        return true;
    }
    if !name.starts_with("usage.") || !name.ends_with(".csv") {
        return false;
    }
    if name.starts_with("usage.backup") {
        return false;
    }
    let stem = name.trim_start_matches("usage.").trim_end_matches(".csv");
    !stem.is_empty()
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

pub fn has_cursor_usage_cache() -> bool {
    let home_dir = match home_dir() {
        Ok(home_dir) => home_dir,
        Err(_) => return false,
    };
    migrate_cache_dir_from_old_path_in_home(&home_dir);
    let cache_dir = cursor_cache_dir(&home_dir);
    if !cache_dir.exists() {
        return false;
    }

    match fs::read_dir(cache_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .any(|name| is_cursor_usage_csv_filename(&name)),
        Err(_) => false,
    }
}

pub fn is_cursor_logged_in() -> bool {
    load_active_credentials().is_some()
}

pub fn load_credentials_for(name_or_id: &str) -> Option<CursorCredentials> {
    let store = load_credentials_store()?;
    let resolved = resolve_account_id(&store, name_or_id)?;
    store.accounts.get(&resolved).cloned()
}

#[derive(Debug)]
pub struct ValidateSessionResult {
    pub valid: bool,
    pub membership_type: Option<String>,
    pub error: Option<String>,
}

pub async fn validate_cursor_session(token: &str) -> ValidateSessionResult {
    let client = reqwest::Client::new();
    let response = match client
        .get(USAGE_SUMMARY_ENDPOINT)
        .headers(build_cursor_headers(token))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            return ValidateSessionResult {
                valid: false,
                membership_type: None,
                error: Some(format!("Failed to connect: {}", e)),
            };
        }
    };

    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        || response.status() == reqwest::StatusCode::FORBIDDEN
    {
        return ValidateSessionResult {
            valid: false,
            membership_type: None,
            error: Some("Session token expired or invalid".to_string()),
        };
    }

    if !response.status().is_success() {
        return ValidateSessionResult {
            valid: false,
            membership_type: None,
            error: Some(format!("API returned status {}", response.status())),
        };
    }

    let data: serde_json::Value = match response.json().await {
        Ok(d) => d,
        Err(e) => {
            return ValidateSessionResult {
                valid: false,
                membership_type: None,
                error: Some(format!("Failed to parse response: {}", e)),
            };
        }
    };

    let has_billing_start = data
        .get("billingCycleStart")
        .and_then(|v| v.as_str())
        .is_some();
    let has_billing_end = data
        .get("billingCycleEnd")
        .and_then(|v| v.as_str())
        .is_some();

    if has_billing_start && has_billing_end {
        let membership_type = data
            .get("membershipType")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        ValidateSessionResult {
            valid: true,
            membership_type,
            error: None,
        }
    } else {
        ValidateSessionResult {
            valid: false,
            membership_type: None,
            error: Some("Invalid response format".to_string()),
        }
    }
}

pub async fn fetch_cursor_usage_csv(session_token: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let response = client
        .get(USAGE_CSV_ENDPOINT)
        .headers(build_cursor_headers(session_token))
        .send()
        .await?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        || response.status() == reqwest::StatusCode::FORBIDDEN
    {
        anyhow::bail!(
            "Cursor session expired. Please run 'bunx tokscale@latest cursor login' to re-authenticate."
        );
    }

    if !response.status().is_success() {
        anyhow::bail!("Cursor API returned status {}", response.status());
    }

    let text = response.text().await?;

    if !text.starts_with("Date,") {
        anyhow::bail!("Invalid response from Cursor API - expected CSV format");
    }

    Ok(text)
}

async fn sync_cursor_cache_with_fetcher<F, Fut>(fetch_usage_csv: F) -> SyncCursorResult
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let home_dir = match home_dir() {
        Ok(home_dir) => home_dir,
        Err(e) => {
            return SyncCursorResult {
                synced: false,
                rows: 0,
                error: Some(format!("Failed to get home dir: {}", e)),
            };
        }
    };

    sync_cursor_cache_with_fetcher_in_home(&home_dir, fetch_usage_csv).await
}

async fn sync_cursor_cache_with_fetcher_in_home<F, Fut>(
    home_dir: &Path,
    fetch_usage_csv: F,
) -> SyncCursorResult
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    migrate_cache_dir_from_old_path_in_home(home_dir);

    let store = match load_credentials_store_from_home(home_dir) {
        Some(s) => s,
        None => {
            return SyncCursorResult {
                synced: false,
                rows: 0,
                error: Some("Not authenticated".to_string()),
            };
        }
    };

    if store.accounts.is_empty() {
        return SyncCursorResult {
            synced: false,
            rows: 0,
            error: Some("Not authenticated".to_string()),
        };
    }

    let cache_dir = cursor_cache_dir(home_dir);
    if let Err(e) = fs::create_dir_all(&cache_dir) {
        return SyncCursorResult {
            synced: false,
            rows: 0,
            error: Some(format!("Failed to create cache dir: {}", e)),
        };
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&cache_dir, fs::Permissions::from_mode(0o700));
    }

    let active_dup = cache_dir.join(format!(
        "usage.{}.csv",
        sanitize_account_id_for_filename(&store.active_account_id)
    ));
    if active_dup.exists() {
        let _ = fs::remove_file(&active_dup);
    }

    let mut total_rows = 0;
    let mut success_count = 0;
    let mut errors: Vec<String> = Vec::new();

    for (account_id, credentials) in &store.accounts {
        let is_active = account_id == &store.active_account_id;

        match fetch_usage_csv(credentials.session_token.clone()).await {
            Ok(csv_text) => {
                let file_path = if is_active {
                    cache_dir.join("usage.csv")
                } else {
                    cache_dir.join(format!(
                        "usage.{}.csv",
                        sanitize_account_id_for_filename(account_id)
                    ))
                };

                let row_count = count_cursor_csv_rows(&csv_text);

                if let Err(e) = atomic_write_file(&file_path, &csv_text) {
                    errors.push(format!("{}: {}", account_id, e));
                } else {
                    total_rows += row_count;
                    success_count += 1;
                }
            }
            Err(e) => {
                errors.push(format!("{}: {}", account_id, e));
            }
        }
    }

    if success_count == 0 {
        return SyncCursorResult {
            synced: false,
            rows: 0,
            error: Some(
                errors
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Cursor sync failed".to_string()),
            ),
        };
    }

    SyncCursorResult {
        synced: true,
        rows: total_rows,
        error: if errors.is_empty() {
            None
        } else {
            Some(format!(
                "Some accounts failed to sync ({}/{})",
                errors.len(),
                store.accounts.len()
            ))
        },
    }
}

pub async fn sync_cursor_cache() -> SyncCursorResult {
    sync_cursor_cache_with_fetcher(|session_token| async move {
        fetch_cursor_usage_csv(&session_token).await
    })
    .await
}

fn archive_cache_file(file_path: &std::path::Path, label: &str) -> Result<()> {
    let cache_dir = get_cursor_cache_dir()?;
    let archive_dir = cache_dir.join("archive");
    if !archive_dir.exists() {
        fs::create_dir_all(&archive_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&archive_dir, fs::Permissions::from_mode(0o700))?;
        }
    }

    let safe_label = sanitize_account_id_for_filename(label);
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let dest = archive_dir.join(format!("{}-{}.csv", safe_label, ts));
    fs::rename(file_path, dest)?;
    Ok(())
}

pub fn run_cursor_login(name: Option<String>) -> Result<()> {
    use colored::Colorize;
    use tokio::runtime::Runtime;

    let rt = Runtime::new()?;

    println!("\n  {}\n", "Cursor IDE - Login".cyan());

    if let Some(ref label) = name {
        if find_account(label).is_some() {
            println!(
                "  {}",
                format!(
                    "Account '{}' already exists. Use 'bunx tokscale@latest cursor logout --name {}' first.",
                    label, label
                )
                .yellow()
            );
            println!();
            return Ok(());
        }
    }

    print!("  Enter Cursor session token: ");
    std::io::stdout().flush()?;
    let token = rpassword::read_password().context("Failed to read session token")?;
    let token = token.trim().to_string();

    if token.is_empty() {
        println!("\n  {}\n", "No token provided.".yellow());
        return Ok(());
    }

    println!();
    println!("{}", "  Validating session token...".bright_black());

    let result = rt.block_on(async { validate_cursor_session(&token).await });

    if !result.valid {
        let msg = result
            .error
            .unwrap_or_else(|| "Invalid session token".to_string());
        println!(
            "\n  {}\n",
            format!("{}. Please check and try again.", msg).red()
        );
        std::process::exit(1);
    }

    let account_id = save_credentials(&token, name.as_deref())?;

    let display_name = name.as_deref().unwrap_or(&account_id);
    println!(
        "\n  {}",
        format!(
            "Successfully logged in to Cursor as {}",
            display_name.bold()
        )
        .green()
    );
    println!("{}", format!("  Account ID: {}", account_id).bright_black());
    println!();

    Ok(())
}

pub fn run_cursor_logout(name: Option<String>, all: bool, purge_cache: bool) -> Result<()> {
    use colored::Colorize;

    if all {
        let accounts = list_accounts();
        if accounts.is_empty() {
            println!("\n  {}\n", "No saved Cursor accounts.".yellow());
            return Ok(());
        }

        remove_all_accounts(purge_cache)?;
        println!("\n  {}\n", "Logged out from all Cursor accounts.".green());
        return Ok(());
    }

    if let Some(ref account_name) = name {
        remove_account(account_name, purge_cache)?;
        println!(
            "\n  {}\n",
            format!("Logged out from Cursor account '{}'.", account_name).green()
        );
        return Ok(());
    }

    let Some(store) = load_credentials_store() else {
        println!("\n  {}\n", "No saved Cursor accounts.".yellow());
        return Ok(());
    };
    let active_id = store.active_account_id.clone();
    let display = store
        .accounts
        .get(&active_id)
        .and_then(|a| a.label.clone())
        .unwrap_or_else(|| active_id.clone());

    remove_account(&active_id, purge_cache)?;
    println!(
        "\n  {}\n",
        format!("Logged out from Cursor account '{}'.", display).green()
    );

    Ok(())
}

pub fn run_cursor_status(name: Option<String>) -> Result<()> {
    use colored::Colorize;
    use tokio::runtime::Runtime;

    let rt = Runtime::new()?;

    let credentials = if let Some(ref account_name) = name {
        load_credentials_for(account_name)
    } else {
        load_active_credentials()
    };

    let credentials = match credentials {
        Some(c) => c,
        None => {
            if let Some(ref account_name) = name {
                println!(
                    "\n  {}\n",
                    format!("Account not found: {}", account_name).red()
                );
            } else {
                println!("\n  {}", "No saved Cursor accounts.".yellow());
                println!(
                    "{}",
                    "  Run 'bunx tokscale@latest cursor login' to authenticate.\n".bright_black()
                );
            }
            return Ok(());
        }
    };

    println!("\n  {}\n", "Cursor IDE - Status".cyan());

    let display_name = credentials.label.as_deref().unwrap_or("(no label)");
    println!("{}", format!("  Account: {}", display_name).white());
    if let Some(ref uid) = credentials.user_id {
        println!("{}", format!("  User ID: {}", uid).bright_black());
    }

    println!("{}", "  Validating session...".bright_black());

    let result = rt.block_on(async { validate_cursor_session(&credentials.session_token).await });

    if result.valid {
        println!("  {}", "Session: Valid".green());
        if let Some(membership) = result.membership_type {
            println!("{}", format!("  Membership: {}", membership).bright_black());
        }
    } else {
        let msg = result
            .error
            .unwrap_or_else(|| "Invalid / Expired".to_string());
        println!("  {}", format!("Session: {}", msg).red());
    }
    println!();

    Ok(())
}

pub fn run_cursor_accounts(json: bool) -> Result<()> {
    use colored::Colorize;

    let accounts = list_accounts();

    if json {
        #[derive(Serialize)]
        struct Output {
            accounts: Vec<AccountInfo>,
        }
        let output = Output { accounts };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if accounts.is_empty() {
        println!("\n  {}\n", "No saved Cursor accounts.".yellow());
        return Ok(());
    }

    println!("{}", "\n  Cursor IDE - Accounts\n".cyan());
    for acct in &accounts {
        let name = if let Some(ref label) = acct.label {
            format!("{} ({})", label, acct.id)
        } else {
            acct.id.clone()
        };
        let marker = if acct.is_active { "*" } else { "-" };
        let marker_colored = if acct.is_active {
            marker.green().to_string()
        } else {
            marker.bright_black().to_string()
        };
        println!("  {} {}", marker_colored, name);
    }
    println!();

    Ok(())
}

pub fn run_cursor_sync(json: bool) -> Result<()> {
    use colored::Colorize;
    use tokio::runtime::Runtime;

    let rt = Runtime::new()?;
    let result = rt.block_on(sync_cursor_cache());

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    println!("\n  {}\n", "Cursor IDE - Sync".cyan());
    if result.synced {
        println!(
            "{}",
            format!("  Synced {} Cursor usage event(s).", result.rows).green()
        );
        if let Some(error) = result.error {
            println!("{}", format!("  Warning: {}", error).yellow());
        }
    } else if let Some(error) = result.error {
        println!("{}", format!("  Sync failed: {}", error).red());
    } else {
        println!("{}", "  Sync failed.".red());
    }
    println!();

    Ok(())
}

pub fn run_cursor_switch(name: &str) -> Result<()> {
    use colored::Colorize;

    set_active_account(name)?;
    println!(
        "\n  {}\n",
        format!("Active Cursor account set to {}", name.bold()).green()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    #[test]
    fn test_extract_user_id_from_session_token_with_url_encoding() {
        // Test URL-encoded separator (%3A%3A)
        assert_eq!(
            extract_user_id_from_session_token("user123%3A%3Atoken456"),
            Some("user123".to_string())
        );
        assert_eq!(
            extract_user_id_from_session_token("  user123%3A%3Atoken456  "),
            Some("user123".to_string())
        );
    }

    #[test]
    fn test_extract_user_id_from_session_token_with_double_colon() {
        // Test plain :: separator
        assert_eq!(
            extract_user_id_from_session_token("user456::token789"),
            Some("user456".to_string())
        );
        assert_eq!(
            extract_user_id_from_session_token("  user456::token789  "),
            Some("user456".to_string())
        );
    }

    #[test]
    fn test_extract_user_id_from_session_token_invalid() {
        // No separator
        assert_eq!(extract_user_id_from_session_token("invalidtoken"), None);
        // Empty user ID
        assert_eq!(extract_user_id_from_session_token("%3A%3Atoken"), None);
        assert_eq!(extract_user_id_from_session_token("::token"), None);
        // Empty string
        assert_eq!(extract_user_id_from_session_token(""), None);
        // Whitespace only
        assert_eq!(extract_user_id_from_session_token("   "), None);
    }

    #[test]
    fn test_derive_account_id_with_user_id() {
        // Should extract user ID when present
        let account_id = derive_account_id("user123%3A%3Atoken456");
        assert_eq!(account_id, "user123");

        let account_id = derive_account_id("user456::token789");
        assert_eq!(account_id, "user456");
    }

    #[test]
    fn test_derive_account_id_without_user_id() {
        // Should generate anon-{hash} when no user ID
        let account_id = derive_account_id("randomtoken");
        assert!(account_id.starts_with("anon-"));
        assert_eq!(account_id.len(), 17); // "anon-" + 12 hex chars

        // Same token should produce same hash
        let account_id2 = derive_account_id("randomtoken");
        assert_eq!(account_id, account_id2);

        // Different tokens should produce different hashes
        let account_id3 = derive_account_id("differenttoken");
        assert_ne!(account_id, account_id3);
    }

    #[test]
    fn test_sanitize_account_id_for_filename_basic() {
        // Alphanumeric, dots, underscores, hyphens should be preserved
        assert_eq!(sanitize_account_id_for_filename("user123"), "user123");
        assert_eq!(
            sanitize_account_id_for_filename("user.name_123-test"),
            "user.name_123-test"
        );
    }

    #[test]
    fn test_sanitize_account_id_for_filename_unsafe_chars() {
        // Unsafe characters should be replaced with hyphens
        assert_eq!(
            sanitize_account_id_for_filename("user@example.com"),
            "user-example.com"
        );
        assert_eq!(
            sanitize_account_id_for_filename("user/name\\test"),
            "user-name-test"
        );
        assert_eq!(sanitize_account_id_for_filename("user name"), "user-name");
    }

    #[test]
    fn test_sanitize_account_id_for_filename_edge_cases() {
        // Uppercase should be lowercased
        assert_eq!(
            sanitize_account_id_for_filename("UserName123"),
            "username123"
        );

        // Leading/trailing hyphens should be trimmed
        assert_eq!(sanitize_account_id_for_filename("---user---"), "user");

        // Empty after sanitization should return "account"
        assert_eq!(sanitize_account_id_for_filename("@@@"), "account");
        assert_eq!(sanitize_account_id_for_filename(""), "account");

        // Whitespace only should return "account"
        assert_eq!(sanitize_account_id_for_filename("   "), "account");
    }

    #[test]
    fn test_sanitize_account_id_for_filename_length_limit() {
        // Should truncate to 80 characters
        let long_id = "a".repeat(100);
        let sanitized = sanitize_account_id_for_filename(&long_id);
        assert_eq!(sanitized.len(), 80);
        assert_eq!(sanitized, "a".repeat(80));

        // Should preserve exactly 80 characters
        let exactly_80 = "b".repeat(80);
        let sanitized = sanitize_account_id_for_filename(&exactly_80);
        assert_eq!(sanitized.len(), 80);
    }

    #[test]
    fn test_count_cursor_csv_rows_valid() {
        // Valid CSV with header
        let csv = "Date,Model,Tokens\n2024-01-01,gpt-4,100\n2024-01-02,gpt-4,200\n";
        assert_eq!(count_cursor_csv_rows(csv), 2);

        // Single row
        let csv = "Date,Model,Tokens\n2024-01-01,gpt-4,100\n";
        assert_eq!(count_cursor_csv_rows(csv), 1);
    }

    #[test]
    fn test_count_cursor_csv_rows_empty() {
        // Header only
        let csv = "Date,Model,Tokens\n";
        assert_eq!(count_cursor_csv_rows(csv), 0);

        // Empty string
        let csv = "";
        assert_eq!(count_cursor_csv_rows(csv), 0);
    }

    #[test]
    fn test_count_cursor_csv_rows_malformed() {
        // CSV reader with flexible=true accepts rows with different column counts
        // This test verifies the actual behavior: all parseable rows are counted
        let csv = "Date,Model,Tokens\n2024-01-01,gpt-4,100\ninvalid,row\n2024-01-02,gpt-4,200\n";
        assert_eq!(count_cursor_csv_rows(csv), 3);
    }

    #[test]
    fn test_sync_cursor_cache_writes_active_and_secondary_account_files() -> Result<()> {
        let temp_dir = TempDir::new()?;

        let mut accounts = HashMap::new();
        accounts.insert(
            "active-account".to_string(),
            CursorCredentials {
                session_token: "token-active".to_string(),
                user_id: Some("active-account".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: None,
                label: Some("work".to_string()),
            },
        );
        accounts.insert(
            "team/account".to_string(),
            CursorCredentials {
                session_token: "token-secondary".to_string(),
                user_id: Some("team/account".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: None,
                label: Some("personal".to_string()),
            },
        );
        save_credentials_store_in_home(
            temp_dir.path(),
            &CursorCredentialsStore {
                version: 1,
                active_account_id: "active-account".to_string(),
                accounts,
            },
        )?;

        let runtime = tokio::runtime::Runtime::new()?;
        let result = runtime.block_on(sync_cursor_cache_with_fetcher_in_home(
            temp_dir.path(),
            |session_token| {
                let csv = match session_token.as_str() {
                    "token-active" => "Date,Model,Tokens\n2026-01-01,gpt-5,100\n",
                    "token-secondary" => {
                        "Date,Model,Tokens\n2026-01-02,gpt-5,200\n2026-01-03,gpt-5,300\n"
                    }
                    _ => "Date,Model,Tokens\n",
                }
                .to_string();
                async move { Ok(csv) }
            },
        ));

        assert!(result.synced);
        assert_eq!(result.rows, 3);
        assert_eq!(result.error, None);

        let cache_dir = cursor_cache_dir(temp_dir.path());
        assert_eq!(
            fs::read_to_string(cache_dir.join("usage.csv"))?,
            "Date,Model,Tokens\n2026-01-01,gpt-5,100\n"
        );
        assert_eq!(
            fs::read_to_string(cache_dir.join("usage.team-account.csv"))?,
            "Date,Model,Tokens\n2026-01-02,gpt-5,200\n2026-01-03,gpt-5,300\n"
        );
        assert!(!cache_dir.join("usage.active-account.csv").exists());

        Ok(())
    }

    #[test]
    fn test_atomic_write_file_basic() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let file_path = temp_dir.path().join("test.txt");
        let contents = "Hello, world!";

        atomic_write_file(&file_path, contents)?;

        // Verify file was created and contains correct content
        assert!(file_path.exists());
        let read_contents = fs::read_to_string(&file_path)?;
        assert_eq!(read_contents, contents);

        Ok(())
    }

    #[test]
    fn test_atomic_write_file_creates_parent_dirs() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let nested_path = temp_dir
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("test.txt");
        let contents = "Nested file";

        atomic_write_file(&nested_path, contents)?;

        // Verify parent directories were created
        assert!(nested_path.exists());
        let read_contents = fs::read_to_string(&nested_path)?;
        assert_eq!(read_contents, contents);

        Ok(())
    }

    #[test]
    fn test_atomic_write_file_overwrites_existing() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let file_path = temp_dir.path().join("test.txt");

        // Write initial content
        atomic_write_file(&file_path, "Initial")?;
        assert_eq!(fs::read_to_string(&file_path)?, "Initial");

        // Overwrite with new content
        atomic_write_file(&file_path, "Updated")?;
        assert_eq!(fs::read_to_string(&file_path)?, "Updated");

        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn test_atomic_write_file_permissions() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new()?;
        let file_path = temp_dir.path().join("test.txt");

        atomic_write_file(&file_path, "Secret")?;

        // Verify file has 0o600 permissions (owner read/write only)
        let metadata = fs::metadata(&file_path)?;
        let permissions = metadata.permissions();
        assert_eq!(permissions.mode() & 0o777, 0o600);

        Ok(())
    }

    #[test]
    fn test_copy_dir_recursive_basic() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let src = temp_dir.path().join("src");
        let dst = temp_dir.path().join("dst");

        // Create source directory structure
        fs::create_dir_all(&src)?;
        fs::write(src.join("file1.txt"), "Content 1")?;
        fs::write(src.join("file2.txt"), "Content 2")?;

        // Create destination directory
        fs::create_dir_all(&dst)?;

        // Copy recursively
        copy_dir_recursive(&src, &dst)?;

        // Verify files were copied
        assert!(dst.join("file1.txt").exists());
        assert!(dst.join("file2.txt").exists());
        assert_eq!(fs::read_to_string(dst.join("file1.txt"))?, "Content 1");
        assert_eq!(fs::read_to_string(dst.join("file2.txt"))?, "Content 2");

        Ok(())
    }

    #[test]
    fn test_copy_dir_recursive_nested() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let src = temp_dir.path().join("src");
        let dst = temp_dir.path().join("dst");

        // Create nested source directory structure
        fs::create_dir_all(src.join("subdir1").join("subdir2"))?;
        fs::write(src.join("root.txt"), "Root")?;
        fs::write(src.join("subdir1").join("file1.txt"), "File 1")?;
        fs::write(
            src.join("subdir1").join("subdir2").join("file2.txt"),
            "File 2",
        )?;

        // Create destination directory
        fs::create_dir_all(&dst)?;

        // Copy recursively
        copy_dir_recursive(&src, &dst)?;

        // Verify nested structure was copied
        assert!(dst.join("root.txt").exists());
        assert!(dst.join("subdir1").join("file1.txt").exists());
        assert!(dst
            .join("subdir1")
            .join("subdir2")
            .join("file2.txt")
            .exists());
        assert_eq!(fs::read_to_string(dst.join("root.txt"))?, "Root");
        assert_eq!(
            fs::read_to_string(dst.join("subdir1").join("file1.txt"))?,
            "File 1"
        );
        assert_eq!(
            fs::read_to_string(dst.join("subdir1").join("subdir2").join("file2.txt"))?,
            "File 2"
        );

        Ok(())
    }

    #[test]
    fn test_copy_dir_recursive_empty_dir() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let src = temp_dir.path().join("src");
        let dst = temp_dir.path().join("dst");

        // Create empty source directory
        fs::create_dir_all(&src)?;
        fs::create_dir_all(&dst)?;

        // Copy recursively (should succeed with no files)
        copy_dir_recursive(&src, &dst)?;

        // Verify destination exists but is empty
        assert!(dst.exists());
        assert_eq!(fs::read_dir(&dst)?.count(), 0);

        Ok(())
    }
}
