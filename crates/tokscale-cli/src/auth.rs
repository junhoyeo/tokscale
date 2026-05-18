use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::IsTerminal;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const API_TOKEN_ENV_VAR: &str = "TOKSCALE_API_TOKEN";

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("Could not determine home directory")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub token: String,
    pub username: String,
    #[serde(rename = "avatarUrl", skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiTokenSource {
    Environment,
    StoredCredentials,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiTokenAuth {
    pub token: String,
    pub username: Option<String>,
    pub source: ApiTokenSource,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    #[serde(rename = "deviceCode")]
    device_code: String,
    #[serde(rename = "userCode")]
    user_code: String,
    #[serde(rename = "verificationUrl")]
    verification_url: String,
    #[serde(rename = "expiresIn")]
    #[allow(dead_code)]
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct PollResponse {
    status: String,
    token: Option<String>,
    user: Option<UserInfo>,
    #[allow(dead_code)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    username: String,
    #[serde(rename = "avatarUrl")]
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenValidationResponse {
    user: UserInfo,
}

fn get_credentials_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".config/tokscale/credentials.json"))
}

fn get_source_id_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".config/tokscale/source-id"))
}

fn get_source_id_lock_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".config/tokscale/source-id.lock"))
}

const SOURCE_ID_LOCK_RETRY_DELAY: Duration = Duration::from_millis(25);
const SOURCE_ID_LOCK_STALE_AFTER: Duration = Duration::from_secs(2);
const SOURCE_ID_LOCK_MAX_WAIT: Duration = Duration::from_secs(10);
const SOURCE_ID_LOCK_FORCE_STALE_AFTER: Duration = SOURCE_ID_LOCK_MAX_WAIT;

fn ensure_config_dir() -> Result<()> {
    let config_dir = home_dir()?.join(".config/tokscale");

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

pub fn save_credentials(credentials: &Credentials) -> Result<()> {
    ensure_config_dir()?;
    let path = get_credentials_path()?;
    let json = serde_json::to_string_pretty(credentials)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(json.as_bytes())?;
    }

    #[cfg(not(unix))]
    {
        fs::write(&path, json)?;
    }

    Ok(())
}

