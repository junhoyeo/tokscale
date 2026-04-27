use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_RPC_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_IDENTITY_PROBE_BYTES: usize = 4096;
const ANTIGRAVITY_MANIFEST_VERSION: i32 = 1;
#[cfg(test)]
const SYNC_LOCK_STALE_SECS: u64 = 600;

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("Could not determine home directory")
}

pub fn get_antigravity_cache_dir() -> Result<PathBuf> {
    // Route through `paths::get_config_dir()` so `TOKSCALE_CONFIG_DIR`
    // covers the antigravity sync cache too — without this, an isolated
    // CI profile would still leak to the host's
    // `~/.config/tokscale/antigravity-cache/`. On macOS and Linux without
    // an override the resolved path is byte-identical to the historic
    // hardcoded `~/.config/tokscale/antigravity-cache/`, so existing
    // users see no path change and no data migration is required.
    Ok(crate::paths::get_config_dir().join("antigravity-cache"))
}

pub fn get_antigravity_sessions_dir() -> Result<PathBuf> {
    Ok(get_antigravity_cache_dir()?.join("sessions"))
}

pub fn get_antigravity_manifest_path() -> Result<PathBuf> {
    Ok(get_antigravity_cache_dir()?.join("manifest.json"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityManifest {
    pub version: i32,
    #[serde(rename = "syncedAt")]
    pub synced_at: Option<String>,
    pub connections: Vec<ManifestConnectionEntry>,
    pub sessions: Vec<ManifestSessionEntry>,
}

#[derive(Debug, Clone)]
pub struct AntigravityConnection {
    pub pid: u32,
    pub port: u16,
    pub csrf_token: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
struct ProcessCandidate {
    pid: u32,
    ppid: u32,
    declared_port: Option<u16>,
    csrf_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectorySummary {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "lastModifiedMs")]
    pub last_modified_ms: Option<i64>,
    #[serde(rename = "stepCount")]
    pub step_count: Option<i32>,
    #[serde(rename = "connectionFingerprint")]
    pub connection_fingerprint: String,
}

impl Default for AntigravityManifest {
    fn default() -> Self {
        Self {
            version: ANTIGRAVITY_MANIFEST_VERSION,
            synced_at: None,
            connections: Vec::new(),
            sessions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestConnectionEntry {
    pub fingerprint: String,
    pub pid: u32,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestSessionEntry {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "artifactPath")]
    pub artifact_path: String,
    #[serde(rename = "lastModifiedMs")]
    pub last_modified_ms: Option<i64>,
    #[serde(rename = "stepCount")]
    pub step_count: Option<i32>,
    #[serde(rename = "connectionFingerprint")]
    pub connection_fingerprint: String,
    #[serde(rename = "artifactHash")]
    pub artifact_hash: Option<String>,
}

#[derive(Debug, Serialize)]
struct AntigravityStatus {
    #[serde(rename = "cacheDir")]
    cache_dir: String,
    #[serde(rename = "manifestPath")]
    manifest_path: String,
    #[serde(rename = "cacheExists")]
    cache_exists: bool,
    #[serde(rename = "sessionsDirExists")]
    sessions_dir_exists: bool,
    #[serde(rename = "manifestExists")]
    manifest_exists: bool,
    #[serde(rename = "detectedConnections")]
    detected_connections: usize,
    #[serde(rename = "cachedSessions")]
    cached_sessions: usize,
    #[serde(rename = "lastSyncedAt")]
    last_synced_at: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionArtifact {
    contents: String,
    last_modified_ms: Option<i64>,
    step_count: Option<i32>,
    artifact_hash: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionCandidate {
    session_id: String,
    last_modified_ms: Option<i64>,
    artifact_path: Option<String>,
}

pub fn run_antigravity_sync() -> Result<()> {
    use colored::Colorize;

    #[cfg(target_os = "windows")]
    anyhow::bail!(
        "Antigravity sync is not supported on Windows yet. Use macOS or Linux for `tokscale antigravity sync`, or remove Antigravity from your release notes for Windows."
    );

    let cache_dir = get_antigravity_cache_dir()?;
    let sessions_dir = get_antigravity_sessions_dir()?;
    ensure_config_dir()?;
    ensure_dir(&cache_dir)?;
    ensure_dir(&sessions_dir)?;

    let _lock = SyncLockGuard::acquire(&cache_dir)?;

    let manifest = load_antigravity_manifest()?;
    let connections = detect_antigravity_connections()?;
    let summaries = list_trajectory_summaries(&connections)?;
    let filesystem_candidates = scan_filesystem_session_candidates()?;
    let export_candidates = merge_export_candidates(&manifest, &summaries, &filesystem_candidates);
    let mut next_manifest = AntigravityManifest {
        version: ANTIGRAVITY_MANIFEST_VERSION,
        synced_at: Some(chrono::Utc::now().to_rfc3339()),
        connections: connections
            .iter()
            .map(|connection| ManifestConnectionEntry {
                fingerprint: connection.fingerprint.clone(),
                pid: connection.pid,
                port: connection.port,
            })
            .collect(),
        sessions: Vec::new(),
    };

    for candidate in &export_candidates {
        if let Some(summary) = find_summary_for_candidate(&summaries, &candidate.session_id) {
            if let Some(artifact) = fetch_session_artifact(summary, &connections)? {
                let path = write_session_artifact(&summary.session_id, &artifact.contents)?;
                let relative_path = to_relative_artifact_path(&path)?;

                next_manifest.sessions.push(ManifestSessionEntry {
                    session_id: summary.session_id.clone(),
                    artifact_path: relative_path,
                    last_modified_ms: artifact.last_modified_ms,
                    step_count: artifact.step_count,
                    connection_fingerprint: summary.connection_fingerprint.clone(),
                    artifact_hash: artifact.artifact_hash,
                });
                continue;
            }
        }

        if let Some(entry) =
            fetch_historical_session_artifact(&candidate.session_id, &connections, candidate)?
        {
            next_manifest.sessions.push(entry);
            continue;
        }

        if let Some(previous) = manifest
            .sessions
            .iter()
            .find(|entry| entry.session_id == candidate.session_id)
        {
            next_manifest.sessions.push(previous.clone());
        }
    }

    next_manifest
        .sessions
        .sort_by(|left, right| left.session_id.cmp(&right.session_id));
    next_manifest
        .sessions
        .dedup_by(|left, right| left.session_id == right.session_id);
    save_antigravity_manifest(&next_manifest)?;
    cleanup_stale_session_artifacts(&manifest, &next_manifest)?;

    println!("\n  {}", "Antigravity sync".cyan());
    println!(
        "  {}",
        "Synced local Antigravity cache from running language servers.".bright_black()
    );
    println!(
        "  {}",
        format!("cache: {}", cache_dir.display()).bright_black()
    );
    println!(
        "  {}",
        format!("known sessions: {}", manifest.sessions.len()).bright_black()
    );
    println!(
        "  {}",
        format!("detected connections: {}", connections.len()).bright_black()
    );
    println!(
        "  {}",
        format!("detected sessions: {}", summaries.len()).bright_black()
    );
    println!(
        "  {}",
        format!("filesystem candidates: {}", filesystem_candidates.len()).bright_black()
    );
    println!(
        "  {}",
        format!("export candidates: {}", export_candidates.len()).bright_black()
    );
    println!(
        "  {}",
        format!(
            "cached sessions after sync: {}",
            next_manifest.sessions.len()
        )
        .bright_black()
    );
    println!();
    Ok(())
}

pub fn run_antigravity_status(json: bool) -> Result<()> {
    use colored::Colorize;

    #[cfg(target_os = "windows")]
    anyhow::bail!(
        "Antigravity status is not supported on Windows yet because local language-server discovery is not implemented there. Use macOS or Linux for `tokscale antigravity status`."
    );

    let cache_dir = get_antigravity_cache_dir()?;
    let sessions_dir = get_antigravity_sessions_dir()?;
    let manifest_path = get_antigravity_manifest_path()?;
    let connections = detect_antigravity_connections()?;
    let manifest = load_antigravity_manifest()?;

    let status = AntigravityStatus {
        cache_dir: cache_dir.display().to_string(),
        manifest_path: manifest_path.display().to_string(),
        cache_exists: cache_dir.exists(),
        sessions_dir_exists: sessions_dir.exists(),
        manifest_exists: manifest_path.exists(),
        detected_connections: connections.len(),
        cached_sessions: manifest.sessions.len(),
        last_synced_at: manifest.synced_at,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    println!("\n  {}", "Antigravity status".cyan());
    println!(
        "  {}",
        format!("cache dir: {}", status.cache_dir).bright_black()
    );
    println!(
        "  {}",
        format!("sessions dir: {}", bool_label(status.sessions_dir_exists)).bright_black()
    );
    println!(
        "  {}",
        format!("manifest: {}", bool_label(status.manifest_exists)).bright_black()
    );
    println!(
        "  {}",
        format!("detected connections: {}", status.detected_connections).bright_black()
    );
    println!(
        "  {}",
        format!("cached sessions: {}", status.cached_sessions).bright_black()
    );
    if let Some(last_synced_at) = &status.last_synced_at {
        println!(
            "  {}",
            format!("last synced: {}", last_synced_at).bright_black()
        );
    }
    println!(
        "  {}",
        "Run `tokscale antigravity sync` to refresh the local cache before reporting."
            .bright_black()
    );
    println!();
    Ok(())
}

pub fn run_antigravity_purge_cache() -> Result<()> {
    use colored::Colorize;

    let cache_dir = get_antigravity_cache_dir()?;
    if cache_dir.exists() {
        fs::remove_dir_all(&cache_dir)?;
        println!(
            "\n  {}\n",
            format!("✓ Deleted {}", cache_dir.display()).green()
        );
    } else {
        println!("\n  {}\n", "No Antigravity cache to delete.".bright_black());
    }
    Ok(())
}

fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

fn ensure_config_dir() -> Result<()> {
    let config_dir = crate::paths::get_config_dir();
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

#[derive(Debug)]
struct SyncLockGuard {
    path: PathBuf,
}

const SYNC_LOCK_ACQUIRE_ATTEMPTS: usize = 3;

impl SyncLockGuard {
    fn acquire(cache_dir: &Path) -> Result<Self> {
        let lock_path = cache_dir.join("sync.lock");
        let mut stale_recoveries = 0usize;
        loop {
            match std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    let pid = std::process::id();
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let _ = writeln!(file, "{pid} {timestamp}");
                    return Ok(SyncLockGuard { path: lock_path });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Only evict the lock when its owner is provably dead.
                    // Long-running syncs MUST keep exclusive access as
                    // long as their PID is alive, or two processes will
                    // overlap on the manifest and delete each other's
                    // artifacts. Age-based eviction was removed for this
                    // reason.
                    if let Some((existing_pid, _)) = read_sync_lock(&lock_path) {
                        if pid_is_alive(existing_pid) {
                            anyhow::bail!(
                                "Another tokscale antigravity sync is in progress (pid {existing_pid}); aborting"
                            );
                        }
                    }
                    if stale_recoveries >= SYNC_LOCK_ACQUIRE_ATTEMPTS {
                        anyhow::bail!(
                            "Could not acquire Antigravity sync lock after {SYNC_LOCK_ACQUIRE_ATTEMPTS} stale-lock recoveries; another process keeps recreating the lock file"
                        );
                    }
                    stale_recoveries += 1;
                    let _ = std::fs::remove_file(&lock_path);
                    continue;
                }
                Err(err) => {
                    return Err(
                        anyhow::Error::new(err).context("Failed to acquire Antigravity sync lock")
                    );
                }
            }
        }
    }
}

impl Drop for SyncLockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn read_sync_lock(path: &Path) -> Option<(u32, u64)> {
    let contents = std::fs::read_to_string(path).ok()?;
    let mut parts = contents.split_whitespace();
    let pid = parts.next()?.parse::<u32>().ok()?;
    let timestamp = parts.next()?.parse::<u64>().ok()?;
    Some((pid, timestamp))
}

fn pid_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        let result = unsafe { libc_kill(pid as i32, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(1)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(unix)]
extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

pub fn load_antigravity_manifest() -> Result<AntigravityManifest> {
    let manifest_path = get_antigravity_manifest_path()?;
    if !manifest_path.exists() {
        return Ok(AntigravityManifest::default());
    }

    let content = fs::read_to_string(&manifest_path).with_context(|| {
        format!(
            "Failed to read Antigravity manifest at {}",
            manifest_path.display()
        )
    })?;

    let manifest = match serde_json::from_str::<AntigravityManifest>(&content) {
        Ok(manifest) => manifest,
        Err(err) => {
            let backup_path = backup_corrupted_manifest(&manifest_path);
            eprintln!(
                "Warning: Antigravity manifest at {} is corrupted: {err}; starting fresh{}",
                manifest_path.display(),
                backup_path
                    .map(|p| format!(" (moved aside to {})", p.display()))
                    .unwrap_or_default()
            );
            return Ok(AntigravityManifest::default());
        }
    };

    if manifest.version > ANTIGRAVITY_MANIFEST_VERSION {
        anyhow::bail!(
            "Manifest from a newer tokscale version detected; refusing to overwrite (got version {}, supported {})",
            manifest.version,
            ANTIGRAVITY_MANIFEST_VERSION
        );
    }

    if manifest.version < ANTIGRAVITY_MANIFEST_VERSION {
        eprintln!(
            "Info: Antigravity manifest at {} is at version {} (current {}); starting fresh",
            manifest_path.display(),
            manifest.version,
            ANTIGRAVITY_MANIFEST_VERSION
        );
        return Ok(AntigravityManifest::default());
    }

    Ok(manifest)
}

fn backup_corrupted_manifest(manifest_path: &Path) -> Option<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file_name = manifest_path.file_name()?.to_string_lossy().to_string();
    let backup_name = format!("{file_name}.corrupt-{timestamp}");
    let backup_path = manifest_path.with_file_name(backup_name);
    fs::rename(manifest_path, &backup_path).ok()?;
    Some(backup_path)
}

pub fn save_antigravity_manifest(manifest: &AntigravityManifest) -> Result<()> {
    ensure_config_dir()?;
    let manifest_path = get_antigravity_manifest_path()?;
    let json = serde_json::to_string_pretty(manifest)?;
    atomic_write_file(&manifest_path, &json)
}

pub fn write_session_artifact(session_id: &str, contents: &str) -> Result<PathBuf> {
    let file_name = session_artifact_file_stem(session_id);
    let path = get_antigravity_sessions_dir()?.join(format!("{}.jsonl", file_name));
    atomic_write_file(&path, contents)?;
    Ok(path)
}

fn to_relative_artifact_path(path: &Path) -> Result<String> {
    Ok(path
        .strip_prefix(get_antigravity_cache_dir()?)
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string()))
}

fn delete_artifact_relative_path(relative_path: &str) -> Result<bool> {
    let artifact_path = resolve_cache_relative_artifact_path(relative_path)?;
    if artifact_path.exists() {
        fs::remove_file(&artifact_path)?;
        return Ok(true);
    }
    Ok(false)
}

fn resolve_cache_relative_artifact_path(relative_path: &str) -> Result<PathBuf> {
    let relative = Path::new(relative_path);
    if relative.is_absolute() {
        anyhow::bail!("Artifact path must stay within cache root");
    }

    if relative.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        anyhow::bail!("Artifact path must stay within cache root");
    }

    let path_text = relative.to_string_lossy();
    if !path_text.starts_with("sessions/") || !path_text.ends_with(".jsonl") {
        anyhow::bail!("Artifact path must point to a session artifact");
    }

    let cache_dir = get_antigravity_cache_dir()?;
    let candidate = cache_dir.join(relative);

    let canonical_root = cache_dir
        .canonicalize()
        .unwrap_or_else(|_| cache_dir.clone());
    let canonical_sessions = canonical_root.join("sessions");

    if candidate.exists() {
        let canonical_candidate = candidate.canonicalize().with_context(|| {
            format!(
                "Failed to canonicalize artifact path {}",
                candidate.display()
            )
        })?;
        if !canonical_candidate.starts_with(&canonical_sessions) {
            anyhow::bail!("Artifact path must stay within sessions cache root");
        }
        return Ok(canonical_candidate);
    }

    Ok(candidate)
}

#[cfg(test)]
pub fn delete_session_artifact(relative_path: &str) -> Result<bool> {
    delete_artifact_relative_path(relative_path)
}

fn sanitize_session_id(session_id: &str) -> String {
    let sanitized: String = session_id
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();

    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        "session".to_string()
    } else {
        trimmed.to_string()
    }
}

fn session_artifact_file_stem(session_id: &str) -> String {
    use sha2::{Digest, Sha256};

    let sanitized = sanitize_session_id(session_id);
    let hash = Sha256::digest(session_id.as_bytes());
    let hash_prefix = format!("{:x}", hash);
    format!("{}-{}", sanitized, &hash_prefix[..16])
}

fn atomic_write_file(path: &Path, contents: &str) -> Result<()> {
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
            .unwrap_or("antigravity"),
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

    if let Err(err) = tokscale_core::fs_atomic::replace_file(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(anyhow::anyhow!(
            "Failed to persist file atomically (temp: {}, final: {}): {}",
            temp_path.display(),
            path.display(),
            err
        ));
    }

    Ok(())
}

fn bool_label(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

pub fn detect_antigravity_connections() -> Result<Vec<AntigravityConnection>> {
    if cfg!(target_os = "windows") {
        anyhow::bail!(
            "Antigravity connection discovery is not supported on Windows yet. Use macOS or Linux, or sync from another machine and copy artifacts manually."
        );
    }

    let candidates = detect_process_candidates()?;
    let mut connections = Vec::new();

    for candidate in candidates {
        let ports = candidate_probe_ports(&candidate, find_listening_ports(candidate.pid)?);
        for port in ports {
            if probe_heartbeat(port, &candidate.csrf_token) {
                connections.push(AntigravityConnection {
                    pid: candidate.pid,
                    port,
                    csrf_token: candidate.csrf_token.clone(),
                    fingerprint: format!("pid:{}:port:{}", candidate.pid, port),
                });
                break;
            }
        }
    }

    connections.sort_by(|left, right| {
        right
            .pid
            .cmp(&left.pid)
            .then_with(|| left.port.cmp(&right.port))
    });
    connections.dedup_by(|left, right| left.pid == right.pid && left.port == right.port);

    Ok(connections)
}

fn candidate_probe_ports(candidate: &ProcessCandidate, mut ports: Vec<u16>) -> Vec<u16> {
    if let Some(declared_port) = candidate.declared_port {
        if !ports.contains(&declared_port) {
            ports.push(declared_port);
        }
    }

    ports.sort_unstable();
    ports.dedup();
    ports
}

fn detect_process_candidates() -> Result<Vec<ProcessCandidate>> {
    let output = run_command("ps", &["-ww", "-eo", "pid,ppid,args"])?;
    let mut candidates = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }

        let Ok(pid) = parts[0].parse::<u32>() else {
            continue;
        };
        let Ok(ppid) = parts[1].parse::<u32>() else {
            continue;
        };
        let command = parts[2..].join(" ");
        if !is_antigravity_process(&command) {
            continue;
        }

        // Defense-in-depth: a same-user process can advertise matching CLI
        // args to poison cache discovery. When exe-path introspection is
        // available, accept the candidate only if the binary path looks
        // like a language server or an antigravity binary, since
        // `is_antigravity_process` already validated the antigravity
        // affiliation via argv (e.g. `--app_data_dir antigravity` invoked
        // against a generic `language_server` binary). Default to true on
        // platforms where exe-path lookup is unavailable so detection does
        // not regress.
        let exe_ok = process_executable_path(pid)
            .map(|path| {
                let lower = path.to_string_lossy().to_lowercase();
                lower.contains("antigravity") || lower.contains("language_server")
            })
            .unwrap_or(true);
        if !exe_ok {
            continue;
        }

        let Some(csrf_token) = extract_csrf_token(&command) else {
            continue;
        };
        let declared_port = extract_declared_port(&command);

        candidates.push(ProcessCandidate {
            pid,
            ppid,
            declared_port,
            csrf_token,
        });
    }

    candidates.sort_by(|left, right| {
        right
            .pid
            .cmp(&left.pid)
            .then_with(|| right.ppid.cmp(&left.ppid))
            .then_with(|| right.declared_port.cmp(&left.declared_port))
    });
    candidates.dedup_by(|left, right| left.pid == right.pid);

    Ok(candidates)
}

fn is_antigravity_process(command: &str) -> bool {
    let lower = command.to_lowercase();
    (lower.contains("language_server")
        && (lower.contains("antigravity") || lower.contains("--app_data_dir antigravity")))
        || lower.contains("/antigravity/")
        || lower.contains("\\antigravity\\")
}

fn process_executable_path(pid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let link = format!("/proc/{pid}/exe");
        std::fs::read_link(&link).ok()
    }
    #[cfg(target_os = "macos")]
    {
        let pid_str = pid.to_string();
        let output = run_command("lsof", &["-p", &pid_str, "-Fn"]).ok()?;
        for line in output.lines() {
            if let Some(rest) = line.strip_prefix('n') {
                if rest.contains(".app/Contents/MacOS/") {
                    return Some(PathBuf::from(rest));
                }
            }
        }
        return None;
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

fn extract_csrf_token(command: &str) -> Option<String> {
    let token = extract_flag_value(command, "--csrf_token")?;
    if token.len() >= 32 && token.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-') {
        Some(token)
    } else {
        None
    }
}

fn extract_declared_port(command: &str) -> Option<u16> {
    extract_flag_value(command, "--extension_server_port")?
        .parse::<u16>()
        .ok()
}

fn extract_flag_value(command: &str, flag: &str) -> Option<String> {
    let compact = format!("{}=", flag);
    if let Some(idx) = command.find(&compact) {
        let rest = &command[idx + compact.len()..];
        return rest
            .split_whitespace()
            .next()
            .map(|value| value.to_string());
    }

    let idx = command.find(flag)?;
    let rest = &command[idx + flag.len()..];
    rest.split_whitespace()
        .find(|value| !value.is_empty())
        .map(|value| value.trim().to_string())
}

fn find_listening_ports(pid: u32) -> Result<Vec<u16>> {
    let pid_str = pid.to_string();
    let mut ports = run_port_query(
        "lsof",
        "lsof",
        &["-Pan", "-p", &pid_str, "-iTCP", "-sTCP:LISTEN"],
    )?;

    if ports.is_empty() {
        ports = run_port_query("lsof", "lsof", &["-Pan", "-p", &pid_str, "-i"])?;
    }

    ports.sort_unstable();
    ports.dedup();
    Ok(ports)
}

fn run_port_query(program: &str, warning_label: &str, args: &[&str]) -> Result<Vec<u16>> {
    match run_command(program, args) {
        Ok(output) => Ok(parse_ports(&output)),
        Err(err) if is_command_not_found(&err) => {
            eprintln!(
                "Warning: {} is unavailable; skipping port discovery",
                warning_label
            );
            Ok(Vec::new())
        }
        Err(err) => Err(err),
    }
}

fn is_command_not_found(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
    })
}

fn parse_ports(output: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in output.lines() {
        if let Some(port) = parse_port_from_line(line) {
            ports.push(port);
        }
    }
    ports
}

fn parse_port_from_line(line: &str) -> Option<u16> {
    for token in line.split_whitespace() {
        if let Some(port) = token
            .strip_prefix("127.0.0.1:")
            .or_else(|| token.strip_prefix("localhost:"))
            .or_else(|| token.strip_prefix("*:"))
            .or_else(|| token.strip_prefix("::1:"))
        {
            let cleaned = port.trim_end_matches("(LISTEN)").trim_end_matches(',');
            if let Ok(parsed) = cleaned.parse::<u16>() {
                return Some(parsed);
            }
        }
    }

    if let Some(idx) = line.rfind(':') {
        let rest = line[idx + 1..].trim();
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !digits.is_empty() {
            return digits.parse::<u16>().ok();
        }
    }

    None
}

fn probe_heartbeat(port: u16, csrf_token: &str) -> bool {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };

    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    let body = r#"{"uuid":"00000000-0000-0000-0000-000000000000"}"#;
    let request = format!(
        "POST /exa.language_server_pb.LanguageServerService/Heartbeat HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnect-Protocol-Version: 1\r\nX-Codeium-Csrf-Token: {}\r\nConnection: close\r\n\r\n{}",
        port,
        body.len(),
        csrf_token,
        body
    );

    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    if reader.read_line(&mut status_line).is_err() {
        return false;
    }

    let status_ok = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .is_some_and(|status| status == 200);
    if !status_ok {
        return false;
    }

    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() {
            return false;
        }
        if header.trim().is_empty() {
            break;
        }
    }

    let mut buffer = String::new();
    let _ = reader
        .by_ref()
        .take(MAX_IDENTITY_PROBE_BYTES as u64)
        .read_to_string(&mut buffer);

    if !heartbeat_response_looks_well_formed(&buffer) {
        return false;
    }

    probe_endpoint_identity(port, csrf_token)
}

