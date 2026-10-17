//! Cursor subscription plan remaining for `tokscale usage`.
//!
//! Auth comes from the desktop `state.vscdb` JWT, then the macOS Keychain items
//! `cursor-access-token` / `cursor-refresh-token`, then a JWT embedded in a
//! saved Tokscale Cursor session cookie. The IDE plan window is
//! `GetCurrentPeriodUsage` on `api2.cursor.sh`. Grok Bot weekly usage is a
//! separate Cursor-metered pool from `GetSandUsageStatus` (not the SuperGrok
//! `cli-chat-proxy` credits used by the Grok card). Refreshed
//! access tokens stay in memory and are never written back.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{UsageMetric, UsageOutput};

const USAGE_URL: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage";
const PLAN_URL: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService/GetPlanInfo";
/// Grok Bot weekly included usage. Metered on the Cursor account, not on xAI.
const SAND_USAGE_URL: &str =
    "https://api2.cursor.sh/aiserver.v1.DashboardService/GetSandUsageStatus";
const REFRESH_URL: &str = "https://api2.cursor.sh/oauth/token";
/// Public Cursor Auth0 client id. Not a user secret.
const CLIENT_ID: &str = "KbZUR41cY7W6zRSdpSUJ7I7mLYBKOCmB";
#[cfg(target_os = "macos")]
const ACCESS_SERVICE: &str = "cursor-access-token";
#[cfg(target_os = "macos")]
const REFRESH_SERVICE: &str = "cursor-refresh-token";
const PROVIDER: &str = "Cursor";
const GROK_BOT_PROVIDER: &str = "Grok Bot";

#[derive(Debug, Clone, PartialEq)]
struct ParsedPlanWindow {
    used_percent: f64,
    remaining_percent: f64,
    reset_at: Option<String>,
}

/// Cursor meters Auto models and named/API models on separate included-usage
/// tracks. `totalPercentUsed` is a blended figure; the UI wants the two tracks.
#[derive(Debug, Clone, PartialEq)]
struct ParsedPlanTracks {
    auto: Option<ParsedPlanWindow>,
    api: Option<ParsedPlanWindow>,
    /// Fallback when neither track percent is present.
    total: Option<ParsedPlanWindow>,
    reset_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ParsedPlanInfo {
    plan_name: Option<String>,
    weekly_included: bool,
    included_reset_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct LocalCursorAuth {
    access_token: String,
    refresh_token: Option<String>,
    email: Option<String>,
}

#[derive(Debug, Clone)]
struct RefreshedCursorAuth {
    access_token: String,
    refresh_token: String,
}

type RefreshKey = [u8; 32];
static REFRESHED_AUTH: OnceLock<Mutex<HashMap<RefreshKey, RefreshedCursorAuth>>> = OnceLock::new();
static REFRESH_GATE: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

pub fn has_credentials() -> bool {
    resolve_local_auth().is_ok()
}

/// Grok Bot is metered on the Cursor account, so the same desktop auth works.
pub fn has_grok_bot_credentials() -> bool {
    has_credentials()
}

pub fn fetch() -> Result<UsageOutput> {
    let auth = resolve_local_auth()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let now = Utc::now().timestamp();
        let mut access = bearer_for_request(&auth, now).await?;
        let client = plan_http_client()?;
        let usage = connect_post_with_retry(
            &client,
            USAGE_URL,
            &auth,
            &mut access,
            "GetCurrentPeriodUsage",
        )
        .await?;
        // Plan cadence is independent of the invoice-cycle reset carried by
        // GetCurrentPeriodUsage, so fetch it on every successful usage read.
        let plan = connect_post_with_retry(&client, PLAN_URL, &auth, &mut access, "GetPlanInfo")
            .await
            .ok();
        let sand = connect_post_with_retry(
            &client,
            SAND_USAGE_URL,
            &auth,
            &mut access,
            "GetSandUsageStatus",
        )
        .await
        .ok();

        let plan_info = parse_plan_info(plan.as_ref());
        let metrics = map_cursor_metrics(&usage, plan.as_ref(), sand.as_ref())?;
        if metrics.is_empty() {
            anyhow::bail!("Cursor plan usage had no usable included quota");
        }

        let plan_name = plan_info.plan_name.or_else(|| {
            usage
                .get("membershipType")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        });
        Ok(UsageOutput {
            provider: PROVIDER.to_string(),
            account: None,
            credential_source: Some("desktop".into()),
            plan: plan_name,
            email: auth.email,
            metrics,
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        })
    })
}

/// Grok Bot weekly included usage for legacy accounts whose Sand pool remains
/// distinct. Unified or weekly Cursor plans render that pool on the Cursor card.
pub fn fetch_grok_bot() -> Result<Vec<UsageOutput>> {
    let auth = resolve_local_auth()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let now = Utc::now().timestamp();
        let mut access = bearer_for_request(&auth, now).await?;
        let client = plan_http_client()?;
        let plan = connect_post_with_retry(&client, PLAN_URL, &auth, &mut access, "GetPlanInfo")
            .await
            .ok();
        let sand = connect_post_with_retry(
            &client,
            SAND_USAGE_URL,
            &auth,
            &mut access,
            "GetSandUsageStatus",
        )
        .await?;
        let Some(parsed) = parse_sand_usage(&sand)? else {
            return Ok(Vec::new());
        };
        if sand_replaces_cursor_tracks(&parse_plan_info(plan.as_ref()), &parsed) {
            return Ok(Vec::new());
        }
        Ok(vec![UsageOutput {
            provider: GROK_BOT_PROVIDER.to_string(),
            account: None,
            credential_source: Some("desktop".into()),
            plan: parsed.plan,
            email: auth.email,
            metrics: vec![parsed.metric],
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        }])
    })
}

