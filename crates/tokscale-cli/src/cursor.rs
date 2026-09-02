use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Timeout for every Cursor HTTP request. Picked to bound the worst case for
/// auto-sync (which runs synchronously before local reports and the TUI) while
/// still tolerating routine API latency. If the network is hung, the report
/// proceeds against cached data after this timeout instead of stalling forever.
/// Explicit `tokscale cursor sync` overrides this for the usage-CSV download
/// with [`CURSOR_EXPLICIT_SYNC_TIMEOUT`].
const CURSOR_HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Per-request timeout for the usage-CSV download during an explicit
/// `tokscale cursor sync`. Large accounts export CSVs that take well over the
/// default [`CURSOR_HTTP_TIMEOUT`] to generate and stream (issue #1175); the
/// user asked for the sync, so waiting longer beats failing fast.
const CURSOR_EXPLICIT_SYNC_TIMEOUT: Duration = Duration::from_secs(120);

/// Skip implicit pre-report sync when every expected Cursor account cache file
/// was modified within this window. Prevents `tokscale models` (and its
/// siblings) from issuing a Cursor API call on every invocation. The manual
/// `tokscale cursor sync` command bypasses this — explicit user intent is
/// always honored.
pub const CURSOR_AUTO_SYNC_FRESHNESS: Duration = Duration::from_secs(5 * 60);

/// The Cursor client's builder, split out of `build_cursor_http_client` so a
/// test can assert which TLS backend it ends up on.
///
/// cursor.com sits behind Vercel bot protection that fingerprints the TLS
/// ClientHello, and rustls' hello is on the block list — since #1138 moved the
/// workspace to rustls, `cursor login` / `cursor sync` have received a
/// challenge page instead of the API response (#1250). This is the one client
/// that asks for the platform's own TLS stack: Security.framework on macOS,
/// schannel on Windows, OpenSSL on Linux. Every other client stays on rustls
/// through `tokscale_core::http`.
fn cursor_http_client_builder() -> reqwest::ClientBuilder {
    #[allow(clippy::disallowed_methods)]
    let builder = reqwest::Client::builder().timeout(CURSOR_HTTP_TIMEOUT);

    // Android is excluded from `native-tls` in crates/tokscale-cli/Cargo.toml,
    // because openssl-src builds Android with `no-stdio` and that leaves
    // OpenSSL unable to read a CA file at all. Keep this predicate textually
    // identical to the manifest's; `tests/tls_policy.rs` checks that it is.
    // The cost is that Cursor login stays broken on Android, where it reports
    // itself as an expired session (see `sync_cursor_usage`).
    #[cfg(not(target_os = "android"))]
    let builder = builder.use_native_tls();

    builder
}

fn build_cursor_http_client() -> Result<reqwest::Client> {
    cursor_http_client_builder()
        .build()
        .context("Failed to build Cursor HTTP client")
}

fn home_dir() -> Result<PathBuf> {
    crate::paths::home_dir().context("Could not determine home directory")
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

const USAGE_EVENTS_JSON_ENDPOINT: &str =
    "https://cursor.com/api/dashboard/get-filtered-usage-events";
const USAGE_SUMMARY_ENDPOINT: &str = "https://cursor.com/api/usage-summary";

/// Number of usage events requested per page from the JSON endpoint. The
/// endpoint paginates, so the fetcher walks pages until it has collected
/// `totalUsageEventsCount` events (or a page comes back short/empty).
const CURSOR_JSON_PAGE_SIZE: usize = 500;

/// Hard ceiling on pages walked in one fetch, so a server that keeps returning
/// full pages (or a mis-reported total) can't spin forever. At the page size
/// above this admits up to a quarter-million events.
const CURSOR_MAX_JSON_PAGES: usize = 500;

/// Cumulative ceiling on the usage-events JSON downloaded in one fetch, enforced
/// *while* each page is read rather than after it has all arrived, and counted
/// across every page so a paginated response can't sidestep it. Without it a
/// download buffers whatever the server sends for as long as the transfer is
/// allowed to run, and [`CURSOR_EXPLICIT_SYNC_TIMEOUT`] deliberately widens that
/// window to 120s — so a malformed or runaway response could grow process memory
/// for two minutes.
///
/// 64 MiB is generous for a full usage history and still bounded, holding peak
/// memory for the download to a fraction of a developer machine's RAM.
const CURSOR_MAX_JSON_BYTES: usize = 64 * 1024 * 1024;

/// Marker file touched at the end of every `sync_cursor_cache` run (even when
/// some accounts fail). Its mtime gates secondary-account freshness checks so
/// a permanently-stale secondary (expired token, removed account, network
/// partition) does not force an implicit sync on every invocation.
const CURSOR_SYNC_ATTEMPT_MARKER: &str = "usage.last-sync-attempt";

/// Cache-file extensions tokscale recognizes, in preference order: JSON is the
/// current format, CSV is the legacy export still read for pre-switch caches.
const CURSOR_CACHE_EXTENSIONS: [&str; 2] = ["json", "csv"];

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

/// Headers for the JSON dashboard endpoints. Adds a JSON `Content-Type` and the
/// `Origin` header the `get-filtered-usage-events` CSRF check requires on top of
/// the shared cookie/UA headers.
fn build_cursor_json_headers(session_token: &str) -> reqwest::header::HeaderMap {
    use reqwest::header::HeaderValue;

    let mut headers = build_cursor_headers(session_token);
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    headers.insert("Origin", HeaderValue::from_static("https://cursor.com"));
    headers
}

/// Count events in an aggregated usage-events JSON document. Invalid JSON or a
/// missing `usageEventsDisplay` array counts as zero.
fn count_cursor_json_events(json_text: &str) -> usize {
    serde_json::from_str::<serde_json::Value>(json_text)
        .ok()
        .and_then(|value| {
            value
                .get("usageEventsDisplay")
                .and_then(|events| events.as_array())
                .map(|events| events.len())
        })
        .unwrap_or(0)
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

/// Candidate paths for Cursor desktop `state.vscdb` (VS Code globalStorage).
///
/// Mirrors the layout used by Cursor Usage Agent on Windows, plus the standard
/// Electron/Chromium config dirs on Linux and macOS.
pub fn cursor_state_vscdb_candidates(home_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "macos")]
    {
        paths.push(
            home_dir.join("Library/Application Support/Cursor/User/globalStorage/state.vscdb"),
        );
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            paths.push(
                PathBuf::from(appdata)
                    .join("Cursor")
                    .join("User/globalStorage/state.vscdb"),
            );
        }
        paths.push(home_dir.join("AppData/Roaming/Cursor/User/globalStorage/state.vscdb"));
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        paths.push(home_dir.join(".config/Cursor/User/globalStorage/state.vscdb"));
    }

    paths
}

fn find_cursor_state_vscdb(home_dir: &Path) -> Option<PathBuf> {
    cursor_state_vscdb_candidates(home_dir)
        .into_iter()
        .find(|path| path.is_file())
}

/// Read `cursorAuth/accessToken` from a Cursor `state.vscdb` SQLite DB.
pub fn read_access_token_from_state_vscdb(db_path: &Path) -> Result<String> {
    use rusqlite::{Connection, OpenFlags};

    // Keep this opener separate from sessions::utils::open_readonly_sqlite:
    // it accepts a URI string (`file:...?mode=ro`), intentionally omits
    // SQLITE_OPEN_NO_MUTEX, and adds the anyhow context needed by this CLI
    // token lookup.
    let uri = format!("file:{}?mode=ro", db_path.display());
    let conn = Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("Failed to open Cursor state DB at {}", db_path.display()))?;

    let token: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'cursorAuth/accessToken'",
            [],
            |row| row.get(0),
        )
        .context("cursorAuth/accessToken not found in Cursor state DB (is Cursor logged in?)")?;

    if token.trim().is_empty() {
        anyhow::bail!("cursorAuth/accessToken is empty");
    }
    Ok(token)
}

/// Extract the Cursor `user_…` id from a JWT `sub` claim (e.g. `auth0|user_abc`).
fn user_id_from_access_token_jwt(access_token: &str) -> Result<String> {
    use base64::Engine;

    let payload_b64 = access_token
        .split('.')
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Invalid Cursor access token JWT"))?;
    let padded = match payload_b64.len() % 4 {
        2 => format!("{}==", payload_b64),
        3 => format!("{}=", payload_b64),
        _ => payload_b64.to_string(),
    };
    let bytes = base64::engine::general_purpose::URL_SAFE
        .decode(padded.as_bytes())
        .or_else(|_| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload_b64.as_bytes())
        })
        .context("Failed to decode Cursor access token JWT payload")?;
    let payload: serde_json::Value =
        serde_json::from_slice(&bytes).context("Failed to parse Cursor access token JWT")?;
    let sub = payload
        .get("sub")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Cursor access token JWT missing sub claim"))?;

    if let Some(idx) = sub.find("user_") {
        let rest = &sub[idx..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let user_id = &rest[..end];
        if user_id.len() > "user_".len() {
            return Ok(user_id.to_string());
        }
    }

    anyhow::bail!("Cannot parse Cursor user id from JWT sub: {sub}");
}

