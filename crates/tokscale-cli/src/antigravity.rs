use crate::process_liveness::pid_is_alive;
use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_RPC_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_IDENTITY_PROBE_BYTES: usize = 4096;
const ANTIGRAVITY_MANIFEST_VERSION: i32 = 1;

/// Wall clock one `sync` may waste on failed trajectory-enrichment RPCs before
/// it stops attempting them at all. See [`TrajectoryEnrichmentBudget`].
const TRAJECTORY_ENRICHMENT_BUDGET: Duration = Duration::from_secs(60);

/// Consecutive enrichment failures on one connection before that connection is
/// written off for the rest of the sync. See [`TrajectoryEnrichmentBudget`].
const TRAJECTORY_ENRICHMENT_FAILURE_THRESHOLD: u32 = 2;

#[cfg(not(target_os = "windows"))]
static HTTPS_RPC_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

#[cfg(not(target_os = "windows"))]
static HTTPS_RPC_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

static RPC_TRANSPORT: OnceLock<Mutex<HashMap<u16, RpcTransport>>> = OnceLock::new();

/// Which transport an Antigravity RPC port has already answered on.
///
/// `rpc_request` tries plaintext first and falls back to TLS. A DesktopAgent
/// that only speaks TLS therefore pays the doomed plaintext leg — and its full
/// 10s read timeout, since a TLS listener given plaintext may simply wait for a
/// ClientHello that never comes — on *every* RPC. `sync` issues one RPC per
/// session (`try_fetch_session_artifact`), so that cost is multiplied by the
/// user's whole history. `probe_heartbeat` already learns the answer during
/// discovery; this is where that answer is kept instead of thrown away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RpcTransport {
    PlainHttp,
    Https,
}

fn rpc_transport_cache() -> &'static Mutex<HashMap<u16, RpcTransport>> {
    RPC_TRANSPORT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A poisoned lock only means some other thread panicked mid-update; the cache
/// is an optimization, so treat that as "unknown" rather than propagating.
fn cached_rpc_transport(port: u16) -> Option<RpcTransport> {
    rpc_transport_cache().lock().ok()?.get(&port).copied()
}

fn remember_rpc_transport(port: u16, transport: RpcTransport) {
    if let Ok(mut cache) = rpc_transport_cache().lock() {
        cache.insert(port, transport);
    }
}

fn home_dir() -> Result<PathBuf> {
    crate::paths::home_dir().context("Could not determine home directory")
}

fn antigravity_data_roots() -> Result<Vec<PathBuf>> {
    let gemini_dir = home_dir()?.join(".gemini");
    let mut roots = Vec::new();
    for name in ["antigravity-ide", "antigravity", "antigravity-backup"] {
        let root = gemini_dir.join(name);
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    Ok(roots)
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

    // One budget for the whole sync: the enrichment RPCs it bounds all run
    // under the exclusive cache lock acquired above.
    let mut enrichment = TrajectoryEnrichmentBudget::new();

    for candidate in &export_candidates {
        if let Some(summary) = find_summary_for_candidate(&summaries, &candidate.session_id) {
            if let Some(artifact) = fetch_session_artifact(summary, &connections, &mut enrichment)?
            {
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

        if let Some(entry) = fetch_historical_session_artifact(
            &candidate.session_id,
            &connections,
            candidate,
            &mut enrichment,
        )? {
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
    let cache_lock = CacheOperationLockGuard::acquire(&cache_dir, "Antigravity cache operation")?;
    if cache_dir.exists() {
        // Hold the same legacy-visible lock as sync while purging. An old
        // binary does not know the parent lock, so a pre-delete PID check
        // alone leaves a TOCTOU window for it to create `sync.lock`.
        let _sync_lock = SyncLockGuard::acquire_with_cache_lock(&cache_dir, cache_lock)?;
        for entry in fs::read_dir(&cache_dir)? {
            let entry = entry?;
            if entry.file_name() == "sync.lock" {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
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
    _cache_lock: CacheOperationLockGuard,
    _os_file: std::fs::File,
    path: PathBuf,
    record: String,
}

/// Serializes operations that create or remove the cache directory itself.
///
/// `sync.lock` lives inside that directory, so it cannot protect its own
/// inode from `purge-cache`: removing the directory unlinks a held lock while
/// a later sync creates and locks a replacement. Keep this lock beside the
/// cache instead, where purge never removes it.
#[derive(Debug)]
struct CacheOperationLockGuard {
    _file: std::fs::File,
}

impl CacheOperationLockGuard {
    fn acquire(cache_dir: &Path, operation: &str) -> Result<Self> {
        let lock_path = cache_dir.with_extension("lock");
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create Antigravity lock directory at {}",
                    parent.display()
                )
            })?;
        }

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| {
                format!(
                    "Failed to open Antigravity cache operation lock at {}",
                    lock_path.display()
                )
            })?;

        match file.try_lock_exclusive() {
            Ok(()) => Ok(Self { _file: file }),
            Err(err) if crate::commands::autosubmit::is_lock_contention(&err) => {
                anyhow::bail!("Another tokscale {operation} is in progress; aborting")
            }
            Err(err) => Err(anyhow::Error::new(err)
                .context("Failed to acquire Antigravity cache operation lock")),
        }
    }
}

impl SyncLockGuard {
    /// Take exclusive ownership of the Antigravity sync for as long as the
    /// returned guard lives.
    ///
    /// Ownership is the kernel's exclusive lock on the file, not the bytes
    /// inside it. The previous protocol created the lock file, then wrote its
    /// pid, then probed that pid's liveness to decide whether to unlink and
    /// retry — a read-decide-unlink sequence that is not atomic. Two
    /// contenders could find the same dead owner and both proceed, a
    /// contender arriving before the pid was written read an empty file and
    /// evicted a live owner, and a recycled pid could make a stranger look
    /// like the owner. The companion OS lock prevents those races between new
    /// binaries, while the visible PID record remains readable by older
    /// binaries during a rolling upgrade.
    ///
    /// On normal release the guard removes only its own visible record while
    /// holding the companion lock. After a crash, a surviving visible record
    /// fails closed and requires the documented user-mediated recovery.
    fn acquire(cache_dir: &Path) -> Result<Self> {
        let cache_lock = CacheOperationLockGuard::acquire(cache_dir, "antigravity sync")?;
        Self::acquire_with_cache_lock(cache_dir, cache_lock)
    }

    fn acquire_with_cache_lock(
        cache_dir: &Path,
        cache_lock: CacheOperationLockGuard,
    ) -> Result<Self> {
        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir).with_context(|| {
                format!(
                    "Failed to create Antigravity cache directory at {}",
                    cache_dir.display()
                )
            })?;
        }

        // Keep the OS-held exclusion out of the legacy PID file. On Windows,
        // locking `sync.lock` can make legacy readers fail and unlink it.
        let os_path = cache_dir.join("sync.os.lock");
        let lock_path = cache_dir.join("sync.lock");
        let os_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&os_path)
            .with_context(|| {
                format!(
                    "Failed to open Antigravity OS sync lock at {}",
                    os_path.display()
                )
            })?;

        match os_file.try_lock_exclusive() {
            Ok(()) => {}
            Err(err) if crate::commands::autosubmit::is_lock_contention(&err) => {
                // Name the owner when the recorded pid is still alive. A pid
                // left by a crashed run would only mislead, and the lock is
                // already ours to wait on either way.
                let owner = read_sync_lock(&lock_path)
                    .filter(|(pid, _)| pid_is_alive(*pid))
                    .map(|(pid, _)| format!(" (pid {pid})"))
                    .unwrap_or_default();
                anyhow::bail!("Another tokscale antigravity sync is in progress{owner}; aborting");
            }
            Err(err) => {
                return Err(
                    anyhow::Error::new(err).context("Failed to acquire Antigravity sync lock")
                );
            }
        }

        // Serialize new-format publication before creating the legacy-visible
        // record. A contender that loses the OS lock has not created any PID
        // file to strand, and normal release removes the PID file before
        // releasing this companion lock.
        let record = publish_legacy_readable_lock(&lock_path)?;

        Ok(SyncLockGuard {
            _cache_lock: cache_lock,
            _os_file: os_file,
            path: lock_path,
            record,
        })
    }
}

/// Atomically publishes a complete record that old binaries recognize before
/// any new binary attempts the OS lock. Existing paths are never reclaimed:
/// a legacy process can replace them between observation and unlink.
fn publish_legacy_readable_lock(lock_path: &Path) -> Result<String> {
    if lock_path.exists() {
        return Err(existing_sync_lock_error(lock_path));
    }

    let temp_path = lock_path.with_extension(format!(
        "lock.{}.{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let mut temp = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .with_context(|| {
            format!(
                "Failed to prepare Antigravity sync lock at {}",
                temp_path.display()
            )
        })?;
    writeln!(
        temp,
        "{} {}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    )?;
    drop(temp);
    let record = std::fs::read_to_string(&temp_path)?;

    let published = fs::hard_link(&temp_path, lock_path);
    let _ = fs::remove_file(&temp_path);
    match published {
        Ok(()) => Ok(record),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let owner = read_sync_lock(lock_path)
                .filter(|(pid, _)| pid_is_alive(*pid))
                .map(|(pid, _)| format!(" (pid {pid})"))
                .unwrap_or_default();
            anyhow::bail!("Another tokscale antigravity sync is in progress{owner}; aborting")
        }
        Err(err) => {
            Err(anyhow::Error::new(err)
                .context("Failed to publish Antigravity sync lock atomically"))
        }
    }
}

fn existing_sync_lock_error(lock_path: &Path) -> anyhow::Error {
    if let Some((pid, _)) = read_sync_lock(lock_path).filter(|(pid, _)| pid_is_alive(*pid)) {
        anyhow::anyhow!(
            "Another tokscale Antigravity sync may be in progress (pid {pid}); do not remove '{}' until that process has stopped. If it has stopped, remove '{}' and retry.",
            lock_path.display(),
            lock_path.display()
        )
    } else {
        anyhow::anyhow!(
            "Antigravity sync lock at '{}' already exists. To avoid overlapping a possible active sync during a rolling upgrade, tokscale will not replace it automatically. Confirm no tokscale Antigravity sync is running, then remove '{}' and retry.",
            lock_path.display(),
            lock_path.display()
        )
    }
}

