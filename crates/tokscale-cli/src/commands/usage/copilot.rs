use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;

use super::helpers::capitalize;
use super::{UsageMetric, UsageOutput};

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct PaidQuotaSnapshot {
    percent_remaining: Option<i64>,
    remaining: Option<i64>,
    entitlement: Option<i64>,
    #[allow(dead_code)]
    quota_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct PaidResponse {
    copilot_plan: Option<String>,
    quota_reset_date: Option<String>,
    quota_snapshots: Option<std::collections::HashMap<String, PaidQuotaSnapshot>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct FreeResponse {
    copilot_plan: Option<String>,
    limited_user_quotas: Option<std::collections::HashMap<String, i64>>,
    monthly_quotas: Option<std::collections::HashMap<String, i64>>,
    limited_user_reset_date: Option<String>,
}

/// The `keyring` service name the GitHub CLI stores its token under.
/// `gh` composes it as `"gh:" + hostname` (cli/cli `keyringServiceName`).
const GH_KEYRING_SERVICE: &str = "gh:github.com";

/// Windows Credential Manager target names to try for the `gh` token, most
/// likely first.
///
/// `CredReadW` is an exact lookup, so the target has to be reproduced exactly,
/// and it is not the service name. Go's `go-keyring` — which `gh` uses —
/// composes the target as `service + ":" + username` (`credName` in
/// keyring_windows.go), and `gh` stores and reads its *active* token with an
/// empty username (`keyring.Get(keyringServiceName(host), "")` in cli/cli
/// internal/config/config.go, reached from `Login` via `activateUser`). So the
/// entry every successful `gh auth login` leaves behind is `"gh:github.com:"`,
/// with a trailing colon.
///
/// The bare service name follows as a last resort. `gh` never writes it, but
/// trying it costs one failed lookup and keeps this a superset of the target
/// this code used to try, so no machine where a token happens to sit there
/// gets worse.
fn gh_wincred_targets(service: &str) -> [String; 2] {
    [format!("{service}:"), service.to_string()]
}

/// The candidate targets, quoted, for probe and error messages. Quoting
/// matters: `gh:github.com` is a prefix of `gh:github.com:`, so an unquoted
/// list reads as one repeated name.
fn gh_wincred_targets_display() -> String {
    gh_wincred_targets(GH_KEYRING_SERVICE)
        .iter()
        .map(|target| format!("\"{target}\""))
        .collect::<Vec<String>>()
        .join(", ")
}

/// Fold per-target lookup outcomes into the first hit, or into one error that
/// says what each candidate actually reported.
///
/// Split out of `read_gh_wincred` because only the `read_wincred` call is
/// Windows-specific — deciding what to tell the user is not, and inside a
/// `#[cfg(target_os = "windows")]` function it was compiled by no CI job that
/// could run its tests.
///
/// The behaviour is also new. The loop this replaces matched with
/// `if let Ok(value)`, which discarded the error from every candidate, so a
/// credential store that answered `ERROR_ACCESS_DENIED` was reported in the
/// same words as one that had simply never been written: "missing". That is
/// the opposite of what a diagnostic probe is for, and it is the failure mode
/// #1194 was opened about.
#[cfg_attr(
    not(target_os = "windows"),
    allow(
        dead_code,
        reason = "windows-only caller; the tests below cover it everywhere"
    )
)]
fn first_wincred_hit<E: std::fmt::Display>(
    attempts: impl IntoIterator<Item = (String, std::result::Result<String, E>)>,
) -> Result<(String, String)> {
    let mut failures = Vec::new();
    for (target, outcome) in attempts {
        match outcome {
            Ok(value) => return Ok((target, value)),
            Err(error) => failures.push(format!("\"{target}\" -> {error}")),
        }
    }
    anyhow::bail!(
        "No `gh` credential in the Windows Credential Manager. \
         Run `gh auth login`, or run `cmdkey /list` and report the target name \
         it shows for GitHub. Tried {}",
        if failures.is_empty() {
            "no targets".to_string()
        } else {
            failures.join("; ")
        }
    )
}