/// Build the `WorkosCursorSessionToken` cookie value from a desktop access token.
///
/// Format matches browser cookies and Cursor Usage Agent:
/// `{user_id}%3A%3A{access_token}` (`%3A%3A` is URL-encoded `::`).
pub fn session_token_from_access_token(access_token: &str) -> Result<String> {
    let user_id = user_id_from_access_token_jwt(access_token)?;
    Ok(format!("{user_id}%3A%3A{access_token}"))
}

/// Read the local Cursor desktop login and build a session token cookie value.
pub fn read_local_cursor_session_token() -> Result<String> {
    let home = home_dir()?;
    let db_path = find_cursor_state_vscdb(&home).ok_or_else(|| {
        anyhow::anyhow!("Cursor desktop state.vscdb not found (install Cursor and sign in first)")
    })?;
    let access_token = read_access_token_from_state_vscdb(&db_path)?;
    session_token_from_access_token(&access_token)
}

/// Save credentials while preserving an existing account label / created_at when
/// the caller does not supply a new label (used by local desktop refresh).
fn upsert_credentials(token: &str, label: Option<&str>) -> Result<String> {
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

    let existing = store.accounts.get(&account_id);
    let created_at = existing
        .map(|c| c.created_at.clone())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let resolved_label = label
        .map(|s| s.to_string())
        .or_else(|| existing.and_then(|c| c.label.clone()));

    let credentials = CursorCredentials {
        session_token: token.to_string(),
        user_id,
        created_at,
        expires_at: None,
        label: resolved_label,
    };

    store.accounts.insert(account_id.clone(), credentials);

    // Switching the active account must also move the cache files, exactly like
    // `set_active_account`. `usage.json` always belongs to the active account, so
    // when the desktop login points at a different account than the stored one we
    // rename the current `usage.json` back under the old account and promote the
    // incoming account's per-account cache to `usage.json`. Without this the sync
    // reconciliation would treat the previous account's `usage.json` as the new
    // active account's and could drop the new account's only cache.
    let old_active_id = store.active_account_id.clone();
    let reconciled = if old_active_id != account_id {
        reconcile_cache_files(&old_active_id, &account_id)
    } else {
        Ok(())
    };

    // A freshly read desktop token is worth keeping even when the cache files
    // could not be moved, so the accounts are saved either way. Only point
    // `active_account_id` at the new account once its cache really is
    // `usage.json`.
    if reconciled.is_ok() {
        store.active_account_id = account_id.clone();
    }
    save_credentials_store(&store)?;
    reconciled?;
    Ok(account_id)
}

/// Best-effort: refresh saved Cursor credentials from the desktop `state.vscdb`.
///
/// Used before sync so tokscale picks up tokens refreshed by the Cursor app
/// without requiring a manual cookie paste.
fn ensure_credentials_from_local_cursor() -> Result<Option<String>> {
    match read_local_cursor_session_token() {
        Ok(token) => Ok(Some(upsert_credentials(&token, None)?)),
        Err(_) => Ok(None),
    }
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
    upsert_credentials(token, label)
}