impl Drop for SyncLockGuard {
    fn drop(&mut self) {
        // The companion OS lock remains held while deleting. New-format
        // contenders cannot publish until this record has gone, and the
        // comparison prevents deleting an unexpected successor.
        if std::fs::read_to_string(&self.path).ok().as_deref() == Some(&self.record) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn read_sync_lock(path: &Path) -> Option<(u32, u64)> {
    let contents = std::fs::read_to_string(path).ok()?;
    let mut parts = contents.split_whitespace();
    let pid = parts.next()?.parse::<u32>().ok()?;
    let timestamp = parts.next()?.parse::<u64>().ok()?;
    Some((pid, timestamp))
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

/// Rewrite `\` to `/` so a stored path parses the same way on every platform.
///
/// `to_relative_artifact_path` renders with `Path::to_string_lossy`, so a
/// manifest written on Windows records `sessions\<id>.jsonl`. Normalising here
/// also tightens the Unix side: `Path` does not treat `\` as a separator
/// there, so `..\..\escape.jsonl` would otherwise reach the traversal check
/// below as one innocuous file name.
///
/// Safe against false positives because `sanitize_session_id` replaces every
/// character outside `[A-Za-z0-9._-]` with `-`, so a generated artifact name
/// can never legitimately contain a backslash.
///
/// `stale_relative_paths` keys its comparison through this same function.
/// Normalising only at deletion made the two disagree about which entries name
/// the same file, and deletion runs after the manifest has been written.
fn normalize_artifact_path_separators(relative_path: &str) -> String {
    relative_path.replace('\\', "/")
}

fn resolve_cache_relative_artifact_path(relative_path: &str) -> Result<PathBuf> {
    let normalized = normalize_artifact_path_separators(relative_path);
    let relative = Path::new(&normalized);
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
    #[cfg(target_os = "windows")]
    {
        return detect_windows_process_candidates();
    }

    #[cfg(not(target_os = "windows"))]
    {
        detect_unix_process_candidates()
    }
}

#[cfg(not(target_os = "windows"))]
fn detect_unix_process_candidates() -> Result<Vec<ProcessCandidate>> {
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

#[cfg(target_os = "windows")]
fn detect_windows_process_candidates() -> Result<Vec<ProcessCandidate>> {
    let script = "$ErrorActionPreference = 'Stop'; Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,ExecutablePath,CommandLine | ConvertTo-Json -Compress";
    let output = run_windows_powershell(script)?;
    if output.trim().is_empty() {
        anyhow::bail!(
            "Windows process discovery returned no data; cannot discover Antigravity language servers"
        );
    }
    parse_windows_process_candidates(&output)
}

#[cfg(any(test, target_os = "windows"))]
fn parse_windows_process_candidates(output: &str) -> Result<Vec<ProcessCandidate>> {
    let value: Value = serde_json::from_str(output.trim())
        .context("Failed to parse Windows process discovery JSON")?;
    let items: Vec<&Value> = match &value {
        Value::Array(values) => values.iter().collect(),
        Value::Object(_) => vec![&value],
        Value::Null => Vec::new(),
        _ => {
            anyhow::bail!("Windows process discovery JSON must be an object or array");
        }
    };
    let mut candidates = Vec::new();

    for item in items {
        let Some(pid) = item
            .get("ProcessId")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
        else {
            continue;
        };
        let ppid = item
            .get("ParentProcessId")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);
        let command = item
            .get("CommandLine")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !is_antigravity_process(command) {
            continue;
        }

        let executable_path = item.get("ExecutablePath").and_then(Value::as_str);
        if !windows_candidate_executable_ok(executable_path, command) {
            continue;
        }

        let Some(csrf_token) = extract_csrf_token(command) else {
            continue;
        };
        let declared_port = extract_declared_port(command);

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

#[cfg(any(test, target_os = "windows"))]
fn windows_candidate_executable_ok(executable_path: Option<&str>, command: &str) -> bool {
    executable_path
        .filter(|path| !path.trim().is_empty())
        .map(executable_path_looks_antigravity)
        .unwrap_or_else(|| command_line_executable_looks_antigravity(command))
}

#[cfg(any(test, target_os = "windows"))]
fn executable_path_looks_antigravity(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("antigravity") || lower.contains("language_server")
}

#[cfg(any(test, target_os = "windows"))]
fn command_line_executable_looks_antigravity(command: &str) -> bool {
    let first = command
        .trim_start()
        .strip_prefix('"')
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| command.split_whitespace().next().unwrap_or_default());
    executable_path_looks_antigravity(first)
}

fn is_antigravity_process(command: &str) -> bool {
    let lower = command.to_lowercase();
    (lower.contains("language_server")
        && (lower.contains("antigravity") || lower.contains("--app_data_dir antigravity")))
        || lower.contains("/antigravity/")
        || lower.contains("\\antigravity\\")
}

#[cfg(not(target_os = "windows"))]
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
        None
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
    #[cfg(target_os = "windows")]
    {
        return find_windows_listening_ports(pid);
    }

    #[cfg(not(target_os = "windows"))]
    {
        find_unix_listening_ports(pid)
    }
}

#[cfg(not(target_os = "windows"))]
fn find_unix_listening_ports(pid: u32) -> Result<Vec<u16>> {
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

#[cfg(target_os = "windows")]
fn find_windows_listening_ports(pid: u32) -> Result<Vec<u16>> {
    let output = run_command_required("netstat", &["-ano", "-p", "TCP"])
        .context("Failed to discover Windows TCP listeners with netstat")?;
    Ok(parse_windows_netstat_ports(&output, pid))
}

#[cfg(any(test, target_os = "windows"))]
fn parse_windows_netstat_ports(output: &str, pid: u32) -> Vec<u16> {
    let mut ports = Vec::new();
    let pid_text = pid.to_string();

    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }
        if !parts[0].eq_ignore_ascii_case("TCP") {
            continue;
        }
        if !parts[3].eq_ignore_ascii_case("LISTENING") || parts[4] != pid_text {
            continue;
        }
        if let Some(port) = parse_port_from_windows_address(parts[1]) {
            ports.push(port);
        }
    }

    ports.sort_unstable();
    ports.dedup();
    ports
}

#[cfg(any(test, target_os = "windows"))]
fn parse_port_from_windows_address(address: &str) -> Option<u16> {
    let (_, port) = address.rsplit_once(':')?;
    port.parse::<u16>().ok()
}

#[cfg(not(target_os = "windows"))]
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

#[cfg(not(target_os = "windows"))]
fn parse_ports(output: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in output.lines() {
        if let Some(port) = parse_port_from_line(line) {
            ports.push(port);
        }
    }
    ports
}

#[cfg(not(target_os = "windows"))]
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
    // Discovery is the one place that legitimately has to try both. Record the
    // winner so the per-session RPCs that follow do not re-derive it.
    if probe_plain_http_heartbeat(port, csrf_token) {
        remember_rpc_transport(port, RpcTransport::PlainHttp);
        return true;
    }

    if probe_https_heartbeat(port, csrf_token) {
        remember_rpc_transport(port, RpcTransport::Https);
        return true;
    }

    false
}

fn probe_https_heartbeat(port: u16, csrf_token: &str) -> bool {
    let connection = AntigravityConnection {
        pid: 0,
        port,
        csrf_token: csrf_token.to_string(),
        fingerprint: format!("port:{port}"),
    };
    let body = serde_json::json!({ "uuid": "00000000-0000-0000-0000-000000000000" });
    let Ok(response) = https_rpc_request(&connection, "Heartbeat", &body) else {
        return false;
    };
    if !heartbeat_value_looks_well_formed(&response) {
        return false;
    }

    true
}

fn probe_plain_http_heartbeat(port: u16, csrf_token: &str) -> bool {
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

fn heartbeat_value_looks_well_formed(value: &Value) -> bool {
    value.is_object() || value.is_array()
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
    if let Some(body) = plain_http_identity_probe_request(port, csrf_token, method) {
        return Some(body);
    }

    https_identity_probe_request(port, csrf_token, method)
}

fn https_identity_probe_request(port: u16, csrf_token: &str, method: &str) -> Option<String> {
    let connection = AntigravityConnection {
        pid: 0,
        port,
        csrf_token: csrf_token.to_string(),
        fingerprint: format!("port:{port}"),
    };
    let response = https_rpc_request(&connection, method, &serde_json::json!({})).ok()?;
    serde_json::to_string(&response).ok()
}

fn plain_http_identity_probe_request(port: u16, csrf_token: &str, method: &str) -> Option<String> {
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

    let status_ok = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .is_some_and(|status| status == 200);

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() {
            return None;
        }
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

    if !status_ok {
        return None;
    }

    // RFC 7230 §3.3.3: when Transfer-Encoding is present, Content-Length MUST
    // be ignored. Check chunked first so a server that sets both headers is
    // decoded correctly.
    if chunked {
        return read_chunked_body_prefix(&mut reader, MAX_IDENTITY_PROBE_BYTES).ok();
    }

    if let Some(length) = content_length {
        let read_length = length.min(MAX_IDENTITY_PROBE_BYTES);
        let mut bytes = vec![0_u8; read_length];
        reader.read_exact(&mut bytes).ok()?;
        return String::from_utf8(bytes).ok();
    }

    let mut buffer = String::new();
    reader
        .by_ref()
        .take(MAX_IDENTITY_PROBE_BYTES as u64)
        .read_to_string(&mut buffer)
        .ok()?;
    Some(buffer)
}

fn response_contains_antigravity_marker(body: &str) -> bool {
    let trimmed = body.trim_start();
    let json_start = trimmed.find(['{', '[']);
    let Some(idx) = json_start else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&trimmed[idx..]) else {
        return prefix_contains_antigravity_marker(&trimmed[idx..]);
    };
    contains_antigravity_marker(&value)
}

fn prefix_contains_antigravity_marker(body: &str) -> bool {
    let trimmed = body.trim_start();
    if !trimmed.starts_with(['{', '[']) {
        return false;
    }

    [
        "\"cascadeId\"",
        "\"cascadeTrajectories\"",
        "\"trajectorySummaries\"",
        "\"generatorMetadata\"",
        "\"serverInfo\"",
        "\"serverCapabilities\"",
    ]
    .iter()
    .any(|marker| {
        trimmed
            .split(marker)
            .skip(1)
            .any(|suffix| suffix.trim_start().starts_with(':'))
    })
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

#[cfg(not(target_os = "windows"))]
fn run_command(program: &str, args: &[&str]) -> Result<String> {
    let output = run_command_output(program, args)?;

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

#[cfg(target_os = "windows")]
fn run_command_required(program: &str, args: &[&str]) -> Result<String> {
    let output = run_command_output(program, args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "{} {} exited with status {}{}",
            program,
            args.join(" "),
            output.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(target_os = "windows")]
fn run_windows_powershell(script: &str) -> Result<String> {
    let args = [
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ];
    match run_command_required("powershell", &args) {
        Ok(output) => Ok(output),
        Err(err) if is_command_not_found(&err) => run_command_required("powershell.exe", &args),
        Err(err) => Err(err),
    }
}

fn run_command_output(program: &str, args: &[&str]) -> Result<std::process::Output> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("Failed to run {} {}", program, args.join(" ")))?;
    Ok(output)
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
    let mut candidates: HashMap<String, SessionCandidate> = HashMap::new();

    for root in antigravity_data_roots()? {
        let brain_dir = root.join("brain");
        let conversations_dir = root.join("conversations");

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
    budget: &mut TrajectoryEnrichmentBudget,
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

    if let Some(artifact) = fetch_session_artifact(&fallback_summary, connections, budget)? {
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
    // A port already known to speak TLS will never answer plaintext, so the
    // plaintext leg here is pure latency — see [`RpcTransport`]. Deliberately
    // no plaintext retry on failure: a listener does not change protocol
    // mid-run, and retrying would reintroduce exactly the cost being removed.
    if cached_rpc_transport(connection.port) == Some(RpcTransport::Https) {
        return https_rpc_request(connection, method, body)
            .with_context(|| format!("HTTPS RPC failed for Antigravity RPC {method}"));
    }

    match rpc_request_plain_http(connection, method, body) {
        Ok(value) => {
            remember_rpc_transport(connection.port, RpcTransport::PlainHttp);
            Ok(value)
        }
        Err(http_err) => {
            let value = https_rpc_request(connection, method, body).with_context(|| {
                format!(
                    "HTTP RPC failed ({http_err:#}); HTTPS fallback also failed for Antigravity RPC {method}"
                )
            })?;
            remember_rpc_transport(connection.port, RpcTransport::Https);
            Ok(value)
        }
    }
}

/// Argument vector for the Windows `curl.exe` RPC fallback.
///
/// Split out of the `cfg(windows)` caller so the two ordering rules it has to
/// satisfy are covered by the test suite on every host instead of only by the
/// Windows CI job.
///
/// `-q` has to be the first argument: curl applies `%USERPROFILE%\_curlrc`
/// (`~/.curlrc`) before it parses anything that comes later, so `-q` anywhere
/// else no longer suppresses it. An explicit `-K` is still honored after `-q`,
/// which is what keeps the CSRF header and the body on stdin.
///
/// `--noproxy "*"` bypasses proxy resolution for every host, including the
/// `HTTPS_PROXY` / `ALL_PROXY` environment variables curl would otherwise
/// obey. Without it a request aimed at the DesktopAgent on loopback can be
/// handed to a configured remote proxy together with the CSRF token and the
/// RPC body. The non-Windows path gets the same guarantee from `reqwest`'s
/// `.no_proxy()`.
#[cfg(any(target_os = "windows", test))]
fn windows_curl_rpc_args(url: &str) -> Vec<&str> {
    vec![
        // Must stay first, see above.
        "-q",
        "--noproxy",
        "*",
        "-k",
        "-sS",
        "--http1.1",
        "--max-time",
        "10",
        "-X",
        "POST",
        url,
        "-H",
        "Content-Type: application/json",
        "-H",
        "Connect-Protocol-Version: 1",
        "-K",
        "-",
        "--write-out",
        "\\n%{http_code}",
    ]
}

/// Read at most `max_bytes + 1` bytes out of `reader`.
///
/// The ceiling is applied while the bytes are being read rather than to the
/// finished buffer, so a writer that keeps producing can never make this
/// allocate past the cap. The one byte of headroom is what separates a
/// response that exactly fills the cap from one that runs past it: the caller
/// rejects anything longer than `max_bytes`.
#[cfg(any(target_os = "windows", test))]
fn read_curl_stdout_with_cap<R: Read>(reader: R, max_bytes: usize) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    reader.take(max_bytes as u64 + 1).read_to_end(&mut body)?;
    Ok(body)
}

/// Keep the first `max_bytes` of `reader`, then read the remainder to EOF and
/// throw it away.
///
/// Both halves are load-bearing. The buffer is bounded because this text only
/// ever reaches a diagnostic message, so a child that talks forever must not
/// be able to grow it. The drain is unconditional because a pipe nobody reads
/// fills up and blocks its writer: with the caller reading the child's stdout
/// to EOF, a child parked on a full stderr buffer would leave both pipes stuck
/// for good. Read errors are swallowed on purpose — whatever was captured
/// before the error is still the most useful thing to report.
#[cfg(any(target_os = "windows", test))]
fn drain_curl_stderr_with_cap<R: Read>(mut reader: R, max_bytes: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    let _ = reader
        .by_ref()
        .take(max_bytes as u64)
        .read_to_end(&mut kept);
    let _ = std::io::copy(&mut reader, &mut std::io::sink());
    kept
}

fn https_rpc_request(
    connection: &AntigravityConnection,
    method: &str,
    body: &Value,
) -> Result<Value> {
    // On Windows, the reqwest (rustls) request to the local DesktopAgent
    // stalls until the timeout, while the in-box curl.exe (SChannel) completes
    // the same request instantly (#1129). The mechanism is unverified, so the
    // request is handed to curl.exe instead of guessing at a rustls-side fix.
    // `--http1.1` is a defensive pin: unlike this workspace's reqwest build
    // (no `http2` feature), curl can negotiate h2 via ALPN.
    #[cfg(target_os = "windows")]
    {
        // curl config-file values are enclosed in double quotes, inside which
        // backslashes and double quotes must be backslash-escaped.
        fn curl_config_quote(value: &str) -> String {
            let mut quoted = String::with_capacity(value.len() + 2);
            quoted.push('"');
            for ch in value.chars() {
                if matches!(ch, '\\' | '"') {
                    quoted.push('\\');
                }
                quoted.push(ch);
            }
            quoted.push('"');
            quoted
        }

        let url = format!(
            "https://127.0.0.1:{}/exa.language_server_pb.LanguageServerService/{}",
            connection.port, method
        );
        let body_str = serde_json::to_string(body)?;

        // curl.exe is invoked by absolute path so a PATH lookup cannot be
        // shadowed by a user-writable directory earlier in the search order.
        // That is the whole of what the absolute path buys. The root still
        // comes from the inherited `SystemRoot`, so a parent process that
        // controls this process' environment can still point
        // `System32\curl.exe` at a binary of its choosing, which then
        // receives the CSRF token and the RPC body on stdin. Dropping that
        // assumption means resolving the system directory through
        // `GetSystemDirectoryW`, which needs a Win32 binding this workspace
        // does not depend on; until then the environment tokscale is launched
        // with is trusted.
        let system32_curl = std::env::var_os("SystemRoot")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("C:\\Windows"))
            .join("System32\\curl.exe");

        // curl.exe stderr only ever carries a short diagnostic (`-sS` drops the
        // progress meter but keeps errors), so this is already generous.
        const MAX_CURL_STDERR_BYTES: usize = 64 * 1024;

        let mut child = std::process::Command::new(&system32_curl)
            .args(windows_curl_rpc_args(&url))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| "Failed to execute curl.exe for Windows RPC fallback")?;

        let child_stdout = child
            .stdout
            .take()
            .context("curl.exe stdout unavailable for Windows RPC fallback")?;
        let child_stderr = child
            .stderr
            .take()
            .context("curl.exe stderr unavailable for Windows RPC fallback")?;

        // stderr gets its own thread, started before anything is written to
        // stdin, and that thread reads it to EOF. Reading one pipe to EOF on
        // this thread while the child sits blocked on a full buffer in the
        // other is the classic two-pipe deadlock, so neither pipe is ever left
        // unattended: stdout is read here, stderr is read there, and both keep
        // moving no matter what the child does with the other one. Only the
        // first MAX_CURL_STDERR_BYTES are retained; the rest is discarded as
        // it arrives instead of being buffered.
        let stderr_drain = std::thread::spawn(move || {
            drain_curl_stderr_with_cap(child_stderr, MAX_CURL_STDERR_BYTES)
        });

        // The CSRF token and body ride stdin (`-K -`) rather than argv, which
        // any same-user process can read.
        let config = format!(
            "header = {}\ndata = {}\n",
            curl_config_quote(&format!("X-Codeium-Csrf-Token: {}", connection.csrf_token)),
            curl_config_quote(&body_str),
        );
        child
            .stdin
            .take()
            .context("curl.exe stdin unavailable for Windows RPC fallback")?
            .write_all(config.as_bytes())
            .context("Failed to write curl.exe config for Windows RPC fallback")?;

        // The cap is enforced while curl is still transferring, not once the
        // whole response is already in memory: at most MAX_RPC_BODY_BYTES + 1
        // bytes are ever held. Loopback is fast and `--max-time` leaves a 10s
        // window, so a DesktopAgent that streams far more than the cap would
        // otherwise be allowed to allocate all of it and only then be
        // rejected. Here it is cut off at the ceiling instead.
        let stdout_bytes = match read_curl_stdout_with_cap(child_stdout, MAX_RPC_BODY_BYTES) {
            Ok(bytes) if bytes.len() <= MAX_RPC_BODY_BYTES => bytes,
            outcome => {
                // Either the ceiling was blown or the pipe read failed. Both
                // end the transfer: kill curl so it stops streaming into a
                // pipe nobody is reading, reap it, and only then join the
                // drain thread, which ends as soon as the child's stderr
                // handle is gone.
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_drain.join();
                let bytes =
                    outcome.context("Failed to read curl.exe response for Windows RPC fallback")?;
                anyhow::bail!(
                    "Antigravity RPC body of {} bytes exceeds {MAX_RPC_BODY_BYTES} cap",
                    bytes.len()
                );
            }
        };

        let status = child
            .wait()
            .with_context(|| "Failed to execute curl.exe for Windows RPC fallback")?;
        let stderr_bytes = stderr_drain.join().unwrap_or_default();

        if !status.success() {
            anyhow::bail!(
                "Windows curl.exe RPC fallback failed (exit code {}): {}",
                status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "unknown".into()),
                String::from_utf8_lossy(&stderr_bytes)
            );
        }

        let stdout = String::from_utf8_lossy(&stdout_bytes);
        let (response_body, status_line) = stdout.rsplit_once('\n').with_context(|| {
            format!("curl.exe returned no HTTP status for Antigravity RPC {method}")
        })?;
        let status: u16 = status_line.trim().parse().with_context(|| {
            format!(
                "curl.exe returned unparseable HTTP status {:?} for Antigravity RPC {method}",
                status_line.trim()
            )
        })?;
        if !(200..300).contains(&status) {
            anyhow::bail!(
                "Antigravity HTTPS RPC {} failed with status {}: {}",
                method,
                status,
                response_body
            );
        }

        serde_json::from_str(response_body).with_context(|| {
            format!("Failed to parse Antigravity RPC {method} response from curl.exe")
        })
    }