pub fn load_credentials() -> Option<Credentials> {
    let path = get_credentials_path().ok()?;
    if !path.exists() {
        return None;
    }

    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn load_api_token_from_env() -> Option<String> {
    let token = std::env::var(API_TOKEN_ENV_VAR).ok()?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

pub fn resolve_api_token() -> Option<ApiTokenAuth> {
    if let Some(token) = load_api_token_from_env() {
        return Some(ApiTokenAuth {
            token,
            username: None,
            source: ApiTokenSource::Environment,
        });
    }

    load_credentials().map(|credentials| ApiTokenAuth {
        token: credentials.token,
        username: Some(credentials.username),
        source: ApiTokenSource::StoredCredentials,
    })
}

pub fn clear_credentials() -> Result<bool> {
    let path = get_credentials_path()?;
    if path.exists() {
        fs::remove_file(path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn get_api_base_url() -> String {
    std::env::var("TOKSCALE_API_URL").unwrap_or_else(|_| "https://tokscale.ai".to_string())
}

fn get_device_name() -> String {
    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    format!("CLI on {}", hostname)
}

fn read_source_id(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceIdLockState {
    pid: u32,
    created_at_ms: u128,
}

fn current_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn serialize_source_id_lock_state(state: SourceIdLockState) -> String {
    format!("pid={}\ncreated_at_ms={}\n", state.pid, state.created_at_ms)
}

fn parse_source_id_lock_state(content: &str) -> Option<SourceIdLockState> {
    let mut pid = None;
    let mut created_at_ms = None;

    for line in content.lines() {
        // Skip lines without `=` (blank lines, trailing whitespace, or any
        // future metadata key we don't recognize) instead of aborting the
        // whole parse. A stray malformed line would otherwise force
        // lock_age into its mtime fallback and delay stale-lock cleanup.
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "pid" => pid = value.trim().parse::<u32>().ok(),
            "created_at_ms" => created_at_ms = value.trim().parse::<u128>().ok(),
            _ => {}
        }
    }

    Some(SourceIdLockState {
        pid: pid?,
        created_at_ms: created_at_ms?,
    })
}

fn read_source_id_lock_state(path: &Path) -> Option<SourceIdLockState> {
    let content = fs::read_to_string(path).ok()?;
    parse_source_id_lock_state(&content)
}

fn lock_age(path: &Path, state: Option<SourceIdLockState>) -> Duration {
    if let Some(state) = state {
        let now_ms = current_unix_ms();
        let age_ms = now_ms.saturating_sub(state.created_at_ms);
        return Duration::from_millis(age_ms.min(u64::MAX as u128) as u64);
    }

    // Malformed lock file (no parseable state). If mtime is unreadable or in
    // the future (clock skew), treat it as stale so we recycle instead of
    // stalling on the per-iteration retry up to FORCE_STALE_AFTER.
    match fs::metadata(path).and_then(|metadata| metadata.modified()) {
        Ok(modified) => modified
            .elapsed()
            .unwrap_or(SOURCE_ID_LOCK_FORCE_STALE_AFTER),
        Err(_) => SOURCE_ID_LOCK_FORCE_STALE_AFTER,
    }
}

// On non-Windows targets the helper is only exercised by unit tests; silence
// the unused-fn lint so `cargo build` stays clean everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
/// Given the stdout of `tasklist /FI "PID eq N" /FO CSV /NH`, decide
/// whether a matching process exists.
///
/// `/FI "PID eq N"` filters server-side. When no PID matches, tasklist
/// still emits a localized banner on stdout — e.g. English:
/// `INFO: No tasks are running which match the specified criteria.`
/// — so "any non-empty line" over-matches. Because `/FO CSV` wraps
/// every field of a data row in double quotes, a CSV data row always
/// starts with `"`, while the banner never does (and the banner is
/// locale-dependent, so we can't string-match it directly). Gating on
/// `line.trim().starts_with('"')` is both locale-agnostic and
/// robust to process names that contain commas (which would break a
/// naive `split(',')` attempt to read the PID column).
fn tasklist_output_indicates_match(stdout: &str) -> bool {
    stdout.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && trimmed.starts_with('"')
    })
}

fn lock_owner_is_alive(pid: u32) -> Option<bool> {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .ok()
            .map(|status| status.success())
    }

    #[cfg(windows)]
    {
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                Some(tasklist_output_indicates_match(&stdout))
            }
            Ok(_) => None,
            Err(_) => None,
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        None
    }
}

fn should_remove_stale_source_id_lock(age: Duration, owner_is_alive: Option<bool>) -> bool {
    if age >= SOURCE_ID_LOCK_FORCE_STALE_AFTER {
        return true;
    }

    match owner_is_alive {
        Some(false) | None => age >= SOURCE_ID_LOCK_STALE_AFTER,
        Some(true) => false,
    }
}

fn write_source_id_lock_state(mut file: fs::File, state: SourceIdLockState) -> Result<()> {
    let payload = serialize_source_id_lock_state(state);
    file.write_all(payload.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn remove_source_id_lock_if_matches(path: &Path, expected: Option<SourceIdLockState>) -> bool {
    let current_state = read_source_id_lock_state(path);
    if current_state != expected {
        return false;
    }

    match fs::remove_file(path) {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => false,
    }
}

struct SourceIdLock {
    path: PathBuf,
    state: SourceIdLockState,
}

impl Drop for SourceIdLock {
    fn drop(&mut self) {
        let _ = remove_source_id_lock_if_matches(&self.path, Some(self.state));
    }
}

fn acquire_source_id_lock() -> Result<SourceIdLock> {
    ensure_config_dir()?;
    let lock_path = get_source_id_lock_path()?;
    let deadline = Instant::now() + SOURCE_ID_LOCK_MAX_WAIT;

    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(file) => {
                let state = SourceIdLockState {
                    pid: std::process::id(),
                    created_at_ms: current_unix_ms(),
                };

                if let Err(err) = write_source_id_lock_state(file, state) {
                    let _ = fs::remove_file(&lock_path);
                    return Err(err);
                }

                return Ok(SourceIdLock {
                    path: lock_path,
                    state,
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let state = read_source_id_lock_state(&lock_path);
                let age = lock_age(&lock_path, state);
                let owner_is_alive = match state {
                    Some(lock_state) => lock_owner_is_alive(lock_state.pid),
                    None => None,
                };

                if should_remove_stale_source_id_lock(age, owner_is_alive) {
                    let _ = remove_source_id_lock_if_matches(&lock_path, state);
                    continue;
                }

                if Instant::now() >= deadline {
                    break;
                }

                thread::sleep(SOURCE_ID_LOCK_RETRY_DELAY);
            }
            Err(err) => return Err(err.into()),
        }
    }

    anyhow::bail!("Could not acquire source ID lock after waiting for stale lock cleanup");
}

fn write_source_id(path: &Path, source_id: &str) -> Result<()> {
    let temp_path = path.with_extension(format!("tmp-{}", std::process::id()));

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp_path)?;
        file.write_all(source_id.as_bytes())?;
        file.write_all(b"\n")?;
    }

    #[cfg(not(unix))]
    {
        fs::write(&temp_path, format!("{source_id}\n"))?;
    }

    fs::rename(&temp_path, path)?;
    Ok(())
}

pub fn get_submit_source_id() -> Result<Option<String>> {
    if let Some(source_id) = std::env::var_os("TOKSCALE_SOURCE_ID") {
        let trimmed = source_id.to_string_lossy().trim().to_string();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed));
        }
    }

    ensure_config_dir()?;
    let path = get_source_id_path()?;

    if let Some(existing) = read_source_id(&path) {
        return Ok(Some(existing));
    }

    let _lock = acquire_source_id_lock()?;

    if let Some(existing) = read_source_id(&path) {
        return Ok(Some(existing));
    }

    let source_id = uuid::Uuid::new_v4().to_string();
    write_source_id(&path, &source_id)?;
    Ok(Some(source_id))
}