fn heartbeat_response_looks_well_formed(body: &str) -> bool {
    let trimmed = body.trim_start();
    let json_start = trimmed.find(['{', '[']).map(|idx| &trimmed[idx..]);
    let Some(slice) = json_start else {
        return false;
    };
    serde_json::from_str::<Value>(slice).is_ok()
}

fn probe_endpoint_identity(port: u16, csrf_token: &str) -> bool {
    for method in [
        "GetCascadeTrajectoryGeneratorMetadata",
        "GetAllCascadeTrajectories",
    ] {
        if let Some(body) = identity_probe_request(port, csrf_token, method) {
            if response_contains_antigravity_marker(&body) {
                return true;
            }
        }
    }
    false
}

fn identity_probe_request(port: u16, csrf_token: &str, method: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    let body = r#"{}"#;
    let request = format!(
        "POST /exa.language_server_pb.LanguageServerService/{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnect-Protocol-Version: 1\r\nX-Codeium-Csrf-Token: {}\r\nConnection: close\r\n\r\n{}",
        method,
        port,
        body.len(),
        csrf_token,
        body
    );

    if stream.write_all(request.as_bytes()).is_err() {
        return None;
    }

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    if reader.read_line(&mut status_line).is_err() {
        return None;
    }

    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() {
            return None;
        }
        if header.trim().is_empty() {
            break;
        }
    }

    let mut buffer = String::new();
    let _ = reader
        .by_ref()
        .take(MAX_IDENTITY_PROBE_BYTES as u64)
        .read_to_string(&mut buffer);
    Some(buffer)
}