/// Windows only: the first candidate target that holds a credential, together
/// with the target it was found at, so callers can report what actually
/// matched instead of what they hoped would match.
#[cfg(target_os = "windows")]
fn read_gh_wincred() -> Result<(String, String)> {
    first_wincred_hit(
        gh_wincred_targets(GH_KEYRING_SERVICE)
            .into_iter()
            .map(|target| {
                let outcome = super::helpers::read_wincred(&target);
                (target, outcome)
            }),
    )
}

/// Reads the raw `gh` secret from the platform credential store.
///
/// Single entry point on purpose: `read_token_from_keychain`,
/// `has_credentials` and `wincred_probe` each used to call
/// `read_keychain("gh:github.com")` themselves, so a fix to one of them left
/// the other two — including the one that gates the provider card — still
/// looking in the wrong place.
fn read_gh_secret() -> Result<String> {
    #[cfg(target_os = "windows")]
    {
        read_gh_wincred().map(|(_target, value)| value)
    }
    #[cfg(not(target_os = "windows"))]
    {
        super::helpers::read_keychain(GH_KEYRING_SERVICE)
    }
}

fn read_token_from_keychain() -> Result<String> {
    let raw = read_gh_secret()?;
    // go-keyring may base64-encode the value
    if let Some(encoded) = raw.strip_prefix("go-keyring-base64:") {
        let decoded = base64_decode(encoded)?;
        Ok(decoded)
    } else {
        Ok(raw)
    }
}

fn token_from_env() -> Option<String> {
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Some(value) = std::env::var_os(var) {
            let value = value.to_string_lossy().trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn hosts_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![gh_config_dir().join("hosts.yml")];
    if cfg!(windows) {
        if let Some(home) = crate::paths::home_dir() {
            let legacy = home.join(".config/gh/hosts.yml");
            if legacy != candidates[0] {
                candidates.push(legacy);
            }
        }
    }
    candidates
}

/// Paths and env vars probed by credential detection, for observability.
/// Never includes the token value itself; only presence/absence is reported.
pub(super) fn credential_probe() -> Vec<String> {
    let mut probes = Vec::new();
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        probes.push(format!(
            "{var}={}",
            if std::env::var_os(var).is_some() {
                "set"
            } else {
                "unset"
            }
        ));
    }
    probes.push(wincred_probe());
    for path in hosts_candidates() {
        probes.push(format!(
            "hosts={} ({})",
            path.display(),
            if path.exists() { "exists" } else { "missing" }
        ));
    }
    probes.push(github_copilot_apps_json_probe());
    probes
}

/// Probe-only: reports whether the Windows Credential Manager holds a `gh`
/// credential. Never returns the token itself — only presence, blob length,
/// and whether it is base64-encoded.
///
/// The probe names the exact target it hit, or every target it tried when it
/// found nothing. That is the whole point: this is the line a #1194 reporter
/// pastes next to `cmdkey /list`, and it is useless if it prints a name the
/// code never looked up.
fn wincred_probe() -> String {
    #[cfg(target_os = "windows")]
    {
        match read_gh_wincred() {
            Ok((target, cred)) => {
                let (prefix, blob_len) = match cred.strip_prefix("go-keyring-base64:") {
                    Some(encoded) => ("go-keyring-base64", encoded.len()),
                    None => ("plain", cred.len()),
                };
                format!(
                    "wincred=\"{target}\" (found, {prefix}, blob={blob_len} bytes, token omitted)"
                )
            }
            // Not "missing": the read can also fail because the store refused
            // us. `error` names every target tried and what each one said.
            Err(error) => format!("wincred=none ({error})"),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        format!(
            "wincred=not-applicable on non-windows (would try: {})",
            gh_wincred_targets_display()
        )
    }
}

/// Where the Copilot editor integrations keep `apps.json`, most likely first.
///
/// The POSIX path was reported on every platform, so on Windows the probe
/// printed a guaranteed "missing" for a path Copilot never writes — a false
/// negative aimed squarely at the users this probe exists to help. Windows
/// keeps it under `%LOCALAPPDATA%`; `~/.config` stays in the list everywhere
/// so a wrong guess about the Windows convention still finds the file.
fn github_copilot_apps_json_candidates() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let home = crate::paths::home_dir();
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("github-copilot"));
        }
        if let Some(home) = home.as_ref() {
            let local = home.join("AppData").join("Local").join("github-copilot");
            if !dirs.contains(&local) {
                dirs.push(local);
            }
        }
    }
    if let Some(home) = home {
        dirs.push(home.join(".config").join("github-copilot"));
    }
    dirs.into_iter().map(|dir| dir.join("apps.json")).collect()
}

