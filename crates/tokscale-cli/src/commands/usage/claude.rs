use anyhow::Result;
use serde::Deserialize;

use super::helpers::capitalize;
use super::{UsageMetric, UsageOutput};

const BETA_HEADER: &str = "oauth-2025-04-20";
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

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
}

#[derive(Debug, Deserialize)]
struct Window {
    utilization: f64,
    resets_at: Option<String>,
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

/// The whole Claude usage path, with the only two things a test cannot supply
/// for real -- the usage endpoint and the credential source -- as parameters.
/// The destructive refresh-and-write this module was fixed for lived in exactly
/// this orchestration, so it is the layer the tests must enter; [`fetch`] is one
/// call with the production values and holds no logic that could diverge.
fn fetch_blocking(usage_url: &str, read: CredentialReader) -> Result<UsageOutput> {
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
}