fn response_contains_antigravity_marker(body: &str) -> bool {
    let trimmed = body.trim_start();
    let json_start = trimmed.find(['{', '[']);
    let Some(idx) = json_start else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&trimmed[idx..]) else {
        return false;
    };
    contains_antigravity_marker(&value)
}

fn contains_antigravity_marker(value: &Value) -> bool {
    const MARKERS: &[&str] = &[
        "cascadeId",
        "cascadeTrajectories",
        "trajectorySummaries",
        "generatorMetadata",
        "serverInfo",
        "serverCapabilities",
    ];
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                if MARKERS.iter().any(|m| m.eq_ignore_ascii_case(key)) {
                    return true;
                }
                if contains_antigravity_marker(val) {
                    return true;
                }
            }
            false
        }
        Value::Array(items) => items.iter().any(contains_antigravity_marker),
        _ => false,
    }
}

fn run_command(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("Failed to run {} {}", program, args.join(" ")))?;

    if !output.status.success() && output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!(
            "Warning: {} {} exited with status {}{}",
            program,
            args.join(" "),
            output.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        );
        return Ok(String::new());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn list_trajectory_summaries(
    connections: &[AntigravityConnection],
) -> Result<Vec<TrajectorySummary>> {
    let mut merged: HashMap<String, TrajectorySummary> = HashMap::new();

    for connection in connections {
        let response = match rpc_request(
            connection,
            "GetAllCascadeTrajectories",
            &serde_json::json!({}),
        ) {
            Ok(response) => response,
            Err(err) => {
                eprintln!(
                    "Warning: failed to list Antigravity trajectories for {}: {err:#}",
                    connection.fingerprint
                );
                continue;
            }
        };

        for summary in normalize_trajectory_summaries(&response, &connection.fingerprint) {
            merge_summary(&mut merged, summary);
        }
    }

    let mut values: Vec<TrajectorySummary> = merged.into_values().collect();
    values.sort_by(|left, right| {
        right
            .last_modified_ms
            .unwrap_or_default()
            .cmp(&left.last_modified_ms.unwrap_or_default())
            .then_with(|| {
                right
                    .step_count
                    .unwrap_or_default()
                    .cmp(&left.step_count.unwrap_or_default())
            })
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    Ok(values)
}

fn merge_summary(merged: &mut HashMap<String, TrajectorySummary>, summary: TrajectorySummary) {
    match merged.get(&summary.session_id) {
        Some(existing) if !is_better_summary(&summary, existing) => {}
        _ => {
            merged.insert(summary.session_id.clone(), summary);
        }
    }
}

fn scan_filesystem_session_candidates() -> Result<Vec<SessionCandidate>> {
    let root = home_dir()?.join(".gemini/antigravity");
    let brain_dir = root.join("brain");
    let conversations_dir = root.join("conversations");
    let mut candidates: HashMap<String, SessionCandidate> = HashMap::new();

    if brain_dir.exists() {
        for entry in fs::read_dir(&brain_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let session_id = entry.file_name().to_string_lossy().to_string();
            if session_id.trim().is_empty() {
                continue;
            }

            let modified = latest_modified_in_dir(&path)?;
            merge_candidate(
                &mut candidates,
                SessionCandidate {
                    session_id,
                    last_modified_ms: modified,
                    artifact_path: None,
                },
            );
        }
    }

    if conversations_dir.exists() {
        for entry in fs::read_dir(&conversations_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("pb") {
                continue;
            }

            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };

            let modified = file_modified_ms(&path)?;
            merge_candidate(
                &mut candidates,
                SessionCandidate {
                    session_id: stem.to_string(),
                    last_modified_ms: modified,
                    artifact_path: None,
                },
            );
        }
    }

    let mut values: Vec<SessionCandidate> = candidates.into_values().collect();
    values.sort_by(|left, right| {
        right
            .last_modified_ms
            .unwrap_or_default()
            .cmp(&left.last_modified_ms.unwrap_or_default())
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    Ok(values)
}

fn merge_export_candidates(
    manifest: &AntigravityManifest,
    summaries: &[TrajectorySummary],
    filesystem: &[SessionCandidate],
) -> Vec<SessionCandidate> {
    let mut merged: HashMap<String, SessionCandidate> = HashMap::new();

    for summary in summaries {
        merge_candidate(
            &mut merged,
            SessionCandidate {
                session_id: summary.session_id.clone(),
                last_modified_ms: summary.last_modified_ms,
                artifact_path: None,
            },
        );
    }

    for candidate in filesystem {
        merge_candidate(&mut merged, candidate.clone());
    }

    for session in &manifest.sessions {
        merge_candidate(
            &mut merged,
            SessionCandidate {
                session_id: session.session_id.clone(),
                last_modified_ms: session.last_modified_ms,
                artifact_path: Some(session.artifact_path.clone()),
            },
        );
    }

    let mut values: Vec<SessionCandidate> = merged.into_values().collect();
    values.sort_by(|left, right| {
        right
            .last_modified_ms
            .unwrap_or_default()
            .cmp(&left.last_modified_ms.unwrap_or_default())
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    values
}

fn merge_candidate(target: &mut HashMap<String, SessionCandidate>, next: SessionCandidate) {
    match target.get(&next.session_id) {
        Some(existing)
            if existing.last_modified_ms.unwrap_or_default()
                > next.last_modified_ms.unwrap_or_default() => {}
        Some(existing)
            if existing.last_modified_ms == next.last_modified_ms
                && existing.artifact_path.is_some() => {}
        _ => {
            target.insert(next.session_id.clone(), next);
        }
    }
}

fn latest_modified_in_dir(path: &Path) -> Result<Option<i64>> {
    let mut latest = file_modified_ms(path)?;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let modified = file_modified_ms(&entry.path())?;
        if modified.unwrap_or_default() > latest.unwrap_or_default() {
            latest = modified;
        }
    }
    Ok(latest)
}

fn file_modified_ms(path: &Path) -> Result<Option<i64>> {
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(None);
    };
    let Ok(modified) = metadata.modified() else {
        return Ok(None);
    };
    let datetime = chrono::DateTime::<chrono::Utc>::from(modified);
    Ok(Some(datetime.timestamp_millis()))
}

fn find_summary_for_candidate<'a>(
    summaries: &'a [TrajectorySummary],
    session_id: &str,
) -> Option<&'a TrajectorySummary> {
    summaries
        .iter()
        .find(|summary| summary.session_id == session_id)
}

fn fetch_historical_session_artifact(
    session_id: &str,
    connections: &[AntigravityConnection],
    candidate: &SessionCandidate,
) -> Result<Option<ManifestSessionEntry>> {
    let fallback_summary = TrajectorySummary {
        session_id: session_id.to_string(),
        last_modified_ms: candidate.last_modified_ms,
        step_count: None,
        connection_fingerprint: connections
            .first()
            .map(|connection| connection.fingerprint.clone())
            .unwrap_or_default(),
    };

    if let Some(artifact) = fetch_session_artifact(&fallback_summary, connections)? {
        let path = write_session_artifact(session_id, &artifact.contents)?;
        return Ok(Some(ManifestSessionEntry {
            session_id: session_id.to_string(),
            artifact_path: to_relative_artifact_path(&path)?,
            last_modified_ms: artifact.last_modified_ms,
            step_count: artifact.step_count,
            connection_fingerprint: fallback_summary.connection_fingerprint,
            artifact_hash: artifact.artifact_hash,
        }));
    }

    Ok(None)
}

fn rpc_request(connection: &AntigravityConnection, method: &str, body: &Value) -> Result<Value> {
    let mut stream = TcpStream::connect(("127.0.0.1", connection.port)).with_context(|| {
        format!(
            "Failed to connect to Antigravity RPC on port {}",
            connection.port
        )
    })?;

    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let body_text = serde_json::to_string(body)?;
    let request = format!(
        "POST /exa.language_server_pb.LanguageServerService/{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnect-Protocol-Version: 1\r\nX-Codeium-Csrf-Token: {}\r\nConnection: close\r\n\r\n{}",
        method,
        connection.port,
        body_text.len(),
        connection.csrf_token,
        body_text
    );

    stream.write_all(request.as_bytes())?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;

    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| anyhow::anyhow!("Malformed HTTP response from Antigravity RPC"))?;

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let trimmed = header.trim();
        if trimmed.is_empty() {
            break;
        }

        let lower = trimmed.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
        if lower.contains("transfer-encoding") && lower.contains("chunked") {
            chunked = true;
        }
    }

    let response_body = if let Some(length) = content_length {
        if length > MAX_RPC_BODY_BYTES {
            anyhow::bail!(
                "Antigravity RPC body of {length} bytes exceeds {MAX_RPC_BODY_BYTES} cap"
            );
        }
        let mut bytes = vec![0_u8; length];
        reader.read_exact(&mut bytes)?;
        String::from_utf8(bytes)?
    } else if chunked {
        read_chunked_body(&mut reader)?
    } else {
        let mut text = String::new();
        reader
            .by_ref()
            .take(MAX_RPC_BODY_BYTES as u64 + 1)
            .read_to_string(&mut text)?;
        if text.len() > MAX_RPC_BODY_BYTES {
            anyhow::bail!(
                "Antigravity RPC body of {} bytes exceeds {MAX_RPC_BODY_BYTES} cap",
                text.len()
            );
        }
        text
    };

    if status_code != 200 {
        return Err(anyhow::anyhow!(
            "Antigravity RPC {} failed with status {}: {}",
            method,
            status_code,
            response_body
        ));
    }

    Ok(serde_json::from_str(&response_body)?)
}

