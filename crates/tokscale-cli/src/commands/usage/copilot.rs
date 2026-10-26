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
///
/// This is the service for the default host only. Everything that looks a
/// token up goes through [`gh_keyring_service`] with the resolved host, so an
/// enterprise login (`gh:ghe.example.com`) is found too.
#[cfg(test)]
const GH_KEYRING_SERVICE: &str = "gh:github.com";

/// The host `gh` falls back to when neither `GH_HOST` nor `hosts.yml` picks one.
const DEFAULT_GH_HOST: &str = "github.com";

/// The keyring service `gh` stores the token for `host` under.
fn gh_keyring_service(host: &str) -> String {
    format!("gh:{host}")
}

/// Normalise a host the way a user is likely to type it into `GH_HOST`:
/// trims whitespace, drops an `https://` / `http://` scheme and a trailing
/// slash, and lowercases.
///
/// The result is parsed, not just filtered: a DNS hostname (dot-separated
/// labels of ASCII letters, digits and inner hyphens, no empty label and so
/// no trailing dot) optionally followed by one `:port` in `1..=65535`. The
/// port is written back in canonical form and dropped when it is `443`, the
/// port every request here goes to anyway. That keeps one spelling per
/// destination, so `github.com:443` is `github.com` both when its credential
/// is chosen and when its URL is built. Anything else returns `None`, so a
/// malformed value can never be spliced into a URL.
fn normalize_gh_host(raw: &str) -> Option<String> {
    let mut host = raw.trim();
    for scheme in ["https://", "http://"] {
        if let Some(rest) = host
            .get(..scheme.len())
            .filter(|prefix| prefix.eq_ignore_ascii_case(scheme))
            .map(|_| &host[scheme.len()..])
        {
            host = rest;
        }
    }
    let host = host.trim_end_matches('/').to_ascii_lowercase();
    let (name, port) = match host.split_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host.as_str(), None),
    };
    if !is_dns_hostname(name) {
        return None;
    }
    let name = normalize_gh_hostname(name);
    let Some(port) = port else {
        return Some(name);
    };
    // Digits only, so a second `:` (or a sign, or whitespace) is rejected
    // here rather than by `parse`.
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match port.parse::<u16>().ok()? {
        0 => None,
        443 => Some(name.to_string()),
        port => Some(format!("{name}:{port}")),
    }
}

/// Match go-gh's hostname aliases before looking up a token or keyring
/// service. GitHub API and upload subdomains share the github.com account;
/// tenancy routes reduce to their tenant label; and GitHub's local test host
/// shares the dotcom token variables.
fn normalize_gh_hostname(name: &str) -> String {
    if name.ends_with(".github.com") {
        return DEFAULT_GH_HOST.to_string();
    }
    if name.ends_with(".github.localhost") {
        return "github.localhost".to_string();
    }
    if let Some(before) = name.strip_suffix(".ghe.com") {
        if let Some(tenant) = before.rsplit('.').next() {
            return format!("{tenant}.ghe.com");
        }
    }
    name.to_string()
}

/// An ASCII DNS hostname: 1-253 characters of dot-separated labels, each
/// 1-63 letters, digits or hyphens and not starting or ending with a hyphen.
fn is_dns_hostname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// The hostname of a host returned by [`normalize_gh_host`], without its port.
fn gh_host_name(host: &str) -> &str {
    host.split_once(':').map_or(host, |(name, _port)| name)
}

/// The only host-level fields this command needs from `gh`'s `hosts.yml`.
///
/// Unknown fields (such as `user`, `users`, and `git_protocol`) intentionally
/// deserialize without affecting credential selection. In particular, a token
/// nested under `users` is not the active host token.
#[derive(Debug, Default, Deserialize)]
struct GhHostConfig {
    oauth_token: Option<String>,
    api_host: Option<String>,
}

/// Parse `gh`'s actual top-level YAML mapping. This deliberately does not
/// approximate YAML with line matching: quoted scalars, comments, aliases, and
/// escapes are part of the file format and all can affect a credential value.
fn parse_hosts_yml(content: &str) -> Option<yaml_serde::Mapping> {
    yaml_serde::from_str(content).ok()
}

