use anyhow::Result;
use serde::Deserialize;
use std::path::{Path, PathBuf};

use super::{UsageMetric, UsageOutput};

const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";

#[derive(Debug, Deserialize)]
struct ApiResponse {
    usage: Option<UsageData>,
}

#[derive(Debug, Deserialize)]
struct UsageData {
    rolling: Option<WindowMetric>,
    weekly: Option<WindowMetric>,
    monthly: Option<WindowMetric>,
}

#[derive(Debug, Deserialize)]
struct WindowMetric {
    status: Option<String>,
    percent: Option<f64>,
    #[serde(rename = "resetsAt")]
    resets_at: Option<String>,
}

/// The server returns `{ type: "error", error: { type, message } }` on 401/403.
#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: Option<ErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    #[serde(rename = "type")]
    error_type: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthDocument {
    #[serde(rename = "opencode-go")]
    opencode_go: Option<AuthEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AuthEntry {
    #[serde(rename = "api")]
    Api { key: String },
    #[serde(other)]
    Other,
}

fn auth_path() -> PathBuf {
    let data_dir = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            crate::paths::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local")
                .join("share")
        });
    data_dir.join("opencode").join("auth.json")
}

fn read_key_from_auth_at(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    parse_key_from_auth(&content)
}

fn read_key_from_auth() -> Option<String> {
    read_key_from_auth_at(&auth_path())
}

fn parse_key_from_auth(content: &str) -> Option<String> {
    let doc: AuthDocument = serde_json::from_str(content).ok()?;
    match doc.opencode_go? {
        AuthEntry::Api { key } if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn read_api_key_at(path: &Path) -> Result<String> {
    if let Some(key) = read_key_from_auth_at(path) {
        return Ok(key);
    }
    if let Ok(key) = std::env::var("OPENCODE_API_KEY") {
        if !key.trim().is_empty() {
            return Ok(key);
        }
    }
    anyhow::bail!(
        "No OpenCode Go credentials found. \
         Run '/connect' in OpenCode to set up OpenCode Go, \
         or set OPENCODE_API_KEY."
    )
}

fn parse_error(body: &str) -> Option<ErrorDetail> {
    serde_json::from_str::<ErrorResponse>(body).ok()?.error
}

fn extract_error_message(body: &str) -> Option<String> {
    parse_error(body)?.message.filter(|m| !m.trim().is_empty())
}

fn is_entitlement_error(body: &str) -> bool {
    parse_error(body)
        .and_then(|d| d.error_type)
        .map(|t| t == "EntitlementError")
        .unwrap_or(false)
}

fn window_to_metric(label: &str, w: &WindowMetric) -> UsageMetric {
    let used = w.percent.unwrap_or(0.0).clamp(0.0, 100.0);
    UsageMetric {
        label: label.into(),
        used_percent: used,
        remaining_percent: 100.0 - used,
        remaining_label: w
            .status
            .as_deref()
            .filter(|s| *s == "rate-limited")
            .map(|_| "rate-limited".into()),
        resets_at: w.resets_at.clone(),
    }
}

fn usage_metrics(resp: &ApiResponse) -> Vec<UsageMetric> {
    let Some(ref usage) = resp.usage else {
        return Vec::new();
    };
    let mut metrics = Vec::new();
    if let Some(ref w) = usage.rolling {
        metrics.push(window_to_metric("Rolling", w));
    }
    if let Some(ref w) = usage.weekly {
        metrics.push(window_to_metric("Weekly", w));
    }
    if let Some(ref w) = usage.monthly {
        metrics.push(window_to_metric("Monthly", w));
    }
    metrics
}

pub fn has_credentials() -> bool {
    read_key_from_auth().is_some()
        || std::env::var("OPENCODE_API_KEY")
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false)
}

/// Injected URL and auth path so tests can point at a local server and a
/// temp credential file without touching the real filesystem or network.
fn fetch_blocking(usage_url: &str, auth_path: &Path) -> Result<Vec<UsageOutput>> {
    let api_key = read_api_key_at(auth_path)?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        let resp = client
            .get(usage_url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Accept", "application/json")
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let server_msg = extract_error_message(&body);

            if status == reqwest::StatusCode::UNAUTHORIZED {
                let detail = server_msg.unwrap_or_else(|| "API key rejected".into());
                anyhow::bail!(
                    "{detail} (HTTP 401). \
                     Run '/connect' in OpenCode to refresh, or check OPENCODE_API_KEY."
                );
            }
            if status == reqwest::StatusCode::FORBIDDEN && is_entitlement_error(&body) {
                return Ok(Vec::new());
            }
            if status == reqwest::StatusCode::FORBIDDEN {
                let detail = server_msg.unwrap_or_else(|| "Forbidden".into());
                anyhow::bail!("{detail} (HTTP 403)");
            }
            anyhow::bail!(
                "usage request failed (HTTP {status}): {}",
                server_msg.unwrap_or_else(|| body.chars().take(200).collect::<String>())
            );
        }

        let body: ApiResponse = resp.json().await?;
        let metrics = usage_metrics(&body);

        Ok(vec![UsageOutput {
            provider: "OpenCode Go".into(),
            account: None,
            credential_source: None,
            plan: Some("Go".into()),
            email: None,
            metrics,
            reset_credits: None,
            credit_status: None,
            spend_control: None,
        }])
    })
}