fn read_chunked_body(reader: &mut BufReader<TcpStream>) -> Result<String> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line)?;
        let chunk_size = parse_chunk_size_line(&size_line)?;
        if chunk_size == 0 {
            break;
        }

        if chunk_size > MAX_RPC_BODY_BYTES
            || body.len().saturating_add(chunk_size) > MAX_RPC_BODY_BYTES
        {
            anyhow::bail!(
                "Antigravity RPC body of {} bytes exceeds {MAX_RPC_BODY_BYTES} cap",
                body.len().saturating_add(chunk_size)
            );
        }

        let mut chunk = vec![0_u8; chunk_size];
        reader.read_exact(&mut chunk)?;
        body.extend_from_slice(&chunk);

        let mut crlf = [0_u8; 2];
        reader.read_exact(&mut crlf)?;
    }

    Ok(String::from_utf8(body)?)
}

fn parse_chunk_size_line(size_line: &str) -> Result<usize> {
    let trimmed = size_line.trim();
    let chunk_size = trimmed
        .split(';')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing chunk size"))?;

    usize::from_str_radix(chunk_size, 16)
        .with_context(|| format!("Invalid chunk size line: {trimmed}"))
}

fn normalize_trajectory_summaries(response: &Value, fingerprint: &str) -> Vec<TrajectorySummary> {
    let items: Vec<Value> = if let Some(array) = response
        .get("trajectorySummaries")
        .and_then(Value::as_array)
    {
        array.to_vec()
    } else if let Some(object) = response
        .get("trajectorySummaries")
        .and_then(Value::as_object)
    {
        object
            .iter()
            .map(|(key, value)| {
                let mut entry = value.clone();
                if entry.get("cascadeId").is_none() {
                    entry["cascadeId"] = Value::String(key.clone());
                }
                entry
            })
            .collect()
    } else if let Some(array) = response
        .get("cascadeTrajectories")
        .and_then(Value::as_array)
    {
        array.to_vec()
    } else {
        Vec::new()
    };

    items
        .into_iter()
        .filter_map(|item| normalize_trajectory_summary(&item, fingerprint))
        .collect()
}

