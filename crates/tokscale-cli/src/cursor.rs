use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Timeout for every Cursor HTTP request, and the wall-clock budget for an
/// implicit pre-report sync. A cached account downloads new events plus a
/// bounded correction overlap from its recent history. If the network is
/// hung, the report proceeds against cached data after this timeout instead of
/// stalling forever. Explicit `tokscale cursor sync` overrides it with
/// [`CURSOR_EXPLICIT_SYNC_TIMEOUT`].
const CURSOR_HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Re-fetch one hour before each account's newest cached event for delayed
/// billing corrections. Older cached history is retained, but corrections
/// outside this bounded window are not refreshed by incremental sync.
const CURSOR_USAGE_CORRECTION_WINDOW_MS: i64 = 60 * 60 * 1000;
// Allow small clock skew, but keep it below the correction overlap so an
// accepted future row cannot move the next window beyond current usage.
const CURSOR_USAGE_CLOCK_SKEW_MS: i64 = 5 * 60 * 1000;

/// Largest explicit sync budget a user can configure, through
/// `tokscale cursor sync --timeout` or `cursor.syncTimeoutSeconds` in
/// settings.json. Twenty-four hours is far beyond any real history walk.
/// Keeping a bound also keeps the `Instant` deadline arithmetic in range: an
/// unbounded 19-digit `--timeout` used to panic with "overflow when adding
/// duration to instant".
pub const CURSOR_MAX_SYNC_TIMEOUT_SECS: u64 = 24 * 60 * 60;

/// Per-request timeout for the usage download during an explicit
/// `tokscale cursor sync`. Large accounts take well over the default
/// [`CURSOR_HTTP_TIMEOUT`] to page (issue #1175); the user asked for the sync,
/// so waiting longer beats failing fast. When this default is in use, the walk
/// may expand it from the advertised event count up to 30 minutes.
pub const CURSOR_EXPLICIT_SYNC_TIMEOUT: Duration = Duration::from_secs(120);

/// Seconds allocated per page when dynamically scaling the explicit sync budget
/// to the advertised `totalUsageEventsCount`.
const CURSOR_DYNAMIC_BUDGET_SECS_PER_PAGE: u64 = 3;

/// Maximum duration dynamic scaling may expand the explicit sync budget to.
const CURSOR_MAX_DYNAMIC_SYNC_TIMEOUT: Duration = Duration::from_secs(1800);

/// Compute the dynamic sync timeout budget for a Cursor usage events walk.
///
/// Accounts with tens of thousands of rows require multiple paginated requests.
/// When no explicit custom timeout is given, Tokscale dynamically scales the budget
/// based on the advertised total count and the observed page yield, allocating
/// 3 seconds per estimated page (clamped between the initial budget and 1,800s).
pub fn compute_dynamic_cursor_budget(
    total_events: u64,
    effective_page_size: usize,
    initial_budget: Duration,
) -> Duration {
    if effective_page_size == 0 {
        return initial_budget;
    }
    let total_pages = (total_events.saturating_add(effective_page_size as u64 - 1))
        / (effective_page_size as u64);
    let estimated_needed =
        Duration::from_secs(total_pages.saturating_mul(CURSOR_DYNAMIC_BUDGET_SECS_PER_PAGE));
    estimated_needed
        .max(initial_budget)
        .min(CURSOR_MAX_DYNAMIC_SYNC_TIMEOUT)
}

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

fn cursor_desktop_auto_import_disabled_path(home_dir: &Path) -> PathBuf {
    home_dir.join(".config/tokscale/cursor-desktop-auto-import.disabled")
}

fn cursor_desktop_auto_import_is_disabled_in_home(home_dir: &Path) -> bool {
    cursor_desktop_auto_import_disabled_path(home_dir).is_file()
}

fn disable_cursor_desktop_auto_import_in_home(home_dir: &Path) -> Result<()> {
    atomic_write_file(
        &cursor_desktop_auto_import_disabled_path(home_dir),
        "disabled by explicit Cursor logout\n",
    )
}

fn clear_cursor_desktop_auto_import_in_home(home_dir: &Path) -> Result<()> {
    let path = cursor_desktop_auto_import_disabled_path(home_dir);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
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

/// Page ceiling for one fetch at the requested page size, so a server that
/// keeps returning full pages (or a mis-reported total) can't spin forever.
/// At the page size above this admits up to a quarter-million events.
const CURSOR_MAX_JSON_PAGES: usize = 500;

/// The event budget behind [`CURSOR_MAX_JSON_PAGES`]. When Cursor clamps the
/// requested page size (it has been seen returning 100 rows for a 500-row
/// request), a fixed page ceiling would quietly shrink this budget fivefold and
/// fail every history above ~50,000 events, so the page ceiling is derived from
/// the page size the server actually honours instead.
const CURSOR_MAX_JSON_EVENTS: usize = CURSOR_JSON_PAGE_SIZE * CURSOR_MAX_JSON_PAGES;

/// Smallest page yield the derived ceiling is sized for, which bounds the
/// walk at `CURSOR_MAX_JSON_EVENTS / 50` = 5,000 pages however hard a server
/// clamps. The byte cap and the time budget still bound the walk as well.
const CURSOR_MIN_PAGE_YIELD_FOR_LIMIT: usize = 50;

/// How many pages one fetch may walk, given the largest page the server has
/// returned so far. Pages are full except the last, so the largest page seen
/// is the page size the server honours. The result never drops below
/// [`CURSOR_MAX_JSON_PAGES`].
fn cursor_page_limit(largest_page_yield: usize) -> usize {
    if largest_page_yield == 0 {
        return CURSOR_MAX_JSON_PAGES;
    }
    let honoured = largest_page_yield.max(CURSOR_MIN_PAGE_YIELD_FOR_LIMIT);
    CURSOR_MAX_JSON_EVENTS
        .div_ceil(honoured)
        .max(CURSOR_MAX_JSON_PAGES)
}

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

/// Account-scoped marker written at the end of every `sync_cursor_cache` run
/// (even when some accounts fail). Its mtime bounds retry suppression, while
/// the hashed account list prevents one account's attempt from suppressing a
/// newly-added or newly-active account.
const CURSOR_SYNC_ATTEMPT_MARKER: &str = "usage.last-sync-attempt";
const CURSOR_SYNC_ATTEMPT_MARKER_VERSION: u32 = 1;

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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorSyncAttemptMarker {
    version: u32,
    account_hashes: Vec<String>,
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

/// Events already stored in an aggregated usage-events cache. Invalid JSON or a
/// missing `usageEventsDisplay` array is an empty history, so a corrupt cache
/// cannot watermark an incremental sync.
fn cached_usage_events(json_text: Option<&str>) -> Vec<serde_json::Value> {
    let Some(json_text) = json_text else {
        return Vec::new();
    };
    serde_json::from_str::<serde_json::Value>(json_text)
        .ok()
        .and_then(|value| {
            value
                .get("usageEventsDisplay")
                .and_then(|events| events.as_array())
                .cloned()
        })
        .unwrap_or_default()
}

/// Only report-usable rows with plausible positive timestamps can advance sync
/// progress or establish pagination ordering. Other rows remain in the raw cache.
fn usable_cursor_event_timestamp_ms(event: &serde_json::Value, max_timestamp: i64) -> Option<i64> {
    tokscale_core::sessions::cursor::parse_cursor_usage_event_timestamp(event)
        .filter(|timestamp| *timestamp > 0 && *timestamp <= max_timestamp)
}

/// Newest usable timestamp in a cache. No usable timestamp forces a full fetch.
fn usage_events_watermark_ms(events: &[serde_json::Value], max_timestamp: i64) -> Option<i64> {
    events
        .iter()
        .filter_map(|event| usable_cursor_event_timestamp_ms(event, max_timestamp))
        .max()
}

/// Identity of a usage event for incremental merge.
///
/// Accepted rows use the report parser's projection of every immutable known
/// field: trimmed model/session ids (one missing-id fallback), integer timestamp
/// and defaulted/coerced/clamped token counts. Mutable billing and unknown fields
/// are omitted so a re-fetched correction replaces its cached row; the multiset
/// merge separately preserves repeated events with the same projection. Rejected
/// rows retain their raw identity; only object key ordering is canonicalized.
fn cursor_usage_event_identity(event: &serde_json::Value) -> String {
    let mut identity = if let Some(parsed) =
        tokscale_core::sessions::cursor::parse_cursor_usage_event_identity(event)
    {
        // Restrict accepted rows to the report parser's immutable projection.
        // Cursor exposes no stable event id, so multiplicity is tracked by the
        // merge rather than by unknown fields that billing corrections may change.
        serde_json::json!({
            "conversationId": parsed.conversation_id,
            "model": parsed.model,
            "timestamp": parsed.timestamp,
            "tokenUsage": {
                "inputTokens": parsed.tokens.input,
                "outputTokens": parsed.tokens.output,
                "cacheReadTokens": parsed.tokens.cache_read,
                "cacheWriteTokens": parsed.tokens.cache_write,
            },
        })
    } else {
        // Rejected rows cannot be projected safely. Keep their complete raw
        // identity so unrelated malformed data is never discarded.
        event.clone()
    };
    identity.sort_all_objects();
    serde_json::to_string(&identity).unwrap_or_default()
}

/// Reconcile `incoming` (just fetched, newest first) with `existing` (the cache).
/// Incoming is authoritative for every identity present in the completed overlap,
/// so all cached occurrences of those identities are replaced. Every incoming
/// occurrence is retained because Cursor has no stable event id and distinct
/// billable events may have identical parser fields. Cached identities the new
/// pages never reached stay in the result.
fn merge_cursor_usage_events(
    existing: Vec<serde_json::Value>,
    incoming: Vec<serde_json::Value>,
) -> Vec<serde_json::Value> {
    let capacity = existing.len().saturating_add(incoming.len());
    let mut incoming_identities = HashSet::with_capacity(incoming.len());
    let mut merged = Vec::with_capacity(capacity);
    for event in incoming {
        incoming_identities.insert(cursor_usage_event_identity(&event));
        merged.push(event);
    }

    for event in existing {
        if !incoming_identities.contains(&cursor_usage_event_identity(&event)) {
            merged.push(event);
        }
    }
    merged
}

/// Keep recent rows and raw rows that cannot establish ordering. Only a usable,
/// plausible older row can end the bounded walk through the correction window.
fn filter_cursor_usage_page(
    events: &[serde_json::Value],
    sync_start: Option<i64>,
    max_timestamp: i64,
) -> (Vec<&serde_json::Value>, bool) {
    let Some(sync_start) = sync_start else {
        return (events.iter().collect(), false);
    };
    let mut crossed = false;
    let fresh = events
        .iter()
        .filter(|event| {
            if usable_cursor_event_timestamp_ms(event, max_timestamp)
                .is_some_and(|timestamp| timestamp < sync_start)
            {
                crossed = true;
                false
            } else {
                true
            }
        })
        .collect();
    (fresh, crossed)
}

fn usage_events_request_body(
    page: usize,
    page_size: usize,
    sync_start: Option<i64>,
    end_ms: i64,
) -> serde_json::Value {
    // An explicit window is required even for a cold cache. teamId: 0 is a
    // filter rather than "all teams" and hides personal-account events.
    serde_json::json!({
        "startDate": sync_start.unwrap_or(0).to_string(),
        "endDate": end_ms.to_string(),
        "page": page,
        "pageSize": page_size,
    })
}

fn cursor_budget_label(budget: Duration) -> String {
    if budget.as_secs() >= 1 {
        format!("{}s", budget.as_secs())
    } else {
        format!("{}ms", budget.as_millis())
    }
}

fn cursor_fetch_progress(
    pages_completed: usize,
    events_collected: usize,
    total: Option<u64>,
) -> String {
    let pages_word = if pages_completed == 1 {
        "page"
    } else {
        "pages"
    };
    let events = match total {
        Some(total) => format!("{events_collected}/{total} events"),
        None => format!("{events_collected} events"),
    };
    format!("{pages_completed} {pages_word}, {events} collected")
}

fn cursor_with_cause(message: String, cause: &str) -> String {
    if cause.is_empty() {
        message
    } else {
        format!("{message}: {cause}")
    }
}

/// What to print when the paginated walk runs out of its wall-clock budget.
/// `reqwest`'s own `Display` is only "error sending request for url (...)",
/// which is how a budget expiry was reported as a network failure.
fn cursor_budget_exhausted_message(
    budget: Duration,
    pages_completed: usize,
    events_collected: usize,
    total: Option<u64>,
    cause: &str,
) -> String {
    cursor_with_cause(
        format!(
            "Cursor usage events fetch exceeded its {} time budget ({})",
            cursor_budget_label(budget),
            cursor_fetch_progress(pages_completed, events_collected, total)
        ),
        cause,
    )
}

/// A single page hit its request timeout while the overall walk budget remains.
/// Naming `page_timeout` here keeps a scaled explicit sync from claiming the
/// expanded overall budget was used up.
fn cursor_request_timeout_message(
    page_timeout: Duration,
    pages_completed: usize,
    events_collected: usize,
    total: Option<u64>,
    cause: &str,
) -> String {
    cursor_with_cause(
        format!(
            "Cursor usage events request timed out after its {} time budget ({})",
            cursor_budget_label(page_timeout),
            cursor_fetch_progress(pages_completed, events_collected, total)
        ),
        cause,
    )
}

fn cursor_sync_error_text(err: &anyhow::Error) -> String {
    tokscale_core::pricing::describe_error(err.as_ref())
}

fn error_is_reqwest_timeout(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|reqwest_err| reqwest_err.is_timeout())
    })
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
        if let Some(config_dir) = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
        {
            paths.push(config_dir.join("Cursor/User/globalStorage/state.vscdb"));
        }
        paths.push(home_dir.join(".config/Cursor/User/globalStorage/state.vscdb"));
    }

    paths.dedup();
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
    upsert_credentials_in_home(&home_dir()?, token, label)
}

fn upsert_credentials_in_home(home: &Path, token: &str, label: Option<&str>) -> Result<String> {
    let account_id = derive_account_id(token);
    let user_id = extract_user_id_from_session_token(token);

    let mut store =
        load_credentials_store_from_home(home).unwrap_or_else(|| CursorCredentialsStore {
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
    let reconciled = prepare_cursor_cache_layout(&cursor_cache_dir(home), &store).and_then(|()| {
        if old_active_id != account_id {
            reconcile_cache_files_in_dir(&cursor_cache_dir(home), &old_active_id, &account_id)
        } else {
            Ok(())
        }
    });

    // A freshly read desktop token is worth keeping even when the cache files
    // could not be moved, so the accounts are saved either way. Only point
    // `active_account_id` at the new account once its cache really is
    // `usage.json`.
    if reconciled.is_ok() {
        store.active_account_id = account_id.clone();
    }
    save_credentials_store_in_home(home, &store)?;
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

/// Hash the exact ID, including case and whitespace. Sanitized names cannot
/// distinguish accounts and are retained only for human-readable archive labels.
fn cursor_account_cache_stem(account_id: &str) -> String {
    format!("account-{:x}", Sha256::digest(account_id.as_bytes()))
}

#[cfg(test)]
#[derive(Deserialize)]
struct CursorCacheOwnership {
    #[serde(rename = "_tokscale")]
    metadata: Option<CursorCacheOwner>,
}

#[cfg(test)]
#[derive(Deserialize)]
struct CursorCacheOwner {
    version: u32,
    #[serde(rename = "accountId")]
    account_id: String,
}

#[cfg(test)]
fn cursor_cache_owner(contents: &str) -> Option<String> {
    let value: CursorCacheOwnership = serde_json::from_str(contents).ok()?;
    let metadata = value.metadata?;
    if metadata.version != 1 {
        return None;
    }
    Some(metadata.account_id)
}

fn legacy_cursor_cache_owner(contents: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(contents).ok()?;
    let object = value.as_object()?;
    if object.contains_key("_owner") || object.contains_key("zzTokscaleOwner") {
        return None;
    }
    let metadata = object.get("_tokscale")?.as_object()?;
    if metadata.get("version")?.as_u64()? != 1 {
        return None;
    }
    metadata.get("accountId")?.as_str().map(str::to_string)
}

/// New caches carry a fixed-size owner hash and random generation at the start,
/// plus the same generation at the end. Reading both bounded regions verifies
/// ownership and catches truncated or mismatched cache moves without traversing
/// a potentially 64 MiB usage array. Older caches fall back to one full parse
/// until the next successful sync writes the envelope.
#[derive(Deserialize)]
struct CursorCacheOwnerHeader {
    version: u32,
    #[serde(rename = "ownerHash")]
    owner_hash: String,
    generation: String,
}

#[derive(Deserialize)]
struct CursorCacheOwnerFooter {
    version: u32,
    generation: String,
}

const CURSOR_CACHE_OWNER_ENVELOPE_BYTES: usize = 1024;
const CURSOR_CACHE_OWNER_HEADER_PREFIX: &[u8] = br#"{"_owner":"#;
const CURSOR_CACHE_OWNER_FOOTER_PREFIX: &[u8] = br#","zzTokscaleOwner":"#;

fn cursor_cache_owner_hash(account_id: &str) -> String {
    format!("{:x}", Sha256::digest(account_id.as_bytes()))
}

fn cursor_cache_owner_hash_from_envelope(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();
    let mut prefix = Vec::with_capacity(CURSOR_CACHE_OWNER_ENVELOPE_BYTES);
    (&mut file)
        .take(CURSOR_CACHE_OWNER_ENVELOPE_BYTES as u64)
        .read_to_end(&mut prefix)
        .ok()?;
    let header_json = prefix.strip_prefix(CURSOR_CACHE_OWNER_HEADER_PREFIX)?;
    let mut header_stream =
        serde_json::Deserializer::from_slice(header_json).into_iter::<CursorCacheOwnerHeader>();
    let header = header_stream.next()?.ok()?;
    if header_json.get(header_stream.byte_offset()) != Some(&b',') {
        return None;
    }

    let suffix_len = file_len.min(CURSOR_CACHE_OWNER_ENVELOPE_BYTES as u64) as usize;
    file.seek(SeekFrom::End(-(suffix_len as i64))).ok()?;
    let mut suffix = vec![0; suffix_len];
    file.read_exact(&mut suffix).ok()?;
    let footer_start = suffix
        .windows(CURSOR_CACHE_OWNER_FOOTER_PREFIX.len())
        .rposition(|window| window == CURSOR_CACHE_OWNER_FOOTER_PREFIX)?;
    let footer_json = &suffix[footer_start + CURSOR_CACHE_OWNER_FOOTER_PREFIX.len()..];
    let mut footer_stream =
        serde_json::Deserializer::from_slice(footer_json).into_iter::<CursorCacheOwnerFooter>();
    let footer = footer_stream.next()?.ok()?;
    if footer_json.get(footer_stream.byte_offset()..) != Some(b"}".as_slice()) {
        return None;
    }

    (header.version == 1
        && footer.version == 1
        && header.generation == footer.generation
        && header.owner_hash.len() == 64
        && header
            .owner_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()))
    .then_some(header.owner_hash)
}

fn cursor_cache_advertises_owner_envelope(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let Ok(file_len) = file.metadata().map(|metadata| metadata.len()) else {
        return false;
    };
    let mut prefix = Vec::with_capacity(CURSOR_CACHE_OWNER_ENVELOPE_BYTES);
    if (&mut file)
        .take(CURSOR_CACHE_OWNER_ENVELOPE_BYTES as u64)
        .read_to_end(&mut prefix)
        .is_err()
    {
        return false;
    }
    if prefix.starts_with(CURSOR_CACHE_OWNER_HEADER_PREFIX) {
        return true;
    }

    let suffix_len = file_len.min(CURSOR_CACHE_OWNER_ENVELOPE_BYTES as u64) as usize;
    if file.seek(SeekFrom::End(-(suffix_len as i64))).is_err() {
        return false;
    }
    let mut suffix = vec![0; suffix_len];
    file.read_exact(&mut suffix).is_ok()
        && suffix
            .windows(CURSOR_CACHE_OWNER_FOOTER_PREFIX.len())
            .any(|window| window == CURSOR_CACHE_OWNER_FOOTER_PREFIX)
}

fn cursor_cache_owner_hash_from_file(path: &Path) -> Option<String> {
    if let Some(owner_hash) = cursor_cache_owner_hash_from_envelope(path) {
        return Some(owner_hash);
    }
    if cursor_cache_advertises_owner_envelope(path) {
        return None;
    }
    let contents = fs::read_to_string(path).ok()?;
    legacy_cursor_cache_owner(&contents).map(|account_id| cursor_cache_owner_hash(&account_id))
}

fn owned_cursor_cache(contents: &str, account_id: &str) -> Result<String> {
    let mut value: serde_json::Value = serde_json::from_str(contents)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Cursor usage cache must be a JSON object"))?;
    let generation = uuid::Uuid::new_v4().to_string();
    object.insert(
        "_owner".to_string(),
        serde_json::json!({
            "version": 1,
            "ownerHash": cursor_cache_owner_hash(account_id),
            "generation": generation.clone(),
        }),
    );
    object.insert(
        "_tokscale".to_string(),
        serde_json::json!({"version": 1, "accountId": account_id}),
    );
    object.insert(
        "zzTokscaleOwner".to_string(),
        serde_json::json!({"version": 1, "generation": generation}),
    );
    Ok(serde_json::to_string(&value)?)
}

/// Unmarked legacy files have no provable owner. Preserve them outside the
/// scanner's live paths before any incremental fetch or account lifecycle move.
/// A failed backup aborts the operation; no account may borrow that watermark.
fn prepare_cursor_cache_layout(cache_dir: &Path, store: &CursorCredentialsStore) -> Result<()> {
    if !cache_dir.exists() {
        return Ok(());
    }
    let mut preserved = 0;
    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_cursor_usage_cache_filename(name) {
            continue;
        }
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            anyhow::bail!("Cursor cache is not a regular file: {}", path.display());
        }
        let owner_hash = cursor_cache_owner_hash_from_file(&path);
        let owner_id = owner_hash.as_deref().and_then(|hash| {
            store
                .accounts
                .keys()
                .find(|id| cursor_cache_owner_hash(id) == hash)
        });
        let verified = owner_id.is_some_and(|id| {
            (name == "usage.json" && id == &store.active_account_id)
                || name == format!("usage.{}.json", cursor_account_cache_stem(id))
        });
        if !verified {
            archive_cache_file_in_dir(cache_dir, &path, "unverified").with_context(|| {
                format!(
                    "Failed to preserve unverified Cursor cache {}",
                    path.display()
                )
            })?;
            preserved += 1;
        }
    }

    // An interrupted switch may leave the active account under its stable name.
    // Only a verified file may be promoted; duplicate copies are backed up.
    let active = cache_dir.join("usage.json");
    let named = cache_dir.join(format!(
        "usage.{}.json",
        cursor_account_cache_stem(&store.active_account_id)
    ));
    if named.exists() {
        if active.exists() {
            // The verified canonical cache remains usable if the refresh fails.
            archive_cache_file_in_dir(cache_dir, &named, "named-copy")?;
            preserved += 1;
        } else {
            fs::rename(named, active)?;
        }
    }
    if preserved > 0 {
        eprintln!(
            "Cursor: preserved {preserved} cache file(s) in {}",
            cache_dir.join("archive").display()
        );
    }
    Ok(())
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
    let home = home_dir()?;
    let account_id = upsert_credentials_in_home(&home, token, label)?;
    clear_cursor_desktop_auto_import_in_home(&home)?;
    Ok(account_id)
}

