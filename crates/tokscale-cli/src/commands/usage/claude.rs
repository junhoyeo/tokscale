use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use serde::Deserialize;

use super::helpers::capitalize;
use super::{UsageMetric, UsageOutput};

const BETA_HEADER: &str = "oauth-2025-04-20";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

// Unix-epoch second; 0 = no cooldown.
static COOLDOWN_UNTIL: AtomicU64 = AtomicU64::new(0);

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn set_cooldown(retry_after_secs: u64) {
    let deadline = now_epoch_secs() + retry_after_secs;
    COOLDOWN_UNTIL.store(deadline, Ordering::Release);
}

fn cooldown_remaining() -> Option<u64> {
    let deadline = COOLDOWN_UNTIL.load(Ordering::Acquire);
    if deadline == 0 {
        return None;
    }
    let now = now_epoch_secs();
    if now >= deadline {
        COOLDOWN_UNTIL.store(0, Ordering::Release);
        None
    } else {
        Some(deadline - now)
    }
}

#[cfg(test)]
fn clear_cooldown() {
    COOLDOWN_UNTIL.store(0, Ordering::Release);
}

// `~/.claude/.credentials.json` belongs to Claude Code, and this module is a
// quota viewer: it reads that file and never writes it. Tokscale used to
// exchange Claude Code's refresh token on 401/403 and write the result back,
// but the write reconstructed the document from the four fields below, dropping
// every field tokscale does not model -- `expiresAt` and `scopes` among them --
// which left Claude Code reporting "Not logged in" (#1001). An expired access
// token is Claude Code's to refresh on its next run, so a rejected token is
// reported as unavailable usage instead.

#[derive(Debug, Deserialize)]
struct Credentials {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<Oauth>,
}

/// Deliberately does not model `refreshToken`: tokscale has no use for a
/// credential it must not spend.
#[derive(Debug, Deserialize)]
struct Oauth {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
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

/// How [`fetch_blocking`] obtains Claude Code's credential document. Injected
/// so tests can also drive the Keychain-sourced shape -- credentials present,
/// no file on disk -- which no test can produce through [`read_credentials`]
/// because the Keychain is a real macOS service.
type CredentialReader = fn() -> Result<Credentials>;

fn read_credentials() -> Result<Credentials> {
    let path = credentials_path();
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(creds) = serde_json::from_str::<Credentials>(&content) {
                return Ok(creds);
            }
        }
    }
    let content = read_keychain()?;
    Ok(serde_json::from_str(&content)?)
}

async fn fetch_usage(
    client: &reqwest::Client,
    usage_url: &str,
    token: &str,
) -> Result<UsageResponse> {
    let resp = client
        .get(usage_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("anthropic-beta", BETA_HEADER)
        .send()
        .await?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!(
            "Claude usage unavailable: stored access token was rejected (HTTP {status}). \
             Run 'claude' so Claude Code can refresh its own login, then retry."
        );
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let wait = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5);
        set_cooldown(wait);
        anyhow::bail!("Claude usage rate-limited (HTTP 429), cooling down for {wait}s");
    }
    if !status.is_success() {
        anyhow::bail!("Claude usage request failed (HTTP {status})");
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
/// this orchestration, so it is the layer the tests must enter; [`fetch`] is one
/// call with the production values and holds no logic that could diverge.
fn fetch_blocking(usage_url: &str, read: CredentialReader) -> Result<UsageOutput> {
    if let Some(remaining) = cooldown_remaining() {
        anyhow::bail!("Claude usage rate-limited — retry in {remaining}s.");
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let creds = read()?;
        let oauth = creds.claude_ai_oauth.ok_or_else(|| {
            anyhow::anyhow!("No Claude OAuth credentials. Run 'claude' to log in.")
        })?;
        let access_token = oauth
            .access_token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("No Claude access token."))?;
        let plan = oauth.subscription_type.as_ref().map(|s| {
            let tier = oauth
                .rate_limit_tier
                .as_deref()
                .and_then(|t| t.rsplit('_').next());
            match tier {
                Some(mult) => format!("{} {}", capitalize(s), mult),
                None => capitalize(s),
            }
        });

        let client = reqwest::Client::new();
        let resp = fetch_usage(&client, usage_url, access_token).await?;

        let metrics = usage_metrics(&resp);

        Ok(UsageOutput {
            provider: "Claude".into(),
            account: None,
            credential_source: None,
            plan,
            email: None,
            metrics,
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        })
    })
}

pub fn fetch() -> Result<UsageOutput> {
    fetch_blocking(USAGE_URL, read_credentials)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::commands::usage::test_server::{spawn_server, Seen};

    /// A Claude Code credential document with the fields tokscale models
    /// (`accessToken`, `subscriptionType`, `rateLimitTier`), the fields it does
    /// not (`refreshToken`, `expiresAt`, `scopes`), and a key that does not
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

    /// #1001: a rejected access token must not make tokscale rewrite Claude
    /// Code's credential file. Byte equality is the assertion that matters --
    /// any reconstruction of the document fails it, whatever fields it keeps.
    /// Entry is through `fetch_blocking` with the real `read_credentials`, so
    /// the file is genuinely read and re-read across the whole orchestration
    /// the destructive code used to live in.
    #[test]
    #[serial_test::serial]
    fn rejected_token_leaves_claude_credentials_untouched() {
        clear_cooldown();
        let home = HomeGuard::new("401");
        home.write_fixture();
        let before = std::fs::read(home.credentials()).expect("read before");
        let (usage_url, log) = spawn_usage_server(401);

        let result = fetch_blocking(&usage_url, read_credentials);

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

        let result = fetch_blocking(&usage_url, read_credentials);

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

        let output =
            fetch_blocking(&usage_url, read_credentials).expect("200 usage response should parse");

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

    /// `read_credentials()` is the source of truth for what tokscale parses.
    /// The refresh token must not survive the round trip: the fix is only real
    /// if the field is absent from the model, not merely unused by the caller.
    #[test]
    #[serial_test::serial]
    fn parsed_credentials_do_not_carry_a_refresh_token() {
        let home = HomeGuard::new("parse");
        home.write_fixture();

        let oauth = read_credentials()
            .expect("fixture credentials parse")
            .claude_ai_oauth
            .expect("fixture has claudeAiOauth");

        assert_eq!(oauth.access_token.as_deref(), Some("stale-access-token"));
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
        assert!(cooldown_remaining().is_none());
    }

    #[test]
    #[serial_test::serial]
    fn set_cooldown_activates_and_expires() {
        clear_cooldown();
        set_cooldown(1);
        assert!(cooldown_remaining().is_some());
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(cooldown_remaining().is_none());
    }

    #[test]
    #[serial_test::serial]
    fn cooldown_blocks_fetch_blocking() {
        clear_cooldown();
        set_cooldown(60);
        let (usage_url, _log) = spawn_usage_server(200);
        let result = fetch_blocking(&usage_url, keychain_credentials);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("rate-limited"),
            "expected cooldown error, got: {err}"
        );
        clear_cooldown();
    }
}