fn fetch_session_artifact(
    summary: &TrajectorySummary,
    connections: &[AntigravityConnection],
) -> Result<Option<SessionArtifact>> {
    let preferred = connections
        .iter()
        .find(|connection| connection.fingerprint == summary.connection_fingerprint);

    let mut ordered: Vec<&AntigravityConnection> = Vec::new();
    if let Some(preferred_connection) = preferred {
        ordered.push(preferred_connection);
    }
    ordered.extend(
        connections
            .iter()
            .filter(|connection| connection.fingerprint != summary.connection_fingerprint),
    );

    for connection in ordered {
        if let Some(artifact) = try_fetch_session_artifact(summary, connection)? {
            return Ok(Some(artifact));
        }
    }

    Ok(None)
}

fn try_fetch_session_artifact(
    summary: &TrajectorySummary,
    connection: &AntigravityConnection,
) -> Result<Option<SessionArtifact>> {
    let response = match rpc_request(
        connection,
        "GetCascadeTrajectoryGeneratorMetadata",
        &serde_json::json!({ "cascadeId": summary.session_id }),
    ) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };

    let metadata = response
        .get("generatorMetadata")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if metadata.is_empty() {
        return Ok(None);
    }

    let lines = normalize_session_metadata(&summary.session_id, &metadata)?;
    if lines.is_empty() {
        return Ok(None);
    }

    let contents = format!("{}\n", lines.join("\n"));
    let artifact_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(contents.as_bytes());
        Some(format!("sha256:{:x}", hasher.finalize()))
    };

    Ok(Some(SessionArtifact {
        contents,
        last_modified_ms: summary.last_modified_ms,
        step_count: summary.step_count,
        artifact_hash,
    }))
}