    #[cfg(not(target_os = "windows"))]
    {
        // `block_on` panics with "Cannot start a runtime from within a
        // runtime" when the calling thread already has a Tokio runtime
        // context entered (#1264): the TUI usage refresh and `tokscale
        // usage` reach this function while `has_credentials`'s own
        // current-thread runtime is mid-`block_on`, because
        // `discover_port` probes the IDE's language server
        // synchronously. A dedicated OS thread never carries a runtime
        // context, so the nested-runtime panic is structurally
        // impossible for every caller; the scope (not
        // `std::thread::spawn`) is what lets the request keep borrowing
        // `connection`/`method`/`body`. The ~microsecond spawn cost is
        // irrelevant next to a loopback RPC with a 10s timeout, and a
        // worker panic now degrades to `Err` instead of unwinding into
        // whatever thread called us.
        let joined: std::result::Result<Result<Value>, Box<dyn std::any::Any + Send>> =
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        antigravity_https_runtime()
                            .block_on(https_rpc_once(connection, method, body))
                    })
                    .join()
            });
        joined.unwrap_or_else(|_| {
            Err(anyhow::anyhow!(
                "Antigravity HTTPS RPC {method} worker thread panicked"
            ))
        })
    }
}

/// The HTTPS request future for [`https_rpc_request`], named so the spawned
/// worker above stays a one-liner: nested four levels deep, the unbreakable
/// URL literal overflowed rustfmt's max_width and forced a hanging layout
/// that reads as broken indentation.
#[cfg(not(target_os = "windows"))]
async fn https_rpc_once(
    connection: &AntigravityConnection,
    method: &str,
    body: &Value,
) -> Result<Value> {
    let url = format!(
        "https://127.0.0.1:{}/exa.language_server_pb.LanguageServerService/{}",
        connection.port, method
    );
    let response = antigravity_https_client()
        .post(url)
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("X-Codeium-Csrf-Token", &connection.csrf_token)
        .json(body)
        .send()
        .await?;
    let status = response.status();
    let response_body = read_reqwest_response_with_cap(response, MAX_RPC_BODY_BYTES).await?;
    if !status.is_success() {
        anyhow::bail!(
            "Antigravity HTTPS RPC {} failed with status {}: {}",
            method,
            status,
            response_body
        );
    }
    Ok(serde_json::from_str(&response_body)?)
}

#[cfg(not(target_os = "windows"))]
fn antigravity_https_runtime() -> &'static tokio::runtime::Runtime {
    HTTPS_RPC_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create Antigravity HTTPS RPC runtime")
    })
}

#[cfg(not(target_os = "windows"))]
fn antigravity_https_client() -> &'static reqwest::Client {
    HTTPS_RPC_CLIENT.get_or_init(|| {
        tokscale_core::http::client_builder()
            // Defensive only — this is NOT what fixed any reported hang, and the
            // earlier claim that it was (#1127) was wrong. `reqwest` is built
            // `default-features = false` without `http2`, so neither it nor
            // `hyper` links `h2` at all: this client never offers h2 over ALPN
            // and has no HTTP/2 implementation to multiplex with. The pin only
            // starts meaning anything if someone enables that feature later, and
            // it stays correct if they do, because `rpc_request` issues a single
            // unary Connect POST over loopback with nothing to multiplex.
            .http1_only()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to create Antigravity HTTPS RPC client")
    })
}

/// Reads at most `max_body_bytes` of a loopback RPC response.
///
/// `Content-Length` is consulted first so an oversized body is refused before a
/// byte of it is read, but it is never the only check: the header is optional
/// and a server is free to understate it, so the loop enforces the same ceiling
/// on what actually arrives.
///
/// Shared with the `/usage` quota provider, which reaches the same language
/// server over plain HTTP and needs the same ceiling for the same reason.
pub(crate) async fn read_reqwest_response_with_cap(
    mut response: reqwest::Response,
    max_body_bytes: usize,
) -> Result<String> {
    if let Some(length) = response.content_length() {
        if length > max_body_bytes as u64 {
            anyhow::bail!("Antigravity RPC body of {length} bytes exceeds {max_body_bytes} cap");
        }
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > max_body_bytes {
            anyhow::bail!(
                "Antigravity RPC body of {} bytes exceeds {} cap",
                body.len().saturating_add(chunk.len()),
                max_body_bytes
            );
        }
        body.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8(body)?)
}