#[derive(Debug, Clone)]
struct ParsedSandUsage {
    metric: UsageMetric,
    plan: Option<String>,
    is_unified: bool,
}

fn parse_plan_info(value: Option<&Value>) -> ParsedPlanInfo {
    let Some(info) = value.and_then(|value| value.get("planInfo")) else {
        return ParsedPlanInfo::default();
    };
    let plan_name = info
        .get("planName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    let weekly_included = info
        .get("includedUsagePeriod")
        .and_then(Value::as_str)
        .is_some_and(|period| period.trim().to_ascii_uppercase().ends_with("WEEKLY"));
    let included_reset_at = billing_reset_iso(info.get("includedUsageResetsAt"));
    ParsedPlanInfo {
        plan_name,
        weekly_included,
        included_reset_at,
    }
}

/// Parse `GetSandUsageStatus`. Hide only when the account explicitly has no
/// personal allowance. A response that promises an allowance but omits its
/// percentage is schema/auth failure, not a successful empty provider.
fn parse_sand_usage(value: &Value) -> Result<Option<ParsedSandUsage>> {
    if value
        .get("usesPooledEnterpriseAllowance")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Ok(None);
    }
    if value
        .get("hasNonZeroIncludedLimit")
        .and_then(Value::as_bool)
        == Some(false)
        || value.get("includedLimitZero").and_then(Value::as_bool) == Some(true)
    {
        return Ok(None);
    }
    let Some(percent) = value.get("usagePercent") else {
        if let Some(code) = value
            .get("code")
            .and_then(Value::as_str)
            .filter(|code| is_short_code(code))
        {
            anyhow::bail!("Cursor GetSandUsageStatus returned code {code}.");
        }
        if value
            .get("hasNonZeroIncludedLimit")
            .and_then(Value::as_bool)
            == Some(true)
            || value.get("hasAvailableUsage").and_then(Value::as_bool) == Some(true)
            || value.get("isUnified").and_then(Value::as_bool) == Some(true)
        {
            anyhow::bail!(
                "Grok Bot GetSandUsageStatus omitted usagePercent for an eligible account"
            );
        }
        return Ok(None);
    };
    if percent.is_null() {
        anyhow::bail!("Grok Bot GetSandUsageStatus returned a null usagePercent");
    }
    let used = percent
        .as_f64()
        .filter(|percent| percent.is_finite() && *percent >= 0.0)
        .map(|percent| percent.min(100.0))
        .ok_or_else(|| {
            anyhow::anyhow!("Grok Bot GetSandUsageStatus had an invalid usagePercent")
        })?;
    let (used_percent, remaining_percent) = round_pair(used);
    let resets_at = sand_reset_iso(value.get("nextResetTimestampUtc"));
    let plan = value
        .get("grokPlanLabel")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("cursorPlanName")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        });
    Ok(Some(ParsedSandUsage {
        metric: UsageMetric {
            label: "Weekly".into(),
            used_percent,
            remaining_percent,
            remaining_label: None,
            resets_at,
        },
        plan,
        is_unified: value.get("isUnified").and_then(Value::as_bool) == Some(true),
    }))
}

fn sand_replaces_cursor_tracks(plan: &ParsedPlanInfo, sand: &ParsedSandUsage) -> bool {
    plan.weekly_included || sand.is_unified
}

fn map_cursor_metrics(
    usage: &Value,
    plan: Option<&Value>,
    sand: Option<&Value>,
) -> Result<Vec<UsageMetric>> {
    let plan_info = parse_plan_info(plan);
    let parsed_sand = sand.and_then(|value| parse_sand_usage(value).ok().flatten());
    let unified = parsed_sand
        .as_ref()
        .is_some_and(|parsed| sand_replaces_cursor_tracks(&plan_info, parsed));
    match (unified, parsed_sand.as_ref()) {
        (true, Some(parsed)) => Ok(vec![parsed.metric.clone()]),
        _ => {
            let tracks = parse_plan_tracks(usage, plan)?;
            let reset_override = if plan_info.weekly_included {
                plan_info.included_reset_at
            } else {
                None
            };
            Ok(metrics_from_tracks(&tracks, reset_override))
        }
    }
}

fn sand_reset_iso(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    DateTime::parse_from_rfc3339(text).ok().map(|parsed| {
        parsed
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true)
    })
}

fn metrics_from_tracks(
    tracks: &ParsedPlanTracks,
    reset_override: Option<String>,
) -> Vec<UsageMetric> {
    let reset_at = reset_override.or_else(|| tracks.reset_at.clone());
    let mut metrics = Vec::new();
    if let Some(auto) = tracks.auto.as_ref() {
        metrics.push(metric_from_window("Cursor Models", auto, reset_at.clone()));
    }
    if let Some(api) = tracks.api.as_ref() {
        metrics.push(metric_from_window("Other Models", api, reset_at.clone()));
    }
    if metrics.is_empty() {
        if let Some(total) = tracks.total.as_ref() {
            metrics.push(metric_from_window("Plan", total, reset_at));
        }
    }
    metrics
}