pub fn remove_account(name_or_id: &str, purge_cache: bool) -> Result<()> {
    remove_account_in_home(&home_dir()?, name_or_id, purge_cache)
}

fn remove_account_in_home(home: &Path, name_or_id: &str, purge_cache: bool) -> Result<()> {
    let mut store = load_credentials_store_from_home(home)
        .ok_or_else(|| anyhow::anyhow!("No saved Cursor accounts"))?;

    let resolved = resolve_account_id(&store, name_or_id)
        .ok_or_else(|| anyhow::anyhow!("Account not found: {}", name_or_id))?;

    // A logout is a durable privacy boundary. Persist the opt-out before
    // deleting credentials or caches so a normal report cannot silently import
    // the still-signed-in Cursor desktop session and undo the logout.
    disable_cursor_desktop_auto_import_in_home(home)?;

    let was_active = resolved == store.active_account_id;

    let cache_dir = cursor_cache_dir(home);
    prepare_cursor_cache_layout(&cache_dir, &store)?;
    if cache_dir.exists() {
        for ext in CURSOR_CACHE_EXTENSIONS {
            let per_account = cache_dir.join(format!(
                "usage.{}.{ext}",
                cursor_account_cache_stem(&resolved)
            ));
            if per_account.exists() {
                if purge_cache {
                    fs::remove_file(&per_account)?;
                } else {
                    archive_cache_file_in_dir(
                        &cache_dir,
                        &per_account,
                        &format!("usage.{}", resolved),
                    )?;
                }
            }
            if was_active {
                let active_file = cache_dir.join(format!("usage.{ext}"));
                if active_file.exists() {
                    if purge_cache {
                        fs::remove_file(&active_file)?;
                    } else {
                        archive_cache_file_in_dir(
                            &cache_dir,
                            &active_file,
                            &format!("usage.active.{}", resolved),
                        )?;
                    }
                }
            }
        }
    }

    store.accounts.remove(&resolved);

    if store.accounts.is_empty() {
        let path = cursor_credentials_path(home);
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
                    cursor_account_cache_stem(&first_id)
                ));
                let active_file = cache_dir.join(format!("usage.{ext}"));
                if new_account_file.exists() {
                    fs::rename(&new_account_file, &active_file)?;
                }
            }
            store.active_account_id = first_id;
        }
    }

    save_credentials_store_in_home(home, &store)?;
    Ok(())
}

pub fn remove_all_accounts(purge_cache: bool) -> Result<()> {
    remove_all_accounts_in_home(&home_dir()?, purge_cache)
}