fn normalize_session_metadata(session_id: &str, metadata: &[Value]) -> Result<Vec<String>> {
    let mut lines = Vec::new();

    for meta in metadata {
        let chat_model = meta.get("chatModel").unwrap_or(meta);
        let model_id = resolve_model_id(chat_model);
        let created_at = chat_model
            .get("chatStartMetadata")
            .and_then(|value| value.get("createdAt"))
            .and_then(parse_timestamp_value);

        lines.push(serde_json::to_string(&serde_json::json!({
            "type": "session_meta",
            "sessionId": session_id,
            "modelId": model_id,
            "timestamp": created_at,
        }))?);

        if let Some(retry_infos) = chat_model.get("retryInfos").and_then(Value::as_array) {
            for retry in retry_infos {
                let usage = retry.get("usage").unwrap_or(retry);
                let input = to_safe_i64(usage.get("inputTokens"));
                let output = to_safe_i64(usage.get("outputTokens"));
                let cache_read = to_safe_i64(usage.get("cacheReadTokens"));
                let reasoning = to_safe_i64(usage.get("thinkingOutputTokens"));
                let timestamp = usage
                    .get("createdAt")
                    .or_else(|| usage.get("timestamp"))
                    .and_then(parse_timestamp_value)
                    .or(created_at);

                if input == 0 && output == 0 && cache_read == 0 && reasoning == 0 {
                    continue;
                }

                lines.push(serde_json::to_string(&serde_json::json!({
                    "type": "usage",
                    "sessionId": session_id,
                    "modelId": model_id,
                    "timestamp": timestamp,
                    "input": input,
                    "output": output,
                    "cacheRead": cache_read,
                    "cacheWrite": 0,
                    "reasoning": reasoning,
                    "responseId": usage.get("responseId").and_then(Value::as_str),
                }))?);
            }
        }
    }

    Ok(lines)
}

fn resolve_model_id(chat_model: &Value) -> String {
    chat_model
        .get("responseModel")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            chat_model
                .get("model")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or("unknown")
        .to_string()
}

fn to_safe_i64(value: Option<&Value>) -> i64 {
    value
        .and_then(|inner| {
            inner
                .as_i64()
                .or_else(|| inner.as_u64().and_then(|number| i64::try_from(number).ok()))
                .or_else(|| inner.as_str().and_then(|text| text.parse::<i64>().ok()))
        })
        .unwrap_or(0)
        .max(0)
}

fn stale_relative_paths(previous: &AntigravityManifest, next: &AntigravityManifest) -> Vec<String> {
    let next_paths: std::collections::HashSet<&str> = next
        .sessions
        .iter()
        .map(|session| session.artifact_path.as_str())
        .collect();

    previous
        .sessions
        .iter()
        .filter(|session| !next_paths.contains(session.artifact_path.as_str()))
        .map(|session| session.artifact_path.clone())
        .collect()
}

fn cleanup_stale_session_artifacts(
    previous: &AntigravityManifest,
    next: &AntigravityManifest,
) -> Result<()> {
    for relative_path in stale_relative_paths(previous, next) {
        delete_artifact_relative_path(&relative_path)?;
    }

    Ok(())
}

fn parse_timestamp_value(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| {
            value.as_str().and_then(|text| {
                text.parse::<i64>().ok().or_else(|| {
                    chrono::DateTime::parse_from_rfc3339(text)
                        .ok()
                        .map(|datetime| datetime.timestamp_millis())
                })
            })
        })
        .filter(|timestamp| *timestamp > 0)
}

fn normalize_trajectory_summary(item: &Value, fingerprint: &str) -> Option<TrajectorySummary> {
    let session_id = first_string(&[
        item.get("cascadeId"),
        item.get("trajectoryId"),
        item.get("id"),
        item.get("sessionId"),
    ])?;

    Some(TrajectorySummary {
        session_id,
        last_modified_ms: parse_timestamp(&[
            item.get("lastModifiedTime"),
            item.get("lastModified"),
            item.get("updatedAt"),
            item.get("modifiedAt"),
        ]),
        step_count: first_i32(&[
            item.get("stepCount"),
            item.get("numSteps"),
            item.get("totalSteps"),
        ]),
        connection_fingerprint: fingerprint.to_string(),
    })
}

fn is_better_summary(next: &TrajectorySummary, current: &TrajectorySummary) -> bool {
    let next_modified = next.last_modified_ms.unwrap_or_default();
    let current_modified = current.last_modified_ms.unwrap_or_default();
    if next_modified != current_modified {
        return next_modified > current_modified;
    }

    next.step_count.unwrap_or_default() > current.step_count.unwrap_or_default()
}

fn first_string(values: &[Option<&Value>]) -> Option<String> {
    values.iter().find_map(|value| {
        value
            .and_then(|inner| inner.as_str())
            .filter(|text| !text.trim().is_empty())
            .map(|text| text.to_string())
    })
}

fn first_i32(values: &[Option<&Value>]) -> Option<i32> {
    values.iter().find_map(|value| {
        value.and_then(|inner| {
            inner
                .as_i64()
                .and_then(|number| i32::try_from(number).ok())
                .or_else(|| inner.as_u64().and_then(|number| i32::try_from(number).ok()))
                .or_else(|| inner.as_str().and_then(|text| text.parse::<i32>().ok()))
        })
    })
}