fn rpc_request_plain_http(
    connection: &AntigravityConnection,
    method: &str,
    body: &Value,
) -> Result<Value> {
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

    let response_body = if chunked {
        read_chunked_body(&mut reader)?
    } else if let Some(length) = content_length {
        if length > MAX_RPC_BODY_BYTES {
            anyhow::bail!(
                "Antigravity RPC body of {length} bytes exceeds {MAX_RPC_BODY_BYTES} cap"
            );
        }
        let mut bytes = vec![0_u8; length];
        reader.read_exact(&mut bytes)?;
        String::from_utf8(bytes)?
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
    read_chunked_body_with_cap(reader, MAX_RPC_BODY_BYTES)
}

fn read_chunked_body_prefix(
    reader: &mut BufReader<TcpStream>,
    max_body_bytes: usize,
) -> Result<String> {
    let mut body = Vec::new();
    while body.len() < max_body_bytes {
        let mut size_line = String::new();
        reader.read_line(&mut size_line)?;
        let chunk_size = parse_chunk_size_line(&size_line)?;
        if chunk_size == 0 {
            break;
        }

        let remaining = max_body_bytes - body.len();
        let read_size = chunk_size.min(remaining);
        let mut chunk = vec![0_u8; read_size];
        reader.read_exact(&mut chunk)?;
        body.extend_from_slice(&chunk);

        if read_size < chunk_size {
            break;
        }

        let mut crlf = [0_u8; 2];
        reader.read_exact(&mut crlf)?;
    }

    Ok(String::from_utf8(body)?)
}

fn read_chunked_body_with_cap(
    reader: &mut BufReader<TcpStream>,
    max_body_bytes: usize,
) -> Result<String> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line)?;
        let chunk_size = parse_chunk_size_line(&size_line)?;
        if chunk_size == 0 {
            break;
        }

        if chunk_size > max_body_bytes || body.len().saturating_add(chunk_size) > max_body_bytes {
            anyhow::bail!(
                "Antigravity RPC body of {} bytes exceeds {} cap",
                body.len().saturating_add(chunk_size),
                max_body_bytes
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
    budget: &mut TrajectoryEnrichmentBudget,
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
        if let Some(artifact) = try_fetch_session_artifact(summary, connection, budget)? {
            return Ok(Some(artifact));
        }
    }

    Ok(None)
}

/// Budget and per-connection circuit breaker for the OPTIONAL
/// `GetCascadeTrajectory` enrichment.
///
/// `sync` holds its exclusive cache lock while it walks every session, and each
/// session whose metadata carries usage without timestamps costs one extra
/// synchronous RPC. That call is best-effort: when it fails the session simply
/// keeps its metadata-derived timestamp, which is what
/// [`try_fetch_session_artifact`] has always done per session. But "fails"
/// means the full 10s transport timeout when the endpoint stalls rather than
/// refuses, and only after that timeout is the error discarded. With metadata
/// still answering, a few hundred affected sessions therefore turn a sync into
/// tens of minutes while the held lock blocks every other sync and purge.
///
/// Two limits bound that, and both only ever count *wasted* time. A healthy
/// enrichment over loopback finishes in milliseconds and is never charged, so a
/// large well-behaved history keeps all of its timestamps no matter how many
/// sessions it has — degrading enrichment must never be the normal outcome, and
/// must never fail the sync.
///
/// - Per connection, [`TRAJECTORY_ENRICHMENT_FAILURE_THRESHOLD`] consecutive
///   failures write that connection off for the remainder of the sync. One
///   failure is indistinguishable from a session whose trajectory genuinely is
///   not available, so tripping on the first would give up enrichment for an
///   entire history because of one bad session. Two in a row on the same
///   connection means the endpoint is the problem, not the session; the extra
///   attempt costs at most one more transport timeout per connection.
/// - Across the whole sync, [`TRAJECTORY_ENRICHMENT_BUDGET`] of accumulated
///   failure time stops enrichment outright. At the 10s transport timeout that
///   is six stalled requests for the entire run, so the worst case this
///   optional work can add to a sync is about a minute regardless of how many
///   sessions or connections are involved. It is the backstop for what the
///   per-connection breaker cannot bound on its own: many connections, or one
///   that alternates between answering and stalling so its consecutive-failure
///   count keeps resetting.
#[derive(Debug)]
struct TrajectoryEnrichmentBudget {
    budget: Duration,
    failure_threshold: u32,
    wasted: Duration,
    consecutive_failures: HashMap<String, u32>,
}

impl TrajectoryEnrichmentBudget {
    fn new() -> Self {
        Self::with_limits(
            TRAJECTORY_ENRICHMENT_BUDGET,
            TRAJECTORY_ENRICHMENT_FAILURE_THRESHOLD,
        )
    }

    /// Split out so the budget decision is exercisable without spending real
    /// wall clock on stalled sockets.
    fn with_limits(budget: Duration, failure_threshold: u32) -> Self {
        Self {
            budget,
            failure_threshold,
            wasted: Duration::ZERO,
            consecutive_failures: HashMap::new(),
        }
    }

    fn should_attempt(&self, fingerprint: &str) -> bool {
        if self.wasted >= self.budget {
            return false;
        }
        self.consecutive_failures
            .get(fingerprint)
            .copied()
            .unwrap_or(0)
            < self.failure_threshold
    }

    /// A connection that answers is given its full allowance back: a single
    /// blip must not accumulate across an otherwise healthy sync.
    fn record_success(&mut self, fingerprint: &str) {
        self.consecutive_failures.remove(fingerprint);
    }

    fn record_failure(&mut self, fingerprint: &str, elapsed: Duration) {
        self.wasted = self.wasted.saturating_add(elapsed);
        *self
            .consecutive_failures
            .entry(fingerprint.to_string())
            .or_insert(0) += 1;
    }
}

/// Best-effort timestamps for sessions whose metadata does not carry its own.
///
/// Every failure mode ends in an empty map rather than an error: the caller
/// falls back to the metadata-derived timestamp, which is the same outcome a
/// failed RPC produced before this was bounded.
fn fetch_usage_timestamps(
    summary: &TrajectorySummary,
    connection: &AntigravityConnection,
    budget: &mut TrajectoryEnrichmentBudget,
) -> HashMap<String, i64> {
    if !budget.should_attempt(&connection.fingerprint) {
        return HashMap::new();
    }

    let started = Instant::now();
    match rpc_request(
        connection,
        "GetCascadeTrajectory",
        &serde_json::json!({ "cascadeId": summary.session_id }),
    ) {
        Ok(trajectory) => {
            budget.record_success(&connection.fingerprint);
            usage_timestamps_from_trajectory(&trajectory)
        }
        Err(_) => {
            budget.record_failure(&connection.fingerprint, started.elapsed());
            HashMap::new()
        }
    }
}

fn try_fetch_session_artifact(
    summary: &TrajectorySummary,
    connection: &AntigravityConnection,
    budget: &mut TrajectoryEnrichmentBudget,
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

    let usage_timestamps = if session_metadata_needs_trajectory_timestamps(&metadata) {
        fetch_usage_timestamps(summary, connection, budget)
    } else {
        HashMap::new()
    };
    let lines = normalize_session_metadata_with_timestamps(
        &summary.session_id,
        &metadata,
        &usage_timestamps,
    )?;
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

fn session_metadata_needs_trajectory_timestamps(metadata: &[Value]) -> bool {
    metadata.iter().any(|meta| {
        let chat_model = meta.get("chatModel").unwrap_or(meta);
        let chat_created_at = chat_model
            .get("chatStartMetadata")
            .and_then(|value| value.get("createdAt"))
            .and_then(parse_timestamp_value);

        chat_model
            .get("retryInfos")
            .and_then(Value::as_array)
            .map(|retry_infos| {
                retry_infos.iter().any(|retry| {
                    let usage = retry.get("usage").unwrap_or(retry);
                    let has_usage = to_safe_i64(usage.get("inputTokens")) > 0
                        || to_safe_i64(usage.get("outputTokens")) > 0
                        || to_safe_i64(usage.get("cacheReadTokens")) > 0
                        || to_safe_i64(usage.get("thinkingOutputTokens")) > 0;
                    let has_timestamp = usage
                        .get("createdAt")
                        .or_else(|| usage.get("timestamp"))
                        .and_then(parse_timestamp_value)
                        .or(chat_created_at)
                        .is_some();
                    has_usage && !has_timestamp
                })
            })
            .unwrap_or(false)
    })
}

fn usage_timestamp_key(prefix: &str, value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("{prefix}:{value}"))
}

fn insert_usage_timestamp(timestamps: &mut HashMap<String, i64>, usage: &Value, timestamp: i64) {
    for key in [
        usage_timestamp_key("response", usage.get("responseId")),
        usage_timestamp_key("message", usage.get("messageId")),
    ]
    .into_iter()
    .flatten()
    {
        timestamps
            .entry(key)
            .and_modify(|current| *current = (*current).min(timestamp))
            .or_insert(timestamp);
    }
}

fn usage_timestamps_from_trajectory(response: &Value) -> HashMap<String, i64> {
    let mut timestamps = HashMap::new();
    let trajectory = response.get("trajectory").unwrap_or(response);
    let Some(steps) = trajectory.get("steps").and_then(Value::as_array) else {
        return timestamps;
    };

    for step in steps {
        let Some(metadata) = step.get("metadata") else {
            continue;
        };
        let Some(usage) = metadata.get("modelUsage") else {
            continue;
        };
        let timestamp = [
            "createdAt",
            "startedAt",
            "completedAt",
            "finishedGeneratingAt",
            "viewableAt",
        ]
        .iter()
        .find_map(|field| metadata.get(*field).and_then(parse_timestamp_value));
        if let Some(timestamp) = timestamp {
            insert_usage_timestamp(&mut timestamps, usage, timestamp);
        }
    }

    timestamps
}

fn timestamp_for_usage(usage: &Value, timestamps: &HashMap<String, i64>) -> Option<i64> {
    [
        usage_timestamp_key("response", usage.get("responseId")),
        usage_timestamp_key("message", usage.get("messageId")),
    ]
    .into_iter()
    .flatten()
    .find_map(|key| timestamps.get(&key).copied())
}

#[cfg(test)]
fn normalize_session_metadata(session_id: &str, metadata: &[Value]) -> Result<Vec<String>> {
    normalize_session_metadata_with_timestamps(session_id, metadata, &HashMap::new())
}

fn normalize_session_metadata_with_timestamps(
    session_id: &str,
    metadata: &[Value],
    usage_timestamps: &HashMap<String, i64>,
) -> Result<Vec<String>> {
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
                    .or_else(|| timestamp_for_usage(usage, usage_timestamps))
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
    // Key both sides the way `delete_artifact_relative_path` resolves them.
    // `to_relative_artifact_path` renders with the native separator, so the
    // same artifact reads `sessions\<id>.jsonl` in a manifest written on
    // Windows and `sessions/<id>.jsonl` in one written on Unix. Comparing the
    // raw strings would retire the old spelling while deletion normalises it
    // back onto the file the new manifest still points at.
    let next_paths: std::collections::HashSet<String> = next
        .sessions
        .iter()
        .map(|session| normalize_artifact_path_separators(&session.artifact_path))
        .collect();

    previous
        .sessions
        .iter()
        .filter(|session| {
            !next_paths.contains(&normalize_artifact_path_separators(&session.artifact_path))
        })
        .map(|session| session.artifact_path.clone())
        .collect()
}