pub fn remove_account(name_or_id: &str, purge_cache: bool) -> Result<()> {
    let mut store =
        load_credentials_store().ok_or_else(|| anyhow::anyhow!("No saved Cursor accounts"))?;

    let resolved = resolve_account_id(&store, name_or_id)
        .ok_or_else(|| anyhow::anyhow!("Account not found: {}", name_or_id))?;

    let was_active = resolved == store.active_account_id;

    let cache_dir = get_cursor_cache_dir()?;
    if cache_dir.exists() {
        for ext in CURSOR_CACHE_EXTENSIONS {
            let per_account = cache_dir.join(format!(
                "usage.{}.{ext}",
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
                let active_file = cache_dir.join(format!("usage.{ext}"));
                if active_file.exists() {
                    if purge_cache {
                        let _ = fs::remove_file(&active_file);
                    } else {
                        let _ =
                            archive_cache_file(&active_file, &format!("usage.active.{}", resolved));
                    }
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
            for ext in CURSOR_CACHE_EXTENSIONS {
                let new_account_file = cache_dir.join(format!(
                    "usage.{}.{ext}",
                    sanitize_account_id_for_filename(&first_id)
                ));
                let active_file = cache_dir.join(format!("usage.{ext}"));
                if new_account_file.exists() {
                    let _ = fs::rename(&new_account_file, &active_file);
                }
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
                if name.starts_with("usage") && (name.ends_with(".json") || name.ends_with(".csv"))
                {
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
        // Do not record the switch when the caches could not be moved: the active
        // `usage.*` would then belong to a different account than
        // `active_account_id` claims, and the next sync would file one account's
        // usage under the other.
        reconcile_cache_files(&old_active_id, &resolved)?;
    }

    store.active_account_id = resolved;
    save_credentials_store(&store)?;

    Ok(())
}

fn reconcile_cache_files(old_account_id: &str, new_account_id: &str) -> Result<()> {
    let cache_dir = get_cursor_cache_dir()?;
    reconcile_cache_files_in_dir(&cache_dir, old_account_id, new_account_id)
}

/// Moves `usage.<ext>` back under `old_account_id` and promotes
/// `usage.<new_account_id>.<ext>` into its place, for every cache extension.
///
/// Kept `cache_dir`-relative for the same reason as [`archive_cache_file_in_dir`]:
/// so tests can point it at a temporary home.
///
/// A failure on one extension does not abort the others. Each extension is an
/// independent pair of caches, so returning early would leave the JSON cache
/// moved while the legacy CSV cache stays under the wrong account. Every failure
/// is collected and reported instead, so the caller can decline to record the
/// switch rather than pointing `active_account_id` at another account's cache.
fn reconcile_cache_files_in_dir(
    cache_dir: &Path,
    old_account_id: &str,
    new_account_id: &str,
) -> Result<()> {
    if !cache_dir.exists() {
        return Ok(());
    }

    let mut failures: Vec<String> = Vec::new();

    // Move both the current JSON cache and any legacy CSV cache so a switch
    // never strands one format under the wrong account.
    for ext in CURSOR_CACHE_EXTENSIONS {
        let active_file = cache_dir.join(format!("usage.{ext}"));
        let old_account_file = cache_dir.join(format!(
            "usage.{}.{ext}",
            sanitize_account_id_for_filename(old_account_id)
        ));
        let new_account_file = cache_dir.join(format!(
            "usage.{}.{ext}",
            sanitize_account_id_for_filename(new_account_id)
        ));

        if active_file.exists() {
            if old_account_file.exists() {
                let _ = archive_cache_file_in_dir(cache_dir, &old_account_file, old_account_id);
            }
            if let Err(err) = fs::rename(&active_file, &old_account_file) {
                failures.push(format!(
                    "could not move usage.{ext} to {}: {err}",
                    old_account_file.display()
                ));
                // `usage.{ext}` still holds the old account's data. Promoting the
                // new account's cache over it would destroy the old account's only
                // copy, so leave this extension untouched.
                continue;
            }
        }

        if new_account_file.exists() {
            if active_file.exists() {
                let _ = archive_cache_file_in_dir(cache_dir, &active_file, "usage.active");
            }
            if let Err(err) = fs::rename(&new_account_file, &active_file) {
                failures.push(format!(
                    "could not promote {} to usage.{ext}: {err}",
                    new_account_file.display()
                ));
            }
        }
    }

    if failures.is_empty() {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "failed to move Cursor cache files while switching from {} to {}: {}",
        old_account_id,
        new_account_id,
        failures.join("; ")
    ))
}

pub fn load_active_credentials() -> Option<CursorCredentials> {
    let store = load_credentials_store()?;
    store.accounts.get(&store.active_account_id).cloned()
}

pub fn has_active_credentials_in_home(home_dir: &Path) -> bool {
    load_credentials_store_from_home(home_dir)
        .and_then(|store| store.accounts.get(&store.active_account_id).cloned())
        .is_some()
}

fn is_cursor_usage_cache_filename(name: &str) -> bool {
    if name == "usage.csv" || name == "usage.json" {
        return true;
    }
    if name.starts_with("usage.backup") {
        return false;
    }
    let Some(stem) = name.strip_prefix("usage.").and_then(|rest| {
        rest.strip_suffix(".json")
            .or_else(|| rest.strip_suffix(".csv"))
    }) else {
        return false;
    };
    !stem.is_empty()
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

pub fn has_cursor_usage_cache_in_home(home_dir: &Path) -> bool {
    migrate_cache_dir_from_old_path_in_home(home_dir);
    let cache_dir = cursor_cache_dir(home_dir);
    if !cache_dir.exists() {
        return false;
    }

    match fs::read_dir(cache_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .any(|name| is_cursor_usage_cache_filename(&name)),
        Err(_) => false,
    }
}

pub fn has_cursor_usage_cache() -> bool {
    let home_dir = match home_dir() {
        Ok(home_dir) => home_dir,
        Err(_) => return false,
    };
    has_cursor_usage_cache_in_home(&home_dir)
}

fn expected_cursor_usage_cache_paths_in(home_dir: &Path) -> Vec<PathBuf> {
    let cache_dir = cursor_cache_dir(home_dir);

    if let Some(store) = load_credentials_store_from_home(home_dir) {
        if !store.accounts.is_empty() {
            let mut paths = store
                .accounts
                .keys()
                .map(|account_id| {
                    if account_id == &store.active_account_id {
                        cache_dir.join("usage.json")
                    } else {
                        cache_dir.join(format!(
                            "usage.{}.json",
                            sanitize_account_id_for_filename(account_id)
                        ))
                    }
                })
                .collect::<Vec<_>>();
            paths.sort_unstable();
            paths.dedup();
            return paths;
        }
    }

    vec![cache_dir.join("usage.json")]
}

fn cursor_usage_cache_file_is_fresh(path: &Path, max_age: Duration) -> bool {
    let Ok(mtime) = path.metadata().and_then(|meta| meta.modified()) else {
        return false;
    };
    match SystemTime::now().duration_since(mtime) {
        Ok(age) => age < max_age,
        // mtime is in the future (clock skew) — treat as fresh; a clock-skew
        // cache is no less authoritative than a freshly-fetched one, and we'd
        // rather not thrash the API while the system clock recovers.
        Err(_) => true,
    }
}

fn cursor_usage_cache_is_fresh_in(home_dir: &Path, max_age: Duration) -> bool {
    let cache_dir = cursor_cache_dir(home_dir);
    if !cache_dir.exists() {
        return false;
    }

    // The active account's cache is non-negotiable: if it is stale or missing,
    // implicit sync must run so reports read current data. A legacy `usage.csv`
    // with no `usage.json` counts as stale so the first run after upgrade
    // migrates the account to the JSON cache.
    let active_path = cache_dir.join("usage.json");
    if !cursor_usage_cache_file_is_fresh(&active_path, max_age) {
        return false;
    }

    // For secondaries, a fresh sync-attempt marker is sufficient. This avoids
    // forcing a sync on every invocation when a secondary account is
    // permanently stale (expired token, removed account, persistent API
    // failure). Without the marker, `.all(...)` would return `false` forever.
    let marker_fresh =
        cursor_usage_cache_file_is_fresh(&cache_dir.join(CURSOR_SYNC_ATTEMPT_MARKER), max_age);

    expected_cursor_usage_cache_paths_in(home_dir)
        .iter()
        .filter(|p| *p != &active_path)
        .all(|p| cursor_usage_cache_file_is_fresh(p, max_age) || marker_fresh)
}

/// True when the active cursor usage cache (`usage.json`) was refreshed within
/// `max_age` AND every secondary account cache is either fresh or a recent
/// sync-attempt marker exists. The active cache is unconditionally required —
/// a stale active means reports would show out-of-date data. Secondaries are
/// best-effort: when a secondary is permanently stale (expired token, removed
/// account, persistent API failure) the marker short-circuits the check so we
/// don't force an implicit sync on every invocation. Used by the implicit
/// pre-report sync path to avoid hitting the Cursor API on every invocation.
/// The manual `tokscale cursor sync` CLI bypasses this — explicit user intent
/// is always honored.
pub fn cursor_usage_cache_is_fresh(max_age: Duration) -> bool {
    let Ok(home_dir) = home_dir() else {
        return false;
    };
    cursor_usage_cache_is_fresh_in(&home_dir, max_age)
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
    let client = match build_cursor_http_client() {
        Ok(client) => client,
        Err(e) => {
            return ValidateSessionResult {
                valid: false,
                membership_type: None,
                error: Some(format!("Failed to build HTTP client: {}", e)),
            };
        }
    };
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

pub async fn fetch_cursor_usage_events_json(
    session_token: &str,
    timeout_override: Option<Duration>,
) -> Result<String> {
    fetch_cursor_usage_events_json_from(
        USAGE_EVENTS_JSON_ENDPOINT,
        session_token,
        timeout_override,
        CURSOR_MAX_JSON_BYTES,
        CURSOR_JSON_PAGE_SIZE,
    )
    .await
}

/// Body of [`fetch_cursor_usage_events_json`] with the endpoint, byte ceiling,
/// and page size injected so tests can drive the real paginating path against a
/// local server.
///
/// Walks pages of `get-filtered-usage-events` (a POST endpoint carrying an
/// `Origin` header for its CSRF check) until it has collected every event, then
/// returns the dashboard response shape with all pages' `usageEventsDisplay`
/// aggregated into one array. `max_body_bytes` is a cumulative ceiling spanning
/// every page, and a page that omits the `usageEventsDisplay` array is an error
/// so a malformed 200 never masquerades as an empty result.
async fn fetch_cursor_usage_events_json_from(
    url: &str,
    session_token: &str,
    timeout_override: Option<Duration>,
    max_body_bytes: usize,
    page_size: usize,
) -> Result<String> {
    let client = build_cursor_http_client()?;
    let mut all_events: Vec<serde_json::Value> = Vec::new();
    let mut total_count: Option<u64> = None;
    let mut bytes_read: usize = 0;

    // Overall wall-clock budget for the entire paginated walk. Without it the
    // per-page timeout would multiply across every page, so a slow server could
    // stall report startup (auto-sync runs first) for that timeout times the page
    // count. Each page's timeout is clamped to what remains of this budget below,
    // and the walk aborts once it is spent.
    let per_page_timeout = timeout_override.unwrap_or(CURSOR_HTTP_TIMEOUT);
    let fetch_deadline = Instant::now() + per_page_timeout;

    let mut completed = false;
    for page in 1..=CURSOR_MAX_JSON_PAGES {
        let remaining_budget = fetch_deadline.saturating_duration_since(Instant::now());
        if remaining_budget.is_zero() {
            anyhow::bail!(
                "Cursor usage events fetch exceeded its overall time budget before the full history was collected"
            );
        }

        let body = serde_json::json!({
            "teamId": 0,
            "page": page,
            "pageSize": page_size,
        });

        let req = client
            .post(url)
            .headers(build_cursor_json_headers(session_token))
            .json(&body)
            .timeout(per_page_timeout.min(remaining_budget));

        let response = req.send().await?;

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

        // The byte ceiling is cumulative across pages: each page may read only
        // what earlier pages left of the budget, so a paginated response can't
        // sidestep the cap by spreading a huge payload over many pages.
        let remaining = match max_body_bytes.checked_sub(bytes_read).filter(|r| *r > 0) {
            Some(remaining) => remaining,
            None => anyhow::bail!(
                "Cursor usage events JSON exceeded the {max_body_bytes} byte limit across pages"
            ),
        };
        let text = read_cursor_body_with_cap(response, remaining, "usage events JSON").await?;
        bytes_read += text.len();

        let page_value: serde_json::Value = serde_json::from_str(&text)
            .context("Invalid response from Cursor API - expected usage events JSON")?;

        if total_count.is_none() {
            total_count = page_value
                .get("totalUsageEventsCount")
                .and_then(|value| value.as_u64());
        }

        // A well-formed page always carries a `usageEventsDisplay` array. A 200
        // that omits it (a WAF challenge, an API change) is a failure, not a
        // silent zero-event result, so the caller never overwrites a good cache
        // with nothing.
        let page_events = match page_value
            .get("usageEventsDisplay")
            .map(|events| events.as_array())
        {
            Some(Some(events)) => events.clone(),
            _ => {
                anyhow::bail!("Invalid response from Cursor API - missing usageEventsDisplay array")
            }
        };
        let received = page_events.len();
        all_events.extend(page_events);

        // Once the advertised total is known, keep paging until it is reached so
        // a server that clamps `pageSize` (a short page while more events remain)
        // doesn't stop the walk early. An empty page before the total is reached
        // means the server truncated the history mid-walk, so it is an error
        // rather than a silently partial cache. Fall back to the short/empty-page
        // heuristic only when no total was reported.
        let done = match total_count {
            Some(total) => {
                if all_events.len() as u64 >= total {
                    true
                } else if received == 0 {
                    anyhow::bail!(
                        "Cursor API returned an empty page before its advertised total of {total} events; refusing to cache a partial history"
                    );
                } else {
                    false
                }
            }
            None => received == 0 || received < page_size,
        };
        if done {
            completed = true;
            break;
        }
    }

    // Exhausting the page cap without a clean stop means only part of the history
    // was collected; caching it would masquerade as a full sync, so fail instead.
    if !completed {
        anyhow::bail!(
            "Cursor usage events exceeded the {CURSOR_MAX_JSON_PAGES}-page fetch limit before the full history was collected; refusing to cache a partial history"
        );
    }

    let aggregated = serde_json::json!({
        "totalUsageEventsCount": all_events.len(),
        "usageEventsDisplay": all_events,
    });
    serde_json::to_string(&aggregated).context("Failed to serialize Cursor usage events cache")
}

/// Reads a Cursor response body (`label` names it for errors, e.g. "usage CSV"
/// or "usage events JSON"), never holding more than `max_body_bytes` of it.
///
/// `Content-Length` is consulted first so an oversized body is refused before a
/// single byte of it is read, but it is never the only check: the header is
/// optional, and a server is free to understate it. The loop below therefore
/// enforces the same ceiling on what actually arrives, aborting at the chunk
/// that would cross it instead of reading to the end and measuring afterwards.
///
/// `reqwest` is built here with `default-features = false`, so `bytes_stream()`
/// (feature `stream`) does not exist. `Response::chunk` is ungated and is the
/// same primitive `antigravity::read_reqwest_response_with_cap` uses for this
/// job. Decoding stays `from_utf8_lossy` because that is what `Response::text`
/// does without the `charset` feature — the bytes a valid export produces are
/// unchanged, and a malformed one still degrades the way it always did rather
/// than turning into a new error.
async fn read_cursor_body_with_cap(
    mut response: reqwest::Response,
    max_body_bytes: usize,
    label: &str,
) -> Result<String> {
    if let Some(advertised) = response.content_length() {
        if advertised > max_body_bytes as u64 {
            anyhow::bail!(
                "Cursor {label} is {advertised} bytes, over the {max_body_bytes} byte limit"
            );
        }
    }

    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .with_context(|| format!("Failed to read the Cursor {label} response"))?
    {
        let read_so_far = body.len().saturating_add(chunk.len());
        if read_so_far > max_body_bytes {
            anyhow::bail!(
                "Cursor {label} exceeds the {max_body_bytes} byte limit (aborted at {read_so_far} bytes)"
            );
        }
        body.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8_lossy(&body).into_owned())
}

async fn sync_cursor_cache_with_fetcher<F, Fut>(fetch_usage_json: F) -> SyncCursorResult
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

    sync_cursor_cache_with_fetcher_in_home(&home_dir, fetch_usage_json).await
}

async fn sync_cursor_cache_with_fetcher_in_home<F, Fut>(
    home_dir: &Path,
    fetch_usage_json: F,
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

    let mut total_rows = 0;
    let mut success_count = 0;
    let mut errors: Vec<String> = Vec::new();

    for (account_id, credentials) in &store.accounts {
        let is_active = account_id == &store.active_account_id;

        match fetch_usage_json(credentials.session_token.clone()).await {
            Ok(json_text) => {
                let file_path = if is_active {
                    cache_dir.join("usage.json")
                } else {
                    cache_dir.join(format!(
                        "usage.{}.json",
                        sanitize_account_id_for_filename(account_id)
                    ))
                };

                let event_count = count_cursor_json_events(&json_text);
                let legacy_csv = file_path.with_extension("csv");

                // A zero-event result is suspicious once cached history exists:
                // overwriting `usage.json` with nothing and archiving the legacy
                // CSV would strip real usage from reports. Keep both caches intact
                // and record it so the next sync can recover instead.
                if event_count == 0 && (file_path.exists() || legacy_csv.exists()) {
                    errors.push(format!(
                        "{}: sync returned zero events; keeping existing cache",
                        account_id
                    ));
                    continue;
                }

                if let Err(e) = atomic_write_file(&file_path, &json_text) {
                    errors.push(format!("{}: {}", account_id, e));
                } else {
                    total_rows += event_count;
                    success_count += 1;
                    // Archive (don't delete) the legacy CSV counterpart now that
                    // JSON is authoritative: pre-migration history survives and
                    // the scanner never parses both and double-counts this account.
                    if legacy_csv.exists() {
                        let label = legacy_csv
                            .file_stem()
                            .and_then(|stem| stem.to_str())
                            .unwrap_or("usage");
                        let _ = archive_cache_file_in_dir(&cache_dir, &legacy_csv, label);
                    }
                }
            }
            Err(e) => {
                errors.push(format!("{}: {}", account_id, e));
            }
        }
    }

    // Reconcile the active account's leftover per-account duplicate. The active
    // account is cached as `usage.json`; a `usage.<active_id>.json` copy left
    // over from when it was a secondary is only safe to drop once `usage.json`
    // actually exists on disk — whether this run just wrote it or a prior run
    // did. Gating on that existence keeps the duplicate when a failed first-time
    // fetch leaves no `usage.json` (it is then the only data we have), while a
    // failed fetch that still has a good `usage.json` clears the duplicate so the
    // scanner never reads both JSON caches and double-counts the active account.
    // The JSON dup is a stale copy (removed); the legacy CSV dup is archived so
    // pre-migration history survives.
    if cache_dir.join("usage.json").exists() {
        let active_sanitized = sanitize_account_id_for_filename(&store.active_account_id);
        let dup_json = cache_dir.join(format!("usage.{active_sanitized}.json"));
        if dup_json.exists() {
            let _ = fs::remove_file(&dup_json);
        }
        let dup_csv = cache_dir.join(format!("usage.{active_sanitized}.csv"));
        if dup_csv.exists() {
            let label = dup_csv
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("usage");
            let _ = archive_cache_file_in_dir(&cache_dir, &dup_csv, label);
        }
    }

    // Touch the sync-attempt marker unconditionally after the per-account loop
    // (regardless of partial failures). The marker's mtime short-circuits the
    // secondary-account freshness check so a permanently-stale secondary
    // doesn't force an implicit sync on every invocation. We ignore errors
    // here — if the marker can't be written (e.g. disk full) the gate simply
    // falls through to the cache-freshness check as before.
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(cache_dir.join(CURSOR_SYNC_ATTEMPT_MARKER));

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

/// Timeout override for the usage download: explicit syncs get the longer
/// [`CURSOR_EXPLICIT_SYNC_TIMEOUT`]; implicit syncs keep the client default.
fn sync_timeout_override(explicit: bool) -> Option<Duration> {
    explicit.then_some(CURSOR_EXPLICIT_SYNC_TIMEOUT)
}

pub async fn sync_cursor_cache(explicit: bool) -> SyncCursorResult {
    // Prefer a fresh token from the local Cursor desktop login when available.
    // This avoids stale manually-pasted cookies after Cursor refreshes its JWT.
    let _ = ensure_credentials_from_local_cursor();

    sync_cursor_cache_with_fetcher(move |session_token| async move {
        fetch_cursor_usage_events_json(&session_token, sync_timeout_override(explicit)).await
    })
    .await
}

fn archive_cache_file(file_path: &std::path::Path, label: &str) -> Result<()> {
    let cache_dir = get_cursor_cache_dir()?;
    archive_cache_file_in_dir(&cache_dir, file_path, label)
}

/// Moves `file_path` into `<cache_dir>/archive/` under a timestamped, sanitized
/// name so a legacy cache is preserved rather than deleted during migration.
///
/// Kept `cache_dir`-relative (rather than resolving the real cache dir itself)
/// so callers inside a synced-home flow archive into the same directory they are
/// operating on and tests can point it at a temporary home.
fn archive_cache_file_in_dir(
    cache_dir: &std::path::Path,
    file_path: &std::path::Path,
    label: &str,
) -> Result<()> {
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
    let ext = file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("csv");
    let dest = archive_dir.join(format!("{}-{}.{}", safe_label, ts, ext));
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

    // Prefer the local Cursor desktop accessToken (state.vscdb) when present.
    println!(
        "{}",
        "  Checking local Cursor desktop login...".bright_black()
    );
    let token = match read_local_cursor_session_token() {
        Ok(token) => {
            if let Some(user_id) = extract_user_id_from_session_token(&token) {
                println!(
                    "{}",
                    format!("  Found local Cursor session ({user_id}).").bright_black()
                );
            } else {
                println!("{}", "  Found local Cursor session.".bright_black());
            }
            token
        }
        Err(local_err) => {
            println!(
                "{}",
                format!("  Local desktop login unavailable: {local_err}").bright_black()
            );
            print!("  Enter Cursor WorkosCursorSessionToken value: ");
            std::io::stdout().flush()?;
            let pasted = rpassword::read_password().context("Failed to read session token")?;
            let pasted = pasted.trim().to_string();
            if pasted.is_empty() {
                println!("\n  {}\n", "No token provided.".yellow());
                return Ok(());
            }
            pasted
        }
    };

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
    let result = rt.block_on(sync_cursor_cache(true));

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

    /// #1250: cursor.com serves a bot-protection challenge instead of the API
    /// response when the ClientHello comes from rustls, so this one client has
    /// to be on the platform TLS stack while everything else stays on rustls
    /// via `tokscale_core::http`.
    ///
    /// reqwest calls its native-tls backend `TlsBackend::Default` and prints
    /// it as `Default`; `Rustls` is the other value this field can take. The
    /// field is only printed when both backends are compiled in, which is
    /// every target except Android — and Android is exactly where
    /// `.use_native_tls()` is cfg'd out.
    #[test]
    #[cfg(not(target_os = "android"))]
    fn the_cursor_client_is_built_on_the_native_tls_backend() {
        let rendered = format!("{:?}", cursor_http_client_builder());
        assert!(
            rendered.contains("tls_backend: Default"),
            "the Cursor client must use native TLS, not rustls, got: {rendered}"
        );
    }

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

    fn make_access_token_jwt(sub: &str) -> String {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!(r#"{{"sub":"{sub}"}}"#).as_bytes());
        format!("{header}.{payload}.sig")
    }

    #[test]
    fn test_session_token_from_access_token_builds_cookie_value() {
        let access = make_access_token_jwt("auth0|user_01ABCXYZ");
        let session = session_token_from_access_token(&access).unwrap();
        assert_eq!(session, format!("user_01ABCXYZ%3A%3A{access}"));
        assert_eq!(
            extract_user_id_from_session_token(&session),
            Some("user_01ABCXYZ".to_string())
        );
    }

    #[test]
    fn test_read_access_token_from_state_vscdb() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let db_path = temp_dir.path().join("state.vscdb");
        let access = make_access_token_jwt("auth0|user_LOCAL123");

        {
            let conn = rusqlite::Connection::open(&db_path)?;
            conn.execute(
                "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)",
                [],
            )?;
            conn.execute(
                "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
                rusqlite::params!["cursorAuth/accessToken", access],
            )?;
        }

        let read = read_access_token_from_state_vscdb(&db_path)?;
        assert_eq!(read, access);
        Ok(())
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
    fn test_build_cursor_http_client_applies_timeout() {
        // Constructing the client must succeed and surface no panics; the
        // configured timeout is the property the HIGH finding flagged.
        let client = build_cursor_http_client().expect("client builds");
        // reqwest::Client doesn't expose its timeout publicly, but we can at
        // least confirm the const wired into the builder is the documented
        // 15s value — a future change to the constant must be deliberate.
        assert_eq!(CURSOR_HTTP_TIMEOUT, std::time::Duration::from_secs(15));
        // Use the client briefly to ensure it's structurally valid.
        let _ = client.get("https://example.invalid").build();
    }

    #[test]
    fn test_sync_timeout_override_only_for_explicit_sync() {
        assert_eq!(sync_timeout_override(true), Some(Duration::from_secs(120)));
        assert_eq!(sync_timeout_override(false), None);
    }

    #[test]
    fn test_count_cursor_json_events() {
        let json = r#"{"usageEventsDisplay":[{"model":"a"},{"model":"b"}]}"#;
        assert_eq!(count_cursor_json_events(json), 2);
        assert_eq!(count_cursor_json_events(r#"{"usageEventsDisplay":[]}"#), 0);
        assert_eq!(count_cursor_json_events("{}"), 0);
        assert_eq!(count_cursor_json_events("not json"), 0);
    }

    /// Byte ceiling the usage-events download tests run against. Small on
    /// purpose: the production [`CURSOR_MAX_JSON_BYTES`] would have to be moved
    /// over a socket to reach it, which is exactly the allocation these tests
    /// exist to prove never happens.
    const TEST_JSON_CAP: usize = 64 * 1024;

    /// Minimal HTTP/1.1 server that serves a queue of usage-events pages.
    ///
    /// Each queued `(headers_extra, body)` is returned for one connection in
    /// order, so a test can drive the paginating fetcher across several pages.
    /// `headers_extra` is written verbatim after the status line so a test can
    /// advertise a `Content-Length` independently of what it actually sends. The
    /// captured request heads (returned via the shared handle) let a test assert
    /// the method, the `Origin` CSRF header, and the requested page numbers. The
    /// thread lingers briefly before dropping each socket so a client that
    /// rejects a response on its headers alone is not racing a FIN.
    fn serve_json_pages(
        pages: Vec<(String, Vec<u8>)>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let requests_thread = std::sync::Arc::clone(&requests);
        std::thread::spawn(move || {
            for (headers_extra, body) in pages {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = [0u8; 8192];
                let read = std::io::Read::read(&mut stream, &mut request).unwrap_or(0);
                requests_thread
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request[..read]).into_owned());
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{headers_extra}Connection: close\r\n\r\n"
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
                std::thread::sleep(Duration::from_millis(200));
            }
        });
        (
            format!("http://{addr}/api/dashboard/get-filtered-usage-events"),
            requests,
        )
    }

    fn json_page(total: u64, events: &[&str]) -> Vec<u8> {
        let events = events.join(",");
        format!(r#"{{"totalUsageEventsCount":{total},"usageEventsDisplay":[{events}]}}"#)
            .into_bytes()
    }

    fn json_event(conversation_id: &str, ts_ms: &str) -> String {
        format!(
            r#"{{"conversationId":"{conversation_id}","timestamp":"{ts_ms}","model":"gpt-5","chargedCents":1,"tokenUsage":{{"inputTokens":10,"outputTokens":5}}}}"#
        )
    }

    #[test]
    fn test_usage_events_json_over_the_cap_is_rejected_while_streaming() {
        // The body used to be read to the end, so a runaway response grew
        // process memory for the whole (now 120s) explicit-sync window before
        // anything looked at it. The cap must abort mid-stream instead.
        let mut body = b"{\"usageEventsDisplay\":[".to_vec();
        while body.len() < 4 * 1024 * 1024 {
            body.extend_from_slice(br#"{"conversationId":"x","timestamp":"1","model":"m"},"#);
        }
        let sent = body.len();
        // No Content-Length: the ceiling has to hold on what actually arrives.
        let (url, _requests) = serve_json_pages(vec![(String::new(), body)]);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                None,
                TEST_JSON_CAP,
                500,
            ))
            .expect_err("a body past the ceiling must not be buffered");
        let message = format!("{err:#}");

        assert!(
            message.contains(&TEST_JSON_CAP.to_string()),
            "the error must name the limit: {message}"
        );
        let aborted_at: usize = message
            .split("aborted at ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|count| count.parse().ok())
            .unwrap_or_else(|| panic!("the error must report where it stopped: {message}"));
        assert!(
            aborted_at < sent / 2,
            "the read must stop near the {TEST_JSON_CAP} byte ceiling rather than buffer all \
             {sent} bytes, but it held {aborted_at}"
        );
    }

    #[test]
    fn test_usage_events_json_over_advertised_content_length_is_rejected_before_reading() {
        // Headers only: the server promises half a gigabyte and sends nothing.
        // Surfacing the ceiling error proves the read never started.
        const ADVERTISED: usize = 512 * 1024 * 1024;
        let (url, _requests) = serve_json_pages(vec![(
            format!("Content-Length: {ADVERTISED}\r\n"),
            Vec::new(),
        )]);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                None,
                TEST_JSON_CAP,
                500,
            ))
            .expect_err("an oversized advertised length must be refused up front");
        let message = format!("{err:#}");

        assert!(
            message.contains(&ADVERTISED.to_string())
                && message.contains(&TEST_JSON_CAP.to_string()),
            "the error must name both the advertised size and the limit: {message}"
        );
    }

    #[test]
    fn test_usage_events_json_sends_origin_and_aggregates_single_page() {
        // A single short page (fewer than page_size events) stops after page 1.
        let body = json_page(
            2,
            &[
                &json_event("aaaa", "1788171994838"),
                &json_event("bbbb", "1788171000000"),
            ],
        );
        let (url, requests) =
            serve_json_pages(vec![(format!("Content-Length: {}\r\n", body.len()), body)]);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let text = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                sync_timeout_override(true),
                CURSOR_MAX_JSON_BYTES,
                500,
            ))
            .expect("a normal page must sync");

        assert_eq!(count_cursor_json_events(&text), 2);

        let captured = requests.lock().unwrap();
        assert_eq!(captured.len(), 1, "a short page must not request a second");
        let head = &captured[0];
        assert!(head.starts_with("POST "), "endpoint must be POSTed: {head}");
        assert!(
            head.to_lowercase().contains("origin: https://cursor.com"),
            "the CSRF Origin header must be sent: {head}"
        );
    }

    #[test]
    fn test_usage_events_json_walks_all_pages() {
        // page_size 2, total 5: two full pages then a short final page.
        let (url, requests) = serve_json_pages(vec![
            (
                String::new(),
                json_page(
                    5,
                    &[
                        &json_event("c1", "1788171994001"),
                        &json_event("c2", "1788171994002"),
                    ],
                ),
            ),
            (
                String::new(),
                json_page(
                    5,
                    &[
                        &json_event("c3", "1788171994003"),
                        &json_event("c4", "1788171994004"),
                    ],
                ),
            ),
            (
                String::new(),
                json_page(5, &[&json_event("c5", "1788171994005")]),
            ),
        ]);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let text = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                None,
                CURSOR_MAX_JSON_BYTES,
                2,
            ))
            .expect("pagination must collect every page");

        assert_eq!(count_cursor_json_events(&text), 5);

        let captured = requests.lock().unwrap();
        assert_eq!(captured.len(), 3, "must walk exactly three pages");
        assert!(captured[0].contains("\"page\":1"));
        assert!(captured[1].contains("\"page\":2"));
        assert!(captured[2].contains("\"page\":3"));
    }

    #[test]
    fn test_usage_events_json_keeps_paging_when_server_clamps_page_size() {
        // page_size 5 is requested, but the server clamps and returns a short
        // first page (2 of an advertised 3). The advertised total must win so the
        // walk continues instead of stopping on the short page and dropping rows.
        let (url, requests) = serve_json_pages(vec![
            (
                String::new(),
                json_page(
                    3,
                    &[
                        &json_event("c1", "1788171994001"),
                        &json_event("c2", "1788171994002"),
                    ],
                ),
            ),
            (
                String::new(),
                json_page(3, &[&json_event("c3", "1788171994003")]),
            ),
        ]);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let text = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                None,
                CURSOR_MAX_JSON_BYTES,
                5,
            ))
            .expect("a clamped short page must not stop pagination early");

        assert_eq!(count_cursor_json_events(&text), 3);
        assert_eq!(requests.lock().unwrap().len(), 2, "must walk both pages");
    }

    #[test]
    fn test_usage_events_json_missing_display_array_is_an_error() {
        // A 200 that omits the usageEventsDisplay array (a WAF challenge or API
        // change) must surface as an error, not a silent zero-event result, so a
        // good cache is never overwritten with nothing.
        let (url, _requests) = serve_json_pages(vec![(
            String::new(),
            br#"{"totalUsageEventsCount":0,"message":"blocked"}"#.to_vec(),
        )]);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                None,
                CURSOR_MAX_JSON_BYTES,
                500,
            ))
            .expect_err("a response without usageEventsDisplay must fail");
        assert!(
            format!("{err:#}").contains("usageEventsDisplay"),
            "the error must name the missing array: {err:#}"
        );
    }

    #[test]
    fn test_usage_events_json_empty_page_before_total_is_an_error() {
        // The server advertises 5 events but returns an empty second page before
        // the total is reached. That is a truncated history, not a clean end, so
        // it must fail rather than cache the two events as a full sync.
        let (url, _requests) = serve_json_pages(vec![
            (
                String::new(),
                json_page(
                    5,
                    &[
                        &json_event("c1", "1788171994001"),
                        &json_event("c2", "1788171994002"),
                    ],
                ),
            ),
            (String::new(), json_page(5, &[])),
        ]);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                None,
                CURSOR_MAX_JSON_BYTES,
                5,
            ))
            .expect_err("an empty page before the advertised total must fail");
        assert!(
            format!("{err:#}").contains("empty page before its advertised total"),
            "the error must explain the truncated history: {err:#}"
        );
    }

    #[test]
    fn test_usage_events_json_forbidden_surfaces_login_hint() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 4096];
                let _ = std::io::Read::read(&mut stream, &mut request);
                let _ = stream.write_all(
                    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                let _ = stream.flush();
                std::thread::sleep(Duration::from_millis(200));
            }
        });
        let url = format!("http://{addr}/api/dashboard/get-filtered-usage-events");

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                None,
                CURSOR_MAX_JSON_BYTES,
                500,
            ))
            .expect_err("a 403 must surface the re-auth hint");
        assert!(
            format!("{err:#}").contains("cursor login"),
            "the error should tell the user to re-authenticate: {err:#}"
        );
    }

    #[test]
    fn test_cursor_usage_cache_is_fresh_returns_false_when_cache_missing() {
        let temp = tempfile::tempdir().unwrap();
        // No cache dir created yet.
        assert!(!cursor_usage_cache_is_fresh_in(
            temp.path(),
            Duration::from_secs(300)
        ));
    }

    #[test]
    fn test_cursor_usage_cache_is_fresh_returns_false_when_no_csv_files() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = cursor_cache_dir(temp.path());
        fs::create_dir_all(&cache_dir).unwrap();
        // Unrelated file present, but no usage*.json.
        fs::write(cache_dir.join("README.txt"), "noise").unwrap();
        assert!(!cursor_usage_cache_is_fresh_in(
            temp.path(),
            Duration::from_secs(300)
        ));
    }

    #[test]
    fn test_cursor_usage_cache_is_fresh_returns_true_for_recent_file() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = cursor_cache_dir(temp.path());
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(cache_dir.join("usage.json"), "Date,Model\n").unwrap();
        // Just-written file is fresh under any reasonable window.
        assert!(cursor_usage_cache_is_fresh_in(
            temp.path(),
            Duration::from_secs(300)
        ));
    }

    #[test]
    fn test_cursor_usage_cache_is_fresh_returns_false_for_old_file() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = cursor_cache_dir(temp.path());
        fs::create_dir_all(&cache_dir).unwrap();
        let path = cache_dir.join("usage.json");
        fs::write(&path, "Date,Model\n").unwrap();
        // Backdate the mtime by an hour. Skip the test if the platform refuses
        // to set mtime (rare on POSIX/Windows but possible on exotic FS).
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let Ok(()) = f.set_modified(SystemTime::now() - Duration::from_secs(3600)) else {
            return;
        };
        drop(f);
        assert!(!cursor_usage_cache_is_fresh_in(
            temp.path(),
            Duration::from_secs(300)
        ));
    }

    #[test]
    fn test_cursor_usage_cache_is_fresh_requires_active_usage_csv_when_secondary_is_fresh() {
        // A recently-synced secondary account must not mask a stale active
        // account cache. The implicit sync gate should refresh the cache that
        // local reports read from `usage.json`.
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = cursor_cache_dir(temp.path());
        fs::create_dir_all(&cache_dir).unwrap();
        let stale_path = cache_dir.join("usage.json");
        fs::write(&stale_path, "Date,Model\n").unwrap();
        let stale = std::fs::OpenOptions::new()
            .write(true)
            .open(&stale_path)
            .unwrap();
        let Ok(()) = stale.set_modified(SystemTime::now() - Duration::from_secs(3600)) else {
            return;
        };
        drop(stale);
        // Secondary account written just now.
        fs::write(cache_dir.join("usage.team-a.json"), "Date,Model\n").unwrap();
        assert!(!cursor_usage_cache_is_fresh_in(
            temp.path(),
            Duration::from_secs(300)
        ));
    }

    #[test]
    fn test_cursor_usage_cache_is_fresh_returns_false_when_active_cache_missing() {
        // A fresh secondary account cache alone is not enough: without the
        // active account's `usage.json`, the next report would use stale/missing
        // active data unless the implicit sync runs.
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = cursor_cache_dir(temp.path());
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(cache_dir.join("usage.team-a.json"), "Date,Model\n").unwrap();
        assert!(!cursor_usage_cache_is_fresh_in(
            temp.path(),
            Duration::from_secs(300)
        ));
    }

    #[test]
    fn test_cursor_usage_cache_is_fresh_requires_all_expected_account_files() -> Result<()> {
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

        let cache_dir = cursor_cache_dir(temp_dir.path());
        fs::create_dir_all(&cache_dir)?;
        fs::write(cache_dir.join("usage.json"), "Date,Model\n")?;

        assert!(!cursor_usage_cache_is_fresh_in(
            temp_dir.path(),
            Duration::from_secs(300)
        ));

        fs::write(cache_dir.join("usage.team-account.json"), "Date,Model\n")?;
        assert!(cursor_usage_cache_is_fresh_in(
            temp_dir.path(),
            Duration::from_secs(300)
        ));

        Ok(())
    }

    #[test]
    fn test_cursor_expected_cache_paths_dedupes_sanitized_account_collisions() {
        let temp_dir = TempDir::new().unwrap();
        let mut accounts = HashMap::new();
        accounts.insert(
            "active-account".to_string(),
            CursorCredentials {
                session_token: "token-active".to_string(),
                user_id: Some("active-account".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: None,
                label: Some("active".to_string()),
            },
        );
        accounts.insert(
            "team/account-a".to_string(),
            CursorCredentials {
                session_token: "token-team-a".to_string(),
                user_id: Some("team/account-a".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: None,
                label: Some("team-a".to_string()),
            },
        );
        accounts.insert(
            "team@account-a".to_string(),
            CursorCredentials {
                session_token: "token-team-b".to_string(),
                user_id: Some("team@account-a".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: None,
                label: Some("team-b".to_string()),
            },
        );
        save_credentials_store_in_home(
            temp_dir.path(),
            &CursorCredentialsStore {
                version: 1,
                active_account_id: "active-account".to_string(),
                accounts,
            },
        )
        .unwrap();

        let paths = expected_cursor_usage_cache_paths_in(temp_dir.path());
        let cache_dir = cursor_cache_dir(temp_dir.path());
        let expected = vec![
            cache_dir.join("usage.json"),
            cache_dir.join("usage.team-account-a.json"),
        ];
        assert_eq!(paths, expected);
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

        // Seed legacy CSV caches from before the JSON switch; the sync must
        // replace them with JSON and archive (not delete) the stale CSVs.
        let cache_dir = cursor_cache_dir(temp_dir.path());
        fs::create_dir_all(&cache_dir)?;
        fs::write(cache_dir.join("usage.csv"), "Date,Model\nold\n")?;
        fs::write(
            cache_dir.join("usage.team-account.csv"),
            "Date,Model\nold\n",
        )?;

        let runtime = tokio::runtime::Runtime::new()?;
        let result = runtime.block_on(sync_cursor_cache_with_fetcher_in_home(
            temp_dir.path(),
            |session_token| {
                let json = match session_token.as_str() {
                    "token-active" => {
                        r#"{"usageEventsDisplay":[{"conversationId":"s1","timestamp":"1","model":"gpt-5","chargedCents":1}]}"#
                    }
                    "token-secondary" => {
                        r#"{"usageEventsDisplay":[{"conversationId":"s2","timestamp":"2","model":"gpt-5","chargedCents":2},{"conversationId":"s3","timestamp":"3","model":"gpt-5","chargedCents":3}]}"#
                    }
                    _ => r#"{"usageEventsDisplay":[]}"#,
                }
                .to_string();
                async move { Ok(json) }
            },
        ));

        assert!(result.synced);
        assert_eq!(result.rows, 3);
        assert_eq!(result.error, None);

        // JSON caches are written for each account and key by conversationId.
        assert_eq!(
            count_cursor_json_events(&fs::read_to_string(cache_dir.join("usage.json"))?),
            1
        );
        assert_eq!(
            count_cursor_json_events(&fs::read_to_string(
                cache_dir.join("usage.team-account.json")
            )?),
            2
        );
        // The active account is never duplicated as a per-account file.
        assert!(!cache_dir.join("usage.active-account.json").exists());
        // Legacy CSVs are moved out of the scan path so they can't double-count,
        // but they are archived rather than deleted so history survives.
        assert!(!cache_dir.join("usage.csv").exists());
        assert!(!cache_dir.join("usage.team-account.csv").exists());
        let archived: Vec<_> = fs::read_dir(cache_dir.join("archive"))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "csv"))
            .collect();
        assert_eq!(
            archived.len(),
            2,
            "both legacy CSVs must be archived, not deleted"
        );

        Ok(())
    }

    #[test]
    fn test_sync_keeps_existing_cache_when_fetch_returns_zero_events() -> Result<()> {
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
        save_credentials_store_in_home(
            temp_dir.path(),
            &CursorCredentialsStore {
                version: 1,
                active_account_id: "active-account".to_string(),
                accounts,
            },
        )?;

        // Seed a good existing JSON cache. A later sync that comes back empty
        // must not overwrite it with nothing.
        let cache_dir = cursor_cache_dir(temp_dir.path());
        fs::create_dir_all(&cache_dir)?;
        let seeded = r#"{"usageEventsDisplay":[{"conversationId":"keep","timestamp":"1","model":"gpt-5","chargedCents":1}]}"#;
        fs::write(cache_dir.join("usage.json"), seeded)?;

        let runtime = tokio::runtime::Runtime::new()?;
        let result = runtime.block_on(sync_cursor_cache_with_fetcher_in_home(
            temp_dir.path(),
            |_session_token| async move { Ok(r#"{"usageEventsDisplay":[]}"#.to_string()) },
        ));

        // The empty result is refused; the seeded cache is left untouched.
        assert!(!result.synced);
        assert_eq!(
            fs::read_to_string(cache_dir.join("usage.json"))?,
            seeded,
            "a zero-event sync must not clobber existing cached usage"
        );

        Ok(())
    }

    #[test]
    fn test_sync_clears_active_duplicate_even_when_fetch_fails() -> Result<()> {
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
        save_credentials_store_in_home(
            temp_dir.path(),
            &CursorCredentialsStore {
                version: 1,
                active_account_id: "active-account".to_string(),
                accounts,
            },
        )?;

        // A good `usage.json` already exists alongside a stale per-account
        // duplicate left over from when this account was a secondary. If the
        // fetch fails, both JSON caches would otherwise remain and the scanner
        // would double-count the active account.
        let cache_dir = cursor_cache_dir(temp_dir.path());
        fs::create_dir_all(&cache_dir)?;
        let good = r#"{"usageEventsDisplay":[{"conversationId":"keep","timestamp":"1","model":"gpt-5","chargedCents":1}]}"#;
        fs::write(cache_dir.join("usage.json"), good)?;
        fs::write(cache_dir.join("usage.active-account.json"), good)?;

        let runtime = tokio::runtime::Runtime::new()?;
        let result = runtime.block_on(sync_cursor_cache_with_fetcher_in_home(
            temp_dir.path(),
            |_session_token| async move { Err(anyhow::anyhow!("network down")) },
        ));

        // The fetch failed, but the good cache survives and the duplicate is
        // reconciled so the scanner reads only one JSON cache for the account.
        assert!(!result.synced);
        assert_eq!(
            fs::read_to_string(cache_dir.join("usage.json"))?,
            good,
            "a failed fetch must not clobber the existing active cache"
        );
        assert!(
            !cache_dir.join("usage.active-account.json").exists(),
            "the stale active duplicate must be cleared so it can't double-count"
        );

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

    /// Helper: build a two-account credentials store in `home_dir`.
    fn setup_two_account_store(home_dir: &std::path::Path) -> Result<()> {
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
            home_dir,
            &CursorCredentialsStore {
                version: 1,
                active_account_id: "active-account".to_string(),
                accounts,
            },
        )
    }

    /// Helper: backdate a file's mtime by `secs` seconds. Returns `false` if
    /// the platform refuses to set mtime (exotic FS), signalling the caller to
    /// skip the test.
    fn backdate_file(path: &std::path::Path, secs: u64) -> bool {
        let f = match std::fs::OpenOptions::new().write(true).open(path) {
            Ok(f) => f,
            Err(_) => return false,
        };
        f.set_modified(SystemTime::now() - Duration::from_secs(secs))
            .is_ok()
    }

    #[test]
    fn test_freshness_gate_passes_when_active_fresh_and_marker_fresh_despite_stale_secondary(
    ) -> Result<()> {
        // Active CSV fresh + stale secondary CSV + fresh marker → gate passes.
        // This is the key scenario: a permanently-stale secondary must not
        // thrash implicit sync when the marker proves we already tried recently.
        let temp_dir = TempDir::new()?;
        setup_two_account_store(temp_dir.path())?;

        let cache_dir = cursor_cache_dir(temp_dir.path());
        fs::create_dir_all(&cache_dir)?;

        // Fresh active cache.
        fs::write(cache_dir.join("usage.json"), "Date,Model\n")?;

        // Stale secondary cache.
        let secondary = cache_dir.join("usage.team-account.json");
        fs::write(&secondary, "Date,Model\n")?;
        if !backdate_file(&secondary, 3600) {
            return Ok(()); // platform can't set mtime — skip
        }

        // Fresh sync-attempt marker.
        fs::write(cache_dir.join(CURSOR_SYNC_ATTEMPT_MARKER), "")?;

        assert!(
            cursor_usage_cache_is_fresh_in(temp_dir.path(), Duration::from_secs(300)),
            "fresh marker should short-circuit stale secondary"
        );
        Ok(())
    }

    #[test]
    fn test_freshness_gate_fails_when_active_fresh_but_no_marker_and_stale_secondary() -> Result<()>
    {
        // Active CSV fresh + stale secondary CSV + NO marker → gate fails so
        // an implicit sync is triggered to try fetching the secondary again.
        let temp_dir = TempDir::new()?;
        setup_two_account_store(temp_dir.path())?;

        let cache_dir = cursor_cache_dir(temp_dir.path());
        fs::create_dir_all(&cache_dir)?;

        fs::write(cache_dir.join("usage.json"), "Date,Model\n")?;

        let secondary = cache_dir.join("usage.team-account.json");
        fs::write(&secondary, "Date,Model\n")?;
        if !backdate_file(&secondary, 3600) {
            return Ok(());
        }

        // No marker written.

        assert!(
            !cursor_usage_cache_is_fresh_in(temp_dir.path(), Duration::from_secs(300)),
            "without marker, stale secondary should trigger sync"
        );
        Ok(())
    }

    #[test]
    fn test_freshness_gate_fails_when_active_stale_even_with_fresh_marker() -> Result<()> {
        // Stale active CSV + fresh marker → gate still fails. The marker must
        // never mask a stale active cache — the active data is what reports
        // read from.
        let temp_dir = TempDir::new()?;
        setup_two_account_store(temp_dir.path())?;

        let cache_dir = cursor_cache_dir(temp_dir.path());
        fs::create_dir_all(&cache_dir)?;

        // Stale active cache.
        let active = cache_dir.join("usage.json");
        fs::write(&active, "Date,Model\n")?;
        if !backdate_file(&active, 3600) {
            return Ok(());
        }

        // Fresh secondary and fresh marker.
        fs::write(cache_dir.join("usage.team-account.json"), "Date,Model\n")?;
        fs::write(cache_dir.join(CURSOR_SYNC_ATTEMPT_MARKER), "")?;

        assert!(
            !cursor_usage_cache_is_fresh_in(temp_dir.path(), Duration::from_secs(300)),
            "stale active cache must always trigger sync regardless of marker"
        );
        Ok(())
    }

    #[test]
    fn test_sync_writes_attempt_marker() -> Result<()> {
        // After sync_cursor_cache_with_fetcher_in_home completes (even with a
        // partial failure), the marker file must exist in the cache dir.
        let temp_dir = TempDir::new()?;
        setup_two_account_store(temp_dir.path())?;

        let runtime = tokio::runtime::Runtime::new()?;
        let _result = runtime.block_on(sync_cursor_cache_with_fetcher_in_home(
            temp_dir.path(),
            |session_token| {
                // Secondary deliberately fails to simulate a broken account.
                let result: Result<String> = match session_token.as_str() {
                    "token-active" => Ok("Date,Model,Tokens\n2026-01-01,gpt-5,10\n".to_string()),
                    _ => Err(anyhow::anyhow!("simulated fetch failure")),
                };
                async move { result }
            },
        ));

        let cache_dir = cursor_cache_dir(temp_dir.path());
        assert!(
            cache_dir.join(CURSOR_SYNC_ATTEMPT_MARKER).exists(),
            "marker must be written even when a secondary account fetch fails"
        );
        Ok(())
    }

    /// A switch has to move the JSON cache and the legacy CSV cache together:
    /// the previous account's data ends up under its own name and the incoming
    /// account's cache is promoted to `usage.<ext>`.
    #[test]
    fn reconcile_cache_files_moves_both_extensions() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let cache_dir = temp_dir.path().join("cache");
        fs::create_dir_all(&cache_dir)?;

        let old_id = "old-account";
        let new_id = "new-account";
        let old_stem = sanitize_account_id_for_filename(old_id);
        let new_stem = sanitize_account_id_for_filename(new_id);

        for ext in CURSOR_CACHE_EXTENSIONS {
            fs::write(cache_dir.join(format!("usage.{ext}")), format!("old-{ext}"))?;
            fs::write(
                cache_dir.join(format!("usage.{new_stem}.{ext}")),
                format!("new-{ext}"),
            )?;
        }

        reconcile_cache_files_in_dir(&cache_dir, old_id, new_id)?;

        for ext in CURSOR_CACHE_EXTENSIONS {
            assert_eq!(
                fs::read_to_string(cache_dir.join(format!("usage.{ext}")))?,
                format!("new-{ext}"),
                "the incoming account's cache must become usage.{ext}"
            );
            assert_eq!(
                fs::read_to_string(cache_dir.join(format!("usage.{old_stem}.{ext}")))?,
                format!("old-{ext}"),
                "the previous account's cache must be filed under its own id"
            );
            assert!(
                !cache_dir.join(format!("usage.{new_stem}.{ext}")).exists(),
                "the promoted cache must not be left behind as a duplicate"
            );
        }
        Ok(())
    }

    /// #1247 follow-up: a failed cache move used to be discarded with `let _`, so
    /// the switch was recorded anyway and the next sync filed one account's usage
    /// under the other. Every extension must be attempted, and every failure must
    /// reach the caller.
    #[test]
    fn reconcile_cache_files_reports_every_failed_extension() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let cache_dir = temp_dir.path().join("cache");
        fs::create_dir_all(&cache_dir)?;

        let old_id = "old-account";
        let new_id = "new-account";
        let old_stem = sanitize_account_id_for_filename(old_id);

        // A regular file named `archive` makes `archive_cache_file_in_dir` fail,
        // so the blocking entries below are not archived out of the way first.
        fs::write(cache_dir.join("archive"), b"not a directory")?;

        for ext in CURSOR_CACHE_EXTENSIONS {
            fs::write(
                cache_dir.join(format!("usage.{ext}")),
                b"active-account-data",
            )?;
            // Renaming a file onto a non-empty directory fails on every platform.
            let blocked = cache_dir.join(format!("usage.{old_stem}.{ext}"));
            fs::create_dir_all(&blocked)?;
            fs::write(blocked.join("occupied"), b"x")?;
        }

        let err = reconcile_cache_files_in_dir(&cache_dir, old_id, new_id)
            .expect_err("a blocked rename must be reported, not discarded");
        let message = err.to_string();

        for ext in CURSOR_CACHE_EXTENSIONS {
            assert!(
                message.contains(&format!("usage.{ext}")),
                "every extension must be attempted and reported, got: {message}"
            );
            assert_eq!(
                fs::read(cache_dir.join(format!("usage.{ext}")))?,
                b"active-account-data",
                "a failed move must leave usage.{ext} in place"
            );
        }
        Ok(())
    }

    /// Failing to move the active cache out of the way must not fall through to
    /// promoting the incoming account's cache: `usage.<ext>` still holds the
    /// previous account's only copy at that point.
    #[test]
    fn reconcile_cache_files_does_not_promote_over_a_stuck_active_cache() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let cache_dir = temp_dir.path().join("cache");
        fs::create_dir_all(&cache_dir)?;

        let old_id = "old-account";
        let new_id = "new-account";
        let old_stem = sanitize_account_id_for_filename(old_id);
        let new_stem = sanitize_account_id_for_filename(new_id);

        fs::write(cache_dir.join("archive"), b"not a directory")?;
        fs::write(cache_dir.join("usage.json"), b"old-account-data")?;

        let blocked = cache_dir.join(format!("usage.{old_stem}.json"));
        fs::create_dir_all(&blocked)?;
        fs::write(blocked.join("occupied"), b"x")?;

        let incoming = cache_dir.join(format!("usage.{new_stem}.json"));
        fs::write(&incoming, b"new-account-data")?;

        reconcile_cache_files_in_dir(&cache_dir, old_id, new_id)
            .expect_err("the blocked rename must be reported");

        assert_eq!(
            fs::read(cache_dir.join("usage.json"))?,
            b"old-account-data",
            "the previous account's cache must survive a failed switch"
        );
        assert_eq!(
            fs::read(&incoming)?,
            b"new-account-data",
            "the incoming cache must stay under its own account id"
        );
        Ok(())
    }
}