/// Probe-only: checks whether the VS Code Copilot extension keeps a
/// credential file in the `github-copilot` config directory (Worth checking:
/// it may hold an OAuth token similar to the one the CLI uses). The probe
/// reports presence, JSON validity, and which token-like fields exist, but
/// the value is never read into `read_credentials` — this is observability
/// only, to be verified before any automatic use is considered.
fn github_copilot_apps_json_probe() -> String {
    let candidates = github_copilot_apps_json_candidates();
    let Some(first) = candidates.first().cloned() else {
        return "apps.json (home dir unavailable)".to_string();
    };
    // Report the file that exists; when none does, report the most likely
    // location for this platform so the "missing" line still names a path
    // worth checking.
    let path = candidates
        .into_iter()
        .find(|candidate| candidate.exists())
        .unwrap_or(first);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return format!("apps.json={} (missing)", path.display());
        }
        Err(_) => return format!("apps.json={} (exists, unreadable)", path.display()),
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return format!("apps.json={} (exists, not valid JSON)", path.display());
    };

    // Only report the shape, never any value: `oauth`/`github.com`/`token`
    // keys may hold the OAuth token. Walk the parsed JSON and match object
    // keys case-insensitively, so an embedded token field is detected
    // without a raw text search that would also flag token values.
    let mut found = std::collections::HashSet::new();
    let mut stack = vec![&json];
    while let Some(value) = stack.pop() {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    if ["oauth", "github.com", "token"]
                        .iter()
                        .any(|n| key.eq_ignore_ascii_case(n))
                    {
                        found.insert(key.to_lowercase());
                    }
                    stack.push(child);
                }
            }
            serde_json::Value::Array(arr) => stack.extend(arr),
            _ => {}
        }
    }
    let fields: Vec<&str> = ["oauth", "github.com", "token"]
        .iter()
        .copied()
        .filter(|n| found.contains(*n))
        .collect();
    let top_level_keys = json.as_object().map(|o| o.keys().count()).unwrap_or(0);
    let fields_desc = if fields.is_empty() {
        "no oauth/token fields".to_string()
    } else {
        format!("fields=[{}]", fields.join(","))
    };
    format!(
        "apps.json={} (exists, valid JSON, top_level_keys={top_level_keys}, {fields_desc}, probe-only)",
        path.display()
    )
}

fn gh_config_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("GH_CONFIG_DIR") {
        return std::path::PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        return std::path::PathBuf::from(dir).join("gh");
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                crate::paths::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
            })
            .join("GitHub CLI");
    }
    let home = crate::paths::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".config").join("gh")
}

fn parse_token_from_hosts() -> Result<String> {
    for path in hosts_candidates() {
        if !path.exists() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Parse YAML-like: look for "oauth_token: <token>" under "github.com:"
        let mut in_github = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "github.com:" {
                in_github = true;
                continue;
            }
            // A non-indented, non-empty, non-comment line starts a new section
            if in_github
                && !line.starts_with(' ')
                && !line.starts_with('\t')
                && !trimmed.is_empty()
                && !trimmed.starts_with('#')
            {
                in_github = false;
            }
            if in_github && trimmed.starts_with("oauth_token:") {
                let token = trimmed.trim_start_matches("oauth_token:").trim();
                if !token.is_empty() {
                    return Ok(token.to_string());
                }
            }
        }
    }
    anyhow::bail!("No oauth_token found in hosts.yml")
}

fn read_token_from_hosts() -> Result<String> {
    parse_token_from_hosts()
}

fn read_credentials() -> Result<String> {
    if let Some(token) = token_from_env() {
        return Ok(token);
    }
    read_token_from_keychain()
        .or_else(|_| read_token_from_hosts())
        .map_err(|_| {
            anyhow::anyhow!(
                "No GitHub Copilot credentials found. Run 'gh auth login' to authenticate."
            )
        })
}

