//! Cursor subscription plan remaining for `tokscale usage`.
//!
//! Auth comes from the desktop `state.vscdb` JWT, then the macOS Keychain items
//! `cursor-access-token` / `cursor-refresh-token`, then a JWT embedded in a
//! saved Tokscale Cursor session cookie. The IDE plan window is
//! `GetCurrentPeriodUsage` on `api2.cursor.sh`. Grok Bot weekly usage is a
//! separate Cursor-metered pool from `GetSandUsageStatus` (not the SuperGrok
//! `cli-chat-proxy` credits used by the Grok card). Refreshed
//! access tokens stay in memory and are never written back.

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde_json::Value;

use super::{UsageMetric, UsageOutput};

const USAGE_URL: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage";
const PLAN_URL: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService/GetPlanInfo";
/// Grok Bot weekly included usage. Metered on the Cursor account, not on xAI.
const SAND_USAGE_URL: &str =
    "https://api2.cursor.sh/aiserver.v1.DashboardService/GetSandUsageStatus";
const REFRESH_URL: &str = "https://api2.cursor.sh/oauth/token";
/// Public Cursor Auth0 client id. Not a user secret.
const CLIENT_ID: &str = "KbZUR41cY7W6zRSdpSUJ7I7mLYBKOCmB";
const ACCESS_SERVICE: &str = "cursor-access-token";
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

#[derive(Debug, Clone, Default)]
struct LocalCursorAuth {
    access_token: String,
    refresh_token: Option<String>,
    email: Option<String>,
}

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
        let access = bearer_for_request(&auth, now).await?;
        let client = plan_http_client()?;
        let usage = connect_post(&client, USAGE_URL, &access, "GetCurrentPeriodUsage").await?;
        let plan = if billing_reset_iso(usage.get("billingCycleEnd")).is_none() {
            connect_post(&client, PLAN_URL, &access, "GetPlanInfo")
                .await
                .ok()
        } else {
            None
        };
        let tracks = parse_plan_tracks(&usage, plan.as_ref())?;
        let plan_name = plan
            .as_ref()
            .and_then(|value| value.pointer("/planInfo/planName"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .or_else(|| {
                usage
                    .get("membershipType")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
            });
        let metrics = metrics_from_tracks(&tracks);
        if metrics.is_empty() {
            anyhow::bail!("Cursor planUsage had no Auto, API, or total percent");
        }

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

/// Grok Bot weekly included usage for the active Cursor desktop account.
pub fn fetch_grok_bot() -> Result<Vec<UsageOutput>> {
    let auth = resolve_local_auth()?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let now = Utc::now().timestamp();
        let access = bearer_for_request(&auth, now).await?;
        let client = plan_http_client()?;
        let sand = connect_post(&client, SAND_USAGE_URL, &access, "GetSandUsageStatus").await?;
        let Some(parsed) = parse_sand_usage(&sand)? else {
            return Ok(Vec::new());
        };
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
}

/// Parse `GetSandUsageStatus`. Hide when the account has no personal included
/// Grok Bot allowance (pooled enterprise, zero limit, or missing percent).
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
    {
        return Ok(None);
    }
    let Some(percent) = value
        .get("usagePercent")
        .filter(|percent| !percent.is_null())
    else {
        return Ok(None);
    };
    let used = percent
        .as_f64()
        .filter(|percent| percent.is_finite() && (0.0..=100.0).contains(percent))
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
    }))
}

fn sand_reset_iso(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    DateTime::parse_from_rfc3339(text).ok().map(|parsed| {
        parsed
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true)
    })
}

fn metrics_from_tracks(tracks: &ParsedPlanTracks) -> Vec<UsageMetric> {
    let mut metrics = Vec::new();
    if let Some(auto) = tracks.auto.as_ref() {
        metrics.push(metric_from_window("Auto", auto, tracks.reset_at.clone()));
    }
    if let Some(api) = tracks.api.as_ref() {
        metrics.push(metric_from_window("API", api, tracks.reset_at.clone()));
    }
    if metrics.is_empty() {
        if let Some(total) = tracks.total.as_ref() {
            metrics.push(metric_from_window("Plan", total, tracks.reset_at.clone()));
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
        resets_at: window.reset_at.clone().or(reset_at),
    }
}

fn plan_http_client() -> Result<reqwest::Client> {
    // Match the Cursor CLI client: cursor.com / api2 sit behind fingerprints
    // that reject rustls on some networks (#1250).
    #[allow(clippy::disallowed_methods)]
    let builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));
    #[cfg(not(target_os = "android"))]
    let builder = builder.use_native_tls();
    builder
        .build()
        .context("Failed to build Cursor plan HTTP client")
}

fn access_token_usable(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3 && parts.iter().all(|part| !part.is_empty())
}

fn needs_refresh(token: &str, now_unix: i64) -> bool {
    jwt_exp(token).is_some_and(|exp| exp <= now_unix.saturating_add(60))
}