fn remove_all_accounts_in_home(home: &Path, purge_cache: bool) -> Result<()> {
    // Write the non-secret tombstone first. If this fails, do not claim a
    // successful logout that the next implicit report would immediately undo.
    disable_cursor_desktop_auto_import_in_home(home)?;

    let cache_dir = cursor_cache_dir(home);
    if purge_cache {
        // `--all --purge-cache` is an explicit deletion request. Remove both
        // application-owned cache locations, including legacy files and earlier
        // archives whose ownership can no longer be proved.
        for dir in [cache_dir.clone(), old_cursor_cache_dir(home)] {
            if dir.exists() {
                fs::remove_dir_all(dir)?;
            }
        }
    } else {
        let store =
            load_credentials_store_from_home(home).unwrap_or_else(|| CursorCredentialsStore {
                version: 1,
                active_account_id: String::new(),
                accounts: HashMap::new(),
            });
        prepare_cursor_cache_layout(&cache_dir, &store)?;
        if cache_dir.exists() {
            for entry in fs::read_dir(&cache_dir)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_string();
                if is_cursor_usage_cache_filename(&name) {
                    archive_cache_file_in_dir(
                        &cache_dir,
                        &entry.path(),
                        &format!("usage.all.{name}"),
                    )?;
                }
            }
        }
    }

    for path in [
        cursor_credentials_path(home),
        old_cursor_credentials_path(home),
    ] {
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub fn set_active_account(name_or_id: &str) -> Result<()> {
    set_active_account_in_home(&home_dir()?, name_or_id)
}

fn set_active_account_in_home(home: &Path, name_or_id: &str) -> Result<()> {
    let mut store = load_credentials_store_from_home(home)
        .ok_or_else(|| anyhow::anyhow!("No saved Cursor accounts"))?;

    let resolved = resolve_account_id(&store, name_or_id)
        .ok_or_else(|| anyhow::anyhow!("Account not found: {}", name_or_id))?;

    prepare_cursor_cache_layout(&cursor_cache_dir(home), &store)?;
    let old_active_id = store.active_account_id.clone();

    if resolved != old_active_id {
        // Do not record the switch when the caches could not be moved: the active
        // `usage.*` would then belong to a different account than
        // `active_account_id` claims, and the next sync would file one account's
        // usage under the other.
        reconcile_cache_files_in_dir(&cursor_cache_dir(home), &old_active_id, &resolved)?;
    }

    store.active_account_id = resolved;
    save_credentials_store_in_home(home, &store)?;

    Ok(())
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
            cursor_account_cache_stem(old_account_id)
        ));
        let new_account_file = cache_dir.join(format!(
            "usage.{}.{ext}",
            cursor_account_cache_stem(new_account_id)
        ));

        if active_file.exists() {
            if old_account_file.exists() {
                if let Err(error) =
                    archive_cache_file_in_dir(cache_dir, &old_account_file, old_account_id)
                {
                    failures.push(format!(
                        "could not preserve {} before moving usage.{ext}: {error}",
                        old_account_file.display()
                    ));
                    continue;
                }
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
                if let Err(error) =
                    archive_cache_file_in_dir(cache_dir, &active_file, "usage.active")
                {
                    failures.push(format!(
                        "could not preserve {} before moving usage.{ext}: {error}",
                        active_file.display()
                    ));
                    continue;
                }
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
                            cursor_account_cache_stem(account_id)
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

fn cursor_usage_cache_file_is_fresh_at(path: &Path, max_age: Duration, now: SystemTime) -> bool {
    let Ok(mtime) = path.metadata().and_then(|meta| meta.modified()) else {
        return false;
    };
    match now.duration_since(mtime) {
        Ok(age) => age < max_age,
        // mtime is in the future (clock skew) — treat as fresh; a clock-skew
        // cache is no less authoritative than a freshly-fetched one, and we'd
        // rather not thrash the API while the system clock recovers.
        Err(_) => true,
    }
}

/// The accounts a recent attempt marker names, when it is fresh and readable.
/// `None` means no fresh marker. `Some(None)` is a legacy marker written by an
/// older binary, which names no account and therefore cannot be account-scoped.
fn cursor_sync_attempt_scope(
    cache_dir: &Path,
    max_age: Duration,
    now: SystemTime,
) -> Option<Option<Vec<String>>> {
    let path = cache_dir.join(CURSOR_SYNC_ATTEMPT_MARKER);
    if !cursor_usage_cache_file_is_fresh_at(&path, max_age, now) {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    match serde_json::from_str::<CursorSyncAttemptMarker>(&text) {
        Ok(marker) if marker.version == CURSOR_SYNC_ATTEMPT_MARKER_VERSION => {
            Some(Some(marker.account_hashes))
        }
        _ => Some(None),
    }
}

/// A fresh attempt for this account. A legacy unscoped marker still excuses a
/// stale or missing secondary (its pre-upgrade meaning) so a rolling upgrade
/// does not start syncing on every invocation.
fn cursor_secondary_attempt_excuses(
    cache_dir: &Path,
    account_id: &str,
    max_age: Duration,
    now: SystemTime,
) -> bool {
    match cursor_sync_attempt_scope(cache_dir, max_age, now) {
        Some(Some(hashes)) => hashes
            .iter()
            .any(|hash| hash == &cursor_cache_owner_hash(account_id)),
        Some(None) => true,
        None => false,
    }
}

/// A fresh account-scoped attempt for this account. A legacy unscoped marker is
/// refused here: it must never excuse a missing active cache, which earlier
/// versions always treated as stale.
fn cursor_active_attempt_excuses(
    cache_dir: &Path,
    account_id: &str,
    max_age: Duration,
    now: SystemTime,
) -> bool {
    matches!(
        cursor_sync_attempt_scope(cache_dir, max_age, now),
        Some(Some(hashes)) if hashes
            .iter()
            .any(|hash| hash == &cursor_cache_owner_hash(account_id))
    )
}

fn write_cursor_sync_attempt_marker(
    cache_dir: &Path,
    store: &CursorCredentialsStore,
) -> Result<()> {
    let mut account_hashes = store
        .accounts
        .keys()
        .map(|id| cursor_cache_owner_hash(id))
        .collect::<Vec<_>>();
    account_hashes.sort_unstable();
    account_hashes.dedup();
    let marker = CursorSyncAttemptMarker {
        version: CURSOR_SYNC_ATTEMPT_MARKER_VERSION,
        account_hashes,
    };
    atomic_write_file(
        &cache_dir.join(CURSOR_SYNC_ATTEMPT_MARKER),
        &serde_json::to_string(&marker)?,
    )
}

fn cursor_usage_cache_is_fresh_in_at(home_dir: &Path, max_age: Duration, now: SystemTime) -> bool {
    let cache_dir = cursor_cache_dir(home_dir);
    if !cache_dir.exists() {
        return false;
    }

    let store = load_credentials_store_from_home(home_dir);
    let active_path = cache_dir.join("usage.json");
    let active_attempt_fresh = store.as_ref().is_some_and(|store| {
        store.accounts.contains_key(&store.active_account_id)
            && cursor_active_attempt_excuses(&cache_dir, &store.active_account_id, max_age, now)
    });
    let active_file_fresh = cursor_usage_cache_file_is_fresh_at(&active_path, max_age, now);

    // A stale active cache must still refresh. A missing first cache may use a
    // recent account-scoped attempt marker so an unavailable desktop session
    // does not add the network timeout to every report. The retry resumes when
    // the bounded freshness window expires.
    if !active_file_fresh && (active_path.exists() || !active_attempt_fresh) {
        return false;
    }

    let expected_paths = expected_cursor_usage_cache_paths_in(home_dir);
    if let Some(store) = &store {
        // Legacy aliases or interrupted-switch duplicates must be preserved
        // before reporting, even when all expected files and the marker are fresh.
        let Ok(entries) = fs::read_dir(&cache_dir) else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else { return false };
            if entry
                .file_name()
                .to_str()
                .is_some_and(is_cursor_usage_cache_filename)
                && !expected_paths.contains(&entry.path())
            {
                return false;
            }
        }
        if active_path.exists() {
            let owner_hash = cursor_cache_owner_hash_from_file(&active_path);
            let expected_hash = cursor_cache_owner_hash(&store.active_account_id);
            if owner_hash.as_deref() != Some(expected_hash.as_str()) {
                return false;
            }
        }
    }

    expected_paths
        .iter()
        .filter(|p| *p != &active_path)
        .all(|path| {
            let account_id = store.as_ref().and_then(|store| {
                store.accounts.keys().find(|id| {
                    *path == cache_dir.join(format!("usage.{}.json", cursor_account_cache_stem(id)))
                })
            });

            // A recent attempt may excuse a missing or stale secondary, but
            // never a live file whose ownership is unknown or disagrees.
            if path.exists() {
                let owner_hash = cursor_cache_owner_hash_from_file(path);
                let verified = owner_hash.as_deref().is_some_and(|hash| {
                    account_id.is_some_and(|id| cursor_cache_owner_hash(id) == hash)
                });
                if !verified {
                    return false;
                }
            }
            cursor_usage_cache_file_is_fresh_at(path, max_age, now)
                || account_id.is_some_and(|id| {
                    cursor_secondary_attempt_excuses(&cache_dir, id, max_age, now)
                })
        })
}

fn cursor_usage_cache_is_fresh_in(home_dir: &Path, max_age: Duration) -> bool {
    cursor_usage_cache_is_fresh_in_at(home_dir, max_age, SystemTime::now())
}

/// True when the active Cursor cache is fresh and every secondary is fresh or
/// has an account-scoped recent attempt. A stale active cache is never excused.
/// A missing first active cache may be excused until its recent attempt expires,
/// which bounds retries when desktop discovery succeeds but the API is offline.
/// The manual `tokscale cursor sync` CLI bypasses this gate.
pub fn cursor_usage_cache_is_fresh(max_age: Duration) -> bool {
    let Ok(home_dir) = home_dir() else {
        return false;
    };
    cursor_usage_cache_is_fresh_in(&home_dir, max_age)
}

pub fn is_cursor_logged_in() -> bool {
    load_active_credentials().is_some()
}

/// True when a Cursor session can be synced.
///
/// A saved tokscale login counts. Otherwise this imports `cursorAuth/accessToken`
/// from the desktop `state.vscdb` when the Cursor app itself is signed in.
/// Report commands used to require `tokscale cursor login` before they would
/// even try, so a desktop session produced no Cursor usage at all.
pub fn ensure_cursor_session_for_sync() -> bool {
    if is_cursor_logged_in() {
        return true;
    }
    let Ok(home) = home_dir() else {
        return false;
    };
    if cursor_desktop_auto_import_is_disabled_in_home(&home) {
        return false;
    }
    matches!(ensure_credentials_from_local_cursor(), Ok(Some(_)))
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
    is_custom_timeout: bool,
    existing_cache: Option<&str>,
) -> Result<String> {
    fetch_cursor_usage_events_json_merging(
        USAGE_EVENTS_JSON_ENDPOINT,
        session_token,
        timeout_override,
        is_custom_timeout,
        CURSOR_MAX_JSON_BYTES,
        CURSOR_JSON_PAGE_SIZE,
        existing_cache,
    )
    .await
}

/// Body of [`fetch_cursor_usage_events_json`] with the endpoint, byte ceiling,
/// and page size injected so tests can drive the real paginating path against a
/// local server. No cached history is merged in.
#[cfg(test)]
async fn fetch_cursor_usage_events_json_from(
    url: &str,
    session_token: &str,
    timeout_override: Option<Duration>,
    is_custom_timeout: bool,
    max_body_bytes: usize,
    page_size: usize,
) -> Result<String> {
    fetch_cursor_usage_events_json_merging(
        url,
        session_token,
        timeout_override,
        is_custom_timeout,
        max_body_bytes,
        page_size,
        None,
    )
    .await
}

/// Walks pages of `get-filtered-usage-events` (a POST endpoint carrying an
/// `Origin` header for its CSRF check).
///
/// With no cached history this collects every event. With a cache, the request
/// sends `startDate` one hour before the newest stored timestamp and stops
/// when a page crosses that lower bound. This bounded overlap refreshes recent
/// billing corrections, including rows older than the newest cached event.
/// Corrections outside that hour are not re-fetched. The dashboard lists events
/// newest-first, so the overlap avoids re-downloading the full account.
/// Returned JSON is the cache with that delta merged in, not the delta alone,
/// so a caught-up account still rewrites a non-empty cache.
///
/// `max_body_bytes` is a cumulative ceiling spanning every page, and a page that
/// omits the `usageEventsDisplay` array is an error so a malformed 200 never
/// masquerades as an empty result. A timed-out walk returns an error and does
/// not merge a partial delta: the newest cached timestamp would otherwise skip
/// the gap on the next sync.
async fn fetch_cursor_usage_events_json_merging(
    url: &str,
    session_token: &str,
    timeout_override: Option<Duration>,
    is_custom_timeout: bool,
    max_body_bytes: usize,
    page_size: usize,
    existing_cache: Option<&str>,
) -> Result<String> {
    let existing_events = cached_usage_events(existing_cache);
    let end_ms = chrono::Utc::now().timestamp_millis();
    let max_timestamp = end_ms.saturating_add(CURSOR_USAGE_CLOCK_SKEW_MS);
    let watermark = usage_events_watermark_ms(&existing_events, max_timestamp);
    let sync_start =
        watermark.map(|ms| ms.saturating_sub(CURSOR_USAGE_CORRECTION_WINDOW_MS).max(0));
    let client = build_cursor_http_client()?;
    let mut fresh_events: Vec<serde_json::Value> = Vec::new();
    let mut total_count: Option<u64> = None;
    let mut bytes_read: usize = 0;

    // Overall wall-clock budget for the entire paginated walk. Without it the
    // per-page timeout would multiply across every page, so a slow server could
    // stall report startup (auto-sync runs first) for that timeout times the page
    // count. Each page's timeout is clamped to what remains of this budget below,
    // and the walk aborts once it is spent.
    let start_time = Instant::now();
    let initial_budget = timeout_override.unwrap_or(CURSOR_HTTP_TIMEOUT);
    let mut overall_budget = initial_budget;
    let mut fetch_deadline = deadline_after(start_time, overall_budget);

    let mut completed = false;
    let mut page_limit = CURSOR_MAX_JSON_PAGES;
    let mut largest_page_yield = 0usize;
    let mut page = 0usize;
    while page < page_limit {
        page += 1;
        let remaining_budget = fetch_deadline.saturating_duration_since(Instant::now());
        if remaining_budget.is_zero() {
            anyhow::bail!(
                "{}",
                cursor_budget_exhausted_message(
                    overall_budget,
                    page - 1,
                    fresh_events.len(),
                    total_count,
                    "",
                )
            );
        }

        let body = usage_events_request_body(page, page_size, sync_start, end_ms);

        let page_timeout = initial_budget.min(remaining_budget);
        let req = client
            .post(url)
            .headers(build_cursor_json_headers(session_token))
            .json(&body)
            .timeout(page_timeout);

        let response = match req.send().await {
            Ok(response) => response,
            Err(err) => {
                let cause = tokscale_core::pricing::describe_error(&err);
                if fetch_deadline
                    .saturating_duration_since(Instant::now())
                    .is_zero()
                {
                    anyhow::bail!(
                        "{}",
                        cursor_budget_exhausted_message(
                            overall_budget,
                            page - 1,
                            fresh_events.len(),
                            total_count,
                            &cause,
                        )
                    );
                }
                if err.is_timeout() {
                    anyhow::bail!(
                        "{}",
                        cursor_request_timeout_message(
                            page_timeout,
                            page - 1,
                            fresh_events.len(),
                            total_count,
                            &cause,
                        )
                    );
                }
                anyhow::bail!("{cause}");
            }
        };

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
        let text = match read_cursor_body_with_cap(response, remaining, "usage events JSON").await {
            Ok(text) => text,
            Err(err) => {
                let cause = cursor_sync_error_text(&err);
                if fetch_deadline
                    .saturating_duration_since(Instant::now())
                    .is_zero()
                {
                    anyhow::bail!(
                        "{}",
                        cursor_budget_exhausted_message(
                            overall_budget,
                            page - 1,
                            fresh_events.len(),
                            total_count,
                            &cause,
                        )
                    );
                }
                if error_is_reqwest_timeout(&err) {
                    anyhow::bail!(
                        "{}",
                        cursor_request_timeout_message(
                            page_timeout,
                            page - 1,
                            fresh_events.len(),
                            total_count,
                            &cause,
                        )
                    );
                }
                return Err(err);
            }
        };
        bytes_read += text.len();

        let page_value: serde_json::Value = serde_json::from_str(&text)
            .context("Invalid response from Cursor API - expected usage events JSON")?;

        if total_count.is_none() {
            if let Some(total) = page_value
                .get("totalUsageEventsCount")
                .and_then(|value| value.as_u64())
            {
                total_count = Some(total);
            }
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
        // Stop at the correction window even when the server ignores startDate
        // and advertises the full account total. Older cached rows survive the
        // merge; only the recent overlap is re-fetched for billing corrections.
        let (fresh, crossed) = filter_cursor_usage_page(&page_events, sync_start, max_timestamp);
        fresh_events.extend(fresh.into_iter().cloned());
        if crossed {
            completed = true;
            break;
        }
        largest_page_yield = largest_page_yield.max(received);
        page_limit = cursor_page_limit(largest_page_yield);

        // When running an explicit sync with the default budget (no custom
        // override specified via flag or environment variable), dynamically
        // scale the wall-clock budget with the advertised total events so
        // accounts with tens of thousands of rows don't abort prematurely.
        // Base the page estimate on observed page yield so server-side page
        // clamping (e.g. Cursor clamping a 500-event request to 100) doesn't
        // underestimate pages and time out.
        if !is_custom_timeout && timeout_override.is_some() {
            if let Some(total) = total_count {
                let effective_page = if received > 0 { received } else { page_size };
                let expanded = compute_dynamic_cursor_budget(total, effective_page, initial_budget);
                if expanded > overall_budget {
                    fetch_deadline = deadline_after(start_time, expanded);
                    overall_budget = expanded;
                }
            }
        }

        // Once the advertised total is known, keep paging until it is reached so
        // a server that clamps `pageSize` (a short page while more events remain)
        // doesn't stop the walk early. An empty page before the total is reached
        // means the server truncated the history mid-walk, so it is an error
        // rather than a silently partial cache. Fall back to the short/empty-page
        // heuristic only when no total was reported.
        let done = match total_count {
            Some(total) => {
                if fresh_events.len() as u64 >= total {
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
            "Cursor usage events exceeded the {page_limit}-page fetch limit ({} events collected) before the full history was collected; refusing to cache a partial history",
            fresh_events.len()
        );
    }

    let events = if !existing_events.is_empty() {
        merge_cursor_usage_events(existing_events, fresh_events)
    } else {
        fresh_events
    };
    let aggregated = serde_json::json!({
        "totalUsageEventsCount": events.len(),
        "usageEventsDisplay": events,
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

/// One account's usage download. `existing_cache` is the aggregated JSON already
/// on disk, when there is one, so the fetcher can request only newer events.
struct CursorUsageFetch {
    session_token: String,
    existing_cache: Option<String>,
}

async fn sync_cursor_cache_with_fetcher<F, Fut>(fetch_usage_json: F) -> SyncCursorResult
where
    F: Fn(CursorUsageFetch) -> Fut,
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
    F: Fn(CursorUsageFetch) -> Fut,
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

    if let Err(error) = prepare_cursor_cache_layout(&cache_dir, &store) {
        return SyncCursorResult {
            synced: false,
            rows: 0,
            error: Some(cursor_sync_error_text(&error)),
        };
    }

    let mut total_rows = 0;
    let mut success_count = 0;
    let mut errors: Vec<String> = Vec::new();

    for (account_id, credentials) in &store.accounts {
        let is_active = account_id == &store.active_account_id;
        let file_path = if is_active {
            cache_dir.join("usage.json")
        } else {
            cache_dir.join(format!(
                "usage.{}.json",
                cursor_account_cache_stem(account_id)
            ))
        };
        let existing_cache = fs::read_to_string(&file_path).ok();

        match fetch_usage_json(CursorUsageFetch {
            session_token: credentials.session_token.clone(),
            existing_cache,
        })
        .await
        {
            Ok(json_text) => {
                let json_text = match owned_cursor_cache(&json_text, account_id) {
                    Ok(contents) => contents,
                    Err(error) => {
                        errors.push(format!("{account_id}: invalid usage response: {error}"));
                        continue;
                    }
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
                errors.push(format!("{account_id}: {}", cursor_sync_error_text(&e)));
            }
        }
    }

    // Record exactly the accounts attempted in this run. The mtime bounds retry
    // suppression, while account hashes keep a later login or account switch
    // from inheriting another account's failed-attempt cooldown. Failure to
    // write the best-effort marker simply leaves the normal retry path enabled.
    let _ = write_cursor_sync_attempt_marker(&cache_dir, &store);

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

fn max_sync_timeout() -> Duration {
    Duration::from_secs(CURSOR_MAX_SYNC_TIMEOUT_SECS)
}

fn is_valid_sync_timeout_secs(secs: u64) -> bool {
    (1..=CURSOR_MAX_SYNC_TIMEOUT_SECS).contains(&secs)
}

/// The explicit sync budget and whether the user chose it.
///
/// Precedence: the `--timeout` flag, then `cursor.syncTimeoutSeconds` from
/// settings.json, then the default budget, which the walk scales with the
/// advertised event count. A user-chosen budget is used as-is, without
/// scaling. Out-of-range settings are dropped by the caller (with a warning)
/// before they get here, and a flag value is clamped as a last line of
/// defence even though clap already rejects anything above the maximum.
fn configured_explicit_sync_timeout(
    explicit_timeout: Option<Duration>,
    configured_secs: Option<u64>,
) -> (Duration, bool) {
    if let Some(timeout) = explicit_timeout.filter(|timeout| !timeout.is_zero()) {
        return (timeout.min(max_sync_timeout()), true);
    }

    if let Some(secs) = configured_secs.filter(|secs| is_valid_sync_timeout_secs(*secs)) {
        return (Duration::from_secs(secs), true);
    }

    (CURSOR_EXPLICIT_SYNC_TIMEOUT, false)
}

/// Timeout override for the usage download: explicit syncs get the longer
/// [`CURSOR_EXPLICIT_SYNC_TIMEOUT`] (or the user's flag/setting); implicit
/// syncs keep the client default.
fn sync_timeout_override(
    explicit: bool,
    explicit_timeout: Option<Duration>,
    configured_secs: Option<u64>,
) -> (Option<Duration>, bool) {
    if explicit {
        let (duration, is_custom) =
            configured_explicit_sync_timeout(explicit_timeout, configured_secs);
        (Some(duration), is_custom)
    } else {
        (None, false)
    }
}

/// A `cursor.syncTimeoutSeconds` value the sync can use: a whole number of
/// seconds in `1..=CURSOR_MAX_SYNC_TIMEOUT_SECS`, written as `600` or `600.0`.
/// Strings, fractions, negative and out-of-range numbers are not.
fn usable_sync_timeout_setting(raw: &serde_json::Value) -> Option<u64> {
    let secs = raw.as_u64().or_else(|| {
        raw.as_f64()
            .filter(|secs| {
                secs.fract() == 0.0 && (0.0..=CURSOR_MAX_SYNC_TIMEOUT_SECS as f64).contains(secs)
            })
            .map(|secs| secs as u64)
    })?;
    is_valid_sync_timeout_secs(secs).then_some(secs)
}

/// `cursor.syncTimeoutSeconds` from settings.json, if it is set and usable.
///
/// Only an explicit `tokscale cursor sync` reads this (never the TUI), so an
/// unusable value is reported on stderr - which leaves `--json` stdout
/// intact - and the default budget is used instead. The file keeps the value
/// as written.
fn configured_sync_timeout_secs() -> Option<u64> {
    let raw = crate::tui::settings::Settings::load()
        .cursor
        .sync_timeout_seconds?;
    let secs = usable_sync_timeout_setting(&raw);
    if secs.is_none() {
        eprintln!(
            "Warning: ignoring cursor.syncTimeoutSeconds = {raw} in settings.json \
             (expected a whole number of seconds, 1-{CURSOR_MAX_SYNC_TIMEOUT_SECS}); \
             using the default sync budget."
        );
    }
    secs
}

/// The persisted budget to consult for this sync, if any. Only an explicit
/// sync without `--timeout` reads the setting: the flag wins outright, and
/// reading the setting anyway could warn about a value this run never uses.
fn setting_budget_for(
    explicit: bool,
    explicit_timeout: Option<Duration>,
    load_setting: impl FnOnce() -> Option<u64>,
) -> Option<u64> {
    let flag_given = explicit_timeout.is_some_and(|timeout| !timeout.is_zero());
    if explicit && !flag_given {
        load_setting()
    } else {
        None
    }
}

/// `start + budget` without the panic `Instant + Duration` raises on
/// overflow. Budgets are bounded by [`CURSOR_MAX_SYNC_TIMEOUT_SECS`], so the
/// fallback never triggers in practice; it keeps any future caller from
/// reintroducing the overflow panic.
fn deadline_after(start: Instant, budget: Duration) -> Instant {
    start
        .checked_add(budget)
        .or_else(|| start.checked_add(max_sync_timeout()))
        .unwrap_or(start)
}

pub async fn sync_cursor_cache(explicit: bool) -> SyncCursorResult {
    sync_cursor_cache_with_explicit_timeout(explicit, None).await
}

pub async fn sync_cursor_cache_with_explicit_timeout(
    explicit: bool,
    explicit_timeout: Option<Duration>,
) -> SyncCursorResult {
    let home = match home_dir() {
        Ok(home) => home,
        Err(error) => {
            return SyncCursorResult {
                synced: false,
                rows: 0,
                error: Some(format!("Failed to get home dir: {error}")),
            };
        }
    };

    // Explicit sync is a deliberate opt-in and restores desktop discovery.
    // Implicit sync must respect a prior logout even when another saved account
    // remains available to sync.
    if explicit {
        if let Err(error) = clear_cursor_desktop_auto_import_in_home(&home) {
            return SyncCursorResult {
                synced: false,
                rows: 0,
                error: Some(format!(
                    "Failed to restore Cursor desktop discovery: {error}"
                )),
            };
        }
    }
    if explicit || !cursor_desktop_auto_import_is_disabled_in_home(&home) {
        // Prefer a fresh token from the local Cursor desktop login when
        // available, avoiding stale manually-pasted cookies after a refresh.
        let _ = ensure_credentials_from_local_cursor();
    }

    let configured_secs =
        setting_budget_for(explicit, explicit_timeout, configured_sync_timeout_secs);
    let (timeout, is_custom) = sync_timeout_override(explicit, explicit_timeout, configured_secs);
    sync_cursor_cache_with_fetcher(move |fetch| async move {
        fetch_cursor_usage_events_json(
            &fetch.session_token,
            timeout,
            is_custom,
            fetch.existing_cache.as_deref(),
        )
        .await
    })
    .await
}

/// Moves `file_path` into a newly created private directory under archive/.
/// Keep its original filename and bytes, and never overwrite an earlier backup.
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
    // Keep the parent short for Windows path limits; the original filename
    // remains intact, and create_dir reserves a destination that cannot exist.
    let short_label = &safe_label[..safe_label.len().min(12)];
    let destination_dir = archive_dir.join(format!("{short_label}-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&destination_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&destination_dir, fs::Permissions::from_mode(0o700))?;
    }
    let name = file_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Cursor cache has no filename"))?;
    fs::rename(file_path, destination_dir.join(name))?;
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
        if accounts.is_empty() && !purge_cache {
            remove_all_accounts(false)?;
            println!("\n  {}\n", "No saved Cursor accounts.".yellow());
            return Ok(());
        }

        remove_all_accounts(purge_cache)?;
        let message = if accounts.is_empty() {
            "Purged all Cursor cache data."
        } else {
            "Logged out from all Cursor accounts."
        };
        println!("\n  {}\n", message.green());
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
        remove_all_accounts(false)?;
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

pub fn run_cursor_sync(json: bool, timeout_secs: Option<u64>) -> Result<()> {
    use colored::Colorize;
    use tokio::runtime::Runtime;

    let timeout_override = timeout_secs.map(Duration::from_secs);
    let rt = Runtime::new()?;
    let result = rt.block_on(sync_cursor_cache_with_explicit_timeout(
        true,
        timeout_override,
    ));

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
    use serial_test::serial;
    use std::collections::HashMap;
    use tempfile::TempDir;

    struct EnvVarGuard {
        name: &'static str,
        original: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let original = std::env::var_os(name);
            unsafe {
                std::env::set_var(name, value);
            }
            Self { name, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => unsafe {
                    std::env::set_var(self.name, value);
                },
                None => unsafe {
                    std::env::remove_var(self.name);
                },
            }
        }
    }

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

    /// Electron stores Linux user data under `$XDG_CONFIG_HOME` when it is set,
    /// so discovery must probe it before the `$HOME/.config` fallback.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    #[serial]
    fn test_state_vscdb_candidates_honour_xdg_config_home() {
        let home = TempDir::new().unwrap();
        let xdg = TempDir::new().unwrap();
        // The guard restores the developer's XDG_CONFIG_HOME even when an
        // assertion below panics, so later #[serial] tests are not poisoned.
        let _guard = EnvVarGuard::set("XDG_CONFIG_HOME", xdg.path().to_str().unwrap());

        let candidates = cursor_state_vscdb_candidates(home.path());
        assert_eq!(
            candidates.first(),
            Some(&xdg.path().join("Cursor/User/globalStorage/state.vscdb")),
            "an absolute XDG root must be probed first: {candidates:?}"
        );
        assert!(
            candidates.contains(
                &home
                    .path()
                    .join(".config/Cursor/User/globalStorage/state.vscdb")
            ),
            "the home fallback must remain: {candidates:?}"
        );

        // A relative or empty override is ignored rather than resolved weirdly.
        for ignored in ["", "relative/config"] {
            let _inner = EnvVarGuard::set("XDG_CONFIG_HOME", ignored);
            assert_eq!(
                cursor_state_vscdb_candidates(home.path()),
                vec![home
                    .path()
                    .join(".config/Cursor/User/globalStorage/state.vscdb")],
                "an unusable XDG_CONFIG_HOME must fall back to home"
            );
        }
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
        assert_eq!(
            sync_timeout_override(true, None, None),
            (Some(Duration::from_secs(120)), false)
        );
        assert_eq!(
            sync_timeout_override(true, Some(Duration::from_secs(300)), None),
            (Some(Duration::from_secs(300)), true)
        );
        assert_eq!(sync_timeout_override(false, None, None), (None, false));
        assert_eq!(
            sync_timeout_override(false, Some(Duration::from_secs(300)), Some(600)),
            (None, false)
        );
    }

    #[test]
    fn test_sync_timeout_override_prefers_flag_then_setting_then_default() {
        // The flag wins over the persisted setting for one run.
        assert_eq!(
            sync_timeout_override(true, Some(Duration::from_secs(300)), Some(600)),
            (Some(Duration::from_secs(300)), true)
        );
        // Without a flag, `cursor.syncTimeoutSeconds` is the whole budget.
        assert_eq!(
            sync_timeout_override(true, None, Some(600)),
            (Some(Duration::from_secs(600)), true)
        );
        assert_eq!(
            sync_timeout_override(true, None, Some(CURSOR_MAX_SYNC_TIMEOUT_SECS)),
            (
                Some(Duration::from_secs(CURSOR_MAX_SYNC_TIMEOUT_SECS)),
                true
            )
        );
        // Neither: the default budget, which the walk may scale.
        assert_eq!(
            sync_timeout_override(true, None, None),
            (Some(CURSOR_EXPLICIT_SYNC_TIMEOUT), false)
        );
    }

    #[test]
    fn test_sync_timeout_override_ignores_out_of_range_settings() {
        for secs in [0, CURSOR_MAX_SYNC_TIMEOUT_SECS + 1, u64::MAX] {
            assert_eq!(
                sync_timeout_override(true, None, Some(secs)),
                (Some(CURSOR_EXPLICIT_SYNC_TIMEOUT), false),
                "cursor.syncTimeoutSeconds = {secs} must fall back to the default budget"
            );
        }
    }

    #[test]
    fn test_sync_timeout_setting_must_be_whole_seconds_in_range() {
        use serde_json::json;
        assert_eq!(usable_sync_timeout_setting(&json!(600)), Some(600));
        assert_eq!(usable_sync_timeout_setting(&json!(1)), Some(1));
        assert_eq!(
            usable_sync_timeout_setting(&json!(CURSOR_MAX_SYNC_TIMEOUT_SECS)),
            Some(CURSOR_MAX_SYNC_TIMEOUT_SECS)
        );
        // `600.0` is a whole number too.
        assert_eq!(usable_sync_timeout_setting(&json!(600.0)), Some(600));
        assert_eq!(usable_sync_timeout_setting(&json!(86400.0)), Some(86_400));
        for unusable in [
            json!(0),
            json!(0.0),
            json!(CURSOR_MAX_SYNC_TIMEOUT_SECS + 1),
            json!(86401.0),
            json!(-5),
            json!(-5.0),
            json!(1.5),
            json!(600.5),
            json!(1e300),
            json!("600"),
            json!(true),
            json!([600]),
            json!({ "seconds": 600 }),
        ] {
            assert_eq!(usable_sync_timeout_setting(&unusable), None, "{unusable}");
        }
    }

    #[test]
    fn test_the_timeout_setting_is_read_only_without_the_flag() {
        let unreachable = || -> Option<u64> { panic!("the setting must not be read") };
        // `--timeout` wins outright, so the settings file is never consulted.
        assert_eq!(
            setting_budget_for(true, Some(Duration::from_secs(300)), unreachable),
            None
        );
        // Implicit (pre-report) syncs never use the explicit budget.
        assert_eq!(setting_budget_for(false, None, unreachable), None);
        // An explicit sync without the flag reads it.
        assert_eq!(setting_budget_for(true, None, || Some(600)), Some(600));
        // A zero duration is not a flag value (clap rejects 0).
        assert_eq!(
            setting_budget_for(true, Some(Duration::ZERO), || Some(600)),
            Some(600)
        );
    }

    #[test]
    fn test_explicit_timeout_is_clamped_to_the_maximum() {
        // clap rejects these, but the budget must stay bounded for any caller.
        assert_eq!(
            sync_timeout_override(true, Some(Duration::from_secs(u64::MAX)), None),
            (
                Some(Duration::from_secs(CURSOR_MAX_SYNC_TIMEOUT_SECS)),
                true
            )
        );
        assert_eq!(
            sync_timeout_override(true, Some(Duration::MAX), None),
            (
                Some(Duration::from_secs(CURSOR_MAX_SYNC_TIMEOUT_SECS)),
                true
            )
        );
    }

    #[test]
    #[serial]
    fn test_sync_timeout_override_ignores_retired_env_vars() {
        // The budget is a persisted setting (`cursor.syncTimeoutSeconds`), not
        // an environment variable; these names must not change it.
        let _g_ms = EnvVarGuard::set("TOKSCALE_CURSOR_SYNC_TIMEOUT_MS", "180000");
        let _g_secs = EnvVarGuard::set("TOKSCALE_CURSOR_SYNC_TIMEOUT_SECS", "240");
        assert_eq!(
            sync_timeout_override(true, None, None),
            (Some(CURSOR_EXPLICIT_SYNC_TIMEOUT), false)
        );
    }

    #[test]
    fn test_deadline_after_never_panics_on_huge_budgets() {
        let start = Instant::now();
        assert_eq!(
            deadline_after(start, Duration::from_secs(5)),
            start + Duration::from_secs(5)
        );
        // `start + Duration::MAX` panics; the helper must not.
        let saturated = deadline_after(start, Duration::MAX);
        assert!(saturated >= start);
        let from_u64_max = deadline_after(start, Duration::from_secs(u64::MAX));
        assert!(from_u64_max >= start);
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
                false,
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
                false,
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
                sync_timeout_override(true, None, None).0,
                false,
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
        assert!(
            head.contains("\"startDate\":\"0\"") && head.contains("\"endDate\":"),
            "the usage query must cover the full history; a missing window comes back empty: {head}"
        );
        assert!(
            !head.contains("\"teamId\""),
            "teamId 0 is not 'no filter' and drops personal usage: {head}"
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
                false,
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
        let bodies: Vec<serde_json::Value> = captured
            .iter()
            .map(|request| serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap())
            .collect();
        let end_ms = bodies[0]["endDate"]
            .as_str()
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert!(end_ms > 0 && end_ms <= chrono::Utc::now().timestamp_millis());
        for body in &bodies {
            assert_eq!(body["startDate"], "0");
            assert_eq!(body["endDate"], bodies[0]["endDate"]);
            assert!(body.get("teamId").is_none());
        }
    }

    #[test]
    fn test_cursor_page_limit_follows_the_page_size_the_server_honours() {
        // Nothing observed yet, or the requested size honoured: today's ceiling.
        assert_eq!(cursor_page_limit(0), CURSOR_MAX_JSON_PAGES);
        assert_eq!(
            cursor_page_limit(CURSOR_JSON_PAGE_SIZE),
            CURSOR_MAX_JSON_PAGES
        );
        // Cursor clamping 500 -> 100 keeps the same 250,000-event budget.
        assert_eq!(cursor_page_limit(100), 2_500);
        assert_eq!(cursor_page_limit(100) * 100, CURSOR_MAX_JSON_EVENTS);
        // Harder clamps are bounded by the minimum-yield floor.
        assert_eq!(cursor_page_limit(1), 5_000);
        assert_eq!(cursor_page_limit(49), 5_000);
        // Oversized pages never lower the ceiling below today's.
        assert_eq!(cursor_page_limit(1_000), CURSOR_MAX_JSON_PAGES);
        assert_eq!(cursor_page_limit(usize::MAX), CURSOR_MAX_JSON_PAGES);
        // A 60,000-event history clamped to 100 per page now fits.
        assert!(cursor_page_limit(100) >= 600);
    }

    #[test]
    fn test_usage_events_json_clamped_pages_can_exceed_the_original_page_limit() {
        // A first sync still needs the full history. Server-side page clamping
        // must expand the page cap even in the incremental fetcher's cold path.
        let total = CURSOR_MAX_JSON_PAGES + 1;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for page in 1..=total {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 8192];
                let _ = std::io::Read::read(&mut stream, &mut request);
                let body = json_page(
                    total as u64,
                    &[&json_event(&format!("c{page}"), "1788171994001")],
                );
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let text = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &format!("http://{addr}/api/dashboard/get-filtered-usage-events"),
                "session-token",
                Some(Duration::from_secs(30)),
                true,
                CURSOR_MAX_JSON_BYTES,
                CURSOR_JSON_PAGE_SIZE,
            ))
            .expect("a clamped first sync must finish beyond the original page cap");
        assert_eq!(count_cursor_json_events(&text), total);
        server.join().unwrap();
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
                false,
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
                false,
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
                false,
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
                false,
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
    fn test_usage_events_json_timeout_reports_clear_diagnostics() {
        // When the wall-clock budget is exceeded mid-walk, the error must clearly
        // state the budget in seconds and how many pages/events were collected
        // rather than failing with an opaque request error.
        let (url, _requests) = serve_json_pages(vec![
            (
                String::new(),
                json_page(
                    10,
                    &[
                        &json_event("c1", "1788171994001"),
                        &json_event("c2", "1788171994002"),
                    ],
                ),
            ),
            (
                String::new(),
                json_page(10, &[&json_event("c3", "1788171994003")]),
            ),
        ]);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        // Give a tiny 0-second budget so it expires immediately after or during page 1.
        let err = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                Some(Duration::from_millis(0)),
                true,
                CURSOR_MAX_JSON_BYTES,
                2,
            ))
            .expect_err("a 0ms budget must expire");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("time budget"),
            "error must report budget exhaustion: {msg}"
        );
    }

    #[test]
    fn test_compute_dynamic_cursor_budget_scaling() {
        let default_budget = Duration::from_secs(120);

        // Zero page size returns initial budget
        assert_eq!(
            compute_dynamic_cursor_budget(10_000, 0, default_budget),
            default_budget
        );

        // Small count inside default budget remains initial budget
        assert_eq!(
            compute_dynamic_cursor_budget(500, 500, default_budget),
            default_budget
        );
        assert_eq!(
            compute_dynamic_cursor_budget(40 * 500, 500, default_budget),
            default_budget
        ); // 40 pages * 3s = 120s

        // 40,000 events with 500 page size = 80 pages * 3s = 240s
        assert_eq!(
            compute_dynamic_cursor_budget(40_000, 500, default_budget),
            Duration::from_secs(240)
        );

        // 40,000 events with 100 clamped page size = 400 pages * 3s = 1200s
        assert_eq!(
            compute_dynamic_cursor_budget(40_000, 100, default_budget),
            Duration::from_secs(1200)
        );

        // Extremely large count caps at 1800s (30m)
        assert_eq!(
            compute_dynamic_cursor_budget(1_000_000, 100, default_budget),
            Duration::from_secs(1800)
        );

        // Custom higher initial budget is preserved if needed is smaller
        let high_budget = Duration::from_secs(300);
        assert_eq!(
            compute_dynamic_cursor_budget(40_000, 500, high_budget),
            high_budget
        );
    }

    #[test]
    fn test_usage_events_json_dynamically_scales_budget_for_large_history() {
        // When an explicit sync runs without custom override and encounters an
        // advertised total of 40k events (80 pages), the budget must dynamically scale
        // past the initial 120s up to total_pages * 3s (240s).
        let (url, _requests) = serve_json_pages(vec![
            (
                String::new(),
                json_page(
                    40_000,
                    &[
                        &json_event("c1", "1788171994001"),
                        &json_event("c2", "1788171994002"),
                    ],
                ),
            ),
            // End early by empty page
            (String::new(), json_page(40_000, &[])),
        ]);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        // With default 120s budget (is_custom_timeout: false), total_count: 40000 scales budget.
        // It will fail because the 2nd page is empty before the 40k advertised total,
        // proving it processed the first page and recognized the advertised total.
        let err = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                Some(Duration::from_secs(120)),
                false,
                CURSOR_MAX_JSON_BYTES,
                500,
            ))
            .expect_err("empty page before 40000 total must fail");
        assert!(
            format!("{err:#}").contains("40000 events"),
            "must process advertised total of 40000: {err:#}"
        );
    }

    #[test]
    fn test_usage_events_json_merges_only_events_in_the_correction_window() {
        // A warm cache must not walk the rest of the account. Page 1 is a full
        // page (so the unfiltered total would demand another request) and it
        // already crosses the correction window, including a new event that
        // shares a cached event's millisecond.
        let cached = format!(
            r#"{{"usageEventsDisplay":[{},{}]}}"#,
            json_event("boundary", "3600200"),
            json_event("old", "150"),
        );
        let body = json_page(
            10_000,
            &[
                &json_event("new", "3600300"),
                &json_event("same-ms", "3600200"),
                &json_event("boundary", "3600200"),
                &json_event("older", "100"),
            ],
        );
        let (url, requests) = serve_json_pages(vec![(String::new(), body)]);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let text = runtime
            .block_on(fetch_cursor_usage_events_json_merging(
                &url,
                "session-token",
                Some(Duration::from_secs(2)),
                true,
                CURSOR_MAX_JSON_BYTES,
                4,
                Some(&cached),
            ))
            .expect("an overlapping page must finish the sync");

        assert_eq!(count_cursor_json_events(&text), 4);
        assert!(text.contains("\"conversationId\":\"new\""));
        assert!(text.contains("\"conversationId\":\"same-ms\""));
        assert!(text.contains("\"conversationId\":\"old\""));
        assert_eq!(text.matches("\"conversationId\":\"boundary\"").count(), 1);
        assert!(
            !text.contains("\"conversationId\":\"older\""),
            "events older than the correction window are already represented by the cache: {text}"
        );

        let captured = requests.lock().unwrap();
        assert_eq!(
            captured.len(),
            1,
            "the walk must stop on the overlapping page"
        );
        assert!(
            captured[0].contains("\"startDate\":\"200\""),
            "the correction window must bound the dashboard query: {}",
            captured[0]
        );
    }

    #[test]
    fn test_usage_events_json_keeps_paging_until_the_correction_window() {
        let cached = format!(
            r#"{{"usageEventsDisplay":[{},{}]}}"#,
            json_event("base", "3600100"),
            json_event("cached-old", "80"),
        );
        let (url, requests) = serve_json_pages(vec![
            (
                String::new(),
                json_page(
                    10_000,
                    &[&json_event("a", "3600400"), &json_event("b", "3600300")],
                ),
            ),
            (
                String::new(),
                json_page(
                    10_000,
                    &[&json_event("c", "3600200"), &json_event("ancient", "50")],
                ),
            ),
        ]);

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let text = runtime
            .block_on(fetch_cursor_usage_events_json_merging(
                &url,
                "session-token",
                Some(Duration::from_secs(2)),
                true,
                CURSOR_MAX_JSON_BYTES,
                2,
                Some(&cached),
            ))
            .expect("pages newer than the cache must be collected");

        assert_eq!(count_cursor_json_events(&text), 5);
        for id in ["a", "b", "c", "base", "cached-old"] {
            assert!(
                text.contains(&format!("\"conversationId\":\"{id}\"")),
                "{id} must be in the merged cache: {text}"
            );
        }
        assert!(!text.contains("\"conversationId\":\"ancient\""));

        let captured = requests.lock().unwrap();
        assert_eq!(
            captured.len(),
            2,
            "must walk the fresh page and the overlap"
        );
        assert!(captured[0].contains("\"page\":1"));
        assert!(captured[1].contains("\"page\":2"));
    }

    fn parser_timestamp_for_watermark_fixture(event: &serde_json::Value) -> Option<i64> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.json");
        fs::write(
            &path,
            serde_json::json!({"usageEventsDisplay":[event]}).to_string(),
        )
        .unwrap();
        let parsed = tokscale_core::sessions::cursor::parse_cursor_file(&path);
        assert!(parsed.len() <= 1);
        parsed.first().map(|row| row.timestamp)
    }

    #[test]
    fn test_watermark_parser_row_semantics() {
        let base: serde_json::Value =
            serde_json::from_str(&json_event("valid", "7200000")).unwrap();
        let cases = [
            ("model", None, None),
            ("model", Some(serde_json::Value::Null), None),
            ("model", Some(serde_json::json!("  ")), None),
            ("model", Some(serde_json::json!(42)), None),
            ("model", Some(serde_json::json!(" gpt-5 ")), Some(7_200_000)),
            (
                "conversationId",
                Some(serde_json::Value::Null),
                Some(7_200_000),
            ),
            ("conversationId", Some(serde_json::json!(42)), None),
            ("tokenUsage", None, Some(7_200_000)),
            ("tokenUsage", Some(serde_json::Value::Null), Some(7_200_000)),
            ("tokenUsage", Some(serde_json::json!([])), Some(7_200_000)),
            ("tokenUsage", Some(serde_json::json!("0")), None),
            (
                "tokenUsage",
                Some(serde_json::json!({"inputTokens":"bad"})),
                Some(7_200_000),
            ),
            (
                "chargedCents",
                Some(serde_json::json!({"bad":true})),
                Some(7_200_000),
            ),
            ("timestamp", None, None),
            ("timestamp", Some(serde_json::json!(0)), None),
            ("timestamp", Some(serde_json::json!(-1)), Some(-1)),
            ("timestamp", Some(serde_json::json!(true)), None),
            ("timestamp", Some(serde_json::json!("not-a-time")), None),
            (
                "timestamp",
                Some(serde_json::json!("1970-01-01T02:00:00Z")),
                Some(7_200_000),
            ),
            (
                "timestamp",
                Some(serde_json::json!(7_200_000.5)),
                Some(7_200_000),
            ),
            (
                "timestamp",
                Some(serde_json::json!(i64::MAX)),
                Some(i64::MAX),
            ),
        ];
        for (field, value, expected) in cases {
            let mut event = base.clone();
            if let Some(value) = value {
                event[field] = value;
            } else {
                event.as_object_mut().unwrap().remove(field);
            }
            assert_eq!(
                parser_timestamp_for_watermark_fixture(&event),
                expected,
                "real parser row: {event}"
            );
            assert_eq!(
                tokscale_core::sessions::cursor::parse_cursor_usage_event_timestamp(&event),
                expected,
                "shared row projection: {event}"
            );
            assert_eq!(
                usable_cursor_event_timestamp_ms(&event, 7_500_000),
                expected.filter(|timestamp| *timestamp > 0 && *timestamp <= 7_500_000),
                "sync-state eligibility: {event}"
            );
        }
    }

    #[test]
    fn test_watermark_clock_skew_boundaries() {
        let now = 7_200_000;
        let max_timestamp = now + CURSOR_USAGE_CLOCK_SKEW_MS;
        const { assert!(CURSOR_USAGE_CLOCK_SKEW_MS < CURSOR_USAGE_CORRECTION_WINDOW_MS) };
        for (timestamp, expected) in [
            (1, Some(1)),
            (now, Some(now)),
            (max_timestamp, Some(max_timestamp)),
            (max_timestamp + 1, None),
            (0, None),
            (-1, None),
        ] {
            let event: serde_json::Value =
                serde_json::from_str(&json_event("valid", &timestamp.to_string())).unwrap();
            assert_eq!(
                usage_events_watermark_ms(std::slice::from_ref(&event), max_timestamp),
                expected
            );
            let (fresh, crossed) =
                filter_cursor_usage_page(std::slice::from_ref(&event), Some(now), max_timestamp);
            assert_eq!(crossed, expected.is_some_and(|timestamp| timestamp < now));
            assert_eq!(fresh.is_empty(), crossed);
        }
        let event: serde_json::Value =
            serde_json::from_str(&json_event("all-history", "1")).unwrap();
        let (fresh, crossed) =
            filter_cursor_usage_page(std::slice::from_ref(&event), None, max_timestamp);
        assert_eq!(fresh, [&event]);
        assert!(!crossed);
    }

    #[test]
    fn test_watermark_regression_valid_far_future_cannot_outrank_history() {
        let valid: serde_json::Value =
            serde_json::from_str(&json_event("valid", "7200000")).unwrap();
        let future: serde_json::Value =
            serde_json::from_str(&json_event("future", &i64::MAX.to_string())).unwrap();
        assert_eq!(
            parser_timestamp_for_watermark_fixture(&future),
            Some(i64::MAX)
        );
        assert_eq!(
            usage_events_watermark_ms(&[valid, future], 7_500_000),
            Some(7_200_000),
            "report acceptance alone must not promote an implausible future timestamp"
        );
    }

    #[test]
    fn test_watermark_regression_invalid_future_row_does_not_stall_later_syncs() {
        let latest: serde_json::Value =
            serde_json::from_str(&json_event("latest", "7200000")).unwrap();
        let old: serde_json::Value = serde_json::from_str(&json_event("keep-old", "1000")).unwrap();
        let bad =
            serde_json::json!({"conversationId":"bad-future","timestamp":i64::MAX,"model":" "});
        let valid_future: serde_json::Value =
            serde_json::from_str(&json_event("future-raw", &i64::MAX.to_string())).unwrap();
        assert_eq!(parser_timestamp_for_watermark_fixture(&bad), None);
        let initial = vec![latest, old, bad.clone(), valid_future.clone()];
        let mut cached = serde_json::json!({"usageEventsDisplay":initial}).to_string();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for (id, timestamp, start) in [
            ("first-new", 7_201_000_i64, 3_600_000),
            ("second-new", 7_202_000, 3_601_000),
        ] {
            let body = json_page(
                10_000,
                &[
                    &json_event(id, &timestamp.to_string()),
                    &json_event("outside-window", "3599999"),
                ],
            );
            let (url, requests) = serve_json_pages(vec![(String::new(), body)]);
            cached = runtime
                .block_on(fetch_cursor_usage_events_json_merging(
                    &url,
                    "session-token",
                    Some(Duration::from_secs(2)),
                    true,
                    CURSOR_MAX_JSON_BYTES,
                    2,
                    Some(&cached),
                ))
                .expect("normal usage must still advance the cached history");
            let rows = cached_usage_events(Some(&cached));
            assert!(
                rows.iter().any(|row| row["conversationId"] == id),
                "future cache rows must not hide {id}"
            );
            assert!(
                rows.contains(&bad) && rows.contains(&valid_future),
                "keep raw history without using it as a watermark"
            );
            assert!(rows.iter().any(|row| row["conversationId"] == "keep-old"));
            let captured = requests.lock().unwrap();
            assert_eq!(captured.len(), 1);
            assert!(
                captured[0].contains(&format!("\"startDate\":\"{start}\"")),
                "query must use the latest usable event: {}",
                captured[0]
            );
        }
        assert!(cached.contains("first-new") && cached.contains("second-new"));
    }

    #[test]
    fn test_watermark_regression_all_invalid_cache_uses_full_fallback() {
        let invalid = vec![
            serde_json::json!({"conversationId":"bad-model","timestamp":i64::MAX,"model":null}),
            serde_json::json!({"conversationId":"bad-shape","timestamp":"7200000","model":"gpt-5","tokenUsage":"bad"}),
            serde_json::json!({"conversationId":"bad-time","timestamp":"invalid","model":"gpt-5"}),
        ];
        for row in &invalid {
            assert_eq!(parser_timestamp_for_watermark_fixture(row), None);
        }
        let cached = serde_json::json!({"usageEventsDisplay":invalid}).to_string();
        let (url, requests) = serve_json_pages(vec![(
            String::new(),
            json_page(1, &[&json_event("recovered", "7200000")]),
        )]);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let text = runtime
            .block_on(fetch_cursor_usage_events_json_merging(
                &url,
                "session-token",
                Some(Duration::from_secs(2)),
                true,
                CURSOR_MAX_JSON_BYTES,
                500,
                Some(&cached),
            ))
            .expect("an unusable cache must fall back to a complete fetch");
        let rows = cached_usage_events(Some(&text));
        assert!(rows.iter().any(|row| row["conversationId"] == "recovered"));
        for row in invalid {
            assert!(
                rows.contains(&row),
                "watermark fallback must not discard cached rows"
            );
        }
        let captured = requests.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert!(
            captured[0].contains("\"startDate\":\"0\""),
            "no usable watermark means a full-history request: {}",
            captured[0]
        );
    }

    #[test]
    fn test_watermark_regression_invalid_old_row_cannot_stop_pagination() {
        let cached = format!(
            r#"{{"usageEventsDisplay":[{},{}]}}"#,
            json_event("latest", "7200000"),
            json_event("keep-old", "1000")
        );
        let invalid =
            serde_json::json!({"conversationId":"unusable-old","timestamp":"1","model":""});
        assert_eq!(parser_timestamp_for_watermark_fixture(&invalid), None);
        let (url, requests) = serve_json_pages(vec![
            (
                String::new(),
                json_page(
                    10_000,
                    &[&json_event("new-a", "7201000"), &invalid.to_string()],
                ),
            ),
            (
                String::new(),
                json_page(
                    10_000,
                    &[
                        &json_event("new-b", "7202000"),
                        &json_event("old-boundary", "3599999"),
                    ],
                ),
            ),
        ]);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let text = runtime
            .block_on(fetch_cursor_usage_events_json_merging(
                &url,
                "session-token",
                Some(Duration::from_secs(2)),
                true,
                CURSOR_MAX_JSON_BYTES,
                2,
                Some(&cached),
            ))
            .expect("an unusable row must not hide the next valid page");
        let rows = cached_usage_events(Some(&text));
        assert!(
            rows.iter().any(|row| row["conversationId"] == "new-b"),
            "invalid ordering evidence must not stop paging"
        );
        assert!(rows.contains(&invalid), "unknown/unusable rows stay raw");
        assert!(rows.iter().any(|row| row["conversationId"] == "keep-old"));
        assert_eq!(requests.lock().unwrap().len(), 2);
    }

    fn cursor_identity_token_fields() -> [&'static str; 4] {
        [
            "inputTokens",
            "outputTokens",
            "cacheReadTokens",
            "cacheWriteTokens",
        ]
    }

    fn cursor_identity_count_cases() -> Vec<(&'static str, Option<serde_json::Value>, i64)> {
        vec![
            ("omitted", None, 0),
            ("null", Some(serde_json::Value::Null), 0),
            ("zero", Some(serde_json::json!(0)), 0),
            ("negative", Some(serde_json::json!(-7)), 0),
            ("negative-float", Some(serde_json::json!(-7.75)), 0),
            ("negative-string", Some(serde_json::json!("-7.75")), 0),
            ("fraction", Some(serde_json::json!(0.75)), 0),
            ("integer", Some(serde_json::json!(10)), 10),
            ("float", Some(serde_json::json!(10.75)), 10),
            ("string", Some(serde_json::json!(" 10.75 ")), 10),
            (
                "malformed-string",
                Some(serde_json::json!("not-a-number")),
                0,
            ),
            ("boolean", Some(serde_json::json!(true)), 0),
            ("array", Some(serde_json::json!([99])), 0),
            ("object", Some(serde_json::json!({"not-a-count":99})), 0),
            (
                "max-minus-one",
                Some(serde_json::json!(i64::MAX - 1)),
                i64::MAX - 1,
            ),
            ("max", Some(serde_json::json!(i64::MAX)), i64::MAX),
            (
                "unsigned-overflow",
                Some(serde_json::json!(u64::MAX)),
                i64::MAX,
            ),
            ("float-overflow", Some(serde_json::json!(1e300)), i64::MAX),
            (
                "string-overflow",
                Some(serde_json::json!("9223372036854775808")),
                i64::MAX,
            ),
            ("infinity", Some(serde_json::json!("inf")), i64::MAX),
            ("huge-exponent", Some(serde_json::json!("1e1000")), i64::MAX),
            ("negative-infinity", Some(serde_json::json!("-inf")), 0),
            ("negative-overflow", Some(serde_json::json!("-1e1000")), 0),
            ("nan", Some(serde_json::json!("NaN")), 0),
        ]
    }

    type CursorIdentityContainerCase = (&'static str, Option<serde_json::Value>, Option<[i64; 4]>);

    fn cursor_identity_container_cases() -> Vec<CursorIdentityContainerCase> {
        vec![
            ("omitted", None, Some([0; 4])),
            ("null", Some(serde_json::Value::Null), Some([0; 4])),
            ("empty-map", Some(serde_json::json!({})), Some([0; 4])),
            ("empty-sequence", Some(serde_json::json!([])), Some([0; 4])),
            (
                "short-sequence",
                Some(serde_json::json!([1])),
                Some([1, 0, 0, 0]),
            ),
            (
                "full-sequence",
                Some(serde_json::json!([1, 2, 3, 4, 5])),
                Some([1, 2, 3, 4]),
            ),
            (
                "null-sequence",
                Some(serde_json::json!([null, null, null, null, null])),
                Some([0; 4]),
            ),
            (
                "long-sequence",
                Some(serde_json::json!([1, 2, 3, 4, 5, 6])),
                None,
            ),
            ("string", Some(serde_json::json!("0")), None),
            ("number", Some(serde_json::json!(0)), None),
            ("boolean", Some(serde_json::json!(false)), None),
        ]
    }

    fn cursor_identity_fixture(usage: Option<serde_json::Value>) -> serde_json::Value {
        let mut event = serde_json::json!({"conversationId":"same-request","timestamp":"7200000","model":"gpt-5","chargedCents":9});
        if let Some(usage) = usage {
            event["tokenUsage"] = usage;
        }
        event
    }

    fn cursor_identity_canonical_fixture(counts: [i64; 4]) -> serde_json::Value {
        let usage = cursor_identity_token_fields()
            .into_iter()
            .zip(counts)
            .map(|(key, value)| (key.to_string(), serde_json::json!(value)))
            .collect::<serde_json::Map<_, _>>();
        cursor_identity_fixture(Some(serde_json::Value::Object(usage)))
    }

    fn parser_counts_for_identity_fixture(event: &serde_json::Value) -> Option<[i64; 4]> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.json");
        fs::write(
            &path,
            serde_json::json!({"usageEventsDisplay":[event]}).to_string(),
        )
        .unwrap();
        let parsed = tokscale_core::sessions::cursor::parse_cursor_file(&path);
        assert!(parsed.len() <= 1);
        parsed.first().map(|row| {
            [
                row.tokens.input,
                row.tokens.output,
                row.tokens.cache_read,
                row.tokens.cache_write,
            ]
        })
    }

    #[test]
    fn test_cursor_parser_count_edge_semantics() {
        for (index, field) in cursor_identity_token_fields().into_iter().enumerate() {
            for (name, value, expected) in cursor_identity_count_cases() {
                let mut event = cursor_identity_canonical_fixture([0; 4]);
                if let Some(value) = value {
                    event["tokenUsage"][field] = value;
                } else {
                    event["tokenUsage"].as_object_mut().unwrap().remove(field);
                }
                let mut counts = [0; 4];
                counts[index] = expected;
                assert_eq!(
                    parser_counts_for_identity_fixture(&event),
                    Some(counts),
                    "real parser: {field}/{name}"
                );
            }
        }
        for (name, value, expected) in cursor_identity_container_cases() {
            let actual = parser_counts_for_identity_fixture(&cursor_identity_fixture(value));
            eprintln!("real parser tokenUsage {name}: {actual:?}");
            assert_eq!(actual, expected, "real parser container: {name}");
        }
    }

    #[test]
    fn test_cursor_count_identities_match_full_parser_semantics() {
        for (index, field) in cursor_identity_token_fields().into_iter().enumerate() {
            let mut seen = Vec::new();
            for (name, value, expected) in cursor_identity_count_cases() {
                let mut event = cursor_identity_canonical_fixture([0; 4]);
                if let Some(value) = value {
                    event["tokenUsage"][field] = value;
                } else {
                    event["tokenUsage"].as_object_mut().unwrap().remove(field);
                }
                let mut counts = [0; 4];
                counts[index] = expected;
                assert_eq!(parser_counts_for_identity_fixture(&event), Some(counts));
                let canonical = cursor_identity_canonical_fixture(counts);
                let merged = merge_cursor_usage_events(vec![canonical], vec![event.clone()]);
                assert_eq!(
                    merged.len(),
                    1,
                    "equivalent parser counts must merge: {field}/{name}"
                );
                assert_eq!(
                    merged[0], event,
                    "keep the raw incoming row: {field}/{name}"
                );
                let identity = cursor_usage_event_identity(&event);
                for (previous_count, previous_identity) in &seen {
                    assert_eq!(
                        identity == *previous_identity,
                        expected == *previous_count,
                        "only equivalent counts may merge: {field}/{name}"
                    );
                }
                seen.push((expected, identity));
            }
        }
    }

    #[test]
    fn test_cursor_count_container_identities_match_parser_acceptance() {
        for (name, value, expected) in cursor_identity_container_cases() {
            let event = cursor_identity_fixture(value);
            assert_eq!(parser_counts_for_identity_fixture(&event), expected);
            if let Some(counts) = expected {
                let merged = merge_cursor_usage_events(
                    vec![cursor_identity_canonical_fixture(counts)],
                    vec![event.clone()],
                );
                assert_eq!(
                    merged.len(),
                    1,
                    "accepted equivalent tokenUsage must merge: {name}"
                );
                assert_eq!(merged[0], event, "raw tokenUsage must survive: {name}");
            } else {
                assert_eq!(
                    merge_cursor_usage_events(
                        vec![cursor_identity_canonical_fixture([0; 4])],
                        vec![event]
                    )
                    .len(),
                    2,
                    "rejected tokenUsage must not become valid zeros: {name}"
                );
            }
        }
    }

    #[test]
    fn test_cursor_count_defaults_preserve_unknown_fields_and_rejected_rows() {
        let zero = cursor_identity_canonical_fixture([0; 4]);
        let mut future_null =
            cursor_identity_fixture(Some(serde_json::json!({"futureCount":null})));
        let mut future_zero = future_null.clone();
        future_zero["tokenUsage"]["futureCount"] = serde_json::json!(0);
        let mut nested_null = cursor_identity_fixture(None);
        nested_null["metadata"] = serde_json::json!({"inputTokens":null});
        let mut nested_zero = nested_null.clone();
        nested_zero["metadata"]["inputTokens"] = serde_json::json!(0);
        let mut other_session = cursor_identity_fixture(None);
        other_session["conversationId"] = serde_json::json!("other-request");
        future_null["chargedCents"] = serde_json::json!(11);
        let valid = vec![
            zero,
            future_null,
            future_zero,
            nested_null,
            nested_zero,
            other_session,
        ];
        for row in &valid {
            assert_eq!(parser_counts_for_identity_fixture(row), Some([0; 4]));
        }
        let rejected: Vec<_> = cursor_identity_container_cases()
            .into_iter()
            .filter(|(_, _, counts)| counts.is_none())
            .map(|(_, value, _)| cursor_identity_fixture(value))
            .collect();
        for row in &rejected {
            assert_eq!(parser_counts_for_identity_fixture(row), None);
        }
        let expected = valid.len() + rejected.len();
        let merged = merge_cursor_usage_events(valid.clone(), rejected.clone());
        assert_eq!(
            merged.len(),
            expected,
            "unknown fields and rejected shapes must not be erased"
        );
        for row in valid.iter().chain(rejected.iter()) {
            assert!(merged.contains(row), "stored rows must remain raw");
        }
    }

    fn parser_key_for_identity_contract(event: &serde_json::Value) -> Option<serde_json::Value> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.json");
        fs::write(
            &path,
            serde_json::json!({"usageEventsDisplay":[event]}).to_string(),
        )
        .unwrap();
        let rows = tokscale_core::sessions::cursor::parse_cursor_file(&path);
        assert!(rows.len() <= 1);
        rows.first().map(|row| serde_json::json!({
            "session":row.session_id,"model":row.model_id,"provider":row.provider_id,
            "timestamp":row.timestamp,"tokens":[row.tokens.input,row.tokens.output,row.tokens.cache_read,row.tokens.cache_write]
        }))
    }

    fn assert_cursor_identity_contract_group(name: &str, rows: &[serde_json::Value]) {
        let expected = parser_key_for_identity_contract(&rows[0]).expect("accepted fixture");
        for row in rows {
            assert_eq!(
                parser_key_for_identity_contract(row),
                Some(expected.clone()),
                "parser: {name}: {row}"
            );
            for previous in rows {
                let merged = merge_cursor_usage_events(vec![previous.clone()], vec![row.clone()]);
                assert_eq!(
                    merged.len(),
                    1,
                    "equivalent {name} must merge: {previous} / {row}"
                );
                assert_eq!(merged[0], *row, "incoming {name} row must stay raw");
            }
        }
    }

    #[test]
    fn test_cursor_identity_regression_session_encodings() {
        let base = cursor_identity_canonical_fixture([10, 20, 30, 40]);
        let mut fallback = Vec::new();
        for value in [
            None,
            Some(serde_json::Value::Null),
            Some(serde_json::json!("")),
            Some(serde_json::json!(" \t\u{2003}")),
        ] {
            let mut row = base.clone();
            if let Some(value) = value {
                row["conversationId"] = value;
            } else {
                row.as_object_mut().unwrap().remove("conversationId");
            }
            fallback.push(row);
        }
        assert_cursor_identity_contract_group("synthetic session defaults", &fallback);
        // An explicit real id remains distinct even if it spells the synthetic
        // session label that reporting derives for this account and day.
        let mut explicit = fallback[0].clone();
        explicit["conversationId"] =
            parser_key_for_identity_contract(&fallback[0]).unwrap()["session"].clone();
        assert_eq!(
            parser_key_for_identity_contract(&explicit),
            parser_key_for_identity_contract(&fallback[0])
        );
        assert_eq!(
            merge_cursor_usage_events(vec![fallback[0].clone()], vec![explicit]).len(),
            2
        );
        let mut numeric = base.clone();
        numeric["conversationId"] = serde_json::json!(42);
        let mut textual = numeric.clone();
        textual["conversationId"] = serde_json::json!("42");
        assert_eq!(parser_key_for_identity_contract(&numeric), None);
        assert!(parser_key_for_identity_contract(&textual).is_some());
        assert_eq!(
            merge_cursor_usage_events(vec![numeric], vec![textual]).len(),
            2
        );
        let mut padded = base.clone();
        padded["conversationId"] = serde_json::json!(" \tsame-request\u{2003}");
        assert_cursor_identity_contract_group("trimmed explicit session", &[base, padded]);
    }

    #[test]
    fn test_cursor_identity_regression_trimmed_model() {
        let base = cursor_identity_canonical_fixture([10, 20, 30, 40]);
        let mut padded = base.clone();
        padded["model"] = serde_json::json!("\u{2003}gpt-5 \n");
        assert_cursor_identity_contract_group("trimmed model", &[base, padded]);
    }

    #[test]
    fn test_cursor_identity_known_field_contract_matrix() {
        let base = cursor_identity_canonical_fixture([10, 20, 30, 40]);
        let mut equivalents = vec![base.clone()];
        for timestamp in [
            serde_json::json!(7_200_000),
            serde_json::json!(7_200_000.75),
            serde_json::json!(" 7200000 "),
            serde_json::json!("1970-01-01T03:00:00+01:00"),
        ] {
            let mut row = base.clone();
            row["timestamp"] = timestamp;
            equivalents.push(row);
        }
        let mut numeric = base.clone();
        numeric["tokenUsage"] = serde_json::json!({"inputTokens":" 10.75 ","outputTokens":20.5,"cacheReadTokens":"3e1","cacheWriteTokens":40,"totalCents":"$1,234.50"});
        equivalents.push(numeric);
        for value in [
            None,
            Some(serde_json::Value::Null),
            Some(serde_json::json!("$1,234.50")),
            Some(serde_json::json!(-1)),
            Some(serde_json::json!("inf")),
            Some(serde_json::json!({"bad":true})),
        ] {
            for field in ["chargedCents", "cursorTokenFee", "totalCents"] {
                let mut row = base.clone();
                let object = if field == "totalCents" {
                    row["tokenUsage"].as_object_mut().unwrap()
                } else {
                    row.as_object_mut().unwrap()
                };
                if let Some(value) = &value {
                    object.insert(field.to_string(), value.clone());
                } else {
                    object.remove(field);
                }
                equivalents.push(row);
            }
        }
        let mut token_sequence = base.clone();
        token_sequence["tokenUsage"] = serde_json::json!([10, 20, 30, 40, 99]);
        equivalents.push(token_sequence);
        equivalents.push(serde_json::json!([
            "same-request",
            7200000,
            "gpt-5",
            99,
            [10, 20, 30, 40, 99]
        ]));
        assert_cursor_identity_contract_group(
            "recognized fields including mutable billing",
            &equivalents,
        );

        for timestamp in [-1, i64::MAX] {
            let mut number = base.clone();
            number["timestamp"] = serde_json::json!(timestamp);
            let mut text = number.clone();
            text["timestamp"] = serde_json::json!(timestamp.to_string());
            assert_cursor_identity_contract_group("accepted timestamp extremes", &[number, text]);
        }

        let zero = cursor_identity_canonical_fixture([0; 4]);
        let mut defaults = vec![
            zero.clone(),
            serde_json::json!(["same-request", 7200000, "gpt-5"]),
        ];
        for (_, usage, counts) in cursor_identity_container_cases() {
            if counts == Some([0; 4]) {
                defaults.push(cursor_identity_fixture(usage));
            }
        }
        let mut malformed_counts = zero.clone();
        malformed_counts["tokenUsage"] = serde_json::json!({"inputTokens":null,"outputTokens":-1,"cacheReadTokens":"bad","cacheWriteTokens":true});
        defaults.push(malformed_counts);
        assert_cursor_identity_contract_group("token defaults and coercion", &defaults);

        let mut distinct = vec![base.clone()];
        for (field, value) in [
            ("conversationId", serde_json::json!("other-request")),
            ("model", serde_json::json!("gpt-5-mini")),
            ("model", serde_json::json!("GPT-5")),
            ("model", serde_json::json!("gpt- 5")),
            ("timestamp", serde_json::json!(7200001)),
        ] {
            let mut row = base.clone();
            row[field] = value;
            distinct.push(row);
        }
        for field in cursor_identity_token_fields() {
            let mut row = base.clone();
            row["tokenUsage"][field] = serde_json::json!(99);
            distinct.push(row);
        }
        for (i, row) in distinct.iter().enumerate() {
            let parsed = parser_key_for_identity_contract(row).unwrap();
            for previous in &distinct[..i] {
                assert_ne!(
                    Some(parsed.clone()),
                    parser_key_for_identity_contract(previous)
                );
                assert_eq!(
                    merge_cursor_usage_events(vec![previous.clone()], vec![row.clone()]).len(),
                    2,
                    "distinct event must survive: {row}"
                );
            }
        }
    }

    #[test]
    fn test_cursor_merge_preserves_unknown_fields_and_rejected_rows() {
        let base = cursor_identity_canonical_fixture([10, 20, 30, 40]);
        let mut unknown = vec![base.clone()];
        for (nested, key, value) in [
            (false, "metadata", serde_json::json!({"model":" gpt-5 "})),
            (false, "metadata", serde_json::json!({"model":"gpt-5"})),
            (true, "futureCount", serde_json::Value::Null),
            (true, "futureCount", serde_json::json!(0)),
            (true, "nested", serde_json::json!({"totalCents":1})),
            (true, "nested", serde_json::json!({"totalCents":2})),
        ] {
            let mut row = base.clone();
            if nested {
                row["tokenUsage"][key] = value;
            } else {
                row[key] = value;
            }
            assert_eq!(
                parser_key_for_identity_contract(&row),
                parser_key_for_identity_contract(&base)
            );
            if nested {
                let sequence =
                    serde_json::json!(["same-request", 7200000, "gpt-5", 9, row["tokenUsage"]]);
                assert_cursor_identity_contract_group(
                    "positional token extensions",
                    &[row.clone(), sequence],
                );
            }
            unknown.push(row);
        }
        let mut rejected = vec![
            serde_json::Value::Null,
            serde_json::json!(42),
            serde_json::json!("row"),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!(["same-request", 7200000, "gpt-5", 1, {}, "extra"]),
        ];
        for (field, values) in [
            (
                "model",
                vec![
                    None,
                    Some(serde_json::Value::Null),
                    Some(serde_json::json!(" ")),
                    Some(serde_json::json!(42)),
                ],
            ),
            (
                "conversationId",
                vec![
                    Some(serde_json::json!(42)),
                    Some(serde_json::json!(true)),
                    Some(serde_json::json!([])),
                ],
            ),
            (
                "timestamp",
                vec![
                    None,
                    Some(serde_json::Value::Null),
                    Some(serde_json::json!(0)),
                    Some(serde_json::json!("0")),
                    Some(serde_json::json!(true)),
                    Some(serde_json::json!("bad")),
                ],
            ),
            (
                "tokenUsage",
                vec![
                    Some(serde_json::json!("0")),
                    Some(serde_json::json!(0)),
                    Some(serde_json::json!(true)),
                    Some(serde_json::json!([1, 2, 3, 4, 5, 6])),
                ],
            ),
        ] {
            for value in values {
                let mut row = base.clone();
                if let Some(value) = value {
                    row[field] = value;
                } else {
                    row.as_object_mut().unwrap().remove(field);
                }
                assert_eq!(
                    parser_key_for_identity_contract(&row),
                    None,
                    "rejected row: {row}"
                );
                rejected.push(row);
            }
        }
        let mut invalid_numeric_timestamp = base.clone();
        invalid_numeric_timestamp["model"] = serde_json::json!(42);
        invalid_numeric_timestamp["timestamp"] = serde_json::json!(7200000);
        rejected.push(invalid_numeric_timestamp);
        let expected = unknown.len() + rejected.len();
        let all = unknown.into_iter().chain(rejected).collect::<Vec<_>>();
        let merged = merge_cursor_usage_events(all.clone(), Vec::new());
        assert_eq!(
            merged.len(),
            expected,
            "unknown fields, rejected representations, and raw multiplicity must survive"
        );
        for row in all {
            assert!(merged.contains(&row), "raw row retained: {row}");
        }
    }

    #[test]
    fn test_regression_cursor_identity_coerces_parser_accepted_token_counts() {
        let integer = serde_json::json!({
            "conversationId":"boundary", "timestamp":"7200000", "model":"gpt-5",
            "chargedCents":1,
            "tokenUsage":{"inputTokens":10,"outputTokens":20,"cacheReadTokens":30,"cacheWriteTokens":40,"totalCents":1}
        });
        let mut floats = integer.clone();
        floats["tokenUsage"] = serde_json::json!({"inputTokens":10.0,"outputTokens":20.5,"cacheReadTokens":30.0,"cacheWriteTokens":40.75,"totalCents":2});
        let mut strings = integer.clone();
        strings["tokenUsage"] = serde_json::json!({"inputTokens":" 10 ","outputTokens":"20.5","cacheReadTokens":"3e1","cacheWriteTokens":"40.75","totalCents":9});
        strings["chargedCents"] = serde_json::json!(9);
        let parsed_cache = tempfile::tempdir().unwrap();
        let parsed_path = parsed_cache.path().join("usage.json");
        for variant in [&integer, &floats, &strings] {
            fs::write(
                &parsed_path,
                serde_json::json!({"usageEventsDisplay":[variant]}).to_string(),
            )
            .unwrap();
            let messages = tokscale_core::sessions::cursor::parse_cursor_file(&parsed_path);
            assert_eq!(messages.len(), 1);
            let tokens = &messages[0].tokens;
            assert_eq!(
                (
                    tokens.input,
                    tokens.output,
                    tokens.cache_read,
                    tokens.cache_write
                ),
                (10, 20, 30, 40)
            );
        }
        assert_eq!(
            cursor_usage_event_identity(&integer),
            cursor_usage_event_identity(&floats)
        );
        let merged = merge_cursor_usage_events(vec![integer.clone()], vec![strings.clone()]);
        assert_eq!(
            merged.len(),
            1,
            "equivalent token encodings must not duplicate usage"
        );
        assert_eq!(
            merged[0], strings,
            "the raw incoming row must remain intact"
        );

        let mut distinct_count = integer.clone();
        distinct_count["tokenUsage"]["inputTokens"] = serde_json::json!(11);
        let mut distinct_session = integer.clone();
        distinct_session["conversationId"] = serde_json::json!("other-session");
        let mut unknown_a = integer.clone();
        unknown_a["extra"] = serde_json::json!({"chargedCents":1,"timestamp":"7200000"});
        let mut unknown_b = unknown_a.clone();
        unknown_b["extra"]["chargedCents"] = serde_json::json!(2);
        let mut unknown_token = integer.clone();
        unknown_token["tokenUsage"]["futureCount"] = serde_json::json!("10");
        let mut unknown_token_number = unknown_token.clone();
        unknown_token_number["tokenUsage"]["futureCount"] = serde_json::json!(10);
        assert_eq!(
            merge_cursor_usage_events(
                vec![
                    integer,
                    distinct_count,
                    distinct_session,
                    unknown_a,
                    unknown_token
                ],
                vec![unknown_b, unknown_token_number],
            )
            .len(),
            4,
            "the complete overlap must replace cached variants and keep incoming multiplicity"
        );
    }

    #[test]
    fn test_regression_cursor_refreshes_billing_older_than_the_newest_cached_event() {
        let newest = json_event("latest", "7200000");
        let corrected = json_event("corrected", "5400000");
        let archived = json_event("archived", "1000");
        let cached = format!(r#"{{"usageEventsDisplay":[{newest},{corrected},{archived}]}}"#);
        let mut updated: serde_json::Value = serde_json::from_str(&corrected).unwrap();
        updated["chargedCents"] = serde_json::json!(9);
        updated["tokenUsage"]["totalCents"] = serde_json::json!(9);
        let updated = updated.to_string();
        let body = json_page(
            10_000,
            &[
                &json_event("new", "7201000"),
                &newest,
                &updated,
                &json_event("outside-window", "3599999"),
            ],
        );
        let (url, requests) = serve_json_pages(vec![(String::new(), body)]);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let text = runtime
            .block_on(fetch_cursor_usage_events_json_merging(
                &url,
                "session-token",
                Some(Duration::from_secs(2)),
                true,
                CURSOR_MAX_JSON_BYTES,
                4,
                Some(&cached),
            ))
            .expect("the bounded correction overlap must complete");
        let stored = cached_usage_events(Some(&text));
        let revised: Vec<_> = stored
            .iter()
            .filter(|e| e["conversationId"] == "corrected")
            .collect();
        assert_eq!(revised.len(), 1);
        assert_eq!(
            revised[0]["chargedCents"],
            serde_json::json!(9),
            "a correction thirty minutes behind the newest row must replace its billing"
        );
        assert_eq!(revised[0]["tokenUsage"]["totalCents"], serde_json::json!(9));
        assert_eq!(stored.len(), 4);
        assert!(
            stored.iter().any(|e| e["conversationId"] == "archived"),
            "old cached history must survive"
        );
        assert!(!stored
            .iter()
            .any(|e| e["conversationId"] == "outside-window"));
        let captured = requests.lock().unwrap();
        assert_eq!(
            captured.len(),
            1,
            "stop at the overlap rather than walking all history"
        );
        assert!(
            captured[0].contains("\"startDate\":\"3600000\""),
            "request only the hour before the newest cached event: {}",
            captured[0]
        );
    }

    #[test]
    fn test_merge_cursor_usage_events_replaces_boundary_row_when_billing_changes() {
        let stored =
            serde_json::from_str::<serde_json::Value>(&json_event("boundary", "200")).unwrap();
        let mut updated = stored.clone();
        updated["chargedCents"] = serde_json::json!(9);
        updated["tokenUsage"]["totalCents"] = serde_json::json!(9);

        let merged = merge_cursor_usage_events(vec![stored], vec![updated]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["chargedCents"], serde_json::json!(9));
    }

    #[test]
    fn test_merge_cursor_usage_events_preserves_identical_event_multiplicity() {
        let stored =
            serde_json::from_str::<serde_json::Value>(&json_event("duplicate", "200")).unwrap();
        let mut corrected = stored.clone();
        corrected["chargedCents"] = serde_json::json!(9);

        let merged = merge_cursor_usage_events(
            vec![stored.clone(), stored.clone()],
            vec![corrected.clone(), corrected.clone()],
        );
        assert_eq!(merged, vec![corrected.clone(), corrected.clone()]);

        let corrected_snapshot =
            merge_cursor_usage_events(vec![stored.clone(), stored], vec![corrected.clone()]);
        assert_eq!(corrected_snapshot, vec![corrected]);
    }

    #[test]
    fn test_merge_cursor_usage_events_ignores_mutable_unknown_billing_fields() {
        let mut stored =
            serde_json::from_str::<serde_json::Value>(&json_event("corrected", "200")).unwrap();
        stored["isChargeable"] = serde_json::json!(false);
        stored["usageBasedCosts"] = serde_json::json!({"totalCents": 1});
        let mut corrected = stored.clone();
        corrected["chargedCents"] = serde_json::json!(9);
        corrected["isChargeable"] = serde_json::json!(true);
        corrected["usageBasedCosts"] = serde_json::json!({"totalCents": 9});

        let merged = merge_cursor_usage_events(vec![stored], vec![corrected.clone()]);
        assert_eq!(merged, vec![corrected]);
    }

    #[test]
    fn test_merge_cursor_usage_events_collapses_equivalent_timestamps() {
        let string_ts =
            serde_json::from_str::<serde_json::Value>(&json_event("boundary", "200")).unwrap();
        let mut number_ts = string_ts.clone();
        number_ts["timestamp"] = serde_json::json!(200);
        let mut rfc3339 = string_ts.clone();
        rfc3339["timestamp"] = serde_json::json!("1970-01-01T00:00:00.200Z");
        rfc3339["chargedCents"] = serde_json::json!(4);

        assert_eq!(
            cursor_usage_event_identity(&string_ts),
            cursor_usage_event_identity(&number_ts)
        );
        assert_eq!(
            cursor_usage_event_identity(&string_ts),
            cursor_usage_event_identity(&rfc3339)
        );
        let merged = merge_cursor_usage_events(vec![number_ts], vec![rfc3339]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["chargedCents"], serde_json::json!(4));

        let mut unread_a =
            serde_json::from_str::<serde_json::Value>(&json_event("boundary", "200")).unwrap();
        unread_a["timestamp"] = serde_json::json!("not-a-time");
        let mut unread_b = unread_a.clone();
        unread_b["timestamp"] = serde_json::json!("also-not");
        assert_eq!(
            merge_cursor_usage_events(vec![unread_a], vec![unread_b]).len(),
            2
        );
    }

    #[test]
    fn test_usage_events_json_timeout_names_the_budget() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 4096];
                let _ = std::io::Read::read(&mut stream, &mut request);
                std::thread::sleep(Duration::from_secs(3));
                let _ = stream;
            }
        });
        let url = format!("http://{addr}/api/dashboard/get-filtered-usage-events");

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                Some(Duration::from_secs(1)),
                true,
                CURSOR_MAX_JSON_BYTES,
                500,
            ))
            .expect_err("a stalled page must hit the time budget");
        let message = format!("{err}");
        assert!(
            message.contains("1s time budget"),
            "a timeout must name the budget, not only the transport error: {message}"
        );
        assert!(
            message.contains("0 pages"),
            "the first page timing out has not collected a page: {message}"
        );
    }

    #[test]
    fn test_usage_events_json_page_timeout_names_the_request_budget() {
        // Page 1 advertises a huge history so the default explicit budget scales
        // toward 30 minutes. Page 2 then stalls. The failure is the 1s request
        // timeout, and the message must not claim the scaled overall budget ran out.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 8192];
                let _ = std::io::Read::read(&mut stream, &mut request);
                let body = json_page(40_000, &[&json_event("c1", "1788171994001")]);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 8192];
                let _ = std::io::Read::read(&mut stream, &mut request);
                std::thread::sleep(Duration::from_secs(3));
                let _ = stream;
            }
        });
        let url = format!("http://{addr}/api/dashboard/get-filtered-usage-events");

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                &url,
                "session-token",
                Some(Duration::from_secs(1)),
                false,
                CURSOR_MAX_JSON_BYTES,
                500,
            ))
            .expect_err("a stalled later page must time out");
        let message = format!("{err}");
        assert!(
            message.contains("timed out after its 1s time budget"),
            "the stalled request must name its own timeout: {message}"
        );
        assert!(
            !message.contains("1800s"),
            "a single stalled page must not be reported as the scaled overall budget: {message}"
        );
        assert!(
            message.contains("1 page"),
            "page 1 was collected before the stall: {message}"
        );
    }

    #[test]
    fn test_usage_events_json_invalid_url_includes_the_cause() {
        // A closed localhost port can exhaust the 2s budget on Windows before
        // its TCP stack reports refusal. URL parsing fails before any network
        // I/O, but still exercises reqwest's source chain through the fetcher.
        let url = "http://[not-an-ipv6-address]/api/dashboard/get-filtered-usage-events";
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let err = runtime
            .block_on(fetch_cursor_usage_events_json_from(
                url,
                "session-token",
                Some(Duration::from_secs(2)),
                true,
                CURSOR_MAX_JSON_BYTES,
                500,
            ))
            .expect_err("an invalid URL must fail before connecting");
        let message = cursor_sync_error_text(&err);
        assert!(
            message.contains("builder error"),
            "reqwest's outer error must survive: {message}"
        );
        assert!(
            message.contains("invalid IPv6 address"),
            "the source chain must name the URL parser's cause: {message}"
        );
        assert!(
            !message.contains("time budget"),
            "a parser error is not a timeout: {message}"
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
        fs::write(
            cache_dir.join("usage.json"),
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, "active-account")?,
        )?;

        assert!(!cursor_usage_cache_is_fresh_in(
            temp_dir.path(),
            Duration::from_secs(300)
        ));

        fs::write(
            cache_dir.join(format!(
                "usage.{}.json",
                cursor_account_cache_stem("team/account")
            )),
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, "team/account")?,
        )?;
        assert!(cursor_usage_cache_is_fresh_in(
            temp_dir.path(),
            Duration::from_secs(300)
        ));

        Ok(())
    }

    #[test]
    fn test_cursor_expected_cache_paths_keep_sanitized_account_collisions_separate() {
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
        let mut expected = vec![
            cache_dir.join("usage.json"),
            cache_dir.join(format!(
                "usage.{}.json",
                cursor_account_cache_stem("team/account-a")
            )),
            cache_dir.join(format!(
                "usage.{}.json",
                cursor_account_cache_stem("team@account-a")
            )),
        ];
        expected.sort_unstable();
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
            |fetch| {
                let json = match fetch.session_token.as_str() {
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

        // JSON caches and their bounded owner indexes are written together.
        let active_path = cache_dir.join("usage.json");
        let secondary_path = cache_dir.join(format!(
            "usage.{}.json",
            cursor_account_cache_stem("team/account")
        ));
        assert_eq!(
            count_cursor_json_events(&fs::read_to_string(&active_path)?),
            1
        );
        assert_eq!(
            count_cursor_json_events(&fs::read_to_string(&secondary_path)?),
            2
        );
        assert_eq!(
            cursor_cache_owner_hash_from_envelope(&active_path).as_deref(),
            Some(cursor_cache_owner_hash("active-account").as_str())
        );
        assert_eq!(
            cursor_cache_owner_hash_from_envelope(&secondary_path).as_deref(),
            Some(cursor_cache_owner_hash("team/account").as_str())
        );
        // The active account is never duplicated as a per-account file.
        assert!(!cache_dir.join("usage.active-account.json").exists());
        // Legacy CSVs are moved out of the scan path so they can't double-count,
        // but they are archived rather than deleted so history survives.
        assert!(!cache_dir.join("usage.csv").exists());
        assert!(!cache_dir.join("usage.team-account.csv").exists());
        let archived: Vec<_> = fs::read_dir(cache_dir.join("archive"))?
            .filter_map(|entry| entry.ok())
            .flat_map(|entry| fs::read_dir(entry.path()).unwrap())
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
    fn test_sync_cleanup_keeps_refreshed_secondary_with_colliding_active_id() -> Result<()> {
        for active_fails in [false, true] {
            let temp = TempDir::new()?;
            let active_id = "team/account-a";
            let secondary_id = "team@account-a";
            let accounts = [active_id, secondary_id]
                .into_iter()
                .map(|id| {
                    (
                        id.to_string(),
                        CursorCredentials {
                            session_token: id.to_string(),
                            user_id: Some(id.to_string()),
                            created_at: "2026-01-01T00:00:00Z".to_string(),
                            expires_at: None,
                            label: None,
                        },
                    )
                })
                .collect();
            save_credentials_store_in_home(
                temp.path(),
                &CursorCredentialsStore {
                    version: 1,
                    active_account_id: active_id.to_string(),
                    accounts,
                },
            )?;
            let credentials_before = fs::read(cursor_credentials_path(temp.path()))?;
            let cache_dir = cursor_cache_dir(temp.path());
            fs::create_dir_all(&cache_dir)?;
            let active_json =
                String::from_utf8(json_page(1, &[&json_event("active-history", "7200000")]))?;
            let active_json = owned_cursor_cache(&active_json, active_id)?;
            fs::write(cache_dir.join("usage.json"), &active_json)?;
            let secondary_json =
                String::from_utf8(json_page(1, &[&json_event("secondary-history", "7200000")]))?;
            let runtime = tokio::runtime::Runtime::new()?;
            let result = runtime.block_on(sync_cursor_cache_with_fetcher_in_home(
                temp.path(),
                |fetch| {
                    let response = if fetch.session_token == active_id {
                        if active_fails {
                            Err(anyhow::anyhow!("active unavailable"))
                        } else {
                            Ok(active_json.clone())
                        }
                    } else {
                        Ok(secondary_json.clone())
                    };
                    async move { response }
                },
            ));
            assert!(result.synced, "secondary sync must succeed: {result:?}");
            assert_eq!(result.error.is_some(), active_fails);
            let active_path = cache_dir.join("usage.json");
            assert_eq!(
                cursor_cache_owner_hash_from_file(&active_path).as_deref(),
                Some(cursor_cache_owner_hash(active_id).as_str())
            );
            assert!(
                cached_usage_events(fs::read_to_string(&active_path).ok().as_deref())
                    .iter()
                    .any(|row| row["conversationId"] == "active-history")
            );
            assert_eq!(
                fs::read(cursor_credentials_path(temp.path()))?,
                credentials_before
            );
            let secondary_path = cache_dir.join(format!(
                "usage.{}.json",
                cursor_account_cache_stem(secondary_id)
            ));
            assert_eq!(
                cursor_cache_owner_hash_from_file(&secondary_path).as_deref(),
                Some(cursor_cache_owner_hash(secondary_id).as_str()),
                "cleanup must retain the freshly written secondary; active_fails={active_fails}"
            );
            let rows = tokscale_core::sessions::cursor::parse_cursor_file(&secondary_path);
            assert_eq!(rows.len(), 1, "secondary history must remain reportable");
            assert_eq!(rows[0].session_id, "secondary-history");
            assert_eq!(rows[0].tokens.input, 10);
            assert_eq!(rows[0].tokens.output, 5);
        }
        Ok(())
    }

    #[test]
    fn test_sync_correction_windows_and_cached_history_are_account_local() -> Result<()> {
        let temp = TempDir::new()?;
        let cache_dir = cursor_cache_dir(temp.path());
        fs::create_dir_all(&cache_dir)?;
        let mut accounts = HashMap::new();
        let mut endpoints = HashMap::new();
        let mut captured = Vec::new();
        for (id, newest) in [("active", 7_200_000_i64), ("secondary", 14_400_000)] {
            accounts.insert(
                id.to_string(),
                CursorCredentials {
                    session_token: id.to_string(),
                    user_id: Some(id.to_string()),
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    expires_at: None,
                    label: None,
                },
            );
            let latest = json_event(&format!("{id}-latest"), &newest.to_string());
            let previous = json_event(
                &format!("{id}-corrected"),
                &(newest - 1_800_000).to_string(),
            );
            let old = json_event(&format!("{id}-old"), "1");
            let path = if id == "active" {
                cache_dir.join("usage.json")
            } else {
                cache_dir.join(format!(
                    "usage.{}.json",
                    cursor_account_cache_stem("secondary")
                ))
            };
            fs::write(
                path,
                owned_cursor_cache(
                    &format!(r#"{{"usageEventsDisplay":[{latest},{previous},{old}]}}"#),
                    id,
                )?,
            )?;
            let mut update: serde_json::Value = serde_json::from_str(&previous)?;
            update["chargedCents"] = serde_json::json!(9);
            let body = json_page(
                10_000,
                &[
                    &update.to_string(),
                    &json_event(&format!("{id}-outside"), &(newest - 3_600_001).to_string()),
                ],
            );
            let (url, requests) = serve_json_pages(vec![(String::new(), body)]);
            endpoints.insert(id.to_string(), url);
            captured.push((id, newest, requests));
        }
        save_credentials_store_in_home(
            temp.path(),
            &CursorCredentialsStore {
                version: 1,
                active_account_id: "active".to_string(),
                accounts,
            },
        )?;
        let runtime = tokio::runtime::Runtime::new()?;
        let result = runtime.block_on(sync_cursor_cache_with_fetcher_in_home(
            temp.path(),
            |fetch| {
                let url = endpoints[&fetch.session_token].clone();
                async move {
                    fetch_cursor_usage_events_json_merging(
                        &url,
                        &fetch.session_token,
                        Some(Duration::from_secs(2)),
                        true,
                        CURSOR_MAX_JSON_BYTES,
                        2,
                        fetch.existing_cache.as_deref(),
                    )
                    .await
                }
            },
        ));
        assert!(result.synced, "{result:?}");
        assert_eq!(result.rows, 6);
        for (id, newest, requests) in captured {
            let path = if id == "active" {
                cache_dir.join("usage.json")
            } else {
                cache_dir.join(format!(
                    "usage.{}.json",
                    cursor_account_cache_stem("secondary")
                ))
            };
            let stored = cached_usage_events(Some(&fs::read_to_string(path)?));
            assert_eq!(stored.len(), 3, "unreturned cached history must survive");
            assert!(
                stored
                    .iter()
                    .all(|e| e["conversationId"].as_str().unwrap().starts_with(id)),
                "account histories must not mix"
            );
            let revised = stored
                .iter()
                .find(|e| e["conversationId"] == format!("{id}-corrected"))
                .unwrap();
            assert_eq!(revised["chargedCents"], serde_json::json!(9));
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(
                requests[0].contains(&format!("\"startDate\":\"{}\"", newest - 3_600_000)),
                "each query must use its own account's overlap"
            );
        }
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
        let seeded = owned_cursor_cache(
            r#"{"usageEventsDisplay":[{"conversationId":"keep","timestamp":"1","model":"gpt-5","chargedCents":1}]}"#,
            "active-account",
        )?;
        fs::write(cache_dir.join("usage.json"), &seeded)?;

        let runtime = tokio::runtime::Runtime::new()?;
        let result = runtime.block_on(sync_cursor_cache_with_fetcher_in_home(
            temp_dir.path(),
            |_fetch| async move { Ok(r#"{"usageEventsDisplay":[]}"#.to_string()) },
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
        let good = owned_cursor_cache(
            r#"{"usageEventsDisplay":[{"conversationId":"keep","timestamp":"1","model":"gpt-5","chargedCents":1}]}"#,
            "active-account",
        )?;
        fs::write(cache_dir.join("usage.json"), &good)?;
        fs::write(cache_dir.join("usage.active-account.json"), &good)?;

        let runtime = tokio::runtime::Runtime::new()?;
        let result = runtime.block_on(sync_cursor_cache_with_fetcher_in_home(
            temp_dir.path(),
            |_fetch| async move { Err(anyhow::anyhow!("network down")) },
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
        fs::write(
            cache_dir.join("usage.json"),
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, "active-account")?,
        )?;

        // Stale secondary cache.
        let secondary = cache_dir.join(format!(
            "usage.{}.json",
            cursor_account_cache_stem("team/account")
        ));
        fs::write(
            &secondary,
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, "team/account")?,
        )?;
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

        fs::write(
            cache_dir.join("usage.json"),
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, "active-account")?,
        )?;

        let secondary = cache_dir.join(format!(
            "usage.{}.json",
            cursor_account_cache_stem("team/account")
        ));
        fs::write(
            &secondary,
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, "team/account")?,
        )?;
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
        fs::write(
            cache_dir.join(format!(
                "usage.{}.json",
                cursor_account_cache_stem("team/account")
            )),
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, "team/account")?,
        )?;
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
            |fetch| {
                // Secondary deliberately fails to simulate a broken account.
                let result: Result<String> = match fetch.session_token.as_str() {
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
        let marker: CursorSyncAttemptMarker = serde_json::from_str(&fs::read_to_string(
            cache_dir.join(CURSOR_SYNC_ATTEMPT_MARKER),
        )?)?;
        assert_eq!(marker.version, CURSOR_SYNC_ATTEMPT_MARKER_VERSION);
        for account in ["active-account", "team/account"] {
            assert!(
                marker
                    .account_hashes
                    .contains(&cursor_cache_owner_hash(account)),
                "every attempted account must be named: {marker:?}"
            );
        }
        // A legacy unscoped marker keeps its pre-upgrade meaning for a stale
        // secondary, but can never excuse a missing active cache.
        fs::write(cache_dir.join(CURSOR_SYNC_ATTEMPT_MARKER), "")?;
        let now = SystemTime::now();
        assert!(cursor_secondary_attempt_excuses(
            &cache_dir,
            "team/account",
            Duration::from_secs(300),
            now
        ));
        assert!(!cursor_active_attempt_excuses(
            &cache_dir,
            "active-account",
            Duration::from_secs(300),
            now
        ));
        Ok(())
    }

    /// A discovered desktop session whose first sync fails must not add the
    /// network timeout to every later report. The attempt is account-scoped, so
    /// it expires with the freshness window and never covers another account.
    #[test]
    fn test_failed_first_sync_attempt_is_bounded_and_account_scoped() -> Result<()> {
        let home = TempDir::new()?;
        let mut accounts = HashMap::new();
        accounts.insert(
            "desktop-account".to_string(),
            CursorCredentials {
                session_token: "desktop-account%3A%3Atoken".to_string(),
                user_id: Some("desktop-account".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: None,
                label: None,
            },
        );
        save_credentials_store_in_home(
            home.path(),
            &CursorCredentialsStore {
                version: 1,
                active_account_id: "desktop-account".to_string(),
                accounts,
            },
        )?;

        let max_age = Duration::from_secs(300);
        let now = SystemTime::now();
        assert!(
            !cursor_usage_cache_is_fresh_in_at(home.path(), max_age, now),
            "no cache and no attempt must sync"
        );

        let runtime = tokio::runtime::Runtime::new()?;
        let result = runtime.block_on(sync_cursor_cache_with_fetcher_in_home(
            home.path(),
            |_fetch| async move { Err(anyhow::anyhow!("offline")) },
        ));
        assert!(!result.synced);
        let cache_dir = cursor_cache_dir(home.path());
        assert!(
            !cache_dir.join("usage.json").exists(),
            "a failed fetch must not write a cache"
        );
        assert!(
            cursor_usage_cache_is_fresh_in_at(home.path(), max_age, now),
            "a recent failed attempt must suppress an immediate retry"
        );
        assert!(
            !cursor_usage_cache_is_fresh_in_at(home.path(), max_age, SystemTime::now() + max_age),
            "suppression must expire with the freshness window"
        );

        // A newly added account cannot inherit the other account's cooldown.
        let mut store = load_credentials_store_from_home(home.path()).unwrap();
        store.accounts.insert(
            "second-account".to_string(),
            CursorCredentials {
                session_token: "second-account%3A%3Atoken".to_string(),
                user_id: Some("second-account".to_string()),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                expires_at: None,
                label: None,
            },
        );
        store.active_account_id = "second-account".to_string();
        save_credentials_store_in_home(home.path(), &store)?;
        assert!(
            !cursor_usage_cache_is_fresh_in_at(home.path(), max_age, now),
            "an account switch must not reuse another account's attempt"
        );
        Ok(())
    }

    /// Logout is a durable boundary: it persists a non-secret tombstone that
    /// stops implicit desktop discovery, and only explicit login or sync
    /// restores it.
    #[test]
    fn test_logout_persists_desktop_auto_import_opt_out() -> Result<()> {
        for purge in [false, true] {
            let home = TempDir::new()?;
            let mut accounts = HashMap::new();
            accounts.insert(
                "desktop-account".to_string(),
                CursorCredentials {
                    session_token: "desktop-account%3A%3Atoken".to_string(),
                    user_id: Some("desktop-account".to_string()),
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    expires_at: None,
                    label: Some("work".to_string()),
                },
            );
            save_credentials_store_in_home(
                home.path(),
                &CursorCredentialsStore {
                    version: 1,
                    active_account_id: "desktop-account".to_string(),
                    accounts,
                },
            )?;
            let cache_dir = cursor_cache_dir(home.path());
            fs::create_dir_all(&cache_dir)?;
            fs::write(
                cache_dir.join("usage.json"),
                owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, "desktop-account")?,
            )?;

            assert!(!cursor_desktop_auto_import_is_disabled_in_home(home.path()));
            remove_account_in_home(home.path(), "desktop-account", purge)?;
            assert!(
                cursor_desktop_auto_import_is_disabled_in_home(home.path()),
                "logout must persist the opt-out (purge={purge})"
            );
            // The tombstone lives outside cursor-cache, so a purge keeps it.
            assert!(!cursor_desktop_auto_import_disabled_path(home.path()).starts_with(&cache_dir));

            clear_cursor_desktop_auto_import_in_home(home.path())?;
            assert!(!cursor_desktop_auto_import_is_disabled_in_home(home.path()));

            disable_cursor_desktop_auto_import_in_home(home.path())?;
            remove_all_accounts_in_home(home.path(), purge)?;
            assert!(
                cursor_desktop_auto_import_is_disabled_in_home(home.path()),
                "logout --all must persist the opt-out (purge={purge})"
            );
        }
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
        let old_stem = cursor_account_cache_stem(old_id);
        let new_stem = cursor_account_cache_stem(new_id);

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
        let old_stem = cursor_account_cache_stem(old_id);

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
        let old_stem = cursor_account_cache_stem(old_id);
        let new_stem = cursor_account_cache_stem(new_id);

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

    #[test]
    fn test_layout_preserves_unverified_json_and_csv_before_switch() -> Result<()> {
        let temp = TempDir::new()?;
        setup_two_account_store(temp.path())?;
        let store = load_credentials_store_from_home(temp.path()).unwrap();
        let dir = cursor_cache_dir(temp.path());
        fs::create_dir_all(&dir)?;
        let files = [
            "usage.json",
            "usage.csv",
            "usage.team-account.json",
            "usage.team-account.csv",
        ];
        for name in files {
            fs::write(dir.join(name), format!("unverified-{name}"))?;
        }
        prepare_cursor_cache_layout(&dir, &store)?;
        for name in files {
            assert!(!dir.join(name).exists());
        }
        let mut saved = Vec::new();
        for entry in fs::read_dir(dir.join("archive"))? {
            for file in fs::read_dir(entry?.path())? {
                let path = file?.path();
                saved.push((
                    path.file_name().unwrap().to_string_lossy().to_string(),
                    fs::read_to_string(path)?,
                ));
            }
        }
        assert_eq!(saved.len(), files.len());
        for name in files {
            assert!(saved.contains(&(name.to_string(), format!("unverified-{name}"))));
        }
        set_active_account_in_home(temp.path(), "team/account")?;
        assert_eq!(
            load_credentials_store_from_home(temp.path())
                .unwrap()
                .active_account_id,
            "team/account"
        );
        assert!(
            !dir.join("usage.json").exists(),
            "unknown history must not be assigned to the selected account"
        );
        Ok(())
    }

    #[test]
    fn test_reconcile_colliding_ids_without_relevant_history() -> Result<()> {
        let temp = TempDir::new()?;
        let missing = temp.path().join("missing");
        reconcile_cache_files_in_dir(&missing, "team/account-a", "team@account-a")?;
        assert!(!missing.exists());
        reconcile_cache_files_in_dir(temp.path(), "team/account-a", "team@account-a")?;
        fs::write(
            temp.path().join("usage.unrelated.json"),
            "unrelated history",
        )?;
        reconcile_cache_files_in_dir(temp.path(), "team/account-a", "team@account-a")?;
        assert_eq!(
            fs::read_to_string(temp.path().join("usage.unrelated.json"))?,
            "unrelated history"
        );
        assert_eq!(fs::read_dir(temp.path())?.count(), 1);
        Ok(())
    }
    #[test]
    fn test_colliding_expected_paths_keep_every_distinct_account() -> Result<()> {
        let prefix = "x".repeat(80);
        let cases = [
            (
                "punctuation",
                "team/account-a".to_string(),
                "team@account-a".to_string(),
            ),
            ("case", "Team_A".to_string(), "team_a".to_string()),
            ("whitespace", " team-a ".to_string(), "team-a".to_string()),
            ("truncation", format!("{prefix}-a"), format!("{prefix}-b")),
            ("empty fallback", "/".to_string(), "@".to_string()),
        ];
        let mut failures = Vec::new();
        for (kind, first, second) in cases {
            let temp = TempDir::new()?;
            let accounts = ["unrelated-active".to_string(), first, second]
                .into_iter()
                .map(|id| {
                    let credentials = CursorCredentials {
                        session_token: format!("fixture-{id}"),
                        user_id: Some(id.clone()),
                        created_at: "2026-01-01T00:00:00Z".to_string(),
                        expires_at: None,
                        label: None,
                    };
                    (id, credentials)
                })
                .collect();
            save_credentials_store_in_home(
                temp.path(),
                &CursorCredentialsStore {
                    version: 1,
                    active_account_id: "unrelated-active".to_string(),
                    accounts,
                },
            )?;
            let paths = expected_cursor_usage_cache_paths_in(temp.path());
            if paths.len() != 3 {
                failures.push(format!("{kind}: {paths:?}"));
            }
        }
        assert!(
            failures.is_empty(),
            "freshness checks must retain an independent path for every account: {}",
            failures.join("; ")
        );
        Ok(())
    }

    #[test]
    fn test_colliding_secondary_accounts_never_share_fetch_history_or_saved_rows() -> Result<()> {
        let temp = TempDir::new()?;
        let mut accounts = HashMap::new();
        let mut endpoints = HashMap::new();
        let mut captured = Vec::new();
        for (id, token) in [
            ("active", "active"),
            ("team/account-a", "team-a"),
            ("team@account-a", "team-b"),
        ] {
            accounts.insert(
                id.to_string(),
                CursorCredentials {
                    session_token: token.to_string(),
                    user_id: Some(id.to_string()),
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    expires_at: None,
                    label: None,
                },
            );
            let body = json_page(
                2,
                &[
                    &json_event(&format!("{token}-new"), "7200000"),
                    &json_event(&format!("{token}-old"), "1"),
                ],
            );
            let (url, requests) = serve_json_pages(vec![(String::new(), body)]);
            endpoints.insert(token.to_string(), url);
            captured.push((token, requests));
        }
        save_credentials_store_in_home(
            temp.path(),
            &CursorCredentialsStore {
                version: 1,
                active_account_id: "active".to_string(),
                accounts,
            },
        )?;
        let inputs = std::sync::Mutex::new(HashMap::new());
        let runtime = tokio::runtime::Runtime::new()?;
        let result = runtime.block_on(sync_cursor_cache_with_fetcher_in_home(
            temp.path(),
            |fetch| {
                inputs
                    .lock()
                    .unwrap()
                    .insert(fetch.session_token.clone(), fetch.existing_cache.clone());
                let url = endpoints[&fetch.session_token].clone();
                async move {
                    fetch_cursor_usage_events_json_merging(
                        &url,
                        &fetch.session_token,
                        Some(Duration::from_secs(2)),
                        true,
                        CURSOR_MAX_JSON_BYTES,
                        2,
                        fetch.existing_cache.as_deref(),
                    )
                    .await
                }
            },
        ));
        let inputs = inputs.into_inner().unwrap();
        eprintln!("actual fetch inputs: {}", serde_json::to_string(&inputs)?);
        let mut saved = Vec::new();
        for entry in fs::read_dir(cursor_cache_dir(temp.path()))? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let rows = cached_usage_events(Some(&fs::read_to_string(&path)?));
                eprintln!(
                    "saved {}: {}",
                    path.file_name().unwrap().to_string_lossy(),
                    serde_json::to_string(&rows)?
                );
                saved.push(rows);
            }
        }
        for (token, requests) in &captured {
            eprintln!("actual HTTP for {token}: {:?}", requests.lock().unwrap());
        }
        assert!(result.synced, "{result:?}");
        assert!(
            inputs.values().all(Option::is_none),
            "cold accounts must never receive another account's just-written cache"
        );
        assert_eq!(saved.len(), 3, "each account must retain its own cache");
        for (token, requests) in captured {
            let requests = requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert!(
                requests[0].contains("\"startDate\":\"0\""),
                "cold {token} must request full history"
            );
            let rows = saved
                .iter()
                .find(|rows| {
                    rows.iter()
                        .any(|row| row["conversationId"] == format!("{token}-new"))
                })
                .unwrap();
            assert_eq!(
                rows.len(),
                2,
                "only {token}'s new and old history belongs in this file"
            );
            assert!(rows
                .iter()
                .all(|row| row["conversationId"].as_str().unwrap().starts_with(token)));
        }
        Ok(())
    }

    #[test]
    fn test_cursor_cache_owner_envelope_handles_long_and_escaped_account_ids() -> Result<()> {
        let home = TempDir::new()?;
        let path = home.path().join("usage.json");
        let account_id = format!(
            "team/\"{}é",
            "x".repeat(CURSOR_CACHE_OWNER_ENVELOPE_BYTES * 2)
        );
        let owned = owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, &account_id)?;
        assert!(owned.starts_with(r#"{"_owner":"#));
        assert!(owned.contains(r#","zzTokscaleOwner":"#));
        fs::write(&path, owned)?;

        let expected_hash = cursor_cache_owner_hash(&account_id);
        assert_eq!(
            cursor_cache_owner_hash_from_envelope(&path).as_deref(),
            Some(expected_hash.as_str())
        );
        Ok(())
    }

    #[test]
    fn test_cursor_cache_owner_envelope_rejects_truncation_and_foreign_owner() -> Result<()> {
        let home = TempDir::new()?;
        let path = home.path().join("usage.json");
        let owned_a = owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, "account-a")?;
        let owned_b = owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, "account-b")?;
        assert_eq!(owned_a.len(), owned_b.len());

        fs::write(&path, &owned_a)?;
        assert_eq!(
            cursor_cache_owner_hash_from_envelope(&path).as_deref(),
            Some(cursor_cache_owner_hash("account-a").as_str())
        );
        fs::write(&path, &owned_b)?;
        assert_eq!(
            cursor_cache_owner_hash_from_envelope(&path).as_deref(),
            Some(cursor_cache_owner_hash("account-b").as_str())
        );

        let mut mismatched: serde_json::Value = serde_json::from_str(&owned_a)?;
        mismatched["zzTokscaleOwner"]["generation"] =
            serde_json::json!("00000000-0000-0000-0000-000000000000");
        fs::write(&path, serde_json::to_string(&mismatched)?)?;
        assert!(cursor_cache_advertises_owner_envelope(&path));
        assert_eq!(cursor_cache_owner_hash_from_envelope(&path), None);
        assert_eq!(
            cursor_cache_owner_hash_from_file(&path),
            None,
            "a cache advertising a broken envelope must not downgrade to legacy trust"
        );

        fs::write(&path, &owned_a.as_bytes()[..owned_a.len() - 1])?;
        assert_eq!(cursor_cache_owner_hash_from_envelope(&path), None);
        assert_eq!(cursor_cache_owner_hash_from_file(&path), None);
        Ok(())
    }

    #[test]
    fn test_cursor_cache_owner_envelope_falls_back_for_valid_legacy_cache() -> Result<()> {
        let home = TempDir::new()?;
        let path = home.path().join("usage.json");
        fs::write(
            &path,
            r#"{"_tokscale":{"version":1,"accountId":"legacy"},"usageEventsDisplay":[]}"#,
        )?;
        assert!(!cursor_cache_advertises_owner_envelope(&path));
        assert_eq!(cursor_cache_owner_hash_from_envelope(&path), None);
        assert_eq!(
            cursor_cache_owner_hash_from_file(&path).as_deref(),
            Some(cursor_cache_owner_hash("legacy").as_str())
        );
        Ok(())
    }

    fn collision_fixture_store(home: &Path) -> Result<CursorCredentialsStore> {
        let ids = ["team/account-a", "team@account-a", "TEAM_ACCOUNT_A"];
        let store = CursorCredentialsStore {
            version: 1,
            active_account_id: ids[0].to_string(),
            accounts: ids
                .into_iter()
                .map(|id| {
                    (
                        id.to_string(),
                        CursorCredentials {
                            session_token: format!("{id}::fixture"),
                            user_id: Some(id.to_string()),
                            created_at: "2026-01-01T00:00:00Z".to_string(),
                            expires_at: None,
                            label: None,
                        },
                    )
                })
                .collect(),
        };
        save_credentials_store_in_home(home, &store)?;
        Ok(store)
    }

    fn preserved_cursor_files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
        let mut files = Vec::new();
        if !dir.join("archive").exists() {
            return Ok(files);
        }
        for entry in fs::read_dir(dir.join("archive"))? {
            for file in fs::read_dir(entry?.path())? {
                let path = file?.path();
                files.push((
                    path.file_name().unwrap().to_string_lossy().to_string(),
                    fs::read(path)?,
                ));
            }
        }
        Ok(files)
    }

    #[test]
    fn test_legacy_collision_recovery_refetches_without_assigning_ownership() -> Result<()> {
        let home = TempDir::new()?;
        let store = collision_fixture_store(home.path())?;
        let dir = cursor_cache_dir(home.path());
        fs::create_dir_all(&dir)?;
        let old = String::from_utf8(json_page(1, &[&json_event("unknown-owner", "9999999")]))?;
        let legacy = [
            "usage.json",
            "usage.team-account-a.json",
            "usage.team-account-a.csv",
        ];
        for name in legacy {
            fs::write(dir.join(name), old.as_bytes())?;
        }
        // Even a new-looking filename is unsafe when the ownership disagrees.
        let wrong = format!("usage.{}.json", cursor_account_cache_stem("team@account-a"));
        let wrong_contents = owned_cursor_cache(&old, "team/account-a")?;
        fs::write(dir.join(&wrong), &wrong_contents)?;
        let failed = tokio::runtime::Runtime::new()?.block_on(
            sync_cursor_cache_with_fetcher_in_home(home.path(), |fetch| {
                assert!(fetch.existing_cache.is_none());
                std::future::ready(Err(anyhow::anyhow!("network unavailable")))
            }),
        );
        assert!(!failed.synced);
        assert!(failed.error.unwrap().contains("network unavailable"));
        assert!(!dir.join("usage.json").exists());
        assert_eq!(preserved_cursor_files(&dir)?.len(), 4);
        // A retry still fetches from the beginning and leaves every backup intact.
        let seen = std::sync::Mutex::new(Vec::new());
        let result = tokio::runtime::Runtime::new()?.block_on(
            sync_cursor_cache_with_fetcher_in_home(home.path(), |fetch| {
                assert!(
                    fetch.existing_cache.is_none(),
                    "legacy or foreign history must never seed a watermark"
                );
                let id = extract_user_id_from_session_token(&fetch.session_token).unwrap();
                seen.lock().unwrap().push(id.clone());
                std::future::ready(
                    String::from_utf8(json_page(1, &[&json_event(&id, "1")])).map_err(Into::into),
                )
            }),
        );
        assert!(result.synced, "{result:?}");
        assert_eq!(result.rows, 3);
        assert_eq!(result.error, None);
        assert_eq!(seen.lock().unwrap().len(), store.accounts.len());
        let saved = preserved_cursor_files(&dir)?;
        assert_eq!(saved.len(), 4);
        for name in legacy {
            assert!(saved.contains(&(name.to_string(), old.as_bytes().to_vec())));
        }
        assert!(saved.contains(&(wrong, wrong_contents.into_bytes())));
        let discovered =
            tokscale_core::scanner::scan_directory(dir.to_str().unwrap(), "usage*.json|usage*.csv");
        assert_eq!(
            discovered.len(),
            3,
            "preserved history must stay outside the live report scan"
        );
        for id in store.accounts.keys() {
            let path = if *id == store.active_account_id {
                dir.join("usage.json")
            } else {
                dir.join(format!("usage.{}.json", cursor_account_cache_stem(id)))
            };
            let text = fs::read_to_string(&path)?;
            assert_eq!(cursor_cache_owner(&text).as_deref(), Some(id.as_str()));
            let messages = tokscale_core::sessions::cursor::parse_cursor_file(&path);
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].session_id, *id);
        }
        Ok(())
    }

    #[test]
    fn test_owned_colliding_accounts_survive_switch_refresh_remove_and_logout() -> Result<()> {
        for purge in [false, true] {
            let home = TempDir::new()?;
            let store = collision_fixture_store(home.path())?;
            let dir = cursor_cache_dir(home.path());
            fs::create_dir_all(&dir)?;
            for id in store.accounts.keys() {
                let path = if *id == store.active_account_id {
                    dir.join("usage.json")
                } else {
                    dir.join(format!("usage.{}.json", cursor_account_cache_stem(id)))
                };
                let json = String::from_utf8(json_page(1, &[&json_event(id, "1")]))?;
                fs::write(path, owned_cursor_cache(&json, id)?)?;
            }
            let first = "team/account-a";
            let second = "team@account-a";
            let third = "TEAM_ACCOUNT_A";
            let third_path = dir.join(format!("usage.{}.json", cursor_account_cache_stem(third)));
            let third_before = fs::read(&third_path)?;
            set_active_account_in_home(home.path(), second)?;
            assert_eq!(
                cursor_cache_owner(&fs::read_to_string(dir.join("usage.json"))?).as_deref(),
                Some(second)
            );
            assert_eq!(
                cursor_cache_owner(&fs::read_to_string(
                    dir.join(format!("usage.{}.json", cursor_account_cache_stem(first)))
                )?)
                .as_deref(),
                Some(first)
            );
            assert_eq!(fs::read(&third_path)?, third_before);
            assert_eq!(
                upsert_credentials_in_home(home.path(), &format!("{first}::fresh-token"), None)?,
                first
            );
            assert_eq!(
                cursor_cache_owner(&fs::read_to_string(dir.join("usage.json"))?).as_deref(),
                Some(first)
            );
            remove_account_in_home(home.path(), second, purge)?;
            assert_eq!(fs::read(&third_path)?, third_before);
            assert!(!dir
                .join(format!("usage.{}.json", cursor_account_cache_stem(second)))
                .exists());
            remove_account_in_home(home.path(), first, purge)?;
            assert_eq!(
                load_credentials_store_from_home(home.path())
                    .unwrap()
                    .active_account_id,
                third
            );
            assert_eq!(fs::read(dir.join("usage.json"))?, third_before);
            // An unknown legacy alias remains recoverable even when purging known accounts.
            fs::write(
                dir.join("usage.team-account-a.csv"),
                b"unknown legacy owner",
            )?;
            remove_all_accounts_in_home(home.path(), purge)?;
            assert!(!cursor_credentials_path(home.path()).exists());
            assert!(!dir.join("usage.json").exists());
            let legacy_preserved = preserved_cursor_files(&dir)?.contains(&(
                "usage.team-account-a.csv".to_string(),
                b"unknown legacy owner".to_vec(),
            ));
            assert_eq!(legacy_preserved, !purge);
        }
        Ok(())
    }

    #[test]
    fn test_remove_all_purge_deletes_unverified_caches_without_archiving() -> Result<()> {
        let home = TempDir::new()?;
        collision_fixture_store(home.path())?;
        let dir = cursor_cache_dir(home.path());
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("usage.json"), b"unverified json history")?;
        fs::write(dir.join("usage.legacy.csv"), b"unverified csv history")?;
        let old_archive = dir.join("archive/old-backup");
        fs::create_dir_all(&old_archive)?;
        fs::write(old_archive.join("usage.json"), b"earlier archived history")?;
        let legacy_dir = old_cursor_cache_dir(home.path());
        fs::create_dir_all(&legacy_dir)?;
        fs::write(legacy_dir.join("usage.json"), b"legacy-location history")?;
        fs::create_dir_all(old_cursor_credentials_path(home.path()).parent().unwrap())?;
        fs::write(
            old_cursor_credentials_path(home.path()),
            b"legacy credentials",
        )?;

        remove_all_accounts_in_home(home.path(), true)?;

        assert!(!cursor_credentials_path(home.path()).exists());
        assert!(!old_cursor_credentials_path(home.path()).exists());
        assert!(
            !dir.exists() && !legacy_dir.exists(),
            "purge-all must remove live, archived, and legacy-location usage"
        );
        Ok(())
    }

    #[test]
    fn test_failed_legacy_preservation_blocks_fetch_switch_and_removal() -> Result<()> {
        let home = TempDir::new()?;
        let store = collision_fixture_store(home.path())?;
        let dir = cursor_cache_dir(home.path());
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("usage.json"), b"unverified history")?;
        fs::write(dir.join("archive"), b"blocks archival")?;
        let credentials = fs::read(cursor_credentials_path(home.path()))?;
        assert!(set_active_account_in_home(home.path(), "team@account-a").is_err());
        assert_eq!(fs::read(cursor_credentials_path(home.path()))?, credentials);
        assert!(remove_account_in_home(home.path(), &store.active_account_id, true).is_err());
        assert!(remove_all_accounts_in_home(home.path(), false).is_err());
        assert_eq!(fs::read(cursor_credentials_path(home.path()))?, credentials);
        let fetched = std::sync::atomic::AtomicBool::new(false);
        let result = tokio::runtime::Runtime::new()?.block_on(
            sync_cursor_cache_with_fetcher_in_home(home.path(), |_| {
                fetched.store(true, std::sync::atomic::Ordering::SeqCst);
                std::future::ready(Err(anyhow::anyhow!("fetch must not start")))
            }),
        );
        assert!(!fetched.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!result.synced);
        assert!(result.error.unwrap().contains("preserve unverified"));
        assert!(
            upsert_credentials_in_home(home.path(), "team@account-a::refreshed", None).is_err()
        );
        let after = load_credentials_store_from_home(home.path()).unwrap();
        assert_eq!(after.active_account_id, store.active_account_id);
        assert_eq!(
            after.accounts["team@account-a"].session_token,
            "team@account-a::refreshed"
        );
        assert_eq!(fs::read(dir.join("usage.json"))?, b"unverified history");
        Ok(())
    }

    #[test]
    fn test_repeated_legacy_backups_keep_each_original_copy() -> Result<()> {
        let home = TempDir::new()?;
        let store = collision_fixture_store(home.path())?;
        let dir = cursor_cache_dir(home.path());
        fs::create_dir_all(&dir)?;
        for contents in [b"first copy", b"other copy"] {
            fs::write(dir.join("usage.json"), contents)?;
            prepare_cursor_cache_layout(&dir, &store)?;
        }
        let files = preserved_cursor_files(&dir)?;
        assert_eq!(files.len(), 2);
        assert!(files.contains(&("usage.json".to_string(), b"first copy".to_vec())));
        assert!(files.contains(&("usage.json".to_string(), b"other copy".to_vec())));
        Ok(())
    }

    #[test]
    fn test_freshness_requires_owned_active_cache_after_upgrade() -> Result<()> {
        let home = TempDir::new()?;
        let store = collision_fixture_store(home.path())?;
        let dir = cursor_cache_dir(home.path());
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("usage.json"), r#"{"usageEventsDisplay":[]}"#)?;
        fs::write(dir.join(CURSOR_SYNC_ATTEMPT_MARKER), "")?;
        assert!(!cursor_usage_cache_is_fresh_in(
            home.path(),
            Duration::from_secs(300)
        ));
        fs::write(
            dir.join("usage.json"),
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, &store.active_account_id)?,
        )?;
        assert!(cursor_usage_cache_is_fresh_in(
            home.path(),
            Duration::from_secs(300)
        ));
        Ok(())
    }
    #[test]
    fn test_freshness_keeps_colliding_secondaries_independent_and_verified() -> Result<()> {
        let home = TempDir::new()?;
        let store = collision_fixture_store(home.path())?;
        let dir = cursor_cache_dir(home.path());
        fs::create_dir_all(&dir)?;
        fs::write(
            dir.join("usage.json"),
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, &store.active_account_id)?,
        )?;
        let a = "team@account-a";
        let b = "TEAM_ACCOUNT_A";
        let path_a = dir.join(format!("usage.{}.json", cursor_account_cache_stem(a)));
        let path_b = dir.join(format!("usage.{}.json", cursor_account_cache_stem(b)));
        fs::write(
            &path_a,
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, a)?,
        )?;
        assert!(
            !cursor_usage_cache_is_fresh_in(home.path(), Duration::from_secs(300)),
            "one fresh secondary cannot stand in for another"
        );
        fs::write(
            &path_b,
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, b)?,
        )?;
        assert!(cursor_usage_cache_is_fresh_in(
            home.path(),
            Duration::from_secs(300)
        ));
        fs::write(
            &path_a,
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, b)?,
        )?;
        fs::write(dir.join(CURSOR_SYNC_ATTEMPT_MARKER), "")?;
        assert!(
            !cursor_usage_cache_is_fresh_in(home.path(), Duration::from_secs(300)),
            "a fresh attempt cannot bless foreign ownership"
        );
        fs::write(
            &path_a,
            owned_cursor_cache(r#"{"usageEventsDisplay":[]}"#, a)?,
        )?;
        fs::write(
            dir.join("usage.team-account-a.csv"),
            "unverified legacy alias",
        )?;
        assert!(
            !cursor_usage_cache_is_fresh_in(home.path(), Duration::from_secs(300)),
            "unexpected legacy files must trigger preservation even with a fresh marker"
        );
        Ok(())
    }
    #[test]
    fn test_verified_active_cache_survives_duplicate_cleanup_and_failed_fetch() -> Result<()> {
        let home = TempDir::new()?;
        let mut store = collision_fixture_store(home.path())?;
        let id = store.active_account_id.clone();
        store.accounts.retain(|key, _| key == &id);
        save_credentials_store_in_home(home.path(), &store)?;
        let dir = cursor_cache_dir(home.path());
        fs::create_dir_all(&dir)?;
        let active = owned_cursor_cache(
            &String::from_utf8(json_page(1, &[&json_event("canonical-history", "7200000")]))?,
            &id,
        )?;
        let duplicate = owned_cursor_cache(
            &String::from_utf8(json_page(1, &[&json_event("duplicate-history", "1")]))?,
            &id,
        )?;
        let named = dir.join(format!("usage.{}.json", cursor_account_cache_stem(&id)));
        fs::write(dir.join("usage.json"), &active)?;
        fs::write(&named, &duplicate)?;
        let result = tokio::runtime::Runtime::new()?.block_on(
            sync_cursor_cache_with_fetcher_in_home(home.path(), |fetch| {
                assert_eq!(
                    fetch.existing_cache.as_deref(),
                    Some(active.as_str()),
                    "a verified canonical cache must remain available to the fetcher"
                );
                std::future::ready(Err(anyhow::anyhow!("network unavailable")))
            }),
        );
        assert!(!result.synced);
        assert!(result.error.unwrap().contains("network unavailable"));
        assert_eq!(fs::read_to_string(dir.join("usage.json"))?, active);
        assert!(!named.exists());
        assert_eq!(
            preserved_cursor_files(&dir)?,
            vec![(
                named.file_name().unwrap().to_string_lossy().to_string(),
                duplicate.into_bytes()
            )]
        );
        Ok(())
    }
}