fn metric_from_window(
    label: &str,
    window: &ParsedPlanWindow,
    reset_at: Option<String>,
) -> UsageMetric {
    UsageMetric {
        label: label.into(),
        used_percent: window.used_percent,
        remaining_percent: window.remaining_percent,
        remaining_label: None,
        resets_at: reset_at.or_else(|| window.reset_at.clone()),
    }
}

fn plan_http_client() -> Result<reqwest::Client> {
    // Match the Cursor CLI client: cursor.com / api2 sit behind fingerprints
    // that reject rustls on some networks (#1250). The one sanctioned
    // native-TLS builder lives in `crate::cursor`; everything else in the
    // workspace stays on rustls through `tokscale_core::http`.
    crate::cursor::cursor_http_client_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Failed to build Cursor plan HTTP client")
}

fn access_token_usable(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3 && parts.iter().all(|part| !part.is_empty())
}

fn needs_refresh(token: &str, now_unix: i64) -> bool {
    jwt_exp(token).is_none_or(|exp| exp <= now_unix.saturating_add(60))
}

fn jwt_payload(token: &str) -> Option<Value> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn jwt_exp(token: &str) -> Option<i64> {
    jwt_payload(token)?.get("exp").and_then(Value::as_i64)
}

fn jwt_subject(token: &str) -> Option<String> {
    jwt_payload(token)?
        .get("sub")?
        .as_str()
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .map(str::to_string)
}

fn same_cursor_principal(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    matches!(
        (jwt_subject(left), jwt_subject(right)),
        (Some(left), Some(right)) if left == right
    )
}

fn account_label(raw: Option<&str>) -> Option<String> {
    let email = raw?.trim();
    if (3..=254).contains(&email.len())
        && email.matches('@').count() == 1
        && !email.contains(char::is_whitespace)
        && !email.contains("Bearer")
    {
        Some(email.to_string())
    } else {
        None
    }
}

fn jwt_from_session_token(session: &str) -> Option<String> {
    let decoded = session.replace("%3A%3A", "::");
    let jwt = decoded.split("::").nth(1)?.trim();
    access_token_usable(jwt).then(|| jwt.to_string())
}

fn resolve_local_auth() -> Result<LocalCursorAuth> {
    let home = crate::paths::home_dir().context("Could not determine home directory")?;
    let db_path = crate::cursor::find_cursor_state_vscdb_for_usage(&home);
    let (db_access, db_refresh, db_email) = if let Some(path) = db_path.as_ref() {
        (
            read_vscdb_value(path, "cursorAuth/accessToken")
                .ok()
                .flatten(),
            read_vscdb_value(path, "cursorAuth/refreshToken")
                .ok()
                .flatten(),
            read_vscdb_value(path, "cursorAuth/cachedEmail")
                .ok()
                .flatten(),
        )
    } else {
        (None, None, None)
    };

    let db_access = db_access.unwrap_or_default();
    let db_email = account_label(db_email.as_deref());
    #[cfg(target_os = "macos")]
    let keychain = {
        let access_token = super::helpers::read_keychain(ACCESS_SERVICE).unwrap_or_default();
        let email = (!db_access.is_empty()
            && !access_token.is_empty()
            && same_cursor_principal(&db_access, &access_token))
        .then(|| db_email.clone())
        .flatten();
        LocalCursorAuth {
            access_token,
            refresh_token: super::helpers::read_keychain(REFRESH_SERVICE)
                .ok()
                .and_then(|token| nonempty(Some(&token))),
            email,
        }
    };
    #[cfg(not(target_os = "macos"))]
    let keychain = LocalCursorAuth::default();

    let db = LocalCursorAuth {
        access_token: db_access.clone(),
        refresh_token: nonempty(db_refresh.as_deref()),
        email: db_email.clone(),
    };
    let now_unix = Utc::now().timestamp();
    if let Some(auth) = select_local_auth(db.clone(), keychain.clone(), None, now_unix) {
        return Ok(auth);
    }
    let session = crate::cursor::load_active_credentials().and_then(|creds| {
        jwt_from_session_token(&creds.session_token).map(|access| {
            let email = (!db_access.is_empty() && same_cursor_principal(&db_access, &access))
                .then(|| db_email.clone())
                .flatten();
            LocalCursorAuth {
                access_token: access,
                refresh_token: None,
                email,
            }
        })
    });
    if let Some(auth) = select_local_auth(db, keychain, session, now_unix) {
        return Ok(auth);
    }

    anyhow::bail!(
        "Cursor plan credentials not found. Sign in to the Cursor desktop app, or run 'tokscale cursor login'."
    )
}

fn auth_record_matches_access(access_token: &str, other: &LocalCursorAuth) -> bool {
    let access_token = access_token.trim();
    if access_token.is_empty() {
        return false;
    }
    nonempty(Some(&other.access_token))
        .into_iter()
        .chain(other.refresh_token.clone())
        .any(|candidate| same_cursor_principal(access_token, &candidate))
}

