use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::IsTerminal;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        let (key, value) = line.split_once('=')?;
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

    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .unwrap_or_default()
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
            .args(["/FI", &format!("PID eq {}", pid)])
            .output();

        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                Some(stdout.contains(&pid.to_string()) && !stdout.contains("No tasks are running"))
            }
            Ok(_) => None,
            Err(_) => None,
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        None
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
                let owner_is_dead = match state {
                    Some(lock_state) => match lock_owner_is_alive(lock_state.pid) {
                        Some(is_alive) => !is_alive,
                        None => age >= SOURCE_ID_LOCK_STALE_AFTER,
                    },
                    None => true,
                };

                if owner_is_dead && age >= SOURCE_ID_LOCK_STALE_AFTER {
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
}