fn base64_decode(input: &str) -> Result<String> {
    // Minimal base64 decode without adding a dependency
    const TABLE: &[Option<u8>; 128] = &{
        let mut table = [None; 128];
        let mut i = 0u8;
        while i < 26 {
            table[(b'A' + i) as usize] = Some(i);
            i += 1;
        }
        let mut i = 0u8;
        while i < 26 {
            table[(b'a' + i) as usize] = Some(26 + i);
            i += 1;
        }
        let mut i = 0u8;
        while i < 10 {
            table[(b'0' + i) as usize] = Some(52 + i);
            i += 1;
        }
        table[b'+' as usize] = Some(62);
        table[b'/' as usize] = Some(63);
        table
    };

    let bytes = input.as_bytes();
    let mut result = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in bytes {
        if b == b'=' {
            break;
        }
        if (b as usize) >= TABLE.len() {
            continue;
        }
        if let Some(v) = TABLE[b as usize] {
            buf = (buf << 6) | v as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                result.push((buf >> bits) as u8);
            }
        }
    }
    Ok(String::from_utf8(result)?)
}

fn pretty_category(key: &str) -> String {
    match key {
        "premium_interactions" => "Premium".into(),
        "chat" => "Chat".into(),
        "completions" => "Completions".into(),
        other => capitalize(other.replace('_', " ").as_str()),
    }
}

async fn fetch_api(client: &reqwest::Client, token: &str) -> Result<serde_json::Value> {
    let resp = client
        .get("https://api.github.com/copilot_internal/user")
        .header("Authorization", format!("token {token}"))
        .header("Accept", "application/json")
        .header("Editor-Version", "vscode/1.96.2")
        .header("Editor-Plugin-Version", "copilot-chat/0.26.7")
        .header("User-Agent", "GitHubCopilotChat/0.26.7")
        .header("X-Github-Api-Version", "2025-04-01")
        .send()
        .await?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("NEEDS_AUTH");
    }
    if !status.is_success() {
        anyhow::bail!("Copilot usage request failed (HTTP {status})");
    }
    Ok(resp.json().await?)
}

pub fn has_credentials() -> bool {
    if token_from_env().is_some() {
        return true;
    }
    if read_gh_secret().is_ok() {
        return true;
    }
    parse_token_from_hosts().is_ok()
}