fn select_local_auth(
    mut db: LocalCursorAuth,
    mut keychain: LocalCursorAuth,
    session: Option<LocalCursorAuth>,
    now_unix: i64,
) -> Option<LocalCursorAuth> {
    // Cursor can split one account's access and refresh JWTs between
    // state.vscdb and Keychain. Transfer a missing refresh token only when an
    // access/refresh subject proves both records belong to the same account.
    // An unprovable or different principal must stay isolated.
    if db.refresh_token.is_none() && auth_record_matches_access(&db.access_token, &keychain) {
        db.refresh_token = keychain.refresh_token.clone();
    }
    if keychain.refresh_token.is_none() && auth_record_matches_access(&keychain.access_token, &db) {
        keychain.refresh_token = db.refresh_token.clone();
    }
    if !db.access_token.trim().is_empty()
        && !keychain.access_token.trim().is_empty()
        && !same_cursor_principal(db.access_token.trim(), keychain.access_token.trim())
    {
        keychain.email = None;
    }
    [Some(db), Some(keychain), session]
        .into_iter()
        .flatten()
        .find_map(|mut auth| {
            auth.access_token = auth.access_token.trim().to_string();
            (access_token_usable(&auth.access_token)
                && (!needs_refresh(&auth.access_token, now_unix) || auth.refresh_token.is_some()))
            .then_some(auth)
        })
}

fn nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn read_vscdb_value(db_path: &std::path::Path, key: &str) -> Result<Option<String>> {
    use rusqlite::{Connection, OpenFlags};

    if !matches!(
        key,
        "cursorAuth/accessToken" | "cursorAuth/refreshToken" | "cursorAuth/cachedEmail"
    ) {
        anyhow::bail!("refused unexpected Cursor state key");
    }

    let uri = format!("file:{}?mode=ro", db_path.display());
    let conn = Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("Failed to open Cursor state DB at {}", db_path.display()))?;

    let value: Option<String> =
        match conn.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |row| {
            row.get(0)
        }) {
            Ok(value) => Some(value),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(err) => return Err(err.into()),
        };
    Ok(value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty()))
}

fn refreshed_auth() -> &'static Mutex<HashMap<RefreshKey, RefreshedCursorAuth>> {
    REFRESHED_AUTH.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_refreshed_auth() -> MutexGuard<'static, HashMap<RefreshKey, RefreshedCursorAuth>> {
    refreshed_auth()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn refresh_key(refresh_token: &str) -> RefreshKey {
    Sha256::digest(refresh_token.as_bytes()).into()
}

async fn bearer_for_request(auth: &LocalCursorAuth, now_unix: i64) -> Result<String> {
    refreshed_bearer(auth, now_unix, None).await
}

async fn refreshed_bearer(
    auth: &LocalCursorAuth,
    now_unix: i64,
    rejected_access: Option<&str>,
) -> Result<String> {
    let Some(original_refresh) = auth.refresh_token.as_deref() else {
        if rejected_access.is_some() || needs_refresh(&auth.access_token, now_unix) {
            anyhow::bail!(
                "Cursor access token is expired or rejected and no refresh token was stored."
            );
        }
        return Ok(auth.access_token.clone());
    };
    let key = refresh_key(original_refresh);
    let cached = lock_refreshed_auth().get(&key).cloned();
    if let Some(cached) = cached.as_ref() {
        let supersedes_rejected =
            rejected_access.is_none_or(|rejected| cached.access_token != rejected);
        if supersedes_rejected && !needs_refresh(&cached.access_token, now_unix) {
            return Ok(cached.access_token.clone());
        }
    }
    if rejected_access.is_none() && !needs_refresh(&auth.access_token, now_unix) {
        return Ok(auth.access_token.clone());
    }

    // Cursor and Grok Bot run on separate usage workers. Serialize their OAuth
    // exchange and re-check the cache after waiting so one rotating grant is
    // consumed once and both requests share its successor.
    let _gate = REFRESH_GATE
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let cached = lock_refreshed_auth().get(&key).cloned();
    if let Some(cached) = cached.as_ref() {
        let supersedes_rejected =
            rejected_access.is_none_or(|rejected| cached.access_token != rejected);
        if supersedes_rejected && !needs_refresh(&cached.access_token, now_unix) {
            return Ok(cached.access_token.clone());
        }
    }
    let refresh = cached
        .as_ref()
        .map(|cached| cached.refresh_token.as_str())
        .unwrap_or(original_refresh);
    let refreshed = refresh_access_token(refresh).await?;
    let access = refreshed.access_token.clone();
    lock_refreshed_auth().insert(key, refreshed);
    Ok(access)
}

async fn refresh_access_token(refresh_token: &str) -> Result<RefreshedCursorAuth> {
    let client = plan_http_client()?;
    let response = client
        .post(REFRESH_URL)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": CLIENT_ID,
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .context("Cursor token refresh request failed")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("Cursor token refresh body could not be read")?;
    if !status.is_success() {
        anyhow::bail!("Cursor token refresh returned HTTP {status}");
    }
    let value: Value = serde_json::from_str(&body).context("Cursor token refresh was not JSON")?;
    if value
        .get("shouldLogout")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        anyhow::bail!("Cursor refresh token was rejected. Sign in again in Cursor.");
    }
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| access_token_usable(token))
        .ok_or_else(|| anyhow::anyhow!("Cursor token refresh returned no access token."))?
        .to_string();
    let refresh = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| access_token_usable(token))
        .map(str::to_string)
        // Cursor currently rotates the returned access JWT into the next
        // refresh grant when no separate refresh_token field is sent.
        .unwrap_or_else(|| access.clone());
    Ok(RefreshedCursorAuth {
        access_token: access,
        refresh_token: refresh,
    })
}

#[derive(Debug)]
struct CursorAuthRejected {
    label: String,
    status: reqwest::StatusCode,
}