fn cleanup_stale_session_artifacts(
    previous: &AntigravityManifest,
    next: &AntigravityManifest,
) -> Result<()> {
    // Second gate, on the resolved path rather than the text. Cleanup runs
    // after `save_antigravity_manifest`, so a deletion here is unrecoverable
    // and unannounced: the manifest already advertises the artifact. Any future
    // spelling that `stale_relative_paths` fails to equate but resolution does
    // stops here instead of destroying live data.
    let live: std::collections::HashSet<PathBuf> = next
        .sessions
        .iter()
        .filter_map(|session| resolve_cache_relative_artifact_path(&session.artifact_path).ok())
        .collect();

    for relative_path in stale_relative_paths(previous, next) {
        let resolved = resolve_cache_relative_artifact_path(&relative_path)?;
        if live.contains(&resolved) {
            continue;
        }
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
    use std::ffi::OsString;

    /// RAII guard that redirects every tokscale config-dir lookup into a
    /// caller-supplied directory and restores the previous environment on
    /// drop (including on panic). Setting `HOME` alone is not sufficient on
    /// Linux CI runners because `dirs::config_dir()` honors
    /// `$XDG_CONFIG_HOME` first; tokscale's own `paths::get_config_dir()`
    /// short-circuits on `TOKSCALE_CONFIG_DIR`, which is the canonical
    /// hermetic override for tests.
    struct TestEnvGuard {
        prev_home: Option<OsString>,
        prev_config_dir: Option<OsString>,
    }

    impl TestEnvGuard {
        fn redirect_to(path: &Path) -> Self {
            let prev_home = std::env::var_os("HOME");
            let prev_config_dir = std::env::var_os("TOKSCALE_CONFIG_DIR");
            std::env::set_var("HOME", path);
            std::env::set_var("TOKSCALE_CONFIG_DIR", path);
            Self {
                prev_home,
                prev_config_dir,
            }
        }
    }

    impl Drop for TestEnvGuard {
        fn drop(&mut self) {
            match self.prev_home.take() {
                Some(home) => std::env::set_var("HOME", home),
                None => std::env::remove_var("HOME"),
            }
            match self.prev_config_dir.take() {
                Some(dir) => std::env::set_var("TOKSCALE_CONFIG_DIR", dir),
                None => std::env::remove_var("TOKSCALE_CONFIG_DIR"),
            }
        }
    }

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
    #[cfg(not(target_os = "windows"))]
    fn parse_port_from_line_reads_lsof_output() {
        assert_eq!(
            parse_port_from_line("proc 123 user 12u IPv4 0x0 0t0 TCP 127.0.0.1:41234 (LISTEN)"),
            Some(41234)
        );
    }

    #[test]
    fn windows_process_candidates_parse_powershell_json() {
        let output = r#"[
            {
                "ProcessId": 4242,
                "ParentProcessId": 100,
                "ExecutablePath": "C:\\Users\\me\\AppData\\Local\\Programs\\Antigravity\\language_server.exe",
                "CommandLine": "\"C:\\Users\\me\\AppData\\Local\\Programs\\Antigravity\\language_server.exe\" --app_data_dir antigravity --extension_server_port=49321 --csrf_token=abcdef0123456789abcdef0123456789"
            },
            {
                "ProcessId": 5000,
                "ParentProcessId": 100,
                "ExecutablePath": "C:\\Windows\\System32\\notepad.exe",
                "CommandLine": "notepad.exe --app_data_dir antigravity --extension_server_port=49322 --csrf_token=abcdef0123456789abcdef0123456789"
            }
        ]"#;

        let candidates = parse_windows_process_candidates(output).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].pid, 4242);
        assert_eq!(candidates[0].ppid, 100);
        assert_eq!(candidates[0].declared_port, Some(49321));
        assert_eq!(candidates[0].csrf_token, "abcdef0123456789abcdef0123456789");
    }

    #[test]
    fn windows_process_candidates_accept_single_json_object() {
        let output = r#"{
            "ProcessId": 4243,
            "ParentProcessId": 101,
            "ExecutablePath": null,
            "CommandLine": "\"C:\\Antigravity\\language_server.exe\" --extension_server_port 49323 --csrf_token abcdef0123456789abcdef0123456789"
        }"#;

        let candidates = parse_windows_process_candidates(output).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].pid, 4243);
        assert_eq!(candidates[0].declared_port, Some(49323));
    }

    #[test]
    fn windows_netstat_ports_filter_listeners_by_pid() {
        let output = r#"
  Proto  Local Address          Foreign Address        State           PID
  TCP    127.0.0.1:49321        0.0.0.0:0              LISTENING       4242
  TCP    [::1]:49322            [::]:0                 LISTENING       4242
  TCP    127.0.0.1:49323        0.0.0.0:0              ESTABLISHED     4242
  TCP    127.0.0.1:49324        0.0.0.0:0              LISTENING       5000
"#;

        assert_eq!(
            parse_windows_netstat_ports(output, 4242),
            vec![49321, 49322]
        );
    }

    #[test]
    fn windows_parse_port_from_address_ipv4() {
        assert_eq!(
            parse_port_from_windows_address("127.0.0.1:49321"),
            Some(49321)
        );
        assert_eq!(parse_port_from_windows_address("0.0.0.0:8080"), Some(8080));
    }

    #[test]
    fn windows_parse_port_from_address_ipv6() {
        assert_eq!(parse_port_from_windows_address("[::1]:49322"), Some(49322));
        assert_eq!(parse_port_from_windows_address("[::]:0"), Some(0));
    }

    #[test]
    fn windows_parse_port_from_address_invalid() {
        assert_eq!(parse_port_from_windows_address("no-colon"), None);
        assert_eq!(parse_port_from_windows_address("127.0.0.1:notaport"), None);
        assert_eq!(parse_port_from_windows_address(""), None);
    }

    #[test]
    fn windows_executable_path_looks_antigravity_matches_case_insensitively() {
        assert!(executable_path_looks_antigravity(
            r"C:\Users\me\AppData\Local\Programs\Antigravity\language_server.exe"
        ));
        assert!(executable_path_looks_antigravity(
            r"C:\ANTIGRAVITY\LANGUAGE_SERVER.EXE"
        ));
        assert!(executable_path_looks_antigravity(
            r"D:\tools\antigravity\app.exe"
        ));
        assert!(executable_path_looks_antigravity(
            r"C:\path\to\language_server.exe"
        ));
    }

    #[test]
    fn windows_executable_path_rejects_unrelated_programs() {
        assert!(!executable_path_looks_antigravity(
            r"C:\Windows\System32\notepad.exe"
        ));
        assert!(!executable_path_looks_antigravity(
            r"C:\Program Files\SomeApp\app.exe"
        ));
        assert!(!executable_path_looks_antigravity(""));
    }

    #[test]
    fn windows_command_line_executable_extracts_quoted_path() {
        assert!(command_line_executable_looks_antigravity(
            r#""C:\Antigravity\language_server.exe" --port=1234"#
        ));
        assert!(!command_line_executable_looks_antigravity(
            r#""C:\Windows\System32\notepad.exe" somefile.txt"#
        ));
    }

    #[test]
    fn windows_command_line_executable_extracts_unquoted_path() {
        assert!(command_line_executable_looks_antigravity(
            r"C:\Antigravity\language_server.exe --flag"
        ));
        assert!(!command_line_executable_looks_antigravity(
            r"notepad.exe file.txt"
        ));
    }

    #[test]
    fn windows_candidate_executable_ok_prefers_path_when_available() {
        assert!(windows_candidate_executable_ok(
            Some(r"C:\Programs\Antigravity\language_server.exe"),
            r#"notepad.exe --csrf_token=abc"#
        ));
        assert!(!windows_candidate_executable_ok(
            Some(r"C:\Windows\notepad.exe"),
            r#""C:\Antigravity\language_server.exe" --flag"#
        ));
    }

    #[test]
    fn windows_candidate_executable_ok_falls_back_to_command_line() {
        assert!(windows_candidate_executable_ok(
            None,
            r#""C:\Antigravity\language_server.exe" --csrf_token=abc"#
        ));
        assert!(windows_candidate_executable_ok(
            Some(""),
            r#""C:\path\language_server.exe" --flag"#
        ));
        assert!(windows_candidate_executable_ok(
            Some("   "),
            r#"C:\antigravity\app.exe"#
        ));
        assert!(!windows_candidate_executable_ok(
            None,
            r"notepad.exe file.txt"
        ));
    }

    #[test]
    fn is_antigravity_process_matches_language_server_variants() {
        assert!(is_antigravity_process(
            "language_server.exe --app_data_dir antigravity --port=1234"
        ));
        assert!(is_antigravity_process(
            "/Applications/Antigravity.app/Contents/MacOS/language_server --flag"
        ));
        assert!(is_antigravity_process(
            r"C:\Users\me\AppData\Local\Antigravity\language_server.exe --flag"
        ));
    }

    #[test]
    fn is_antigravity_process_matches_directory_patterns() {
        assert!(is_antigravity_process(
            "/home/user/.config/antigravity/server"
        ));
        assert!(is_antigravity_process(
            r"C:\Programs\antigravity\server.exe"
        ));
    }

    #[test]
    fn is_antigravity_process_rejects_unrelated_commands() {
        assert!(!is_antigravity_process("notepad.exe somefile.txt"));
        assert!(!is_antigravity_process("language_server --other_app"));
        assert!(!is_antigravity_process("some_other_gravity_app"));
        assert!(!is_antigravity_process(""));
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
    #[cfg(not(target_os = "windows"))]
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
    fn antigravity_process_detection_accepts_antigravity_ide_language_server() {
        assert!(is_antigravity_process(
            "/opt/antigravity-ide/resources/app/extensions/antigravity/bin/language_server_linux_x64 --csrf_token abc --app_data_dir antigravity-ide"
        ));
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
    fn standalone_usage_uses_matching_trajectory_step_timestamp() {
        let metadata = vec![
            serde_json::json!({
                "chatModel": {
                    "responseModel": "gemini-3.7-flash",
                    "retryInfos": [{
                        "usage": {
                            "inputTokens": 10,
                            "outputTokens": 5,
                            "thinkingOutputTokens": 1,
                            "responseId": "response-1",
                            "messageId": "message-1"
                        }
                    }]
                }
            }),
            serde_json::json!({
                "chatModel": {
                    "responseModel": "gemini-3.7-flash",
                    "retryInfos": [{
                        "usage": {
                            "inputTokens": 20,
                            "outputTokens": 7,
                            "thinkingOutputTokens": 2,
                            "responseId": "response-2",
                            "messageId": "message-2"
                        }
                    }]
                }
            }),
        ];
        let trajectory = serde_json::json!({
            "trajectory": {
                "steps": [
                    {
                        "metadata": {
                            "createdAt": "2026-08-19T06:34:26.163405500Z",
                            "modelUsage": {
                                "responseId": "response-1",
                                "messageId": "message-1"
                            }
                        }
                    },
                    {
                        "metadata": {
                            "createdAt": "2026-08-19T07:16:35.787801300Z",
                            "modelUsage": {
                                "responseId": "response-2",
                                "messageId": "message-2"
                            }
                        }
                    }
                ]
            }
        });

        let timestamps = usage_timestamps_from_trajectory(&trajectory);
        let lines = normalize_session_metadata_with_timestamps("session-1", &metadata, &timestamps)
            .unwrap();
        let usages: Vec<Value> = lines
            .iter()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|value| value.get("type").and_then(Value::as_str) == Some("usage"))
            .collect();

        assert_eq!(usages.len(), 2);
        assert_eq!(
            usages[0].get("timestamp").and_then(Value::as_i64),
            parse_timestamp_value(&serde_json::json!("2026-08-19T06:34:26.163405500Z"))
        );
        assert_eq!(
            usages[1].get("timestamp").and_then(Value::as_i64),
            parse_timestamp_value(&serde_json::json!("2026-08-19T07:16:35.787801300Z"))
        );
    }

    #[test]
    fn standalone_usage_falls_back_to_matching_message_id() {
        let metadata = vec![serde_json::json!({
            "chatModel": {
                "responseModel": "gemini-3.7-flash",
                "retryInfos": [{
                    "usage": {
                        "inputTokens": 10,
                        "outputTokens": 5,
                        "messageId": "message-1"
                    }
                }]
            }
        })];
        let trajectory = serde_json::json!({
            "trajectory": {
                "steps": [{
                    "metadata": {
                        "createdAt": "2026-08-19T06:34:26.163405500Z",
                        "modelUsage": {
                            "responseId": "response-1",
                            "messageId": "message-1"
                        }
                    }
                }]
            }
        });

        let timestamps = usage_timestamps_from_trajectory(&trajectory);
        let lines = normalize_session_metadata_with_timestamps("session-1", &metadata, &timestamps)
            .unwrap();
        let usage: Value = serde_json::from_str(&lines[1]).unwrap();

        assert_eq!(
            usage.get("timestamp").and_then(Value::as_i64),
            parse_timestamp_value(&serde_json::json!("2026-08-19T06:34:26.163405500Z"))
        );
    }

    #[test]
    fn direct_usage_timestamp_precedes_matching_trajectory_timestamp() {
        let metadata = vec![serde_json::json!({
            "chatModel": {
                "responseModel": "gemini-3.7-flash",
                "retryInfos": [{
                    "usage": {
                        "inputTokens": 10,
                        "outputTokens": 5,
                        "createdAt": "2026-08-19T06:30:00Z",
                        "responseId": "response-1"
                    }
                }]
            }
        })];
        let trajectory = serde_json::json!({
            "trajectory": {
                "steps": [{
                    "metadata": {
                        "createdAt": "2026-08-19T06:34:26.163405500Z",
                        "modelUsage": { "responseId": "response-1" }
                    }
                }]
            }
        });

        let timestamps = usage_timestamps_from_trajectory(&trajectory);
        let lines = normalize_session_metadata_with_timestamps("session-1", &metadata, &timestamps)
            .unwrap();
        let usage: Value = serde_json::from_str(&lines[1]).unwrap();

        assert_eq!(
            usage.get("timestamp").and_then(Value::as_i64),
            parse_timestamp_value(&serde_json::json!("2026-08-19T06:30:00Z"))
        );
    }

    #[test]
    fn trajectory_timestamp_lookup_keeps_earliest_duplicate_step() {
        let trajectory = serde_json::json!({
            "trajectory": {
                "steps": [
                    {
                        "metadata": {
                            "createdAt": "2026-08-19T06:35:00Z",
                            "modelUsage": { "responseId": "response-1" }
                        }
                    },
                    {
                        "metadata": {
                            "createdAt": "2026-08-19T06:34:00Z",
                            "modelUsage": { "responseId": "response-1" }
                        }
                    }
                ]
            }
        });
        let usage = serde_json::json!({ "responseId": "response-1" });

        let timestamps = usage_timestamps_from_trajectory(&trajectory);

        assert_eq!(
            timestamp_for_usage(&usage, &timestamps),
            parse_timestamp_value(&serde_json::json!("2026-08-19T06:34:00Z"))
        );
    }

    #[test]
    fn resolved_ide_metadata_does_not_request_trajectory_timestamps() {
        let metadata = vec![serde_json::json!({
            "chatModel": {
                "chatStartMetadata": { "createdAt": "2026-08-19T06:30:00Z" },
                "retryInfos": [{
                    "usage": { "inputTokens": 10, "outputTokens": 5 }
                }]
            }
        })];

        assert!(!session_metadata_needs_trajectory_timestamps(&metadata));
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

    /// The same artifact is spelled `sessions\<id>.jsonl` in a manifest written
    /// on Windows and `sessions/<id>.jsonl` in one written on Unix, so a
    /// textual compare calls the old spelling stale while the new spelling is
    /// still live. Both sides must be keyed the same way `delete_artifact_relative_path`
    /// resolves them.
    #[test]
    fn stale_relative_paths_ignores_separator_spelling() {
        let mut previous = sample_manifest();
        previous.sessions[0].artifact_path = "sessions\\session-1.jsonl".to_string();
        let next = sample_manifest();

        assert!(stale_relative_paths(&previous, &next).is_empty());
    }

    #[test]
    #[serial]
    fn cleanup_stale_session_artifacts_removes_legacy_files_after_migration() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = TestEnvGuard::redirect_to(temp_dir.path());

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
    }

    /// Cleanup runs *after* `save_antigravity_manifest`, so anything it deletes
    /// is gone while the manifest still advertises it. A manifest carried from
    /// Windows spells the entry `sessions\<id>.jsonl` and the re-sync on Unix
    /// rewrites it to `sessions/<id>.jsonl`; those are the same file, and
    /// retiring the old spelling would silently destroy the artifact the new
    /// manifest points at.
    #[test]
    #[serial]
    fn cleanup_stale_session_artifacts_keeps_a_live_artifact_respelled_across_platforms() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = TestEnvGuard::redirect_to(temp_dir.path());

        let sessions_dir = get_antigravity_sessions_dir().unwrap();
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let artifact = sessions_dir.join("session-one.jsonl");
        std::fs::write(&artifact, "live\n").unwrap();

        let entry = |artifact_path: &str| ManifestSessionEntry {
            session_id: "session-one".to_string(),
            artifact_path: artifact_path.to_string(),
            last_modified_ms: None,
            step_count: None,
            connection_fingerprint: "pid:1:port:1111".to_string(),
            artifact_hash: None,
        };

        let previous = AntigravityManifest {
            sessions: vec![entry("sessions\\session-one.jsonl")],
            ..AntigravityManifest::default()
        };
        let next = AntigravityManifest {
            sessions: vec![entry("sessions/session-one.jsonl")],
            ..AntigravityManifest::default()
        };

        cleanup_stale_session_artifacts(&previous, &next).unwrap();
        assert!(
            artifact.exists(),
            "cleanup deleted an artifact the new manifest still references"
        );
    }

    #[test]
    #[serial]
    fn delete_artifact_relative_path_rejects_paths_outside_cache_root() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = TestEnvGuard::redirect_to(temp_dir.path());

        let err = delete_artifact_relative_path("../outside.jsonl").unwrap_err();
        assert!(err.to_string().contains("cache root"));

        let absolute = temp_dir.path().join("outside.jsonl");
        let err = delete_artifact_relative_path(absolute.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("cache root"));

        let err = delete_artifact_relative_path("manifest.json").unwrap_err();
        assert!(err.to_string().contains("session artifact"));
    }

    /// A manifest written on Windows stores `sessions\<id>.jsonl`, because
    /// `to_relative_artifact_path` renders with the native separator. Rejecting
    /// that form made `cleanup_stale_session_artifacts` — and with it the whole
    /// `antigravity sync` — fail on every run that had an artifact to retire.
    #[test]
    #[serial]
    fn delete_artifact_relative_path_accepts_windows_separators() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = TestEnvGuard::redirect_to(temp_dir.path());

        let sessions_dir = get_antigravity_sessions_dir().unwrap();
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let artifact = sessions_dir.join("session-one.jsonl");
        std::fs::write(&artifact, "{}\n").unwrap();

        assert!(delete_artifact_relative_path("sessions\\session-one.jsonl").unwrap());
        assert!(!artifact.exists());
    }

    /// Normalising separators before the component check also closes a hole on
    /// Unix, where `Path` reads `..\..\outside.jsonl` as a single file name and
    /// never sees a `ParentDir` component.
    #[test]
    #[serial]
    fn delete_artifact_relative_path_rejects_backslash_traversal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = TestEnvGuard::redirect_to(temp_dir.path());

        let err = delete_artifact_relative_path("..\\..\\outside.jsonl").unwrap_err();
        assert!(err.to_string().contains("cache root"));

        let err = delete_artifact_relative_path("sessions\\..\\..\\outside.jsonl").unwrap_err();
        assert!(err.to_string().contains("cache root"));
    }

    #[test]
    #[serial]
    #[cfg(unix)]
    fn delete_artifact_relative_path_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();
        let _env = TestEnvGuard::redirect_to(temp_dir.path());

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
    }

    #[test]
    #[serial]
    fn filesystem_scan_finds_brain_and_conversation_candidates() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = TestEnvGuard::redirect_to(temp_dir.path());

        let legacy_root = temp_dir.path().join(".gemini/antigravity");
        std::fs::create_dir_all(legacy_root.join("brain/session-a")).unwrap();
        std::fs::create_dir_all(legacy_root.join("brain/session-b")).unwrap();
        std::fs::create_dir_all(legacy_root.join("conversations")).unwrap();
        std::fs::write(legacy_root.join("conversations/session-c.pb"), b"pb").unwrap();

        let ide_root = temp_dir.path().join(".gemini/antigravity-ide");
        std::fs::create_dir_all(ide_root.join("brain/session-d")).unwrap();
        std::fs::create_dir_all(ide_root.join("conversations")).unwrap();
        std::fs::write(ide_root.join("conversations/session-e.pb"), b"pb").unwrap();

        let backup_root = temp_dir.path().join(".gemini/antigravity-backup");
        std::fs::create_dir_all(backup_root.join("conversations")).unwrap();
        std::fs::write(backup_root.join("conversations/session-f.pb"), b"pb").unwrap();

        let candidates = scan_filesystem_session_candidates().unwrap();
        let ids: Vec<String> = candidates
            .into_iter()
            .map(|candidate| candidate.session_id)
            .collect();
        assert!(ids.contains(&"session-a".to_string()));
        assert!(ids.contains(&"session-b".to_string()));
        assert!(ids.contains(&"session-c".to_string()));
        assert!(ids.contains(&"session-d".to_string()));
        assert!(ids.contains(&"session-e".to_string()));
        assert!(ids.contains(&"session-f".to_string()));
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
        let _env = TestEnvGuard::redirect_to(temp_dir.path());

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
    }

    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
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

    /// Listener that classifies each connection by its first byte and then
    /// closes without answering: a TLS ClientHello starts with 0x16, a
    /// plaintext Connect POST with `P`. Closing silently is what makes the
    /// wrong-transport leg expensive in production, and it lets these tests
    /// count the legs that were actually attempted.
    fn serve_transport_counter() -> (u16, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let plaintext = Arc::new(AtomicUsize::new(0));
        let tls = Arc::new(AtomicUsize::new(0));
        let (plaintext_seen, tls_seen) = (Arc::clone(&plaintext), Arc::clone(&tls));
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let mut first = [0_u8; 1];
                match std::io::Read::read(&mut stream, &mut first) {
                    Ok(1) if first[0] == 0x16 => tls_seen.fetch_add(1, Ordering::SeqCst),
                    Ok(1) => plaintext_seen.fetch_add(1, Ordering::SeqCst),
                    _ => 0,
                };
            }
        });
        (port, plaintext, tls)
    }

    fn wait_for_at_least(counter: &AtomicUsize, target: usize) -> usize {
        for _ in 0..100 {
            let seen = counter.load(Ordering::SeqCst);
            if seen >= target {
                return seen;
            }
            thread::sleep(Duration::from_millis(20));
        }
        counter.load(Ordering::SeqCst)
    }

    #[test]
    fn rpc_request_skips_the_plaintext_leg_once_a_port_is_known_to_speak_tls() {
        // The regression this guards: `sync` issues one RPC per session, and
        // before the transport was remembered every one of them re-attempted
        // plaintext against a TLS-only DesktopAgent, paying that leg's read
        // timeout each time.
        let (port, plaintext, tls) = serve_transport_counter();
        remember_rpc_transport(port, RpcTransport::Https);
        let connection = AntigravityConnection {
            pid: 1,
            port,
            csrf_token: "abcdef0123456789abcdef0123456789".to_string(),
            fingerprint: format!("pid:1:port:{port}"),
        };

        // Errors either way -- there is no real TLS server behind the counter.
        // What matters is which legs were attempted, not that they succeeded.
        let _ = rpc_request(&connection, "X", &serde_json::json!({}));
        let _ = rpc_request(&connection, "X", &serde_json::json!({}));

        assert!(
            wait_for_at_least(&tls, 1) >= 1,
            "the HTTPS leg must still be attempted"
        );
        assert_eq!(
            plaintext.load(Ordering::SeqCst),
            0,
            "a port known to speak TLS must never be re-probed in plaintext"
        );
    }

    #[test]
    fn rpc_request_remembers_a_port_that_answered_plain_http() {
        let body = br#"{"ok":true}"#.to_vec();
        let port = serve_once(body.clone(), &format!("Content-Length: {}\r\n", body.len()));
        let connection = AntigravityConnection {
            pid: 1,
            port,
            csrf_token: "abcdef0123456789abcdef0123456789".to_string(),
            fingerprint: format!("pid:1:port:{port}"),
        };

        rpc_request(&connection, "X", &serde_json::json!({})).unwrap();

        assert_eq!(
            cached_rpc_transport(port),
            Some(RpcTransport::PlainHttp),
            "a working plaintext port must be remembered, so the TLS leg is not tried later"
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn https_rpc_request_tolerates_an_entered_tokio_runtime_context() {
        // Regression for #1264: the TUI usage refresh reaches this function
        // while `has_credentials`'s own runtime is mid-`block_on`, and the
        // nested `block_on` used to panic with "Cannot start a runtime from
        // within a runtime". Entering a runtime here reproduces the caller
        // side exactly; nothing listens on the port because the point is
        // that the call returns an error instead of panicking.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let connection = AntigravityConnection {
            pid: 1,
            port,
            csrf_token: "abcdef0123456789abcdef0123456789".to_string(),
            fingerprint: format!("pid:1:port:{port}"),
        };

        let error = rt
            .block_on(async { https_rpc_request(&connection, "GetUsage", &serde_json::json!({})) })
            .unwrap_err();

        // Asserting only the absence of "runtime" would also be satisfied by
        // the panic-catch path above, whose message ("worker thread panicked")
        // does not contain the word either -- a swallowed panic would keep this
        // test green. Pinning the connection error the request actually
        // produces means only the fixed path can pass.
        // Matched on reqwest's own predicates rather than its `Display` text:
        // that wording is an undocumented internal format a version bump can
        // reword, while a swallowed panic does not downcast to a
        // `reqwest::Error` at all, so this still separates the two paths.
        let transport = error
            .downcast_ref::<reqwest::Error>()
            .unwrap_or_else(|| panic!("the failure must be the request's own, got: {error:#}"));
        assert!(
            transport.is_connect(),
            "the request must fail connecting to the closed port, got: {error:#}"
        );
        assert_eq!(
            transport.url().and_then(|url| url.port()),
            Some(port),
            "the failure must name the closed port, got: {error:#}"
        );
    }

    /// Local Antigravity RPC listener whose answer per method a test controls.
    ///
    /// The handler receives the RPC method name and returns the JSON body to
    /// send, or `None` to close without answering — which is how a broken
    /// endpoint reaches `rpc_request` as an error without a test having to
    /// spend the 10s transport timeout to get there. Connections that do not
    /// open with a plaintext POST are the HTTPS fallback leg probing the same
    /// port; dropping them makes that leg fail immediately too.
    fn serve_rpc_methods<F>(handler: F) -> (u16, Arc<Mutex<Vec<String>>>)
    where
        F: Fn(&str) -> Option<String> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let methods = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&methods);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let mut buf = [0_u8; 4096];
                let Ok(read) = std::io::Read::read(&mut stream, &mut buf) else {
                    continue;
                };
                let request = String::from_utf8_lossy(&buf[..read]).to_string();
                if !request.starts_with("POST ") {
                    continue;
                }
                let method = request
                    .split_whitespace()
                    .nth(1)
                    .and_then(|path| path.rsplit('/').next())
                    .unwrap_or_default()
                    .to_string();
                if let Ok(mut log) = seen.lock() {
                    log.push(method.clone());
                }
                let Some(body) = handler(&method) else {
                    continue;
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (port, methods)
    }

    /// Metadata carrying usage with no timestamp anywhere, which is the only
    /// shape that asks for the optional trajectory enrichment.
    fn metadata_needing_enrichment() -> String {
        serde_json::json!({
            "generatorMetadata": [{
                "chatModel": {
                    "responseModel": "gemini-3.7-flash",
                    "retryInfos": [{
                        "usage": {
                            "inputTokens": 10,
                            "outputTokens": 5,
                            "responseId": "response-1",
                            "messageId": "message-1"
                        }
                    }]
                }
            }]
        })
        .to_string()
    }

    fn enrichment_test_connection(port: u16) -> AntigravityConnection {
        AntigravityConnection {
            pid: 1,
            port,
            csrf_token: "abcdef0123456789abcdef0123456789".to_string(),
            fingerprint: format!("pid:1:port:{port}"),
        }
    }

    #[test]
    fn trajectory_enrichment_stops_after_a_connection_keeps_failing() {
        // The regression this guards: `sync` walks every session under its
        // exclusive cache lock, and each session whose metadata lacks usage
        // timestamps used to issue a `GetCascadeTrajectory` whose error was
        // only discarded after it returned. With metadata still answering and
        // the trajectory endpoint stalling, that is one full transport timeout
        // per session with the lock held.
        let metadata = metadata_needing_enrichment();
        let (port, seen) = serve_rpc_methods(move |method| match method {
            "GetCascadeTrajectoryGeneratorMetadata" => Some(metadata.clone()),
            _ => None,
        });
        // The transport cache is keyed by port and outlives individual tests,
        // so pin it rather than depend on which leg a recycled port last used.
        remember_rpc_transport(port, RpcTransport::PlainHttp);
        let connection = enrichment_test_connection(port);
        let mut budget = TrajectoryEnrichmentBudget::new();

        const SESSIONS: usize = 5;
        for index in 0..SESSIONS {
            let summary = TrajectorySummary {
                session_id: format!("session-{index}"),
                last_modified_ms: Some(1),
                step_count: Some(1),
                connection_fingerprint: connection.fingerprint.clone(),
            };
            let artifact = try_fetch_session_artifact(&summary, &connection, &mut budget)
                .expect("a failed enrichment must never fail the sync")
                .expect("the session must still be written from its metadata");
            assert!(
                artifact.contents.contains(&format!("session-{index}")),
                "the metadata-derived artifact is the documented fallback: {}",
                artifact.contents
            );
        }

        let methods = seen.lock().unwrap();
        let metadata_calls = methods
            .iter()
            .filter(|method| *method == "GetCascadeTrajectoryGeneratorMetadata")
            .count();
        let trajectory_calls = methods
            .iter()
            .filter(|method| *method == "GetCascadeTrajectory")
            .count();

        assert_eq!(
            metadata_calls, SESSIONS,
            "metadata is not the optional part and must still be fetched per session"
        );
        assert_eq!(
            trajectory_calls, TRAJECTORY_ENRICHMENT_FAILURE_THRESHOLD as usize,
            "the breaker must write the connection off after \
             {TRAJECTORY_ENRICHMENT_FAILURE_THRESHOLD} consecutive failures instead of paying \
             for every one of the {SESSIONS} sessions"
        );
    }

    #[test]
    fn trajectory_enrichment_keeps_running_while_the_endpoint_answers() {
        // The breaker must not cost a healthy history its timestamps.
        let metadata = metadata_needing_enrichment();
        let trajectory = serde_json::json!({
            "trajectory": {
                "steps": [{
                    "metadata": {
                        "createdAt": "2026-08-19T06:34:26.163405500Z",
                        "modelUsage": { "responseId": "response-1", "messageId": "message-1" }
                    }
                }]
            }
        })
        .to_string();
        let (port, seen) = serve_rpc_methods(move |method| match method {
            "GetCascadeTrajectoryGeneratorMetadata" => Some(metadata.clone()),
            "GetCascadeTrajectory" => Some(trajectory.clone()),
            _ => None,
        });
        remember_rpc_transport(port, RpcTransport::PlainHttp);
        let connection = enrichment_test_connection(port);
        let mut budget = TrajectoryEnrichmentBudget::new();

        const SESSIONS: usize = 5;
        for index in 0..SESSIONS {
            let summary = TrajectorySummary {
                session_id: format!("session-{index}"),
                last_modified_ms: Some(1),
                step_count: Some(1),
                connection_fingerprint: connection.fingerprint.clone(),
            };
            let artifact = try_fetch_session_artifact(&summary, &connection, &mut budget)
                .unwrap()
                .expect("the session must be written");
            let usage: Value = artifact
                .contents
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|value| value.get("type").and_then(Value::as_str) == Some("usage"))
                .expect("the artifact must carry a usage line");
            assert_eq!(
                usage.get("timestamp").and_then(Value::as_i64),
                parse_timestamp_value(&serde_json::json!("2026-08-19T06:34:26.163405500Z")),
                "an answering endpoint must keep enriching every session"
            );
        }

        let trajectory_calls = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|method| *method == "GetCascadeTrajectory")
            .count();
        assert_eq!(
            trajectory_calls, SESSIONS,
            "a working endpoint must not be rate limited by the breaker"
        );
    }

    #[test]
    fn enrichment_budget_trips_only_on_consecutive_failures() {
        let mut budget = TrajectoryEnrichmentBudget::with_limits(Duration::from_secs(60), 2);

        assert!(budget.should_attempt("a"));
        budget.record_failure("a", Duration::from_millis(1));
        assert!(
            budget.should_attempt("a"),
            "one failure is indistinguishable from a session with no trajectory"
        );

        budget.record_success("a");
        budget.record_failure("a", Duration::from_millis(1));
        assert!(
            budget.should_attempt("a"),
            "an answering connection gets its full allowance back"
        );

        budget.record_failure("a", Duration::from_millis(1));
        assert!(
            !budget.should_attempt("a"),
            "two failures in a row mean the endpoint, not the session, is broken"
        );
        assert!(
            budget.should_attempt("b"),
            "the breaker is per connection, not global"
        );
    }

    #[test]
    fn enrichment_budget_stops_every_connection_once_it_is_spent() {
        // Simulated elapsed time: the real path charges what a stalled RPC
        // actually took, and no test should spend a transport timeout to prove
        // the arithmetic.
        let mut budget = TrajectoryEnrichmentBudget::with_limits(Duration::from_secs(60), 2);

        for index in 0..6 {
            let fingerprint = format!("connection-{index}");
            assert!(
                budget.should_attempt(&fingerprint),
                "a fresh connection is attempted while budget remains"
            );
            budget.record_failure(&fingerprint, Duration::from_secs(10));
        }

        assert!(
            !budget.should_attempt("connection-6"),
            "six stalled requests at the 10s transport timeout spend the whole sync budget, \
             so a seventh connection must not be attempted either"
        );
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
            body.extend(std::iter::repeat_n(b'a', chunk_size));
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
    fn identity_probe_request_decodes_chunked_antigravity_response() {
        let json = r#"{"trajectorySummaries":{"session-1":{"cascadeId":"session-1"}}}"#;
        let mut body = Vec::new();
        body.extend_from_slice(format!("{:x}\r\n", json.len()).as_bytes());
        body.extend_from_slice(json.as_bytes());
        body.extend_from_slice(b"\r\n0\r\n\r\n");

        let port = serve_once(body, "Transfer-Encoding: chunked\r\n");
        let response = identity_probe_request(
            port,
            "abcdef0123456789abcdef0123456789",
            "GetAllCascadeTrajectories",
        )
        .unwrap();

        assert!(response_contains_antigravity_marker(&response));
    }

    #[test]
    fn identity_probe_request_uses_probe_cap_for_large_bodies() {
        let prefix = r#"{"trajectorySummaries":{"session-1":{"cascadeId":"session-1"}}}"#;
        let mut content_length_body = prefix.as_bytes().to_vec();
        content_length_body.resize(MAX_IDENTITY_PROBE_BYTES + 1, b'a');
        let content_length_port = serve_once(
            content_length_body,
            &format!("Content-Length: {}\r\n", MAX_IDENTITY_PROBE_BYTES + 1),
        );
        let content_length_response = identity_probe_request(
            content_length_port,
            "abcdef0123456789abcdef0123456789",
            "GetAllCascadeTrajectories",
        )
        .unwrap();
        assert_eq!(content_length_response.len(), MAX_IDENTITY_PROBE_BYTES);
        assert!(response_contains_antigravity_marker(
            &content_length_response
        ));

        let chunk_size = MAX_IDENTITY_PROBE_BYTES + 1;
        let mut chunked_body = Vec::new();
        chunked_body.extend_from_slice(format!("{:x}\r\n", chunk_size).as_bytes());
        chunked_body.extend_from_slice(prefix.as_bytes());
        chunked_body.extend(std::iter::repeat_n(b'a', chunk_size - prefix.len()));
        chunked_body.extend_from_slice(b"\r\n0\r\n\r\n");
        let chunked_port = serve_once(chunked_body, "Transfer-Encoding: chunked\r\n");
        let chunked_response = identity_probe_request(
            chunked_port,
            "abcdef0123456789abcdef0123456789",
            "GetAllCascadeTrajectories",
        )
        .unwrap();
        assert_eq!(chunked_response.len(), MAX_IDENTITY_PROBE_BYTES);
        assert!(response_contains_antigravity_marker(&chunked_response));
    }

    #[test]
    fn identity_probe_request_prefers_chunked_over_content_length() {
        let json = r#"{"trajectorySummaries":{"session-1":{"cascadeId":"session-1"}}}"#;
        let mut body = Vec::new();
        body.extend_from_slice(format!("{:x}\r\n", json.len()).as_bytes());
        body.extend_from_slice(json.as_bytes());
        body.extend_from_slice(b"\r\n0\r\n\r\n");

        let port = serve_once(body, "Transfer-Encoding: chunked\r\nContent-Length: 1\r\n");
        let response = identity_probe_request(
            port,
            "abcdef0123456789abcdef0123456789",
            "GetAllCascadeTrajectories",
        )
        .unwrap();

        assert!(response_contains_antigravity_marker(&response));
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
        let _env = TestEnvGuard::redirect_to(temp_dir.path());

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
    }

    #[test]
    #[serial]
    fn load_antigravity_manifest_treats_older_version_as_fresh_start() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = TestEnvGuard::redirect_to(temp_dir.path());

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
    }

    #[test]
    #[serial]
    fn load_antigravity_manifest_recovers_from_corrupted_json() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = TestEnvGuard::redirect_to(temp_dir.path());

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
    }

    /// Regression (#1010): a lock file exists before it means anything. The
    /// old protocol created it, then wrote the pid on the next line, so a
    /// contender arriving in that window read an empty file, concluded nobody
    /// owned it, and unlinked a lock a live process had just taken. Ownership
    /// has to come from the OS, not from the bytes in the file.
    #[test]
    #[serial]
    fn sync_lock_guard_refuses_a_held_lock_that_has_no_pid_written_yet() {
        use fs2::FileExt;

        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().to_path_buf();
        let lock_path = cache_dir.join("sync.lock");

        let holder = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        holder.try_lock_exclusive().unwrap();

        let err = SyncLockGuard::acquire(&cache_dir).unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "a held lock must never be evicted, got: {err:#}"
        );

        FileExt::unlock(&holder).unwrap();
    }

    /// A second sync must be refused for as long as the first guard lives,
    /// and must succeed once it is dropped. This is the property the lock
    /// exists for; the previous protocol could only approximate it by probing
    /// a recorded pid.
    #[test]
    #[serial]
    fn sync_lock_guard_excludes_a_second_sync_until_the_first_is_dropped() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().to_path_buf();

        let guard = SyncLockGuard::acquire(&cache_dir).unwrap();
        let err = SyncLockGuard::acquire(&cache_dir).unwrap_err();
        assert!(err.to_string().contains("in progress"), "got: {err:#}");

        drop(guard);
        SyncLockGuard::acquire(&cache_dir)
            .expect("the lock must be free once the guard is dropped");
    }

    /// A contender must take the companion lock before publishing the legacy
    /// PID path. When it loses, no visible record is left behind.
    #[test]
    #[serial]
    fn sync_lock_guard_losing_contender_leaves_no_orphan_after_release() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().to_path_buf();
        let lock_path = cache_dir.join("sync.lock");

        let owner = SyncLockGuard::acquire(&cache_dir).unwrap();
        let err = SyncLockGuard::acquire(&cache_dir).unwrap_err();
        assert!(err.to_string().contains("in progress"));
        assert!(lock_path.exists(), "only the owner's record is visible");

        drop(owner);
        assert!(!lock_path.exists(), "owner release removes its record");
        let successor = SyncLockGuard::acquire(&cache_dir).unwrap();
        drop(successor);
        assert!(!lock_path.exists(), "no contender record is stranded");
    }

    #[test]
    #[serial]
    fn sync_lock_guard_acquires_when_no_lock_present() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().to_path_buf();
        let lock_path = cache_dir.join("sync.lock");
        {
            let _guard = SyncLockGuard::acquire(&cache_dir).unwrap();
            assert!(lock_path.exists());
        }
        assert!(!lock_path.exists(), "normal release removes its own lock");
    }

    /// Existing lock paths are not reclaimed: a legacy process can replace
    /// them between observation and deletion.
    #[test]
    #[serial]
    fn sync_lock_guard_refuses_to_reclaim_a_stale_lock_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().to_path_buf();
        let lock_path = cache_dir.join("sync.lock");
        std::fs::write(&lock_path, "999999 1776000000").unwrap();

        let err = SyncLockGuard::acquire(&cache_dir).unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert!(lock_path.exists());
    }

    #[test]
    fn existing_sync_lock_error_names_the_exact_stale_lock_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let lock_path = temp_dir.path().join("sync.lock");
        std::fs::write(&lock_path, "999999 1\n").unwrap();

        let err = existing_sync_lock_error(&lock_path).to_string();
        let quoted_path = format!("'{}'", lock_path.display());
        assert!(err.contains(&quoted_path));
        assert!(err.contains("Confirm no tokscale Antigravity sync is running"));
        assert!(err.contains("remove"));
    }

    /// During a rolling upgrade an older binary still uses `sync.lock` as a
    /// PID-file lock. The new OS lock is not evidence that the old owner has
    /// stopped, so its live record must prevent takeover.
    #[test]
    #[serial]
    fn sync_lock_guard_preserves_a_live_legacy_pid_lock() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().to_path_buf();
        let lock_path = cache_dir.join("sync.lock");
        std::fs::write(&lock_path, format!("{} 1\n", std::process::id())).unwrap();

        let err = SyncLockGuard::acquire(&cache_dir).unwrap_err();
        assert!(
            err.to_string()
                .contains("Another tokscale Antigravity sync may be in progress"),
            "a live legacy owner must be preserved, got: {err:#}"
        );
        assert_eq!(
            read_sync_lock(&lock_path).map(|(pid, _)| pid),
            Some(std::process::id()),
            "the new binary must not overwrite the legacy owner's record"
        );
    }

    /// An old binary's create-new acquisition must recognize a live new
    /// owner, then leave its inode alone.
    #[test]
    #[serial]
    fn sync_lock_guard_remains_readable_to_the_legacy_protocol() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path().to_path_buf();
        let lock_path = cache_dir.join("sync.lock");
        let guard = SyncLockGuard::acquire(&cache_dir).unwrap();

        let legacy_open = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
            .unwrap_err();
        assert_eq!(legacy_open.kind(), std::io::ErrorKind::AlreadyExists);
        let (pid, _) = read_sync_lock(&lock_path).expect("legacy PID record");
        assert_eq!(pid, std::process::id());
        assert!(pid_is_alive(pid), "legacy sync would preserve a live owner");
        assert!(
            lock_path.exists(),
            "legacy sync must not unlink the live inode"
        );
        drop(guard);
    }

    #[test]
    fn publish_lock_never_exposes_an_empty_inode_to_legacy_acquire() {
        let temp_dir = tempfile::tempdir().unwrap();
        let lock_path = temp_dir.path().join("sync.lock");

        publish_legacy_readable_lock(&lock_path).unwrap();
        let legacy_open = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
            .unwrap_err();
        assert_eq!(legacy_open.kind(), std::io::ErrorKind::AlreadyExists);
        let (pid, timestamp) = read_sync_lock(&lock_path).expect("complete legacy record");
        assert_eq!(pid, std::process::id());
        assert!(timestamp > 0);
    }

    /// An old binary creates its PID-file before writing the record. It may be
    /// paused in that exact interval and holds no OS lock, so new code must
    /// still fail closed rather than unlink its pending inode.
    #[test]
    fn sync_lock_guard_refuses_an_empty_legacy_inode_without_an_os_lock() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_dir = temp_dir.path();
        let lock_path = cache_dir.join("sync.lock");
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
            .unwrap();

        let err = SyncLockGuard::acquire(cache_dir).unwrap_err();
        assert!(err.to_string().contains("already exists"));
        assert!(lock_path.exists(), "the pending legacy inode must survive");
    }

    /// `purge-cache` must use a lock outside the cache directory. Otherwise
    /// it can unlink a sync's held `sync.lock` inode and let another sync lock
    /// a new file at the same path.
    #[test]
    #[serial]
    fn purge_cache_refuses_while_sync_holds_the_parent_lock() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = TestEnvGuard::redirect_to(temp_dir.path());
        let cache_dir = get_antigravity_cache_dir().unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::write(cache_dir.join("manifest.json"), "{}").unwrap();

        let sync = SyncLockGuard::acquire(&cache_dir).unwrap();
        let err = run_antigravity_purge_cache().unwrap_err();
        assert!(
            err.to_string()
                .contains("Another tokscale Antigravity cache operation is in progress"),
            "purge must not unlink an active sync lock, got: {err:#}"
        );
        assert!(cache_dir.join("sync.lock").exists());
        assert!(cache_dir.join("manifest.json").exists());
        drop(sync);
    }

    #[test]
    #[serial]
    fn purge_cache_refuses_a_live_legacy_pid_lock() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _env = TestEnvGuard::redirect_to(temp_dir.path());
        let cache_dir = get_antigravity_cache_dir().unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::write(cache_dir.join("manifest.json"), "{}").unwrap();
        std::fs::write(
            cache_dir.join("sync.lock"),
            format!("{} 1\n", std::process::id()),
        )
        .unwrap();

        let err = run_antigravity_purge_cache().unwrap_err();
        assert!(
            err.to_string()
                .contains("Another tokscale Antigravity sync may be in progress"),
            "purge must preserve a live legacy sync, got: {err:#}"
        );
        assert!(cache_dir.join("manifest.json").exists());
        assert!(cache_dir.join("sync.lock").exists());
    }

    /// The Windows fallback is compiled out on this host, but the argument
    /// vector is not: `windows_curl_rpc_args` is built on every target under
    /// `cfg(test)` so the two rules that make it safe are actually checked.
    #[test]
    fn windows_curl_rpc_args_ignore_curlrc_and_every_proxy() {
        let url = "https://127.0.0.1:4321/exa.language_server_pb.LanguageServerService/GetUsage";
        let args = windows_curl_rpc_args(url);

        // curl only honors `-q` in the first position; anywhere later and the
        // user's curlrc has already been applied by the time it is parsed.
        assert_eq!(
            args.first().copied(),
            Some("-q"),
            "-q must be the first argument, got: {args:?}"
        );

        // The loopback RPC carries the CSRF token, so it must never be routed
        // through HTTPS_PROXY / ALL_PROXY / a curlrc proxy directive.
        let noproxy = args
            .iter()
            .position(|arg| *arg == "--noproxy")
            .expect("--noproxy is missing from the curl.exe fallback arguments");
        assert_eq!(
            args.get(noproxy + 1).copied(),
            Some("*"),
            "--noproxy must bypass every host, got: {args:?}"
        );

        // The hardening must not have displaced the rest of the invocation:
        // the config (and with it the CSRF header and the body) still arrives
        // on stdin, the timeout is still set, and the status code is still
        // appended to the response.
        let config = args
            .iter()
            .position(|arg| *arg == "-K")
            .expect("-K is missing from the curl.exe fallback arguments");
        assert_eq!(args.get(config + 1).copied(), Some("-"));
        let max_time = args
            .iter()
            .position(|arg| *arg == "--max-time")
            .expect("--max-time is missing from the curl.exe fallback arguments");
        assert_eq!(args.get(max_time + 1).copied(), Some("10"));
        assert!(args.contains(&"\\n%{http_code}"), "{args:?}");
        assert!(args.contains(&url), "{args:?}");
    }

    /// The Windows fallback itself is compiled out on this host, but the two
    /// readers that give it its memory bound are not: both are built on every
    /// target under `cfg(test)`, so the properties that matter are checked
    /// here instead of only by the Windows CI job.
    #[test]
    fn curl_stdout_cap_stops_the_read_instead_of_measuring_the_result() {
        // `io::repeat` never ends. Anything that buffered the response first
        // and compared its length afterwards would run until it exhausted
        // memory; stopping at the ceiling is the only way this returns at all.
        let body = read_curl_stdout_with_cap(std::io::repeat(b'x'), 64).unwrap();
        assert_eq!(
            body.len(),
            65,
            "the read must stop one byte past the cap, not at whatever the writer sends"
        );
        assert!(
            body.len() > 64,
            "that extra byte is what the caller tests to reject an over-cap response"
        );
    }

    #[test]
    fn curl_stdout_exactly_at_the_cap_survives_intact() {
        // The headroom byte must not turn a response that merely fills the cap
        // into a rejected one.
        let source = vec![b'x'; 64];
        let body = read_curl_stdout_with_cap(source.as_slice(), 64).unwrap();
        assert_eq!(
            body, source,
            "a response at the cap is passed through whole"
        );
        assert!(body.len() <= 64, "and is not seen as over the cap");
    }

    #[test]
    fn curl_stderr_keeps_a_prefix_but_still_consumes_everything() {
        let mut source = std::io::Cursor::new(vec![b'e'; 4096]);
        let kept = drain_curl_stderr_with_cap(&mut source, 128);
        assert_eq!(
            kept.len(),
            128,
            "only the prefix is buffered for diagnostics"
        );
        assert_eq!(
            source.position(),
            4096,
            "the remainder is still consumed; bytes left in the pipe are what block the child"
        );
    }
}