/// Top-level host keys of a `gh` `hosts.yml`, in its file order.
fn hosts_in_hosts_yml(content: &str) -> Vec<String> {
    let Some(hosts) = parse_hosts_yml(content) else {
        return Vec::new();
    };
    hosts
        .keys()
        .filter_map(yaml_serde::Value::as_str)
        .filter_map(normalize_gh_host)
        .collect()
}

/// Read one requested host's typed configuration record from `hosts.yml`.
fn host_config_for_host_in_hosts_yml(content: &str, host: &str) -> Option<GhHostConfig> {
    let hosts = parse_hosts_yml(content)?;
    hosts.into_iter().find_map(|(key, value)| {
        let key = key.as_str()?;
        (normalize_gh_host(key).as_deref() == Some(host))
            .then(|| yaml_serde::from_value(value).ok())
            .flatten()
    })
}

/// The active `oauth_token` stored directly under `host:` in a `hosts.yml`.
fn token_for_host_in_hosts_yml(content: &str, host: &str) -> Option<String> {
    host_config_for_host_in_hosts_yml(content, host)?
        .oauth_token
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

/// The configured API transport for a logical `gh` host, if it is a valid
/// hostname. `gh` credentials remain keyed by the logical host, not this API
/// transport hostname.
fn api_host_for_host_in_hosts_yml(content: &str, host: &str) -> Option<String> {
    host_config_for_host_in_hosts_yml(content, host)?
        .api_host
        .as_deref()
        .and_then(normalize_gh_host)
}

/// Pick the host the same way `gh` does: `GH_HOST`, else the only host in
/// `hosts.yml`, else `github.com`. With several configured hosts and no
/// `GH_HOST`, `gh` itself defaults to `github.com`, and so do we.
fn select_gh_host(gh_host_env: Option<&str>, configured: &[String]) -> String {
    if let Some(host) = gh_host_env.and_then(normalize_gh_host) {
        return host;
    }
    match configured {
        [only] => only.clone(),
        _ => DEFAULT_GH_HOST.to_string(),
    }
}

fn configured_gh_hosts() -> Vec<String> {
    for path in hosts_candidates() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            let hosts = hosts_in_hosts_yml(&content);
            if !hosts.is_empty() {
                return hosts;
            }
        }
    }
    Vec::new()
}

/// The configured API transport for `host`, if its `hosts.yml` record has one.
fn configured_api_host(host: &str) -> Option<String> {
    for path in hosts_candidates() {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(api_host) = api_host_for_host_in_hosts_yml(&content, host) {
            return Some(api_host);
        }
    }
    None
}

/// The Copilot API hostname for one logical `gh` host. An explicit `api_host`
/// is the transport configured by `gh`; otherwise Copilot's documented
/// enterprise convention is the `api.` subdomain of the logical host.
fn copilot_api_host(host: &str, configured_api_host: Option<&str>) -> String {
    configured_api_host
        .map(str::to_owned)
        .unwrap_or_else(|| format!("api.{host}"))
}

/// The GitHub host Copilot usage is read for.
fn current_gh_host() -> String {
    let env = std::env::var("GH_HOST").ok();
    select_gh_host(env.as_deref(), &configured_gh_hosts())
}

/// `github.com` and GHE.com data-residency tenants (`*.ghe.com`) share
/// github.com's token variables in `gh`; every other host is a GHES appliance.
///
/// Classified by hostname alone, so the port can never pick a different
/// credential than the host the request is sent to.
fn is_dotcom_style_host(host: &str) -> bool {
    let name = gh_host_name(host);
    name == DEFAULT_GH_HOST || name == "github.localhost" || name.ends_with(".ghe.com")
}

/// Environment variables holding a token for `host`, in `gh`'s precedence
/// (cli/go-gh `auth.TokenForHost`). An enterprise appliance reads only the
/// enterprise variables, so a github.com `GH_TOKEN` is never sent to it and
/// vice versa.
fn token_env_vars(host: &str) -> [&'static str; 2] {
    if is_dotcom_style_host(host) {
        ["GH_TOKEN", "GITHUB_TOKEN"]
    } else {
        ["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"]
    }
}