impl std::fmt::Display for CursorAuthRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Cursor {} returned HTTP {}",
            self.label, self.status
        )
    }
}

impl std::error::Error for CursorAuthRejected {}

async fn connect_post_with_retry(
    client: &reqwest::Client,
    url: &str,
    auth: &LocalCursorAuth,
    access_token: &mut String,
    label: &str,
) -> Result<Value> {
    match connect_post(client, url, access_token, label).await {
        Err(error)
            if error.downcast_ref::<CursorAuthRejected>().is_some()
                && auth.refresh_token.is_some() =>
        {
            let rejected = access_token.clone();
            *access_token = refreshed_bearer(auth, Utc::now().timestamp(), Some(&rejected)).await?;
            connect_post(client, url, access_token, label).await
        }
        result => result,
    }
}

async fn connect_post(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
    label: &str,
) -> Result<Value> {
    let response = client
        .post(url)
        .bearer_auth(access_token)
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .body("{}")
        .send()
        .await
        .with_context(|| format!("Cursor {label} request failed"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .with_context(|| format!("Cursor {label} body could not be read"))?;
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(CursorAuthRejected {
            label: label.to_string(),
            status,
        }
        .into());
    }
    if !status.is_success() {
        anyhow::bail!("Cursor {label} returned HTTP {status}");
    }
    serde_json::from_str(&body).with_context(|| format!("Cursor {label} was not JSON"))
}

fn parse_plan_tracks(usage: &Value, plan: Option<&Value>) -> Result<ParsedPlanTracks> {
    if usage.get("enabled").and_then(Value::as_bool) == Some(false) {
        anyhow::bail!("Cursor GetCurrentPeriodUsage reported no active subscription.");
    }
    let plan_usage = usage
        .get("planUsage")
        .filter(|value| value.is_object())
        .ok_or_else(|| plan_usage_missing(usage))?;
    let reset_at = billing_reset_iso(usage.get("billingCycleEnd")).or_else(|| {
        billing_reset_iso(plan.and_then(|value| value.pointer("/planInfo/billingCycleEnd")))
    });
    let auto =
        percent_field(plan_usage, "autoPercentUsed").map(|(used, remaining)| ParsedPlanWindow {
            used_percent: used,
            remaining_percent: remaining,
            reset_at: reset_at.clone(),
        });
    let api =
        percent_field(plan_usage, "apiPercentUsed").map(|(used, remaining)| ParsedPlanWindow {
            used_percent: used,
            remaining_percent: remaining,
            reset_at: reset_at.clone(),
        });
    let total = plan_percents(plan_usage)
        .ok()
        .map(|(used_percent, remaining_percent)| ParsedPlanWindow {
            used_percent,
            remaining_percent,
            reset_at: reset_at.clone(),
        });
    if auto.is_none() && api.is_none() && total.is_none() {
        anyhow::bail!(
            "Cursor planUsage had no finite remaining percent (autoPercentUsed, apiPercentUsed, totalPercentUsed, or remaining and limit)."
        );
    }
    Ok(ParsedPlanTracks {
        auto,
        api,
        total,
        reset_at,
    })
}

fn percent_field(plan_usage: &Value, key: &str) -> Option<(f64, f64)> {
    let used = plan_usage.get(key).and_then(Value::as_f64)?;
    (used.is_finite() && used >= 0.0).then(|| round_pair(used.min(100.0)))
}

#[cfg(test)]
fn parse_plan_window(usage: &Value, plan: Option<&Value>) -> Result<ParsedPlanWindow> {
    let tracks = parse_plan_tracks(usage, plan)?;
    tracks
        .total
        .or(tracks.auto)
        .or(tracks.api)
        .ok_or_else(|| anyhow::anyhow!("Cursor planUsage had no usable percent"))
}

fn plan_usage_missing(usage: &Value) -> anyhow::Error {
    match usage.get("code").and_then(Value::as_str) {
        Some(code) if is_short_code(code) => {
            anyhow::anyhow!("Cursor GetCurrentPeriodUsage returned code {code}.")
        }
        _ => anyhow::anyhow!("Cursor GetCurrentPeriodUsage had no planUsage object."),
    }
}

fn is_short_code(code: &str) -> bool {
    (1..40).contains(&code.len())
        && code
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn plan_percents(plan_usage: &Value) -> Result<(f64, f64)> {
    if let Some(used) = plan_usage.get("totalPercentUsed").and_then(Value::as_f64) {
        if used.is_finite() && used >= 0.0 {
            return Ok(round_pair(used.min(100.0)));
        }
    }
    let remaining = plan_usage.get("remaining").and_then(Value::as_f64);
    let limit = plan_usage.get("limit").and_then(Value::as_f64);
    match (remaining, limit) {
        (Some(remaining), Some(limit))
            if remaining.is_finite() && limit.is_finite() && limit > 0.0 =>
        {
            let used = ((limit - remaining) / limit * 100.0).clamp(0.0, 100.0);
            Ok(round_pair(used))
        }
        _ => anyhow::bail!(
            "Cursor planUsage had no finite remaining percent (totalPercentUsed, or remaining and limit)."
        ),
    }
}

fn round_pair(used: f64) -> (f64, f64) {
    let used = (used * 100.0).round() / 100.0;
    let remaining = ((100.0 - used) * 100.0).round() / 100.0;
    (used, remaining)
}

fn billing_reset_iso(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(text) = value.as_str() {
        let text = text.trim();
        if let Ok(raw) = text.parse::<i64>() {
            return millis_or_secs_iso(raw);
        }
        return DateTime::parse_from_rfc3339(text).ok().map(|parsed| {
            parsed
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        });
    }
    value.as_i64().and_then(millis_or_secs_iso)
}

fn millis_or_secs_iso(raw: i64) -> Option<String> {
    let ms = if raw.unsigned_abs() >= 1_000_000_000_000 {
        raw
    } else {
        raw.checked_mul(1000)?
    };
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|instant| instant.to_rfc3339_opts(SecondsFormat::Millis, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::usage::test_server::spawn_server;
    use serde_json::json;

    #[test]
    fn parse_plan_window_from_total_percent_used() {
        let usage = json!({
            "planUsage": { "totalPercentUsed": 37.5 },
            "billingCycleEnd": "1767225600000"
        });
        let window = parse_plan_window(&usage, None).unwrap();
        assert!((window.used_percent - 37.5).abs() < f64::EPSILON);
        assert!((window.remaining_percent - 62.5).abs() < f64::EPSILON);
        assert_eq!(window.reset_at.as_deref(), Some("2026-01-01T00:00:00.000Z"));
    }

    #[test]
    fn parse_plan_tracks_prefers_auto_and_api_over_total() {
        let usage = json!({
            "planUsage": {
                "autoPercentUsed": 16.0,
                "apiPercentUsed": 36.0,
                "totalPercentUsed": 19.0
            },
            "billingCycleEnd": "1767225600000"
        });
        let tracks = parse_plan_tracks(&usage, None).unwrap();
        let metrics = metrics_from_tracks(&tracks, None);
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].label, "Cursor Models");
        assert!((metrics[0].remaining_percent - 84.0).abs() < f64::EPSILON);
        assert_eq!(metrics[1].label, "Other Models");
        assert!((metrics[1].remaining_percent - 64.0).abs() < f64::EPSILON);
    }

    #[test]
    fn plan_tracks_clamp_usage_above_the_included_limit() {
        let usage = json!({
            "planUsage": {
                "autoPercentUsed": 140.0,
                "apiPercentUsed": 25.0,
                "totalPercentUsed": 120.0
            }
        });
        let metrics = metrics_from_tracks(&parse_plan_tracks(&usage, None).unwrap(), None);
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].label, "Cursor Models");
        assert_eq!(metrics[0].used_percent, 100.0);
        assert_eq!(metrics[0].remaining_percent, 0.0);
    }

    #[test]
    fn disabled_cursor_plan_is_not_reported_from_stale_usage_fields() {
        let usage = json!({
            "enabled": false,
            "planUsage": { "autoPercentUsed": 10.0, "apiPercentUsed": 20.0 }
        });
        assert!(parse_plan_tracks(&usage, None).is_err());
    }

    #[test]
    fn unified_sand_pool_replaces_invoice_cycle_tracks() {
        let usage = json!({
            "billingCycleEnd": "2026-11-01T00:00:00Z",
            "planUsage": { "autoPercentUsed": 0.0, "apiPercentUsed": 0.0 }
        });
        let plan = json!({
            "planInfo": {
                "planName": "Ultra",
                "includedUsagePeriod": "INCLUDED_USAGE_PERIOD_WEEKLY",
                "includedUsageResetsAt": "1792752426910"
            }
        });
        let sand = json!({
            "usagePercent": 15.5,
            "hasNonZeroIncludedLimit": true,
            "isUnified": true,
            "nextResetTimestampUtc": "2026-10-23T10:47:06.910Z"
        });

        let metrics = map_cursor_metrics(&usage, Some(&plan), Some(&sand)).unwrap();
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].label, "Weekly");
        assert_eq!(metrics[0].used_percent, 15.5);
        assert_eq!(
            metrics[0].resets_at.as_deref(),
            Some("2026-10-23T10:47:06.910Z")
        );
        let parsed_sand = parse_sand_usage(&sand).unwrap().unwrap();
        assert!(sand_replaces_cursor_tracks(
            &parse_plan_info(Some(&plan)),
            &parsed_sand
        ));
    }

    #[test]
    fn weekly_plan_without_sand_retargets_both_cursor_tracks() {
        let usage = json!({
            "billingCycleEnd": "2026-11-01T00:00:00Z",
            "planUsage": { "autoPercentUsed": 4.0, "apiPercentUsed": 64.0 }
        });
        let plan = json!({
            "planInfo": {
                "includedUsagePeriod": "included_usage_period_weekly",
                "includedUsageResetsAt": "1792752426910"
            }
        });
        let metrics = map_cursor_metrics(&usage, Some(&plan), None).unwrap();
        assert_eq!(
            metrics
                .iter()
                .map(|metric| metric.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Cursor Models", "Other Models"]
        );
        assert!(metrics
            .iter()
            .all(|metric| { metric.resets_at.as_deref() == Some("2026-10-23T10:47:06.910Z") }));
    }

    #[test]
    fn monthly_plan_keeps_distinct_grok_bot_pool() {
        let plan = json!({
            "planInfo": { "includedUsagePeriod": "INCLUDED_USAGE_PERIOD_MONTHLY" }
        });
        let sand = json!({
            "usagePercent": 18.5,
            "hasNonZeroIncludedLimit": true,
            "isUnified": false
        });
        let parsed = parse_sand_usage(&sand).unwrap().unwrap();
        assert!(!sand_replaces_cursor_tracks(
            &parse_plan_info(Some(&plan)),
            &parsed
        ));
    }

    #[test]
    fn parse_sand_usage_weekly_percent() {
        let sand = json!({
            "hasNonZeroIncludedLimit": true,
            "usagePercent": 15.954741,
            "nextResetTimestampUtc": "2026-10-06T09:12:29.574Z",
            "grokPlanLabel": "Grok Bot Plan",
            "cursorPlanName": "Ultra"
        });
        let parsed = parse_sand_usage(&sand).unwrap().unwrap();
        assert_eq!(parsed.metric.label, "Weekly");
        assert!((parsed.metric.used_percent - 15.95).abs() < 0.01);
        assert!((parsed.metric.remaining_percent - 84.05).abs() < 0.01);
        assert_eq!(
            parsed.metric.resets_at.as_deref(),
            Some("2026-10-06T09:12:29.574Z")
        );
        assert_eq!(parsed.plan.as_deref(), Some("Grok Bot Plan"));
    }

    #[test]
    fn no_personal_grok_bot_quota_is_a_successful_empty_result() {
        for response in [
            json!({"usesPooledEnterpriseAllowance": true, "usagePercent": 10.0}),
            json!({"hasNonZeroIncludedLimit": false, "usagePercent": 10.0}),
            json!({"includedLimitZero": true, "usagePercent": 0.0}),
            json!({}),
        ] {
            assert!(
                parse_sand_usage(&response).unwrap().is_none(),
                "no personal quota must not create a fetch diagnostic"
            );
        }
    }

    #[test]
    fn invalid_grok_bot_percent_is_still_reported() {
        for percent in [json!(-1), json!("unrecognized")] {
            assert!(parse_sand_usage(
                &json!({"hasNonZeroIncludedLimit": true, "usagePercent": percent})
            )
            .is_err());
        }
        for response in [
            json!({"usagePercent": null}),
            json!({"hasNonZeroIncludedLimit": true}),
            json!({"hasAvailableUsage": true}),
            json!({"isUnified": true}),
            json!({"code": "unauthenticated"}),
        ] {
            assert!(parse_sand_usage(&response).is_err());
        }
    }

    #[test]
    fn grok_bot_overage_is_clamped_to_zero_remaining() {
        let parsed = parse_sand_usage(&json!({
            "hasNonZeroIncludedLimit": true,
            "usagePercent": 140.0
        }))
        .unwrap()
        .unwrap();
        assert_eq!(parsed.metric.used_percent, 100.0);
        assert_eq!(parsed.metric.remaining_percent, 0.0);
    }

    #[test]
    fn parse_plan_window_from_remaining_and_limit() {
        let usage = json!({
            "planUsage": { "remaining": 25.0, "limit": 100.0 },
            "billingCycleEnd": null
        });
        let plan = json!({ "planInfo": { "billingCycleEnd": "2026-02-01T00:00:00Z" } });
        let window = parse_plan_window(&usage, Some(&plan)).unwrap();
        assert!((window.used_percent - 75.0).abs() < f64::EPSILON);
        assert!((window.remaining_percent - 25.0).abs() < f64::EPSILON);
        assert!(window
            .reset_at
            .as_deref()
            .unwrap()
            .starts_with("2026-02-01"));
    }

    #[test]
    fn extreme_cursor_timestamps_are_rejected_without_panicking() {
        assert!(millis_or_secs_iso(i64::MIN).is_none());
        assert!(millis_or_secs_iso(i64::MAX).is_none());
    }

    #[test]
    fn jwt_from_session_token_accepts_encoded_separator() {
        let jwt = "hdr.eyJzdWIiOiJ1c2VyX2FiYyIsImV4cCI6OTk5OTk5OTk5OX0.sig";
        let session = format!("user_abc%3A%3A{jwt}");
        assert_eq!(jwt_from_session_token(&session).as_deref(), Some(jwt));
    }

    #[test]
    fn access_token_usable_requires_three_jwt_segments() {
        assert!(access_token_usable("a.b.c"));
        assert!(!access_token_usable("ciphertext-or-empty"));
        assert!(!access_token_usable("a.b"));
    }

    fn auth_with_expiry(exp: i64, refresh: Option<&str>) -> LocalCursorAuth {
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(json!({"exp": exp}).to_string());
        LocalCursorAuth {
            access_token: format!("header.{payload}.signature"),
            refresh_token: refresh.map(str::to_string),
            email: None,
        }
    }

    fn auth_with_subject(
        exp: Option<i64>,
        subject: &str,
        refresh: Option<&str>,
        email: Option<&str>,
    ) -> LocalCursorAuth {
        use base64::Engine;
        let mut claims = serde_json::Map::from_iter([("sub".to_string(), json!(subject))]);
        if let Some(exp) = exp {
            claims.insert("exp".to_string(), json!(exp));
        }
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Value::Object(claims).to_string());
        LocalCursorAuth {
            access_token: format!("header.{payload}.signature"),
            refresh_token: refresh.map(str::to_string),
            email: email.map(str::to_string),
        }
    }

    #[test]
    fn db_jwt_keeps_matching_keychain_refresh_even_before_expiry() {
        for expiry in [900, 2000] {
            let db = auth_with_subject(Some(expiry), "auth0|account-a", None, None);
            let keychain_refresh =
                auth_with_subject(Some(2000), "auth0|account-a", None, None).access_token;
            let selected = select_local_auth(
                db.clone(),
                LocalCursorAuth {
                    refresh_token: Some(keychain_refresh.clone()),
                    ..Default::default()
                },
                None,
                1000,
            )
            .unwrap();
            assert_eq!(selected.access_token, db.access_token);
            assert_eq!(
                selected.refresh_token.as_deref(),
                Some(keychain_refresh.as_str())
            );
        }
    }

    #[test]
    fn expired_db_without_refresh_falls_through_to_keychain() {
        let keychain = auth_with_expiry(2000, None);
        let selected =
            select_local_auth(auth_with_expiry(900, None), keychain.clone(), None, 1000).unwrap();
        assert_eq!(selected.access_token, keychain.access_token);
    }

    #[test]
    fn refresh_tokens_are_not_crossed_between_cursor_accounts() {
        let db = auth_with_subject(Some(2000), "auth0|account-a", None, Some("a@example.com"));
        let keychain = auth_with_subject(
            Some(2000),
            "auth0|account-b",
            Some("account-b-refresh"),
            Some("a@example.com"),
        );
        let selected = select_local_auth(db.clone(), keychain, None, 1000).unwrap();
        assert_eq!(selected.access_token, db.access_token);
        assert!(selected.refresh_token.is_none());
    }

    #[test]
    fn refresh_only_keychain_entry_must_match_the_db_subject() {
        let db = auth_with_subject(Some(900), "auth0|account-a", None, None);
        let different_refresh =
            auth_with_subject(Some(2000), "auth0|account-b", None, None).access_token;
        let selected = select_local_auth(
            db,
            LocalCursorAuth {
                refresh_token: Some(different_refresh),
                ..Default::default()
            },
            None,
            1000,
        );
        assert!(selected.is_none());
    }

    #[test]
    fn fallback_account_does_not_inherit_another_accounts_email() {
        let keychain = auth_with_subject(
            Some(2000),
            "auth0|account-b",
            Some("account-b-refresh"),
            Some("a@example.com"),
        );
        let selected = select_local_auth(
            auth_with_subject(Some(900), "auth0|account-a", None, Some("a@example.com")),
            keychain.clone(),
            None,
            1000,
        )
        .unwrap();
        assert_eq!(selected.access_token, keychain.access_token);
        assert!(selected.email.is_none());
    }

    #[test]
    fn jwt_without_expiry_needs_refresh_or_falls_through() {
        let keychain = auth_with_subject(Some(2000), "auth0|account-b", None, None);
        let selected = select_local_auth(
            auth_with_subject(None, "auth0|account-a", None, None),
            keychain.clone(),
            None,
            1000,
        )
        .unwrap();
        assert_eq!(selected.access_token, keychain.access_token);
    }

    #[test]
    fn shared_refreshed_bearer_is_reused_by_both_usage_workers() {
        let refresh = "synthetic-shared-refresh-a";
        let successor = auth_with_subject(Some(3000), "auth0|account-a", None, None).access_token;
        lock_refreshed_auth().insert(
            refresh_key(refresh),
            RefreshedCursorAuth {
                access_token: successor.clone(),
                refresh_token: "synthetic-next-refresh-a".into(),
            },
        );
        let auth = auth_with_subject(Some(900), "auth0|account-a", Some(refresh), None);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        for _ in 0..2 {
            let bearer = runtime
                .block_on(bearer_for_request(&auth, 1000))
                .expect("cached successor");
            assert_eq!(bearer, successor);
        }
    }

    #[test]
    fn rejected_cursor_bearer_retries_once_with_shared_successor() {
        // `connect_post_with_retry` consults the real clock, so the synthetic
        // JWTs must be far-future. An expired successor would fall through the
        // cache into a live refresh request against Cursor's production
        // endpoint from inside this test.
        let refresh = "synthetic-shared-refresh-b";
        let old = auth_with_subject(Some(4_000_000_000), "auth0|account-a", Some(refresh), None);
        let successor =
            auth_with_subject(Some(5_000_000_000), "auth0|account-a", None, None).access_token;
        lock_refreshed_auth().insert(
            refresh_key(refresh),
            RefreshedCursorAuth {
                access_token: successor.clone(),
                refresh_token: "synthetic-next-refresh-b".into(),
            },
        );
        let (base, log) = spawn_server(|_, calls| {
            if calls == 0 {
                (401, "{}".into())
            } else {
                (200, r#"{"planUsage":{"totalPercentUsed":1}}"#.into())
            }
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut access = old.access_token.clone();
        let value = runtime
            .block_on(connect_post_with_retry(
                &tokscale_core::http::client(),
                &format!("{base}/usage"),
                &old,
                &mut access,
                "test",
            ))
            .unwrap();
        assert!(value.get("planUsage").is_some());
        assert_eq!(access, successor);
        let seen = log.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].bearer.as_deref(), Some(old.access_token.as_str()));
        assert_eq!(seen[1].bearer.as_deref(), Some(successor.as_str()));
    }

    #[test]
    fn expired_desktop_tokens_without_refresh_fall_through_to_saved_session() {
        let session = auth_with_expiry(2000, None);
        let selected = select_local_auth(
            auth_with_expiry(900, None),
            auth_with_expiry(800, None),
            Some(session.clone()),
            1000,
        )
        .unwrap();
        assert_eq!(selected.access_token, session.access_token);
        assert!(select_local_auth(
            auth_with_expiry(900, None),
            LocalCursorAuth::default(),
            None,
            1000
        )
        .is_none());
    }
}