pub fn fetch() -> Result<UsageOutput> {
    let token = read_credentials()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = tokscale_core::http::client();
        let resp = fetch_api(&client, &token).await.map_err(|e| {
            if e.to_string().contains("NEEDS_AUTH") {
                anyhow::anyhow!(
                    "GitHub token found but not authorized for Copilot. \
                     Run 'gh auth login' or check your GH_TOKEN/GITHUB_TOKEN."
                )
            } else {
                e
            }
        })?;

        let plan = resp
            .get("copilot_plan")
            .and_then(|v| v.as_str())
            .map(capitalize);

        let mut metrics = Vec::new();

        // Try paid tier response (quota_snapshots)
        if let Some(snapshots) = resp.get("quota_snapshots").and_then(|v| v.as_object()) {
            let reset_date = resp
                .get("quota_reset_date")
                .and_then(|v| v.as_str())
                .map(String::from);

            for (key, value) in snapshots {
                let remaining = value.get("remaining").and_then(|v| v.as_i64());
                let entitlement = value.get("entitlement").and_then(|v| v.as_i64());
                let pct_remaining = value
                    .get("percent_remaining")
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| match (remaining, entitlement) {
                        (Some(r), Some(e)) if e > 0 => {
                            (r as f64 / e as f64 * 100.0).clamp(0.0, 100.0)
                        }
                        _ => 100.0,
                    })
                    .clamp(0.0, 100.0);

                let used_pct = 100.0 - pct_remaining;
                let remaining_pct = pct_remaining;

                let remaining_label = match (remaining, entitlement) {
                    (Some(r), Some(e)) => Some(format!("{r}/{e} left")),
                    _ => None,
                };

                metrics.push(UsageMetric {
                    label: pretty_category(key),
                    used_percent: used_pct,
                    remaining_percent: remaining_pct,
                    remaining_label,
                    resets_at: reset_date.clone(),
                });
            }
        }

        // Try free tier response (limited_user_quotas)
        if metrics.is_empty() {
            if let Some(quotas) = resp.get("limited_user_quotas").and_then(|v| v.as_object()) {
                let monthly = resp.get("monthly_quotas").and_then(|v| v.as_object());
                let reset_date = resp
                    .get("limited_user_reset_date")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                for (key, value) in quotas {
                    let remaining = value.as_i64().unwrap_or(0);
                    let total = monthly
                        .and_then(|m| m.get(key))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(remaining);

                    if total > 0 {
                        let used = (total - remaining).max(0);
                        let used_pct = (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0);
                        metrics.push(UsageMetric {
                            label: pretty_category(key),
                            used_percent: used_pct,
                            remaining_percent: 100.0 - used_pct,
                            remaining_label: Some(format!("{remaining}/{total} left")),
                            resets_at: reset_date.clone(),
                        });
                    }
                }
            }
        }

        if metrics.is_empty() {
            anyhow::bail!(
                "Copilot returned no parseable usage (quota response format may have changed)"
            );
        }

        Ok(UsageOutput {
            provider: "Copilot".into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::ffi::{OsStr, OsString};
    use std::path::Path;
    use tempfile::TempDir;

    /// RAII restore of process-global env vars, mirroring
    /// `tokscale_core::paths::test_env::EnvGuard` (which is `pub(crate)` to
    /// the core crate and cannot be imported here). The manual
    /// save/restore pairs this replaces only ran the restore when the test
    /// reached the end of its body — a failing assertion panics first and
    /// leaves the redirect in place. Restoring on `Drop` unwinds correctly.
    struct EnvGuard(Vec<(&'static str, Option<OsString>)>);

    impl EnvGuard {
        fn capture(keys: &[&'static str]) -> Self {
            Self(
                keys.iter()
                    .map(|key| (*key, std::env::var_os(key)))
                    .collect(),
            )
        }

        fn set(&mut self, key: &str, value: impl AsRef<OsStr>) {
            unsafe { std::env::set_var(key, value) };
        }

        fn remove(&mut self, key: &str) {
            unsafe { std::env::remove_var(key) };
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, previous) in self.0.drain(..) {
                unsafe {
                    match previous {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
    }

    /// Every environment variable that can steer `github_copilot_apps_json_candidates`
    /// to a real user's file.
    ///
    /// `HOME` alone is not enough. On Windows the `%LOCALAPPDATA%` candidate is
    /// tried *first*, and the probe takes the first path that exists, so a test
    /// that redirects only `HOME` reads the developer's or the runner's real
    /// `github-copilot/apps.json` — a file that holds a live GitHub OAuth
    /// token. `USERPROFILE` is here because `dirs::home_dir()` falls back to it
    /// whenever the `HOME` override is rejected as non-absolute.
    const HOME_ENV_KEYS: [&str; 3] = ["HOME", "LOCALAPPDATA", "USERPROFILE"];

    /// Point every home-ish variable at `dir`, so the probe can only ever
    /// resolve inside the caller's `TempDir`.
    fn redirect_home(guard: &mut EnvGuard, dir: &Path) {
        guard.set("HOME", dir);
        guard.set("LOCALAPPDATA", dir.join("AppData").join("Local"));
        guard.set("USERPROFILE", dir);
    }

    /// The guard above is only worth anything if it actually contains every
    /// candidate. This asserts that on whichever platform it runs, rather than
    /// trusting that the `#[cfg(windows)]` arm of the candidate list was
    /// remembered when someone adds a fourth location.
    #[test]
    #[serial]
    fn apps_json_candidates_stay_inside_a_redirected_home() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("home");
        std::fs::create_dir_all(&dir).unwrap();

        let mut guard = EnvGuard::capture(&HOME_ENV_KEYS);
        redirect_home(&mut guard, &dir);
        let candidates = github_copilot_apps_json_candidates();

        assert!(
            !candidates.is_empty(),
            "the probe must offer at least one candidate under a redirected home"
        );
        for candidate in &candidates {
            assert!(
                candidate.starts_with(&dir),
                "candidate {} escapes the redirected home {}; a test using it \
                 would read the real user's Copilot credentials",
                candidate.display(),
                dir.display()
            );
        }
    }

    /// Mirrors `credName` from go-keyring's keyring_windows.go, which is what
    /// composes every Windows target name `gh` reads or writes.
    fn go_keyring_cred_name(service: &str, username: &str) -> String {
        format!("{service}:{username}")
    }

    #[test]
    fn primary_wincred_target_is_the_slot_gh_writes_for_the_active_account() {
        // This is the regression guard for #1194. The Windows lookup used to
        // pass the bare service name as the CredReadW target, which cannot
        // match: `gh` stores its active token through go-keyring with an empty
        // username, so the target is the service name plus a trailing colon.
        // CredReadW is an exact lookup, so "one colon short" means
        // ERROR_NOT_FOUND on every machine.
        let targets = gh_wincred_targets(GH_KEYRING_SERVICE);
        assert_eq!(targets[0], go_keyring_cred_name(GH_KEYRING_SERVICE, ""));
        assert_eq!(targets[0], "gh:github.com:");
        assert_ne!(
            targets[0], GH_KEYRING_SERVICE,
            "the primary target must not be the bare service name"
        );
    }

    #[test]
    fn wincred_targets_keep_the_bare_service_name_as_a_distinct_fallback() {
        let targets = gh_wincred_targets(GH_KEYRING_SERVICE);
        assert_eq!(targets[1], GH_KEYRING_SERVICE);
        assert_ne!(targets[0], targets[1], "candidates must not duplicate");
    }

    #[test]
    fn wincred_targets_compose_from_the_service_argument() {
        // Not hardcoded to github.com, so a future enterprise-host change
        // keeps the composition rule.
        let targets = gh_wincred_targets("gh:ghe.example.com");
        assert_eq!(targets[0], "gh:ghe.example.com:");
        assert_eq!(targets[1], "gh:ghe.example.com");
    }

    #[test]
    fn wincred_targets_display_quotes_each_candidate() {
        // Unquoted, "gh:github.com" is a prefix of "gh:github.com:" and the
        // list reads as the same name twice.
        let display = gh_wincred_targets_display();
        assert_eq!(display, "\"gh:github.com:\", \"gh:github.com\"");
    }

    /// The whole point of the split: these run on Linux and macOS, where the
    /// Windows credential path is not even compiled.
    #[test]
    fn first_wincred_hit_returns_the_target_that_matched() {
        let attempts: Vec<(String, std::result::Result<String, String>)> = vec![
            ("gh:github.com:".to_string(), Err("not found".to_string())),
            ("gh:github.com".to_string(), Ok("gho_token".to_string())),
        ];
        let (target, value) = first_wincred_hit(attempts).unwrap();
        assert_eq!(target, "gh:github.com");
        assert_eq!(value, "gho_token");
    }

    #[test]
    fn first_wincred_hit_stops_at_the_first_match() {
        let attempts: Vec<(String, std::result::Result<String, String>)> = vec![
            ("first".to_string(), Ok("a".to_string())),
            ("second".to_string(), Ok("b".to_string())),
        ];
        assert_eq!(first_wincred_hit(attempts).unwrap().0, "first");
    }

    /// The regression this replaced: `if let Ok(..)` threw the reason away, so
    /// a store that refused the read and a store that was never written both
    /// came out as the word "missing".
    #[test]
    fn first_wincred_hit_reports_why_each_target_failed() {
        let attempts: Vec<(String, std::result::Result<String, String>)> = vec![
            (
                "gh:github.com:".to_string(),
                Err("CredReadW failed: Access is denied. (os error 5)".to_string()),
            ),
            (
                "gh:github.com".to_string(),
                Err("CredReadW failed: Element not found. (os error 1168)".to_string()),
            ),
        ];
        let error = first_wincred_hit(attempts).unwrap_err().to_string();

        assert!(error.contains("Access is denied"), "got: {error}");
        assert!(error.contains("Element not found"), "got: {error}");
        for target in ["gh:github.com:", "gh:github.com"] {
            assert!(
                error.contains(&format!("\"{target}\"")),
                "error must name every target it tried ({target}), got: {error}"
            );
        }
    }

    /// `gh_wincred_targets` returns two, so this cannot happen today; the
    /// helper is generic and must not produce a dangling "Tried " either way.
    #[test]
    fn first_wincred_hit_survives_an_empty_candidate_list() {
        let attempts: Vec<(String, std::result::Result<String, String>)> = Vec::new();
        let error = first_wincred_hit(attempts).unwrap_err().to_string();
        assert!(error.contains("no targets"), "got: {error}");
    }

    #[test]
    #[serial]
    fn wincred_probe_names_every_target_it_tries() {
        let probe = wincred_probe();
        assert!(probe.starts_with("wincred="), "got: {probe}");
        let targets = gh_wincred_targets(GH_KEYRING_SERVICE);
        if probe.contains("(found") {
            // A real credential exists on this machine (possible on a Windows
            // runner): the probe must name the target that actually matched.
            assert!(
                targets
                    .iter()
                    .any(|target| probe.contains(&format!("\"{target}\""))),
                "found-probe must name a candidate target, got: {probe}"
            );
        } else {
            for target in &targets {
                assert!(
                    probe.contains(&format!("\"{target}\"")),
                    "probe must name the target it tried ({target}), got: {probe}"
                );
            }
        }
    }

    #[test]
    #[serial]
    fn wincred_probe_never_leaks_token_on_non_windows() {
        // On Linux the probe must not attempt a credential read at all.
        // The compile-time cfg keeps the windows import out of non-windows
        // builds, so this is both a behavioral and a linkage check.
        let probe = wincred_probe();
        if cfg!(not(target_os = "windows")) {
            assert!(
                probe.contains("not-applicable"),
                "non-windows probe should report not-applicable, got: {probe}"
            );
        }
    }

    #[test]
    #[serial]
    fn apps_json_probe_reports_missing_file_without_error() {
        let temp = TempDir::new().unwrap();
        // Point HOME at an empty dir so the probe resolves a missing path
        // and reports it as such (silent not-found, no panic).
        let dir = temp.path().join("home");
        std::fs::create_dir_all(&dir).unwrap();
        let mut guard = EnvGuard::capture(&HOME_ENV_KEYS);
        redirect_home(&mut guard, &dir);
        let probe = github_copilot_apps_json_probe();
        assert!(
            probe.contains("(missing)") || probe.contains("home dir unavailable"),
            "expected missing-file probe, got: {probe}"
        );
    }

    #[test]
    #[serial]
    fn apps_json_probe_flags_token_fields_without_printing_values() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("home");
        let apps = dir.join(".config").join("github-copilot").join("apps.json");
        std::fs::create_dir_all(apps.parent().unwrap()).unwrap();
        // A realistic-ish VS Code Copilot apps.json with an OAuth token.
        std::fs::write(
            &apps,
            r#"{"github.com":{"oauth":"gho_AAAA_secret","token":"gho_BBBB"}}"#,
        )
        .unwrap();

        let mut guard = EnvGuard::capture(&HOME_ENV_KEYS);
        redirect_home(&mut guard, &dir);
        let probe = github_copilot_apps_json_probe();

        assert!(probe.contains("valid JSON"), "got: {probe}");
        assert!(
            probe.contains("fields=[oauth,github.com,token]"),
            "got: {probe}"
        );
        assert!(
            !probe.contains("gho_"),
            "probe must not leak token values, got: {probe}"
        );
        assert!(
            probe.contains("probe-only"),
            "probe should be marked probe-only, got: {probe}"
        );
    }

    #[test]
    #[serial]
    fn apps_json_probe_handles_invalid_json_gracefully() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("home");
        let apps = dir.join(".config").join("github-copilot").join("apps.json");
        std::fs::create_dir_all(apps.parent().unwrap()).unwrap();
        std::fs::write(&apps, "this is { not json").unwrap();

        let mut guard = EnvGuard::capture(&HOME_ENV_KEYS);
        redirect_home(&mut guard, &dir);
        let probe = github_copilot_apps_json_probe();

        assert!(
            probe.contains("not valid JSON"),
            "invalid JSON should be reported without error, got: {probe}"
        );
    }

    #[test]
    #[serial]
    fn credential_probe_contains_wincred_and_apps_json_entries() {
        let probes = credential_probe();
        assert!(
            probes.iter().any(|p| p.starts_with("wincred=")),
            "missing wincred probe: {probes:?}"
        );
        assert!(
            probes.iter().any(|p| p.starts_with("apps.json=")),
            "missing apps.json probe: {probes:?}"
        );
    }
}