/// The Copilot user endpoint for the resolved API transport hostname.
fn copilot_user_url(api_host: &str) -> String {
    format!("https://{api_host}/copilot_internal/user")
}

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
#[cfg(test)]
fn gh_wincred_targets_display() -> String {
    gh_wincred_targets_display_for(GH_KEYRING_SERVICE)
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
fn gh_wincred_targets_display_for(service: &str) -> String {
    gh_wincred_targets(service)
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
fn read_gh_wincred(host: &str) -> Result<(String, String)> {
    first_wincred_hit(
        gh_wincred_targets(&gh_keyring_service(host))
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
fn read_gh_secret(host: &str) -> Result<String> {
    #[cfg(target_os = "windows")]
    {
        read_gh_wincred(host).map(|(_target, value)| value)
    }
    #[cfg(not(target_os = "windows"))]
    {
        super::helpers::read_keychain(&gh_keyring_service(host))
    }
}

fn read_token_from_keychain(host: &str) -> Result<String> {
    let raw = read_gh_secret(host)?;
    // go-keyring may base64-encode the value
    if let Some(encoded) = raw.strip_prefix("go-keyring-base64:") {
        let decoded = base64_decode(encoded)?;
        Ok(decoded)
    } else {
        Ok(raw)
    }
}

fn token_from_env(host: &str) -> Option<String> {
    for var in token_env_vars(host) {
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
    let host = current_gh_host();
    let configured_api_host = configured_api_host(&host);
    let api_host = copilot_api_host(&host, configured_api_host.as_deref());
    probes.push(format!(
        "GH_HOST={} (host={host}, api={})",
        if std::env::var_os("GH_HOST").is_some() {
            "set"
        } else {
            "unset"
        },
        copilot_user_url(&api_host)
    ));
    for var in token_env_vars(&host) {
        probes.push(format!(
            "{var}={}",
            if std::env::var_os(var).is_some() {
                "set"
            } else {
                "unset"
            }
        ));
    }
    probes.push(wincred_probe_for(&host));
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
#[cfg_attr(not(test), allow(dead_code))]
fn wincred_probe() -> String {
    wincred_probe_for(&current_gh_host())
}

fn wincred_probe_for(host: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        match read_gh_wincred(host) {
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
            gh_wincred_targets_display_for(&gh_keyring_service(host))
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

fn parse_token_from_hosts(host: &str) -> Result<String> {
    for path in hosts_candidates() {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(token) = token_for_host_in_hosts_yml(&content, host) {
            return Ok(token);
        }
    }
    anyhow::bail!("No oauth_token for {host} found in hosts.yml")
}

/// The token for logical `host`, from (in order) the host's environment
/// variables, the platform keyring entry `gh:<host>`, and the `<host>:`
/// section of `hosts.yml`. `api_host` can deliberately select a different API
/// transport; it never changes the logical host used for credential lookup.
fn read_credentials(host: &str) -> Result<String> {
    if let Some(token) = token_from_env(host) {
        return Ok(token);
    }
    read_token_from_keychain(host)
        .or_else(|_| parse_token_from_hosts(host))
        .map_err(|_| {
            anyhow::anyhow!(
                "No GitHub Copilot credentials found for {host}. Run 'gh auth login{}' to authenticate.",
                if host == DEFAULT_GH_HOST {
                    String::new()
                } else {
                    format!(" --hostname {host}")
                }
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

async fn fetch_api(
    client: &reqwest::Client,
    api_host: &str,
    token: &str,
) -> Result<serde_json::Value> {
    let resp = client
        .get(copilot_user_url(api_host))
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
    let host = current_gh_host();
    if token_from_env(&host).is_some() {
        return true;
    }
    if read_gh_secret(&host).is_ok() {
        return true;
    }
    parse_token_from_hosts(&host).is_ok()
}

pub fn fetch() -> Result<UsageOutput> {
    let host = current_gh_host();
    let token = read_credentials(&host)?;
    let configured_api_host = configured_api_host(&host);
    let api_host = copilot_api_host(&host, configured_api_host.as_deref());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = tokscale_core::http::client();
        let resp = fetch_api(&client, &api_host, &token).await.map_err(|e| {
            if e.to_string().contains("NEEDS_AUTH") {
                {
                    let [primary, secondary] = token_env_vars(&host);
                    anyhow::anyhow!(
                        "GitHub token for {host} found but not authorized for Copilot. \
                         Run 'gh auth login' or check your {primary}/{secondary}."
                    )
                }
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
            freshness: None,
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

    #[test]
    fn host_selection_follows_gh_precedence() {
        let none: Vec<String> = Vec::new();
        let one = vec!["ghe.example.com".to_string()];
        let many = vec!["github.com".to_string(), "ghe.example.com".to_string()];
        assert_eq!(select_gh_host(None, &none), "github.com");
        assert_eq!(select_gh_host(None, &one), "ghe.example.com");
        assert_eq!(select_gh_host(None, &many), "github.com");
        assert_eq!(
            select_gh_host(Some("https://GHE.Corp.example/"), &many),
            "ghe.corp.example"
        );
        // An unusable GH_HOST is ignored rather than spliced into a URL.
        assert_eq!(
            select_gh_host(Some("evil.com/x?y"), &one),
            "ghe.example.com"
        );
        assert_eq!(select_gh_host(Some("   "), &none), "github.com");
    }

    #[test]
    fn hosts_yml_lists_every_top_level_host() {
        let yml = "# comment\ngithub.com:\n    user: a\n    oauth_token: gho_dotcom\nghe.example.com:\n    users:\n        b:\n            oauth_token: gho_ghe\n    oauth_token: gho_ghe\n    git_protocol: https\n";
        assert_eq!(
            hosts_in_hosts_yml(yml),
            vec!["github.com".to_string(), "ghe.example.com".to_string()]
        );
    }

    #[test]
    fn hosts_yml_token_is_read_from_the_requested_host_only() {
        let yml = "github.com:\n    oauth_token: gho_dotcom\nghe.example.com:\n    oauth_token: gho_ghe\n";
        assert_eq!(
            token_for_host_in_hosts_yml(yml, "github.com").as_deref(),
            Some("gho_dotcom")
        );
        assert_eq!(
            token_for_host_in_hosts_yml(yml, "ghe.example.com").as_deref(),
            Some("gho_ghe")
        );
        assert_eq!(token_for_host_in_hosts_yml(yml, "other.example"), None);
        // The case #1260 reports: an enterprise-only file used to bail.
        let enterprise_only = "ghe.example.com:\n    user: b\n    oauth_token: gho_ghe\n";
        assert_eq!(hosts_in_hosts_yml(enterprise_only), vec!["ghe.example.com"]);
        assert_eq!(
            token_for_host_in_hosts_yml(enterprise_only, "github.com"),
            None
        );
        assert_eq!(
            token_for_host_in_hosts_yml(enterprise_only, "ghe.example.com").as_deref(),
            Some("gho_ghe")
        );
    }

    #[test]
    fn keyring_service_and_default_endpoint_follow_the_logical_host() {
        assert_eq!(gh_keyring_service("github.com"), GH_KEYRING_SERVICE);
        assert_eq!(gh_keyring_service("ghe.example.com"), "gh:ghe.example.com");
        assert_eq!(
            copilot_user_url(&copilot_api_host("github.com", None)),
            "https://api.github.com/copilot_internal/user"
        );
        assert_eq!(
            copilot_user_url(&copilot_api_host("ghe.example.com:8443", None)),
            "https://api.ghe.example.com:8443/copilot_internal/user"
        );
        assert_eq!(
            gh_wincred_targets(&gh_keyring_service("ghe.example.com"))[0],
            "gh:ghe.example.com:"
        );
    }

    #[test]
    fn token_env_vars_never_cross_hosts() {
        assert_eq!(token_env_vars("github.com"), ["GH_TOKEN", "GITHUB_TOKEN"]);
        assert_eq!(token_env_vars("acme.ghe.com"), ["GH_TOKEN", "GITHUB_TOKEN"]);
        assert_eq!(
            token_env_vars("ghe.example.com"),
            ["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"]
        );
    }

    #[test]
    #[serial]
    fn github_token_is_not_sent_to_an_enterprise_host() {
        let keys = [
            "GH_HOST",
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "GH_ENTERPRISE_TOKEN",
            "GITHUB_ENTERPRISE_TOKEN",
        ];
        let mut guard = EnvGuard::capture(&keys);
        for key in keys {
            guard.remove(key);
        }
        guard.set("GH_TOKEN", "gho_dotcom");
        assert_eq!(token_from_env("ghe.example.com"), None);
        assert_eq!(token_from_env("github.com").as_deref(), Some("gho_dotcom"));
        guard.set("GH_ENTERPRISE_TOKEN", "gho_ghe");
        assert_eq!(
            token_from_env("ghe.example.com").as_deref(),
            Some("gho_ghe")
        );
        assert_eq!(token_from_env("github.com").as_deref(), Some("gho_dotcom"));
    }

    #[test]
    #[serial]
    fn current_host_reads_gh_host_and_an_enterprise_only_hosts_yml() {
        let temp = TempDir::new().unwrap();
        let mut guard = EnvGuard::capture(&["GH_HOST", "GH_CONFIG_DIR"]);
        guard.remove("GH_HOST");
        guard.set("GH_CONFIG_DIR", temp.path());
        assert_eq!(current_gh_host(), "github.com");
        std::fs::write(
            temp.path().join("hosts.yml"),
            "ghe.example.com:\n    oauth_token: gho_ghe\n    api_host: copilot-api.example.net:8443\n",
        )
        .unwrap();
        assert_eq!(current_gh_host(), "ghe.example.com");
        assert_eq!(
            parse_token_from_hosts("ghe.example.com").unwrap(),
            "gho_ghe"
        );
        let api_host = configured_api_host("ghe.example.com").unwrap();
        assert_eq!(api_host, "copilot-api.example.net:8443");
        assert_eq!(
            copilot_user_url(&copilot_api_host("ghe.example.com", Some(&api_host))),
            "https://copilot-api.example.net:8443/copilot_internal/user"
        );
        assert!(parse_token_from_hosts("github.com").is_err());
        guard.set("GH_HOST", "other.example");
        assert_eq!(current_gh_host(), "other.example");
        let probe = credential_probe().join("\n");
        assert!(probe.contains("host=other.example"), "got: {probe}");
        assert!(probe.contains("GH_ENTERPRISE_TOKEN="), "got: {probe}");
        assert!(!probe.contains("gho_ghe"), "probe leaked a token: {probe}");
    }

    #[test]
    fn host_ports_are_parsed_and_443_is_dropped() {
        let cases = [
            ("github.com:443", Some("github.com")),
            ("tenant.ghe.com:443", Some("tenant.ghe.com")),
            ("https://GitHub.com:0443/", Some("github.com")),
            ("ghe.example.com:8443", Some("ghe.example.com:8443")),
            ("ghe.example.com:08443", Some("ghe.example.com:8443")),
            ("github.com:", None),
            ("github.com:abc", None),
            ("github.com:+443", None),
            ("github.com:0", None),
            ("github.com:65536", None),
            ("github.com:443:1", None),
            ("ghe.example.com::8443", None),
            (":443", None),
            ("github.com.", None),
            ("github..com", None),
            ("-ghe.example.com", None),
            ("ghe-.example.com", None),
        ];
        for (raw, want) in cases {
            assert_eq!(normalize_gh_host(raw).as_deref(), want, "input {raw:?}");
        }
    }

    #[test]
    fn gh_host_aliases_match_gh_credential_normalization() {
        let cases = [
            ("api.github.com", "github.com"),
            ("uploads.github.com", "github.com"),
            ("api.github.localhost", "github.localhost"),
            ("api.tenant.ghe.com", "tenant.ghe.com"),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalize_gh_host(raw).as_deref(), Some(expected), "{raw}");
        }
        assert_eq!(
            token_env_vars("github.localhost"),
            ["GH_TOKEN", "GITHUB_TOKEN"]
        );
    }

    #[test]
    fn default_api_endpoint_matches_the_credential_host() {
        for raw in [
            "github.com",
            "github.com:443",
            "acme.ghe.com:443",
            "ghe.example.com",
            "ghe.example.com:443",
            "ghe.example.com:8443",
        ] {
            let host = normalize_gh_host(raw).unwrap();
            // Parse the URL the way `fetch_api` sends it, and check the host
            // it reaches is the one the credential was chosen for.
            let api_host = copilot_api_host(&host, None);
            let url = reqwest::Url::parse(&copilot_user_url(&api_host)).unwrap();
            let reached = url.host_str().unwrap().strip_prefix("api.").unwrap();
            assert_eq!(reached, gh_host_name(&host), "input {raw:?}");
            let dotcom = reached == "github.com" || reached.ends_with(".ghe.com");
            assert_eq!(is_dotcom_style_host(&host), dotcom, "input {raw:?}");
        }
        assert_eq!(
            token_env_vars(&normalize_gh_host("github.com:443").unwrap()),
            ["GH_TOKEN", "GITHUB_TOKEN"]
        );
        assert_eq!(
            token_env_vars(&normalize_gh_host("tenant.ghe.com:443").unwrap()),
            ["GH_TOKEN", "GITHUB_TOKEN"]
        );
        assert_eq!(
            token_env_vars("github.com:8443"),
            ["GH_TOKEN", "GITHUB_TOKEN"]
        );
        // A malformed GH_HOST is ignored, not classified on its own.
        assert_eq!(select_gh_host(Some("github.com:"), &[]), "github.com");
        assert_eq!(select_gh_host(Some("github.com:443"), &[]), "github.com");
    }

    #[test]
    fn hosts_yml_allows_comments_and_quoted_keys() {
        let yml = "# gh hosts\n\
                   \"github.com\": # personal\n    oauth_token: gho_dotcom # active\n\
                   'ghe.example.com':   # work\n    user: b\n    oauth_token: 'gho_ghe'\n\
                   ghe2.example.com:8443:\n    oauth_token: \"gho_#2\"\n";
        assert_eq!(
            hosts_in_hosts_yml(yml),
            vec!["github.com", "ghe.example.com", "ghe2.example.com:8443"]
        );
        assert_eq!(
            token_for_host_in_hosts_yml(yml, "github.com").as_deref(),
            Some("gho_dotcom")
        );
        assert_eq!(
            token_for_host_in_hosts_yml(yml, "ghe.example.com").as_deref(),
            Some("gho_ghe")
        );
        assert_eq!(
            token_for_host_in_hosts_yml(yml, "ghe2.example.com:8443").as_deref(),
            Some("gho_#2")
        );
    }

    #[test]
    fn hosts_yml_reads_the_active_token_not_a_per_user_one() {
        let yml = "ghe.example.com:\r\n    users:\r\n        other:\r\n            oauth_token: gho_other\r\n        me:\r\n            oauth_token: gho_me\r\n    oauth_token: gho_me # active\r\n    user: me\r\n";
        assert_eq!(
            token_for_host_in_hosts_yml(yml, "ghe.example.com").as_deref(),
            Some("gho_me")
        );
        let no_active =
            "ghe.example.com:\n    users:\n        other:\n            oauth_token: gho_other\n";
        assert_eq!(
            token_for_host_in_hosts_yml(no_active, "ghe.example.com"),
            None
        );
    }

    #[test]
    fn hosts_yml_key_with_port_443_matches_the_bare_host() {
        let yml = "github.com:443:\n    oauth_token: gho_dotcom\n";
        assert_eq!(hosts_in_hosts_yml(yml), vec!["github.com"]);
        assert_eq!(
            token_for_host_in_hosts_yml(yml, "github.com").as_deref(),
            Some("gho_dotcom")
        );
    }

    #[test]
    fn hosts_yml_resolves_yaml_aliases() {
        let yml = "ghe.example.com:\n    oauth_token: &active gho_ghe\nother.example.com:\n    oauth_token: *active\n";
        assert_eq!(
            token_for_host_in_hosts_yml(yml, "other.example.com").as_deref(),
            Some("gho_ghe")
        );
    }

    #[test]
    fn hosts_yml_routes_the_api_request_without_rekeying_credentials() {
        let yml = "ghe.example.com:\n    oauth_token: gho_ghe\n    api_host: https://copilot-api.example.net:8443/\n";
        let logical_host = "ghe.example.com";
        let api_host = api_host_for_host_in_hosts_yml(yml, logical_host).unwrap();

        assert_eq!(api_host, "copilot-api.example.net:8443");
        assert_eq!(gh_keyring_service(logical_host), "gh:ghe.example.com");
        assert_eq!(
            copilot_user_url(&copilot_api_host(logical_host, Some(&api_host))),
            "https://copilot-api.example.net:8443/copilot_internal/user"
        );
    }
}