pub fn fetch_all() -> Result<Vec<UsageOutput>> {
    fetch_blocking(USAGE_URL, &auth_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::commands::usage::test_server::{spawn_server, Seen};

    #[test]
    fn parses_api_key_from_auth_document() {
        let key = parse_key_from_auth(r#"{"opencode-go":{"type":"api","key":"sk-test-key-123"}}"#);
        assert_eq!(key.as_deref(), Some("sk-test-key-123"));
    }

    #[test]
    fn ignores_oauth_entry_in_auth_document() {
        let key = parse_key_from_auth(
            r#"{"opencode-go":{"type":"oauth","access":"tok","refresh":"ref","expires":9999}}"#,
        );
        assert!(key.is_none(), "oauth entries must not produce an API key");
    }

    #[test]
    fn ignores_empty_api_key() {
        assert!(parse_key_from_auth(r#"{"opencode-go":{"type":"api","key":""}}"#).is_none());
        assert!(parse_key_from_auth(r#"{"opencode-go":{"type":"api","key":"   "}}"#).is_none());
    }

    #[test]
    fn returns_none_when_opencode_go_entry_missing() {
        assert!(parse_key_from_auth(
            r#"{"openai":{"type":"oauth","access":"tok","accountId":"acct"}}"#
        )
        .is_none());
    }

    #[test]
    fn returns_none_for_malformed_json() {
        assert!(parse_key_from_auth("not json").is_none());
        assert!(parse_key_from_auth("").is_none());
    }

    #[test]
    fn reads_key_from_auth_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("auth.json");
        std::fs::write(
            &path,
            r#"{"opencode-go":{"type":"api","key":"sk-file-key"}}"#,
        )
        .unwrap();

        assert_eq!(read_key_from_auth_at(&path).as_deref(), Some("sk-file-key"));
    }

    #[test]
    fn returns_none_for_missing_auth_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.json");
        assert!(read_key_from_auth_at(&path).is_none());
    }

    #[test]
    #[serial_test::serial]
    fn auth_path_honors_xdg_data_home() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = EnvVarGuard::set("XDG_DATA_HOME", tmp.path().to_str().unwrap());

        assert_eq!(auth_path(), tmp.path().join("opencode").join("auth.json"));
    }

    #[test]
    #[serial_test::serial]
    fn auth_path_ignores_blank_xdg_data_home() {
        let _guard = EnvVarGuard::set("XDG_DATA_HOME", "");
        let unset = {
            let _g = EnvVarGuard::remove("XDG_DATA_HOME");
            auth_path()
        };

        let blank = auth_path();
        assert_eq!(blank, unset);
    }

    #[test]
    #[serial_test::serial]
    fn prefers_auth_file_over_env_var() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("auth.json");
        std::fs::write(
            &path,
            r#"{"opencode-go":{"type":"api","key":"sk-from-file"}}"#,
        )
        .unwrap();
        let _guard = EnvVarGuard::set("OPENCODE_API_KEY", "sk-from-env");

        let key = read_api_key_at(&path).unwrap();
        assert_eq!(key, "sk-from-file");
    }

    #[test]
    #[serial_test::serial]
    fn falls_back_to_env_var_when_auth_file_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("no-file.json");
        let _guard = EnvVarGuard::set("OPENCODE_API_KEY", "sk-env-fallback");

        let key = read_api_key_at(&path).unwrap();
        assert_eq!(key, "sk-env-fallback");
    }

    #[test]
    #[serial_test::serial]
    fn errors_when_no_credentials_available() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("no-file.json");
        let _guard = EnvVarGuard::remove("OPENCODE_API_KEY");

        let err = read_api_key_at(&path).unwrap_err();
        assert!(
            err.to_string().contains("/connect"),
            "error should guide the user: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn blank_env_var_is_not_a_credential() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("no-file.json");
        let _guard = EnvVarGuard::set("OPENCODE_API_KEY", "   ");

        assert!(read_api_key_at(&path).is_err());
    }

    #[test]
    fn window_to_metric_normal_usage() {
        let w = WindowMetric {
            status: Some("ok".into()),
            percent: Some(42.5),
            resets_at: Some("2026-09-01T00:00:00Z".into()),
        };
        let m = window_to_metric("Rolling", &w);
        assert_eq!(m.label, "Rolling");
        assert!((m.used_percent - 42.5).abs() < f64::EPSILON);
        assert!((m.remaining_percent - 57.5).abs() < f64::EPSILON);
        assert!(m.remaining_label.is_none());
        assert_eq!(m.resets_at.as_deref(), Some("2026-09-01T00:00:00Z"));
    }

    #[test]
    fn window_to_metric_rate_limited() {
        let w = WindowMetric {
            status: Some("rate-limited".into()),
            percent: Some(100.0),
            resets_at: None,
        };
        let m = window_to_metric("Weekly", &w);
        assert_eq!(m.remaining_label.as_deref(), Some("rate-limited"));
    }

    #[test]
    fn window_to_metric_clamps_out_of_range_percent() {
        let over = WindowMetric {
            status: None,
            percent: Some(150.0),
            resets_at: None,
        };
        let m = window_to_metric("X", &over);
        assert_eq!(m.used_percent, 100.0);
        assert_eq!(m.remaining_percent, 0.0);

        let under = WindowMetric {
            status: None,
            percent: Some(-10.0),
            resets_at: None,
        };
        let m = window_to_metric("X", &under);
        assert_eq!(m.used_percent, 0.0);
        assert_eq!(m.remaining_percent, 100.0);
    }

    #[test]
    fn window_to_metric_missing_percent_defaults_to_zero() {
        let w = WindowMetric {
            status: None,
            percent: None,
            resets_at: None,
        };
        let m = window_to_metric("Rolling", &w);
        assert_eq!(m.used_percent, 0.0);
        assert_eq!(m.remaining_percent, 100.0);
    }

    #[test]
    fn usage_metrics_parses_all_three_windows() {
        let resp: ApiResponse = serde_json::from_str(FULL_USAGE_BODY).unwrap();
        let metrics = usage_metrics(&resp);

        assert_eq!(metrics.len(), 3);
        assert_eq!(metrics[0].label, "Rolling");
        assert!((metrics[0].used_percent - 25.0).abs() < f64::EPSILON);
        assert_eq!(metrics[1].label, "Weekly");
        assert!((metrics[1].used_percent - 50.0).abs() < f64::EPSILON);
        assert_eq!(metrics[2].label, "Monthly");
        assert!((metrics[2].used_percent - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn usage_metrics_handles_partial_response() {
        let resp: ApiResponse = serde_json::from_str(
            r#"{"usage":{"weekly":{"status":"ok","percent":33,"resetsAt":"2026-09-01T00:00:00Z"}}}"#,
        )
        .unwrap();
        let metrics = usage_metrics(&resp);
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].label, "Weekly");
    }

    #[test]
    fn usage_metrics_empty_when_no_usage() {
        let resp: ApiResponse = serde_json::from_str(r#"{}"#).unwrap();
        assert!(usage_metrics(&resp).is_empty());
    }

    #[test]
    fn extract_error_message_parses_server_error() {
        let msg = extract_error_message(
            r#"{"type":"error","error":{"type":"AuthError","message":"Missing API key."}}"#,
        );
        assert_eq!(msg.as_deref(), Some("Missing API key."));
    }

    #[test]
    fn extract_error_message_returns_none_for_non_error_body() {
        assert!(extract_error_message(r#"{"usage":{}}"#).is_none());
        assert!(extract_error_message("not json").is_none());
        assert!(extract_error_message("").is_none());
    }

    const USAGE_PATH: &str = "/zen/go/v1/usage";

    const FULL_USAGE_BODY: &str = r#"{
  "usage": {
    "rolling": {
      "status": "ok",
      "percent": 25.0,
      "resetsAt": "2026-09-01T02:15:00Z"
    },
    "weekly": {
      "status": "ok",
      "percent": 50.0,
      "resetsAt": "2026-09-05T00:00:00Z"
    },
    "monthly": {
      "status": "ok",
      "percent": 10.0,
      "resetsAt": "2026-10-01T00:00:00Z"
    }
  }
}"#;

    const AUTH_ERROR_BODY: &str =
        r#"{"type":"error","error":{"type":"AuthError","message":"Missing API key."}}"#;
    const ENTITLEMENT_ERROR_BODY: &str = r#"{"type":"error","error":{"type":"EntitlementError","message":"OpenCode Go subscription required."}}"#;

    /// Non-JSON HTML error page longer than 200 chars; the marker sits past
    /// the truncation point.
    const CDN_ERROR_BODY: &str = concat!(
        "<html><head><title>502 Bad Gateway</title></head><body>",
        "........................................................",
        "........................................................",
        "........................................................",
        "<p>TAIL-MARKER</p></body></html>"
    );

    fn write_auth(dir: &Path, key: &str) -> PathBuf {
        let path = dir.join("auth.json");
        std::fs::write(
            &path,
            format!(r#"{{"opencode-go":{{"type":"api","key":"{key}"}}}}"#),
        )
        .unwrap();
        path
    }

    fn spawn_usage_server(status: u16, body: &'static str) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let (base, log) = spawn_server(move |path, _| {
            if path == USAGE_PATH {
                (status, body.to_string())
            } else {
                (404, "{}".to_string())
            }
        });
        (format!("{base}{USAGE_PATH}"), log)
    }

    #[test]
    #[serial_test::serial]
    fn successful_fetch_returns_all_metrics() {
        let tmp = tempfile::TempDir::new().unwrap();
        let auth = write_auth(tmp.path(), "sk-test");
        let _guard = EnvVarGuard::remove("OPENCODE_API_KEY");
        let (url, log) = spawn_usage_server(200, FULL_USAGE_BODY);

        let outputs = fetch_blocking(&url, &auth).expect("200 should parse");
        assert_eq!(outputs.len(), 1);
        let output = &outputs[0];

        assert_eq!(output.provider, "OpenCode Go");
        assert_eq!(output.plan.as_deref(), Some("Go"));
        assert_eq!(output.metrics.len(), 3);
        assert_eq!(output.metrics[0].label, "Rolling");
        assert_eq!(output.metrics[1].label, "Weekly");
        assert_eq!(output.metrics[2].label, "Monthly");

        let seen = log.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].request, format!("GET {USAGE_PATH}"));
        assert_eq!(seen[0].bearer.as_deref(), Some("sk-test"));
    }

    /// The server returns `AuthError` with "Missing API key." on 401. The
    /// error must surface that server message alongside actionable guidance.
    #[test]
    #[serial_test::serial]
    fn unauthorized_response_surfaces_server_message() {
        let tmp = tempfile::TempDir::new().unwrap();
        let auth = write_auth(tmp.path(), "sk-expired");
        let _guard = EnvVarGuard::remove("OPENCODE_API_KEY");
        let (url, _log) = spawn_usage_server(401, AUTH_ERROR_BODY);

        let err = fetch_blocking(&url, &auth).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Missing API key."),
            "should include server message: {msg}"
        );
        assert!(msg.contains("401"), "should mention HTTP status: {msg}");
        assert!(
            msg.contains("/connect"),
            "should guide user to /connect: {msg}"
        );
        assert!(
            !msg.starts_with("OpenCode Go:"),
            "error must not duplicate the provider prefix: {msg}"
        );
    }

    /// 403 means the user has an API key but no active Go subscription.
    /// This is not an error — the provider is silently omitted.
    #[test]
    #[serial_test::serial]
    fn forbidden_response_returns_empty_output() {
        let tmp = tempfile::TempDir::new().unwrap();
        let auth = write_auth(tmp.path(), "sk-no-sub");
        let _guard = EnvVarGuard::remove("OPENCODE_API_KEY");
        let (url, _log) = spawn_usage_server(403, ENTITLEMENT_ERROR_BODY);

        let outputs = fetch_blocking(&url, &auth).expect("403 entitlement must not be an error");
        assert!(outputs.is_empty(), "no-subscription must return empty vec");
    }

    #[test]
    #[serial_test::serial]
    fn non_entitlement_forbidden_is_an_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let auth = write_auth(tmp.path(), "sk-other");
        let _guard = EnvVarGuard::remove("OPENCODE_API_KEY");
        let (url, _log) = spawn_usage_server(
            403,
            r#"{"type":"error","error":{"type":"OtherError","message":"Something else."}}"#,
        );

        let err = fetch_blocking(&url, &auth).unwrap_err();
        assert!(
            err.to_string().contains("403"),
            "non-entitlement 403 must surface as error: {err}"
        );
    }

    /// A CDN 502 returns a full HTML page, not the API's JSON error shape.
    /// The one-line diagnostic must not dump the whole body.
    #[test]
    #[serial_test::serial]
    fn generic_failure_truncates_non_json_body() {
        let tmp = tempfile::TempDir::new().unwrap();
        let auth = write_auth(tmp.path(), "sk-cdn");
        let _guard = EnvVarGuard::remove("OPENCODE_API_KEY");
        let (url, _log) = spawn_usage_server(502, CDN_ERROR_BODY);

        let err = fetch_blocking(&url, &auth).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("502"), "should mention HTTP status: {msg}");
        assert!(
            !msg.contains("TAIL-MARKER"),
            "body beyond 200 chars must not appear: {msg}"
        );
    }

    #[test]
    fn is_entitlement_error_detects_entitlement_type() {
        assert!(is_entitlement_error(ENTITLEMENT_ERROR_BODY));
        assert!(!is_entitlement_error(AUTH_ERROR_BODY));
        assert!(!is_entitlement_error(
            r#"{"type":"error","error":{"type":"OtherError"}}"#
        ));
        assert!(!is_entitlement_error("not json"));
        assert!(!is_entitlement_error(""));
    }

    #[test]
    #[serial_test::serial]
    fn fetch_uses_env_var_when_auth_file_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let auth = tmp.path().join("no-file.json");
        let _guard = EnvVarGuard::set("OPENCODE_API_KEY", "sk-env-key");
        let (url, log) = spawn_usage_server(200, FULL_USAGE_BODY);

        let outputs = fetch_blocking(&url, &auth).expect("env var fallback should work");
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].metrics.len(), 3);

        let seen = log.lock().unwrap();
        assert_eq!(seen[0].bearer.as_deref(), Some("sk-env-key"));
    }

    #[test]
    #[serial_test::serial]
    fn fetch_errors_with_no_credentials() {
        let tmp = tempfile::TempDir::new().unwrap();
        let auth = tmp.path().join("no-file.json");
        let _guard = EnvVarGuard::remove("OPENCODE_API_KEY");

        let err = fetch_blocking("http://127.0.0.1:1/unused", &auth).unwrap_err();
        assert!(
            err.to_string().contains("No OpenCode Go credentials"),
            "should report missing credentials: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn auth_file_not_touched_on_rejected_token() {
        let tmp = tempfile::TempDir::new().unwrap();
        let auth = write_auth(tmp.path(), "sk-readonly");
        let _guard = EnvVarGuard::remove("OPENCODE_API_KEY");
        let before = std::fs::read(&auth).unwrap();
        let (url, _log) = spawn_usage_server(401, AUTH_ERROR_BODY);

        let _ = fetch_blocking(&url, &auth);

        let after = std::fs::read(&auth).unwrap();
        assert_eq!(
            before, after,
            "tokscale must never rewrite OpenCode's auth.json"
        );
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }
}