fn parse_timestamp(values: &[Option<&Value>]) -> Option<i64> {
    values.iter().find_map(|value| {
        value.and_then(|inner| {
            inner
                .as_i64()
                .or_else(|| inner.as_u64().and_then(|number| i64::try_from(number).ok()))
                .or_else(|| {
                    inner
                        .as_str()
                        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
                        .map(|datetime| datetime.timestamp_millis())
                })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn sample_manifest() -> AntigravityManifest {
        AntigravityManifest {
            version: ANTIGRAVITY_MANIFEST_VERSION,
            synced_at: Some("2026-03-24T00:00:00Z".to_string()),
            connections: vec![ManifestConnectionEntry {
                fingerprint: "pid:1:port:1234".to_string(),
                pid: 1,
                port: 1234,
            }],
            sessions: vec![ManifestSessionEntry {
                session_id: "session-1".to_string(),
                artifact_path: "sessions/session-1.jsonl".to_string(),
                last_modified_ms: Some(100),
                step_count: Some(2),
                connection_fingerprint: "pid:1:port:1234".to_string(),
                artifact_hash: Some("sha256:abc".to_string()),
            }],
        }
    }

    #[test]
    fn extract_flag_value_supports_space_and_equals() {
        assert_eq!(
            extract_flag_value("binary --csrf_token abcd-1234", "--csrf_token"),
            Some("abcd-1234".to_string())
        );
        assert_eq!(
            extract_flag_value(
                "binary --extension_server_port=4321",
                "--extension_server_port"
            ),
            Some("4321".to_string())
        );
    }

    #[test]
    fn parse_port_from_line_reads_lsof_output() {
        assert_eq!(
            parse_port_from_line("proc 123 user 12u IPv4 0x0 0t0 TCP 127.0.0.1:41234 (LISTEN)"),
            Some(41234)
        );
    }

    #[test]
    fn normalize_trajectory_summary_prefers_expected_fields() {
        let value = serde_json::json!({
            "cascadeId": "session-123",
            "lastModifiedTime": "2026-03-24T10:00:00Z",
            "stepCount": 9
        });

        let summary = normalize_trajectory_summary(&value, "pid:1:port:1000").unwrap();
        assert_eq!(summary.session_id, "session-123");
        assert_eq!(summary.step_count, Some(9));
        assert_eq!(summary.connection_fingerprint, "pid:1:port:1000");
        assert!(summary.last_modified_ms.is_some());
    }

    #[test]
    fn session_artifact_file_stem_avoids_collisions_for_sanitized_ids() {
        let left = session_artifact_file_stem("session/one");
        let right = session_artifact_file_stem("session:one");

        assert_ne!(left, right);
        assert!(left.starts_with("session-one-"));
        assert!(right.starts_with("session-one-"));
    }

    #[test]
    fn parse_chunk_size_line_supports_extensions() {
        assert_eq!(parse_chunk_size_line("1a;foo=bar\r\n").unwrap(), 26);
    }

    #[test]
    fn parse_chunk_size_line_rejects_invalid_sizes() {
        let err = parse_chunk_size_line("bogus\r\n").unwrap_err();
        assert!(err.to_string().contains("Invalid chunk size line"));
    }

    #[test]
    fn merge_summary_prefers_better_entries() {
        let mut merged = HashMap::new();
        merge_summary(
            &mut merged,
            TrajectorySummary {
                session_id: "session-1".to_string(),
                last_modified_ms: Some(10),
                step_count: Some(1),
                connection_fingerprint: "pid:1:port:1111".to_string(),
            },
        );
        merge_summary(
            &mut merged,
            TrajectorySummary {
                session_id: "session-1".to_string(),
                last_modified_ms: Some(20),
                step_count: Some(3),
                connection_fingerprint: "pid:2:port:2222".to_string(),
            },
        );

        let summary = merged.get("session-1").unwrap();
        assert_eq!(summary.last_modified_ms, Some(20));
        assert_eq!(summary.step_count, Some(3));
        assert_eq!(summary.connection_fingerprint, "pid:2:port:2222");
    }

    #[test]
    fn run_port_query_treats_missing_lsof_as_empty() {
        let ports = run_port_query(
            "__tokscale_missing_lsof__",
            "lsof",
            &["-Pan", "-p", "1", "-i"],
        )
        .unwrap();

        assert!(ports.is_empty());
    }

    #[test]
    fn candidate_probe_ports_falls_back_to_declared_port() {
        let candidate = ProcessCandidate {
            pid: 1,
            ppid: 0,
            declared_port: Some(4242),
            csrf_token: "token".to_string(),
        };

        assert_eq!(candidate_probe_ports(&candidate, Vec::new()), vec![4242]);
        assert_eq!(candidate_probe_ports(&candidate, vec![4242]), vec![4242]);
        assert_eq!(
            candidate_probe_ports(&candidate, vec![5555]),
            vec![4242, 5555]
        );
    }

    #[test]
    fn normalize_session_metadata_emits_meta_and_usage_rows() {
        let metadata = vec![serde_json::json!({
            "chatModel": {
                "responseModel": "claude-sonnet-4.6",
                "chatStartMetadata": { "createdAt": "2026-03-24T10:00:00Z" },
                "retryInfos": [{
                    "usage": {
                        "inputTokens": 10,
                        "outputTokens": 5,
                        "cacheReadTokens": 2,
                        "thinkingOutputTokens": 1,
                        "responseId": "resp-1"
                    }
                }]
            }
        })];

        let lines = normalize_session_metadata("session-1", &metadata).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines
            .iter()
            .any(|line| line.contains("\"type\":\"session_meta\"")));
        assert!(lines.iter().any(|line| line.contains("\"type\":\"usage\"")));
    }

    #[test]
    fn normalize_session_metadata_accepts_numeric_retry_timestamps() {
        let metadata = vec![serde_json::json!({
            "chatModel": {
                "responseModel": "claude-sonnet-4.6",
                "retryInfos": [{
                    "usage": {
                        "inputTokens": 10,
                        "outputTokens": 5,
                        "cacheReadTokens": 2,
                        "thinkingOutputTokens": 1,
                        "timestamp": 1_711_447_200_000_i64,
                        "responseId": "resp-1"
                    }
                }]
            }
        })];

        let lines = normalize_session_metadata("session-1", &metadata).unwrap();
        let usage: Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(
            usage.get("timestamp").and_then(Value::as_i64),
            Some(1_711_447_200_000)
        );
    }

    #[test]
    fn stale_relative_paths_finds_removed_artifacts() {
        let previous = sample_manifest();
        let next = AntigravityManifest::default();
        assert_eq!(
            stale_relative_paths(&previous, &next),
            vec!["sessions/session-1.jsonl".to_string()]
        );
    }

    #[test]
    #[serial]
    fn cleanup_stale_session_artifacts_removes_legacy_files_after_migration() {
        let temp_dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        let sessions_dir = get_antigravity_sessions_dir().unwrap();
        std::fs::create_dir_all(&sessions_dir).unwrap();

        let legacy_relative = "sessions/session-one.jsonl".to_string();
        let legacy_path = get_antigravity_cache_dir().unwrap().join(&legacy_relative);
        std::fs::write(&legacy_path, "legacy\n").unwrap();

        let new_path = write_session_artifact("session/one", "new\n").unwrap();
        let new_relative = to_relative_artifact_path(&new_path).unwrap();

        let previous = AntigravityManifest {
            sessions: vec![ManifestSessionEntry {
                session_id: "session/one".to_string(),
                artifact_path: legacy_relative,
                last_modified_ms: None,
                step_count: None,
                connection_fingerprint: "pid:1:port:1111".to_string(),
                artifact_hash: None,
            }],
            ..AntigravityManifest::default()
        };
        let next = AntigravityManifest {
            sessions: vec![ManifestSessionEntry {
                session_id: "session/one".to_string(),
                artifact_path: new_relative,
                last_modified_ms: None,
                step_count: None,
                connection_fingerprint: "pid:1:port:1111".to_string(),
                artifact_hash: None,
            }],
            ..AntigravityManifest::default()
        };

        cleanup_stale_session_artifacts(&previous, &next).unwrap();
        assert!(!legacy_path.exists());
        assert!(new_path.exists());

        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    #[serial]
    fn delete_artifact_relative_path_rejects_paths_outside_cache_root() {
        let temp_dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        let err = delete_artifact_relative_path("../outside.jsonl").unwrap_err();
        assert!(err.to_string().contains("cache root"));

        let absolute = temp_dir.path().join("outside.jsonl");
        let err = delete_artifact_relative_path(absolute.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("cache root"));

        let err = delete_artifact_relative_path("manifest.json").unwrap_err();
        assert!(err.to_string().contains("session artifact"));

        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    #[serial]
    #[cfg(unix)]
    fn delete_artifact_relative_path_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        let cache_dir = get_antigravity_cache_dir().unwrap();
        let sessions_dir = cache_dir.join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        let outside_dir = temp_dir.path().join("escape");
        std::fs::create_dir_all(&outside_dir).unwrap();
        let outside_file = outside_dir.join("secret.jsonl");
        std::fs::write(&outside_file, "secret").unwrap();

        let symlink_path = sessions_dir.join("escape.jsonl");
        symlink(&outside_file, &symlink_path).unwrap();

        let err = delete_artifact_relative_path("sessions/escape.jsonl").unwrap_err();
        assert!(err.to_string().contains("sessions cache root"));
        assert!(outside_file.exists());

        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    #[serial]
    fn filesystem_scan_finds_brain_and_conversation_candidates() {
        let temp_dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        let root = temp_dir.path().join(".gemini/antigravity");
        std::fs::create_dir_all(root.join("brain/session-a")).unwrap();
        std::fs::create_dir_all(root.join("brain/session-b")).unwrap();
        std::fs::create_dir_all(root.join("conversations")).unwrap();
        std::fs::write(root.join("conversations/session-c.pb"), b"pb").unwrap();

        let candidates = scan_filesystem_session_candidates().unwrap();
        let ids: Vec<String> = candidates
            .into_iter()
            .map(|candidate| candidate.session_id)
            .collect();
        assert!(ids.contains(&"session-a".to_string()));
        assert!(ids.contains(&"session-b".to_string()));
        assert!(ids.contains(&"session-c".to_string()));

        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn merge_export_candidates_keeps_summary_filesystem_and_manifest_union() {
        let manifest = sample_manifest();
        let summaries = vec![TrajectorySummary {
            session_id: "session-2".to_string(),
            last_modified_ms: Some(200),
            step_count: Some(3),
            connection_fingerprint: "pid:2:port:2222".to_string(),
        }];
        let filesystem = vec![SessionCandidate {
            session_id: "session-3".to_string(),
            last_modified_ms: Some(300),
            artifact_path: None,
        }];

        let merged = merge_export_candidates(&manifest, &summaries, &filesystem);
        let ids: Vec<String> = merged
            .into_iter()
            .map(|candidate| candidate.session_id)
            .collect();
        assert!(ids.contains(&"session-1".to_string()));
        assert!(ids.contains(&"session-2".to_string()));
        assert!(ids.contains(&"session-3".to_string()));
    }

    #[test]
    #[serial]
    fn manifest_round_trip_and_artifact_write() {
        let temp_dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        let manifest = sample_manifest();
        save_antigravity_manifest(&manifest).unwrap();
        let loaded = load_antigravity_manifest().unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.connections.len(), 1);

        let artifact_path = write_session_artifact("session/one", "{}\n").unwrap();
        assert!(artifact_path.exists());
        assert!(artifact_path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with("session-one-")));

        let cache_dir = get_antigravity_cache_dir().unwrap();
        let relative = artifact_path
            .strip_prefix(cache_dir)
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(delete_session_artifact(&relative).unwrap());
        assert!(!artifact_path.exists());

        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }

    use std::net::TcpListener;
    use std::thread;

    fn serve_once(body: Vec<u8>, headers_extra: &str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let header_owned = headers_extra.to_string();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\n{}Connection: close\r\n\r\n",
                header_owned
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&body);
        });
        port
    }

    #[test]
    fn rpc_request_rejects_oversized_content_length_body() {
        let port = serve_once(
            vec![b'a'; 32],
            &format!("Content-Length: {}\r\n", MAX_RPC_BODY_BYTES + 1),
        );
        let connection = AntigravityConnection {
            pid: 1,
            port,
            csrf_token: "abcdef0123456789abcdef0123456789".to_string(),
            fingerprint: format!("pid:1:port:{port}"),
        };
        let err = rpc_request(&connection, "X", &serde_json::json!({})).unwrap_err();
        assert!(
            err.to_string().contains("exceeds"),
            "expected cap error, got: {err:#}"
        );
    }

    #[test]
    fn read_chunked_body_rejects_oversized_accumulated_chunks() {
        let chunk_size = MAX_RPC_BODY_BYTES / 4 + 1;
        let mut body = Vec::new();
        for _ in 0..5 {
            body.extend_from_slice(format!("{:x}\r\n", chunk_size).as_bytes());
            body.extend(std::iter::repeat(b'a').take(chunk_size));
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(b"0\r\n\r\n");
        let port = serve_once(body, "Transfer-Encoding: chunked\r\n");
        let connection = AntigravityConnection {
            pid: 1,
            port,
            csrf_token: "abcdef0123456789abcdef0123456789".to_string(),
            fingerprint: format!("pid:1:port:{port}"),
        };
        let err = rpc_request(&connection, "X", &serde_json::json!({})).unwrap_err();
        assert!(
            err.to_string().contains("exceeds"),
            "expected cap error, got: {err:#}"
        );
    }

    #[test]
    fn contains_antigravity_marker_accepts_known_keys() {
        let v: Value = serde_json::json!({
            "trajectorySummaries": [{"cascadeId": "abc"}]
        });
        assert!(contains_antigravity_marker(&v));

        let nested: Value = serde_json::json!({
            "data": {"serverInfo": {"name": "x"}}
        });
        assert!(contains_antigravity_marker(&nested));
    }

    #[test]
    fn contains_antigravity_marker_rejects_html_and_arbitrary_json() {
        assert!(!response_contains_antigravity_marker(
            "<html><body>not json"
        ));
        assert!(!response_contains_antigravity_marker(r#"{"foo":"bar"}"#));
        assert!(!response_contains_antigravity_marker(r#"[]"#));
    }

    #[test]
    fn response_contains_antigravity_marker_accepts_real_shape() {
        let body = r#"{"trajectorySummaries":[{"cascadeId":"sess-1","stepCount":3}]}"#;
        assert!(response_contains_antigravity_marker(body));
    }

    #[test]
    #[serial]
    fn load_antigravity_manifest_rejects_newer_version() {
        let temp_dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        ensure_config_dir().unwrap();
        let cache_dir = get_antigravity_cache_dir().unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        let manifest_path = get_antigravity_manifest_path().unwrap();
        std::fs::write(
            &manifest_path,
            r#"{"version":2,"syncedAt":null,"connections":[],"sessions":[]}"#,
        )
        .unwrap();

        let err = load_antigravity_manifest().unwrap_err();
        assert!(err.to_string().contains("newer tokscale version"));

        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    #[serial]
    fn load_antigravity_manifest_treats_older_version_as_fresh_start() {
        let temp_dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        ensure_config_dir().unwrap();
        let cache_dir = get_antigravity_cache_dir().unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        let manifest_path = get_antigravity_manifest_path().unwrap();
        std::fs::write(
            &manifest_path,
            r#"{"version":0,"syncedAt":null,"connections":[],"sessions":[]}"#,
        )
        .unwrap();

        let manifest = load_antigravity_manifest().unwrap();
        assert_eq!(manifest.version, ANTIGRAVITY_MANIFEST_VERSION);
        assert!(manifest.sessions.is_empty());

        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    #[serial]
    fn load_antigravity_manifest_recovers_from_corrupted_json() {
        let temp_dir = tempfile::tempdir().unwrap();
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", temp_dir.path());

        ensure_config_dir().unwrap();
        let cache_dir = get_antigravity_cache_dir().unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        let manifest_path = get_antigravity_manifest_path().unwrap();
        std::fs::write(&manifest_path, "{ this is not valid json").unwrap();

        let manifest = load_antigravity_manifest().unwrap();
        assert_eq!(manifest.version, ANTIGRAVITY_MANIFEST_VERSION);
        assert!(manifest.sessions.is_empty());

        let parent = manifest_path.parent().unwrap();
        let backups: Vec<_> = std::fs::read_dir(parent)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("manifest.json.corrupt-")
            })
            .collect();
        assert_eq!(backups.len(), 1, "expected one backup file");

        match previous_home {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    #[serial]
    fn sync_lock_guard_blocks_when_self_pid_lock_present() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().to_path_buf();
        let lock_path = cache_dir.join("sync.lock");
        let pid = std::process::id();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        std::fs::write(&lock_path, format!("{pid} {now}")).unwrap();

        let err = SyncLockGuard::acquire(&cache_dir).unwrap_err();
        assert!(
            err.to_string()
                .contains("Another tokscale antigravity sync"),
            "got: {err:#}"
        );

        std::fs::remove_file(&lock_path).unwrap();
    }

    #[test]
    #[serial]
    fn sync_lock_guard_acquires_when_no_lock_present() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().to_path_buf();
        {
            let _guard = SyncLockGuard::acquire(&cache_dir).unwrap();
            assert!(cache_dir.join("sync.lock").exists());
        }
        assert!(
            !cache_dir.join("sync.lock").exists(),
            "guard drop should remove lock"
        );
    }

    #[test]
    #[serial]
    fn sync_lock_guard_overwrites_stale_lock() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().to_path_buf();
        let lock_path = cache_dir.join("sync.lock");
        let stale_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            .saturating_sub(SYNC_LOCK_STALE_SECS + 60);
        std::fs::write(&lock_path, format!("999999 {stale_ts}")).unwrap();

        let _guard = SyncLockGuard::acquire(&cache_dir).unwrap();
        assert!(lock_path.exists());
    }
}