fn jwt_exp(token: &str) -> Option<i64> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value.get("exp").and_then(Value::as_i64)
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

    #[cfg(target_os = "macos")]
    let keychain = LocalCursorAuth {
        access_token: super::helpers::read_keychain(ACCESS_SERVICE).unwrap_or_default(),
        refresh_token: super::helpers::read_keychain(REFRESH_SERVICE)
            .ok()
            .and_then(|token| nonempty(Some(&token))),
        email: account_label(db_email.as_deref()),
    };
    #[cfg(not(target_os = "macos"))]
    let keychain = LocalCursorAuth::default();

    let db = LocalCursorAuth {
        access_token: db_access.unwrap_or_default(),
        refresh_token: nonempty(db_refresh.as_deref()),
        email: account_label(db_email.as_deref()),
    };
    let now_unix = Utc::now().timestamp();
    if let Some(auth) = select_local_auth(db.clone(), keychain.clone(), None, now_unix) {
        return Ok(auth);
    }
    let session = crate::cursor::load_active_credentials().and_then(|creds| {
        jwt_from_session_token(&creds.session_token).map(|access| LocalCursorAuth {
            access_token: access,
            refresh_token: None,
            email: account_label(db_email.as_deref()),
        })
    });
    if let Some(auth) = select_local_auth(db, keychain, session, now_unix) {
        return Ok(auth);
    }

    anyhow::bail!(
        "Cursor plan credentials not found. Sign in to the Cursor desktop app, or run 'tokscale cursor login'."
    )
}

fn select_local_auth(
    mut db: LocalCursorAuth,
    mut keychain: LocalCursorAuth,
    session: Option<LocalCursorAuth>,
    now_unix: i64,
) -> Option<LocalCursorAuth> {
    // Cursor can keep the access JWT in state.vscdb and the refresh token in
    // Keychain. Merge the two stores before considering an expired JWT.
    keychain.refresh_token = keychain.refresh_token.or_else(|| db.refresh_token.clone());
    db.refresh_token = db.refresh_token.or_else(|| keychain.refresh_token.clone());
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

async fn bearer_for_request(auth: &LocalCursorAuth, now_unix: i64) -> Result<String> {
    if !needs_refresh(&auth.access_token, now_unix) {
        return Ok(auth.access_token.clone());
    }
    let Some(refresh) = auth.refresh_token.as_deref() else {
        anyhow::bail!("Cursor access token is expired and no refresh token was stored.");
    };
    refresh_access_token(refresh).await
}

async fn refresh_access_token(refresh_token: &str) -> Result<String> {
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
        .ok_or_else(|| anyhow::anyhow!("Cursor token refresh returned no access token."))?;
    Ok(access.to_string())
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
    if !status.is_success() {
        anyhow::bail!("Cursor {label} returned HTTP {status}");
    }
    serde_json::from_str(&body).with_context(|| format!("Cursor {label} was not JSON"))
}

fn parse_plan_tracks(usage: &Value, plan: Option<&Value>) -> Result<ParsedPlanTracks> {
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
    if used.is_finite() && (0.0..=100.0).contains(&used) {
        Some(round_pair(used))
    } else {
        None
    }
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
        if used.is_finite() && (0.0..=100.0).contains(&used) {
            return Ok(round_pair(used));
        }
    }
    let remaining = plan_usage.get("remaining").and_then(Value::as_f64);
    let limit = plan_usage.get("limit").and_then(Value::as_f64);
    match (remaining, limit) {
        (Some(remaining), Some(limit))
            if remaining.is_finite()
                && limit.is_finite()
                && limit > 0.0
                && remaining >= 0.0
                && remaining <= limit =>
        {
            let used = ((limit - remaining) / limit) * 100.0;
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
    let ms = if raw.abs() >= 1_000_000_000_000 {
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
        let metrics = metrics_from_tracks(&tracks);
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].label, "Auto");
        assert!((metrics[0].remaining_percent - 84.0).abs() < f64::EPSILON);
        assert_eq!(metrics[1].label, "API");
        assert!((metrics[1].remaining_percent - 64.0).abs() < f64::EPSILON);
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
            json!({"hasNonZeroIncludedLimit": true}),
        ] {
            assert!(
                parse_sand_usage(&response).unwrap().is_none(),
                "no personal quota must not create a fetch diagnostic"
            );
        }
    }

    #[test]
    fn invalid_grok_bot_percent_is_still_reported() {
        for percent in [json!(-1), json!(101), json!("unrecognized")] {
            assert!(parse_sand_usage(
                &json!({"hasNonZeroIncludedLimit": true, "usagePercent": percent})
            )
            .is_err());
        }
        assert!(parse_sand_usage(&json!({"usagePercent": null}))
            .unwrap()
            .is_none());
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

    #[test]
    fn db_jwt_keeps_keychain_refresh_even_before_expiry() {
        for expiry in [900, 2000] {
            let db = auth_with_expiry(expiry, None);
            let selected = select_local_auth(
                db.clone(),
                LocalCursorAuth {
                    refresh_token: Some("keychain-refresh".into()),
                    ..Default::default()
                },
                None,
                1000,
            )
            .unwrap();
            assert_eq!(selected.access_token, db.access_token);
            assert_eq!(selected.refresh_token.as_deref(), Some("keychain-refresh"));
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