pub fn get_submit_source_name() -> Option<String> {
    std::env::var("TOKSCALE_SOURCE_NAME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| Some(get_device_name()))
}

#[cfg(target_os = "linux")]
fn has_non_empty_env_var(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

#[cfg(target_os = "linux")]
fn should_auto_open_browser() -> bool {
    has_non_empty_env_var("DISPLAY") || has_non_empty_env_var("WAYLAND_DISPLAY")
}

#[cfg(not(target_os = "linux"))]
fn should_auto_open_browser() -> bool {
    true
}

fn open_browser(url: &str) -> bool {
    if !should_auto_open_browser() {
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        return std::process::Command::new("open").arg(url).spawn().is_ok();
    }

    #[cfg(target_os = "windows")]
    {
        return std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .is_ok();
    }

    #[cfg(target_os = "linux")]
    {
        return std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .is_ok();
    }

    #[allow(unreachable_code)]
    false
}

pub async fn login() -> Result<()> {
    use colored::Colorize;

    if let Some(creds) = load_credentials() {
        println!(
            "\n  {}",
            format!("Already logged in as {}", creds.username.bold()).yellow()
        );
        println!(
            "{}",
            "  Run 'bunx tokscale@latest logout' to sign out first.\n".bright_black()
        );
        return Ok(());
    }

    let base_url = get_api_base_url();

    println!("\n  {}\n", "Tokscale - Login".cyan());
    println!("{}", "  Requesting authorization code...".bright_black());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let device_code_response = client
        .post(format!("{}/api/auth/device", base_url))
        .json(&serde_json::json!({
            "deviceName": get_device_name()
        }))
        .send()
        .await?;

    if !device_code_response.status().is_success() {
        anyhow::bail!("Server returned {}", device_code_response.status());
    }

    let device_data: DeviceCodeResponse = device_code_response.json().await?;

    println!();
    println!("{}", "  Open this URL in your browser:".white());
    let url_display = if std::io::stdout().is_terminal() {
        format!(
            "\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\",
            device_data.verification_url, device_data.verification_url
        )
    } else {
        device_data.verification_url.clone()
    };
    println!("{}", format!("  {}\n", url_display).cyan());
    println!("{}", "  Enter this code:".white());
    println!(
        "{}\n",
        format!("  {}", device_data.user_code).green().bold()
    );

    if !open_browser(&device_data.verification_url) {
        println!(
            "{}",
            "  Browser auto-open unavailable in this environment. Continue with the URL above.\n"
                .bright_black()
        );
    }

    println!("{}", "  Waiting for authorization...".bright_black());

    let poll_interval = std::time::Duration::from_secs(device_data.interval);
    let max_attempts = 180;

    for attempt in 0..max_attempts {
        tokio::time::sleep(poll_interval).await;

        let poll_response = client
            .post(format!("{}/api/auth/device/poll", base_url))
            .json(&serde_json::json!({
                "deviceCode": device_data.device_code
            }))
            .send()
            .await;

        match poll_response {
            Ok(response) => {
                if let Ok(data) = response.json::<PollResponse>().await {
                    if data.status == "complete" {
                        if let (Some(token), Some(user)) = (data.token, data.user) {
                            let credentials = Credentials {
                                token,
                                username: user.username.clone(),
                                avatar_url: user.avatar_url,
                                created_at: chrono::Utc::now().to_rfc3339(),
                            };

                            save_credentials(&credentials)?;

                            println!(
                                "\n  {}",
                                format!("Success! Logged in as {}", user.username.bold()).green()
                            );
                            println!(
                                "{}",
                                "  You can now use 'bunx tokscale@latest submit' to share your usage.\n"
                                    .bright_black()
                            );
                            return Ok(());
                        }
                    }

                    if data.status == "expired" {
                        anyhow::bail!("Authorization code expired. Please try again.");
                    }

                    print!("{}", ".".bright_black());
                    use std::io::Write;
                    std::io::stdout().flush()?;
                }
            }
            Err(_) => {
                print!("{}", "!".red());
                use std::io::Write;
                std::io::stdout().flush()?;
            }
        }

        if attempt >= max_attempts - 1 {
            anyhow::bail!("Timeout: Authorization took too long. Please try again.");
        }
    }

    Ok(())
}

pub async fn login_with_token(token: &str) -> Result<()> {
    use colored::Colorize;

    let token = token.trim();
    if token.is_empty() {
        anyhow::bail!("API token cannot be empty.");
    }
    if !token.starts_with("tt_") {
        anyhow::bail!("Tokscale API tokens must start with `tt_`.");
    }

    let base_url = get_api_base_url();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let response = client
        .get(format!("{}/api/auth/token", base_url))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        let error = body
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("API token validation failed");
        anyhow::bail!("{} ({})", error, status);
    }

    let data: TokenValidationResponse = response.json().await?;
    let credentials = Credentials {
        token: token.to_string(),
        username: data.user.username.clone(),
        avatar_url: data.user.avatar_url,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    save_credentials(&credentials)?;

    println!(
        "\n  {}",
        format!("Success! Logged in as {}", credentials.username.bold()).green()
    );
    println!(
        "{}",
        "  You can now use 'bunx tokscale@latest submit' to share your usage.\n".bright_black()
    );

    Ok(())
}

pub fn logout() -> Result<()> {
    use colored::Colorize;

    let credentials = load_credentials();

    let Some(creds) = credentials else {
        println!("\n  {}\n", "Not logged in.".yellow());
        return Ok(());
    };

    let username = creds.username;
    let cleared = clear_credentials()?;

    if cleared {
        println!(
            "\n  {}\n",
            format!("Logged out from {}", username.bold()).green()
        );
    } else {
        anyhow::bail!("Failed to clear credentials.");
    }

    Ok(())
}

pub fn whoami() -> Result<()> {
    use colored::Colorize;

    let Some(creds) = load_credentials() else {
        println!("\n  {}", "Not logged in.".yellow());
        println!(
            "{}",
            "  Run 'bunx tokscale@latest login' to authenticate.\n".bright_black()
        );
        return Ok(());
    };

    println!("\n  {}\n", "Tokscale - Account Info".cyan());
    println!(
        "{}",
        format!("  Username:  {}", creds.username.bold()).white()
    );

    if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&creds.created_at) {
        println!(
            "{}",
            format!("  Logged in: {}", created.format("%Y-%m-%d")).bright_black()
        );
    }

    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    struct TestEnvGuard {
        name: &'static str,
        original: Option<std::ffi::OsString>,
    }

    impl TestEnvGuard {
        fn set(name: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let original = env::var_os(name);
            unsafe {
                env::set_var(name, value);
            }
            Self { name, original }
        }
    }

    impl Drop for TestEnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => unsafe {
                    env::set_var(self.name, value);
                },
                None => unsafe {
                    env::remove_var(self.name);
                },
            }
        }
    }

    #[cfg(target_os = "linux")]
    struct EnvVarGuard {
        name: &'static str,
        original: Option<std::ffi::OsString>,
    }

    #[cfg(target_os = "linux")]
    impl EnvVarGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let original = env::var_os(name);
            unsafe {
                env::set_var(name, value);
            }
            Self { name, original }
        }

        fn remove(name: &'static str) -> Self {
            let original = env::var_os(name);
            unsafe {
                env::remove_var(name);
            }
            Self { name, original }
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => unsafe {
                    env::set_var(self.name, value);
                },
                None => unsafe {
                    env::remove_var(self.name);
                },
            }
        }
    }

    #[test]
    #[serial]
    fn test_get_api_base_url_default() {
        unsafe {
            env::remove_var("TOKSCALE_API_URL");
        }
        assert_eq!(get_api_base_url(), "https://tokscale.ai");
    }

    #[test]
    #[serial]
    fn test_get_api_base_url_custom() {
        unsafe {
            env::set_var("TOKSCALE_API_URL", "https://custom.api.url");
        }
        assert_eq!(get_api_base_url(), "https://custom.api.url");
        unsafe {
            env::remove_var("TOKSCALE_API_URL");
        }
    }

    #[test]
    fn test_credentials_serialization() {
        let creds = Credentials {
            token: "test_token_123".to_string(),
            username: "testuser".to_string(),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&creds).unwrap();
        let deserialized: Credentials = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.token, creds.token);
        assert_eq!(deserialized.username, creds.username);
        assert_eq!(deserialized.avatar_url, creds.avatar_url);
        assert_eq!(deserialized.created_at, creds.created_at);
    }

    #[test]
    fn test_credentials_serialization_without_avatar() {
        let creds = Credentials {
            token: "test_token_456".to_string(),
            username: "testuser2".to_string(),
            avatar_url: None,
            created_at: "2024-01-02T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&creds).unwrap();
        let deserialized: Credentials = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.token, creds.token);
        assert_eq!(deserialized.username, creds.username);
        assert_eq!(deserialized.avatar_url, None);
        assert_eq!(deserialized.created_at, creds.created_at);

        assert!(!json.contains("avatarUrl"));
    }

    #[test]
    #[serial]
    #[cfg(target_os = "linux")]
    fn test_should_not_auto_open_browser_without_desktop_session() {
        let _display = EnvVarGuard::remove("DISPLAY");
        let _wayland = EnvVarGuard::remove("WAYLAND_DISPLAY");

        assert!(!should_auto_open_browser());
    }

    #[test]
    #[serial]
    #[cfg(target_os = "linux")]
    fn test_should_auto_open_browser_with_display() {
        let _display = EnvVarGuard::set("DISPLAY", ":0");
        let _wayland = EnvVarGuard::remove("WAYLAND_DISPLAY");

        assert!(should_auto_open_browser());
    }

    #[test]
    #[serial]
    #[cfg(target_os = "linux")]
    fn test_should_auto_open_browser_with_wayland_display() {
        let _display = EnvVarGuard::remove("DISPLAY");
        let _wayland = EnvVarGuard::set("WAYLAND_DISPLAY", "wayland-0");

        assert!(should_auto_open_browser());
    }

    #[test]
    #[serial]
    fn test_get_credentials_path() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let path = get_credentials_path().unwrap();
        let expected = temp_dir.path().join(".config/tokscale/credentials.json");

        assert_eq!(path, expected);

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_get_submit_source_id_uses_env_override() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
            env::set_var("TOKSCALE_SOURCE_ID", "  source-from-env  ");
        }

        let source_id = get_submit_source_id().unwrap();

        assert_eq!(source_id.as_deref(), Some("source-from-env"));
        assert!(!get_source_id_path().unwrap().exists());

        unsafe {
            env::remove_var("TOKSCALE_SOURCE_ID");
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_get_submit_source_id_persists_generated_value() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
            env::remove_var("TOKSCALE_SOURCE_ID");
        }

        let first = get_submit_source_id().unwrap();
        let second = get_submit_source_id().unwrap();
        let path = get_source_id_path().unwrap();

        assert!(path.exists());
        assert_eq!(first, second);
        assert_eq!(read_source_id(&path), first);

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_get_submit_source_name_uses_trimmed_env_override() {
        unsafe {
            env::set_var("TOKSCALE_SOURCE_NAME", "  Work Laptop  ");
        }

        assert_eq!(get_submit_source_name().as_deref(), Some("Work Laptop"));

        unsafe {
            env::remove_var("TOKSCALE_SOURCE_NAME");
        }
    }

    #[test]
    fn test_should_remove_stale_source_id_lock_when_owner_dead_after_stale_threshold() {
        assert!(should_remove_stale_source_id_lock(
            SOURCE_ID_LOCK_STALE_AFTER,
            Some(false)
        ));
    }

    #[test]
    fn test_should_remove_stale_source_id_lock_when_probe_is_unknown_after_stale_threshold() {
        assert!(should_remove_stale_source_id_lock(
            SOURCE_ID_LOCK_STALE_AFTER,
            None
        ));
    }

    #[test]
    fn test_should_remove_stale_source_id_lock_when_age_exceeds_force_threshold_even_if_pid_is_alive(
    ) {
        assert!(should_remove_stale_source_id_lock(
            SOURCE_ID_LOCK_FORCE_STALE_AFTER,
            Some(true)
        ));
    }

    #[test]
    fn test_should_not_remove_live_source_id_lock_before_force_threshold() {
        assert!(!should_remove_stale_source_id_lock(
            SOURCE_ID_LOCK_STALE_AFTER,
            Some(true)
        ));
    }

    #[test]
    #[serial]
    fn test_save_credentials() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let creds = Credentials {
            token: "save_test_token".to_string(),
            username: "saveuser".to_string(),
            avatar_url: Some("https://example.com/save.png".to_string()),
            created_at: "2024-01-03T00:00:00Z".to_string(),
        };

        save_credentials(&creds).unwrap();

        let path = get_credentials_path().unwrap();
        assert!(path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&path).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o600);
        }

        let content = fs::read_to_string(&path).unwrap();
        let loaded: Credentials = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.token, creds.token);
        assert_eq!(loaded.username, creds.username);

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_load_credentials() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let creds = Credentials {
            token: "load_test_token".to_string(),
            username: "loaduser".to_string(),
            avatar_url: None,
            created_at: "2024-01-04T00:00:00Z".to_string(),
        };

        save_credentials(&creds).unwrap();

        let loaded = load_credentials().unwrap();

        assert_eq!(loaded.token, creds.token);
        assert_eq!(loaded.username, creds.username);
        assert_eq!(loaded.avatar_url, creds.avatar_url);
        assert_eq!(loaded.created_at, creds.created_at);

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_load_credentials_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let loaded = load_credentials();
        assert!(loaded.is_none());

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_clear_credentials() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let creds = Credentials {
            token: "clear_test_token".to_string(),
            username: "clearuser".to_string(),
            avatar_url: None,
            created_at: "2024-01-05T00:00:00Z".to_string(),
        };

        save_credentials(&creds).unwrap();
        let path = get_credentials_path().unwrap();
        assert!(path.exists());

        let cleared = clear_credentials().unwrap();
        assert!(cleared);
        assert!(!path.exists());

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_clear_credentials_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let cleared = clear_credentials().unwrap();
        assert!(!cleared);

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_ensure_config_dir() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let config_dir = temp_dir.path().join(".config/tokscale");
        assert!(!config_dir.exists());

        ensure_config_dir().unwrap();

        assert!(config_dir.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&config_dir).unwrap();
            let permissions = metadata.permissions();
            assert_eq!(permissions.mode() & 0o777, 0o700);
        }

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_save_and_load_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let original = Credentials {
            token: "roundtrip_token".to_string(),
            username: "roundtripuser".to_string(),
            avatar_url: Some("https://example.com/roundtrip.png".to_string()),
            created_at: "2024-01-06T12:34:56Z".to_string(),
        };

        save_credentials(&original).unwrap();
        let loaded = load_credentials().unwrap();

        assert_eq!(loaded.token, original.token);
        assert_eq!(loaded.username, original.username);
        assert_eq!(loaded.avatar_url, original.avatar_url);
        assert_eq!(loaded.created_at, original.created_at);

        unsafe {
            env::remove_var("HOME");
        }
    }

    // =====================================================================
    // Lock state serialization / parsing
    // =====================================================================

    #[test]
    fn test_serialize_source_id_lock_state_emits_expected_format() {
        let state = SourceIdLockState {
            pid: 4242,
            created_at_ms: 1_700_000_000_000,
        };
        assert_eq!(
            serialize_source_id_lock_state(state),
            "pid=4242\ncreated_at_ms=1700000000000\n"
        );
    }

    #[test]
    fn test_parse_source_id_lock_state_round_trips_serialized_output() {
        let state = SourceIdLockState {
            pid: 99,
            created_at_ms: 1_699_999_999_999,
        };
        let serialized = serialize_source_id_lock_state(state);

        let parsed = parse_source_id_lock_state(&serialized).unwrap();
        assert_eq!(parsed, state);
    }

    #[test]
    fn test_parse_source_id_lock_state_tolerates_whitespace_and_unknown_keys() {
        let content = "  pid = 1234  \n  created_at_ms =  42  \nunknown_key=ignored\n";
        let parsed = parse_source_id_lock_state(content).unwrap();
        assert_eq!(parsed.pid, 1234);
        assert_eq!(parsed.created_at_ms, 42);
    }

    #[test]
    fn test_parse_source_id_lock_state_returns_none_on_missing_fields() {
        assert!(parse_source_id_lock_state("pid=1\n").is_none());
        assert!(parse_source_id_lock_state("created_at_ms=1\n").is_none());
        assert!(parse_source_id_lock_state("").is_none());
    }

    #[test]
    fn test_parse_source_id_lock_state_skips_lines_without_equals() {
        // Lines without '=' (blank line, garbage, future metadata) must be
        // skipped — not abort the whole parse. Key/value pairs around them
        // still produce a valid state.
        let parsed =
            parse_source_id_lock_state("garbage\npid=1\n\ncreated_at_ms=2\nrandom-trailer\n")
                .unwrap();
        assert_eq!(parsed.pid, 1);
        assert_eq!(parsed.created_at_ms, 2);
    }

    // =====================================================================
    // lock_age
    // =====================================================================

    #[test]
    fn test_lock_age_from_state_clamps_future_created_at_to_zero() {
        // If the lock claims a created_at in the future (clock skew on a
        // cross-machine file system), saturating_sub makes age = 0. That's
        // the "treat as fresh" branch — the stale checker takes over via
        // the force-stale timer.
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("irrelevant.lock");
        let future_ms = current_unix_ms() + 10_000;
        let state = SourceIdLockState {
            pid: 1,
            created_at_ms: future_ms,
        };
        let age = lock_age(&path, Some(state));
        assert_eq!(age, Duration::ZERO);
    }

    #[test]
    fn test_lock_age_from_state_returns_elapsed_ms_for_past_created_at() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("irrelevant.lock");
        let past_ms = current_unix_ms().saturating_sub(250);
        let state = SourceIdLockState {
            pid: 1,
            created_at_ms: past_ms,
        };
        let age = lock_age(&path, Some(state));
        assert!(
            age >= Duration::from_millis(200) && age < Duration::from_secs(5),
            "unexpected age: {:?}",
            age
        );
    }

    #[test]
    fn test_lock_age_without_state_returns_force_stale_when_metadata_missing() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("does-not-exist.lock");
        assert_eq!(lock_age(&missing, None), SOURCE_ID_LOCK_FORCE_STALE_AFTER);
    }

    // =====================================================================
    // tasklist_output_indicates_match (Windows PID-probe helper,
    // compiled on all platforms for testability)
    // =====================================================================

    #[test]
    fn test_tasklist_output_indicates_match_matches_csv_data_row() {
        let stdout = "\"chrome.exe\",\"1234\",\"Console\",\"1\",\"256,000 K\"\n";
        assert!(tasklist_output_indicates_match(stdout));
    }

    #[test]
    fn test_tasklist_output_indicates_match_rejects_english_info_banner() {
        // This is the exact failure mode of the earlier naive
        // "any non-empty line" check — tasklist prints the banner even
        // when the PID filter found nothing.
        let stdout = "INFO: No tasks are running which match the specified criteria.\n";
        assert!(!tasklist_output_indicates_match(stdout));
    }

    #[test]
    fn test_tasklist_output_indicates_match_rejects_localized_info_banner() {
        // Non-English Windows banners don't start with "INFO:" either, but
        // they still don't start with a double quote. The CSV-data
        // heuristic is locale-agnostic precisely because /FO CSV wraps
        // fields in quotes deterministically.
        let stdout = "정보: 지정된 조건과 일치하는 태스크가 실행되고 있지 않습니다.\n";
        assert!(!tasklist_output_indicates_match(stdout));
    }

    #[test]
    fn test_tasklist_output_indicates_match_rejects_empty_output() {
        assert!(!tasklist_output_indicates_match(""));
        assert!(!tasklist_output_indicates_match("\n\n"));
    }

    #[test]
    fn test_tasklist_output_indicates_match_tolerates_process_name_with_commas() {
        // A commaed image name is CSV-quoted and the row still starts
        // with a double quote, so the heuristic identifies the match
        // without needing to parse the PID column.
        let stdout = "\"evil,name.exe\",\"4242\",\"Services\",\"0\",\"1,024 K\"\n";
        assert!(tasklist_output_indicates_match(stdout));
    }

    #[test]
    fn test_tasklist_output_indicates_match_ignores_blank_lines_around_data() {
        let stdout = "\n\"chrome.exe\",\"1234\",\"Console\",\"1\",\"256,000 K\"\n\n";
        assert!(tasklist_output_indicates_match(stdout));
    }

    // =====================================================================
    // read_source_id_lock_state / remove_source_id_lock_if_matches
    // =====================================================================

    #[test]
    fn test_read_source_id_lock_state_reads_and_parses_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("present.lock");
        let state = SourceIdLockState {
            pid: 7,
            created_at_ms: 123,
        };
        fs::write(&path, serialize_source_id_lock_state(state)).unwrap();
        assert_eq!(read_source_id_lock_state(&path), Some(state));
    }

    #[test]
    fn test_read_source_id_lock_state_returns_none_when_file_missing() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("missing.lock");
        assert!(read_source_id_lock_state(&missing).is_none());
    }

    #[test]
    fn test_remove_source_id_lock_if_matches_deletes_only_on_exact_match() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("match.lock");
        let state = SourceIdLockState {
            pid: 10,
            created_at_ms: 1000,
        };
        fs::write(&path, serialize_source_id_lock_state(state)).unwrap();

        // Wrong expected state → do not delete.
        let other = SourceIdLockState {
            pid: 11,
            created_at_ms: 1000,
        };
        assert!(!remove_source_id_lock_if_matches(&path, Some(other)));
        assert!(path.exists());

        // Matching expected state → delete.
        assert!(remove_source_id_lock_if_matches(&path, Some(state)));
        assert!(!path.exists());
    }

    #[test]
    fn test_remove_source_id_lock_if_matches_returns_false_when_file_missing() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("gone.lock");
        assert!(!remove_source_id_lock_if_matches(&missing, None));
    }

    // =====================================================================
    // read_source_id / write_source_id
    // =====================================================================

    #[test]
    fn test_read_source_id_returns_trimmed_content() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("source-id");
        fs::write(&path, "  abc-123  \n").unwrap();
        assert_eq!(read_source_id(&path).as_deref(), Some("abc-123"));
    }

    #[test]
    fn test_read_source_id_returns_none_for_empty_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("empty");
        fs::write(&path, "   \n").unwrap();
        assert!(read_source_id(&path).is_none());
    }

    #[test]
    fn test_read_source_id_returns_none_when_file_missing() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("never-written");
        assert!(read_source_id(&missing).is_none());
    }

    #[test]
    fn test_write_source_id_creates_file_with_trailing_newline() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("source-id");
        write_source_id(&path, "fresh-id").unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "fresh-id\n");
    }

    #[test]
    fn test_write_source_id_overwrites_existing_file_atomically() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("source-id");
        fs::write(&path, "old-id\n").unwrap();
        write_source_id(&path, "new-id").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new-id\n");

        // Temp file name is of the form "source-id.tmp-<pid>"; it must not
        // be left behind after a successful rename.
        let temp_pattern = format!("source-id.tmp-{}", std::process::id());
        assert!(!temp.path().join(temp_pattern).exists());
    }

    // =====================================================================
    // get_device_name
    // =====================================================================

    #[test]
    fn test_get_device_name_is_prefixed_with_cli_on() {
        let name = get_device_name();
        assert!(
            name.starts_with("CLI on "),
            "unexpected device name format: {}",
            name
        );
        assert!(
            name.len() > "CLI on ".len(),
            "device name missing host component: {}",
            name
        );
    }

    // =====================================================================
    // should_remove_stale_source_id_lock: remaining branches
    // =====================================================================

    #[test]
    fn test_should_not_remove_lock_when_owner_dead_but_age_below_stale_threshold() {
        // Dead owner alone is not enough; we still need SOURCE_ID_LOCK_STALE_AFTER.
        let tiny = Duration::from_millis(100);
        assert!(!should_remove_stale_source_id_lock(tiny, Some(false)));
    }

    #[test]
    fn test_should_not_remove_lock_when_probe_unknown_but_age_below_stale_threshold() {
        let tiny = Duration::from_millis(100);
        assert!(!should_remove_stale_source_id_lock(tiny, None));
    }

    // =====================================================================
    // acquire_source_id_lock end-to-end
    // =====================================================================

    #[test]
    #[serial]
    fn test_acquire_source_id_lock_creates_and_drops_lock_file() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let lock_path = get_source_id_lock_path().unwrap();
        {
            let _lock = acquire_source_id_lock().unwrap();
            assert!(lock_path.exists());
        }
        assert!(!lock_path.exists(), "lock should be released on Drop");

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_acquire_source_id_lock_takes_over_a_stale_lock_past_force_threshold() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        ensure_config_dir().unwrap();
        let lock_path = get_source_id_lock_path().unwrap();

        // Plant a stale lock with an ancient created_at_ms — past the
        // FORCE_STALE threshold, so the acquire loop must reclaim it.
        let ancient = SourceIdLockState {
            pid: u32::MAX,    // Very unlikely to match a real PID on this host.
            created_at_ms: 1, // epoch-adjacent
        };
        fs::write(&lock_path, serialize_source_id_lock_state(ancient)).unwrap();

        {
            let lock = acquire_source_id_lock().unwrap();
            // The new owner's state replaces the stale one.
            let on_disk = read_source_id_lock_state(&lock_path).unwrap();
            assert_ne!(on_disk, ancient);
            assert_eq!(on_disk.pid, std::process::id());
            drop(lock);
        }
        assert!(!lock_path.exists());

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_get_source_id_path_is_under_home_config_tokscale() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }

        let expected = temp_dir.path().join(".config/tokscale/source-id");
        assert_eq!(get_source_id_path().unwrap(), expected);

        let lock_expected = temp_dir.path().join(".config/tokscale/source-id.lock");
        assert_eq!(get_source_id_lock_path().unwrap(), lock_expected);

        unsafe {
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_get_submit_source_id_uses_env_override_skipping_disk() {
        // Covers the early return that does NOT touch the config dir.
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
            env::set_var("TOKSCALE_SOURCE_ID", "env-wins");
        }

        let result = get_submit_source_id().unwrap();
        assert_eq!(result.as_deref(), Some("env-wins"));
        // No disk file was created.
        assert!(!get_source_id_path().unwrap().exists());

        unsafe {
            env::remove_var("TOKSCALE_SOURCE_ID");
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_get_submit_source_id_falls_through_when_env_is_whitespace() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("HOME", temp_dir.path());
            env::set_var("TOKSCALE_SOURCE_ID", "   ");
        }

        let id = get_submit_source_id().unwrap().unwrap();
        assert!(!id.is_empty());
        // Whitespace-only env var is ignored; value is persisted.
        assert!(get_source_id_path().unwrap().exists());

        unsafe {
            env::remove_var("TOKSCALE_SOURCE_ID");
            env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn test_get_submit_source_name_falls_back_to_device_name() {
        unsafe {
            env::remove_var("TOKSCALE_SOURCE_NAME");
        }
        let name = get_submit_source_name().unwrap();
        assert!(
            name.starts_with("CLI on "),
            "fallback must use get_device_name: {}",
            name
        );
    }

    #[test]
    #[serial]
    fn test_get_submit_source_name_ignores_whitespace_only_env() {
        unsafe {
            env::set_var("TOKSCALE_SOURCE_NAME", "   ");
        }
        let name = get_submit_source_name().unwrap();
        assert!(
            name.starts_with("CLI on "),
            "whitespace env should fall back to device name: {}",
            name
        );
        unsafe {
            env::remove_var("TOKSCALE_SOURCE_NAME");
        }
    }

    // =====================================================================
    // current_unix_ms (trivial but covers the happy path)
    // =====================================================================

    #[test]
    fn test_current_unix_ms_is_in_plausible_range() {
        let now = current_unix_ms();
        // 2025-01-01 UTC = 1735689600000 ms — we expect something later.
        assert!(
            now > 1_735_689_600_000,
            "clock reported unexpected time: {}",
            now
        );
    }

    #[test]
    #[serial]
    fn test_load_api_token_from_env_trims_value() {
        let _token = TestEnvGuard::set("TOKSCALE_API_TOKEN", "  tt_ci_token  ");

        assert_eq!(load_api_token_from_env(), Some("tt_ci_token".to_string()));
    }

    #[test]
    #[serial]
    fn test_load_api_token_from_env_ignores_empty_values() {
        let _token = TestEnvGuard::set("TOKSCALE_API_TOKEN", "   ");

        assert_eq!(load_api_token_from_env(), None);
    }

    #[test]
    #[serial]
    fn test_resolve_api_token_prefers_env_over_saved_credentials() {
        let temp_dir = TempDir::new().unwrap();
        let _home = TestEnvGuard::set("HOME", temp_dir.path());
        let _token = TestEnvGuard::set("TOKSCALE_API_TOKEN", "tt_env_token");

        save_credentials(&Credentials {
            token: "tt_saved_token".to_string(),
            username: "saved-user".to_string(),
            avatar_url: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        })
        .unwrap();

        let auth = resolve_api_token().unwrap();

        assert_eq!(auth.token, "tt_env_token");
        assert_eq!(auth.username.as_deref(), None);
        assert_eq!(auth.source, ApiTokenSource::Environment);
    }
}
