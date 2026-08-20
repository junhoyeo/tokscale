//! Parallel file scanner for session directories
//!
//! Uses walkdir with rayon for parallel directory traversal.

use rayon::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::clients::ClientId;
use crate::sessions::{normalize_workspace_key, workspace_label_from_key};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Emit a one-time `tracing::warn!` if `path` does not start with the scan's
/// supplied home directory. The scan is NOT blocked — this is a heads-up only.
fn warn_if_escapes_home(home: &Path, client_id: ClientId, path: &Path) {
    if !path.starts_with(home) {
        tracing::warn!(
            client = client_id.as_str(),
            path = %path.display(),
            home = %home.display(),
            "extra scan path is outside $HOME — verify this is intentional"
        );
    }
}

/// User-controlled scanner settings loaded from a config file.
///
/// This is the persistent, declarative counterpart to environment variables
/// like `TOKSCALE_EXTRA_DIRS` — it lives on the `scanner` key inside
/// `~/.config/tokscale/settings.json` and is threaded down into
/// [`scan_all_clients_with_scanner_settings`].
///
/// `#[serde(default)]` at both the struct and field level guarantees that
/// older settings.json files (which have no `scanner` key at all, or an
/// empty `{}`) deserialize cleanly without errors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScannerSettings {
    /// Absolute paths to additional OpenCode SQLite databases to scan.
    ///
    /// Use this when the opencode binary was launched with `OPENCODE_DB`
    /// pointing at a location outside the default `~/.local/share/opencode`
    /// data directory, so tokscale's auto-discovery can't find it.
    ///
    /// Paths are merged into the auto-discovered
    /// [`ScanResult::opencode_dbs`] list; duplicates (by canonical path)
    /// are removed and non-existent entries are silently skipped so stale
    /// config does not break the scan. WAL/SHM sidecar files are rejected
    /// with the same [`is_opencode_db_filename`] check used for
    /// auto-discovery.
    #[serde(default)]
    pub opencode_db_paths: Vec<PathBuf>,
    /// Additional per-client scan roots loaded from settings.json.
    ///
    /// Keys use public client ids like `codex`, `gemini`, and `openclaw`
    /// so the JSON stays stable and human-editable.
    #[serde(default)]
    pub extra_scan_paths: BTreeMap<String, Vec<PathBuf>>,
    /// IANA name of the timezone this device buckets usage days into, e.g.
    /// `"Asia/Seoul"`.
    ///
    /// Which local calendar day a message lands in used to be read from
    /// `chrono::Local` on every scan, so rescanning the same history from a
    /// different zone re-split it across days. The server's monotonic per-day
    /// guard then kept the stale value on one day and accepted the new one on
    /// its neighbour, inflating the total permanently. Recording the zone once
    /// makes the bucket key stable.
    ///
    /// `None` means the device has never pinned, and day keys keep following
    /// `chrono::Local` exactly as before — pinning changes nothing until it
    /// happens. The CLI fills this in on first run (see
    /// `tui::settings::pin_bucket_timezone_if_unset`); `tokscale config set
    /// timezone <zone>` can initialize an unset value or recover an invalid
    /// one. A valid pin cannot be changed because submitted day rows are
    /// monotonic.
    ///
    /// An unparseable value degrades to unpinned rather than failing the scan.
    #[serde(default)]
    pub bucket_timezone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrushDbSource {
    pub db_path: PathBuf,
    pub workspace_key: Option<String>,
    pub workspace_label: Option<String>,
}

/// Result of scanning all session directories
#[derive(Debug)]
pub struct ScanResult {
    pub files: [Vec<PathBuf>; ClientId::COUNT],
    /// All OpenCode SQLite databases discovered under the data dir.
    ///
    /// Includes the default `opencode.db` (used by `latest`/`beta` channels
    /// and anyone with `OPENCODE_DISABLE_CHANNEL_DB=1`) as well as any
    /// channel-suffixed variants such as `opencode-stable.db`,
    /// `opencode-nightly.db`, etc. See upstream logic in opencode's
    /// `packages/opencode/src/storage/db.ts` (`getChannelPath`).
    pub opencode_dbs: Vec<PathBuf>,
    pub copilot_desktop_db: Option<PathBuf>,
    pub synthetic_db: Option<PathBuf>,
    pub kilo_db: Option<PathBuf>,
    pub hermes_db: Option<PathBuf>,
    pub goose_db: Option<PathBuf>,
    pub zed_db: Option<PathBuf>,
    pub kiro_db: Option<PathBuf>,
    pub crush_dbs: Vec<CrushDbSource>,
    /// ZCode v2 CLI usage database at `~/.zcode/cli/db/db.sqlite`.
    pub zcode_db: Option<PathBuf>,
    /// MiMo Code SQLite databases discovered under the data dir.
    pub micode_dbs: Vec<PathBuf>,
    /// Path to the OpenCode legacy JSON directory (for migration cache stat checks)
    pub opencode_json_dir: Option<PathBuf>,
    /// Devin CLI SQLite databases, including the default data path and any
    /// user-configured scan roots.
    pub devin_dbs: Vec<PathBuf>,
    /// VS Code Copilot chat session JSONL files discovered under
    /// `workspaceStorage/*/chatSessions/*.jsonl`.
    pub copilot_vscode_sessions: Vec<PathBuf>,
}

impl Default for ScanResult {
    fn default() -> Self {
        Self {
            files: std::array::from_fn(|_| Vec::new()),
            opencode_dbs: Vec::new(),
            copilot_desktop_db: None,
            synthetic_db: None,
            kilo_db: None,
            hermes_db: None,
            goose_db: None,
            zed_db: None,
            kiro_db: None,
            crush_dbs: Vec::new(),
            zcode_db: None,
            micode_dbs: Vec::new(),
            opencode_json_dir: None,
            devin_dbs: Vec::new(),
            copilot_vscode_sessions: Vec::new(),
        }
    }
}

impl ScanResult {
    pub fn get(&self, client: ClientId) -> &Vec<PathBuf> {
        &self.files[client as usize]
    }

    pub fn get_mut(&mut self, client: ClientId) -> &mut Vec<PathBuf> {
        &mut self.files[client as usize]
    }

    /// Get total number of files found
    pub fn total_files(&self) -> usize {
        self.files.iter().map(|v| v.len()).sum()
    }

    /// Get all files as a single vector
    pub fn all_files(&self) -> Vec<(ClientId, PathBuf)> {
        let mut result = Vec::with_capacity(self.total_files());

        for client in ClientId::iter() {
            for path in self.get(client) {
                result.push((client, path.clone()));
            }
        }

        result
    }

    /// Return every Hermes SQLite database that should be parsed.
    ///
    /// Hermes has a default `state.db` path plus optional profile databases
    /// discovered through `scanner.extraScanPaths.hermes`. The generic
    /// `files` bucket carries the extra profile DBs, so this helper gives
    /// callers a single deduped view without changing older `hermes_db`
    /// consumers that only expect the default path.
    pub fn hermes_db_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        let mut push = |path: &Path| {
            let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if seen.insert(key) {
                paths.push(path.to_path_buf());
            }
        };

        if let Some(path) = &self.hermes_db {
            push(path);
        }

        for path in self.get(ClientId::Hermes) {
            push(path);
        }

        paths
    }

    /// Return every Zed threads SQLite database that should be parsed.
    pub fn zed_db_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        let mut push = |path: &Path| {
            let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            if seen.insert(key) {
                paths.push(path.to_path_buf());
            }
        };

        if let Some(path) = &self.zed_db {
            push(path);
        }

        for path in self.get(ClientId::Zed) {
            push(path);
        }

        paths
    }
}

fn expand_tilde_path_with_home(value: &str, home_dir: &str) -> PathBuf {
    if value == "~" {
        return PathBuf::from(home_dir);
    }
    if let Some(relative) = value.strip_prefix("~/") {
        return PathBuf::from(home_dir).join(relative);
    }
    PathBuf::from(value)
}

#[derive(Debug, PartialEq, Eq)]
enum PrimeSessionDirSetting {
    Default,
    Path(PathBuf),
    CurrentDirectory(PathBuf),
}

fn prime_agent_session_dir_from_settings_files(
    global_settings: &Path,
    project_settings: Option<&Path>,
    home_dir: &str,
    current_dir: Option<&Path>,
) -> Option<PrimeSessionDirSetting> {
    fn read_session_dir(path: &Path) -> Option<Option<String>> {
        let content = std::fs::read_to_string(path).ok()?;
        let settings: Value = serde_json::from_str(&content).ok()?;
        match settings.as_object()?.get("sessionDir")? {
            Value::Null => Some(None),
            Value::String(path) => Some(Some(path.clone())),
            _ => None,
        }
    }

    let global = read_session_dir(global_settings);
    let project = project_settings.and_then(read_session_dir);
    project.or(global).map(|setting| match setting {
        None => PrimeSessionDirSetting::Default,
        Some(path) if path.is_empty() => PrimeSessionDirSetting::CurrentDirectory(
            current_dir.unwrap_or_else(|| Path::new("")).to_path_buf(),
        ),
        Some(path) => PrimeSessionDirSetting::Path(expand_tilde_path_with_home(&path, home_dir)),
    })
}

fn prime_agent_session_dir_from_settings(
    agent_dir: &Path,
    home_dir: &str,
) -> Option<PrimeSessionDirSetting> {
    let current_dir = std::env::current_dir().ok();
    let project_settings = current_dir
        .as_ref()
        .map(|cwd| cwd.join(".prime/agent/settings.json"));
    prime_agent_session_dir_from_settings_files(
        &agent_dir.join("settings.json"),
        project_settings.as_deref(),
        home_dir,
        current_dir.as_deref(),
    )
}

/// Resolve Prime Agent's root-session and RLM-artifact scan roots using the
/// same environment precedence and tilde expansion as Prime Agent itself.
pub fn prime_agent_session_roots_with_env_strategy(
    home_dir: &str,
    use_env_roots: bool,
) -> [PathBuf; 2] {
    fn sessions_with_sibling_artifacts(sessions: PathBuf) -> [PathBuf; 2] {
        let artifacts = sessions
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("session-artifacts");
        [sessions, artifacts]
    }

    if !use_env_roots {
        let agent_dir = PathBuf::from(home_dir).join(".prime/agent");
        return [
            agent_dir.join("sessions"),
            agent_dir.join("session-artifacts"),
        ];
    }

    let session_override = std::env::var("PRIME_AGENT_SESSION_DIR")
        .ok()
        .or_else(|| std::env::var("PRIME_AGENT_CODING_AGENT_SESSION_DIR").ok());
    if let Some(path) = session_override.filter(|value| !value.is_empty()) {
        return sessions_with_sibling_artifacts(expand_tilde_path_with_home(&path, home_dir));
    }

    let agent_dir = std::env::var("PRIME_AGENT_CODING_AGENT_DIR")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|path| expand_tilde_path_with_home(&path, home_dir))
        .unwrap_or_else(|| PathBuf::from(home_dir).join(".prime/agent"));
    match prime_agent_session_dir_from_settings(&agent_dir, home_dir) {
        Some(PrimeSessionDirSetting::Path(sessions)) => sessions_with_sibling_artifacts(sessions),
        Some(PrimeSessionDirSetting::CurrentDirectory(current_dir)) => {
            [current_dir.clone(), current_dir.join("session-artifacts")]
        }
        Some(PrimeSessionDirSetting::Default) | None => [
            agent_dir.join("sessions"),
            agent_dir.join("session-artifacts"),
        ],
    }
}

pub fn headless_roots_with_env_strategy(home_dir: &str, use_env_roots: bool) -> Vec<PathBuf> {
    if use_env_roots {
        if let Ok(path) = std::env::var("TOKSCALE_HEADLESS_DIR") {
            return vec![PathBuf::from(path)];
        }
    }

    let mut roots = Vec::new();
    roots.push(PathBuf::from(join_native(
        home_dir,
        ".config/tokscale/headless",
    )));

    let mac_root = PathBuf::from(join_native(
        home_dir,
        "Library/Application Support/tokscale/headless",
    ));
    roots.push(mac_root);

    roots
}

pub fn headless_roots(home_dir: &str) -> Vec<PathBuf> {
    headless_roots_with_env_strategy(home_dir, true)
}

pub fn copilot_exporter_path_with_env_strategy(use_env_roots: bool) -> Option<PathBuf> {
    if !use_env_roots {
        return None;
    }

    let path = std::env::var("COPILOT_OTEL_FILE_EXPORTER_PATH").ok()?;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(PathBuf::from(trimmed))
}

pub fn copilot_exporter_path() -> Option<PathBuf> {
    copilot_exporter_path_with_env_strategy(true)
}

/// Scan a single directory for session files
pub fn scan_directory(root: &str, pattern: &str) -> Vec<PathBuf> {
    if !std::path::Path::new(root).exists() {
        return Vec::new();
    }

    let mut paths: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .par_bridge()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            // WalkDir already knows the entry type from the directory read, so
            // trust it for the common regular-file case and avoid a redundant
            // stat() per file (warm scans over huge trees were stat-bound).
            // Symlinks still fall back to a following stat to preserve behavior.
            let file_type = e.file_type();
            let is_file = file_type.is_file() || (file_type.is_symlink() && path.is_file());
            if !is_file {
                return false;
            }

            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            let is_in_archive_dir = path.components().any(|c| {
                c.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("archive")
            });

            match pattern {
                "*.json" => file_name.ends_with(".json"),
                "*.json|*.jsonl" => file_name.ends_with(".json") || file_name.ends_with(".jsonl"),
                "*.jsonl" => file_name.ends_with(".jsonl"),
                "prime-agent-session" => {
                    file_name.ends_with(".jsonl") && file_name != "rlm-subagents.jsonl"
                }
                "*.ndjson" => file_name.ends_with(".ndjson"),
                "*.log" => file_name.ends_with(".log"),
                "codebuddy-extension-log" => {
                    file_name.ends_with(".log")
                        && path.components().any(|component| {
                            component
                                .as_os_str()
                                .to_string_lossy()
                                .eq_ignore_ascii_case("Tencent-Cloud.coding-copilot")
                        })
                }
                // OpenClaw: also match archived transcripts
                // (<uuid>.jsonl.deleted.<ts>, <uuid>.jsonl.reset.<ts>)
                "*.jsonl*" => {
                    file_name.ends_with(".jsonl")
                        || file_name.contains(".jsonl.deleted.")
                        || file_name.contains(".jsonl.reset.")
                }
                "*.csv" => file_name.ends_with(".csv"),
                "usage*.csv" => {
                    if is_in_archive_dir {
                        return false;
                    }

                    if file_name == "usage.csv" {
                        return true;
                    }

                    // Accept only per-account files: usage.<account>.csv
                    if !file_name.starts_with("usage.") || !file_name.ends_with(".csv") {
                        return false;
                    }

                    // Exclude legacy backups like usage.backup-<ts>.csv
                    if file_name.starts_with("usage.backup") {
                        return false;
                    }

                    true
                }
                "usage*.json" => {
                    if is_in_archive_dir {
                        return false;
                    }

                    if file_name == "usage.json" {
                        return true;
                    }

                    if !file_name.starts_with("usage.") || !file_name.ends_with(".json") {
                        return false;
                    }

                    if file_name.starts_with("usage.backup") {
                        return false;
                    }

                    true
                }
                "session-*.json" => {
                    file_name.starts_with("session-") && file_name.ends_with(".json")
                }
                "session_*.json" => {
                    file_name.starts_with("session_") && file_name.ends_with(".json")
                }
                "T-*.json" => file_name.starts_with("T-") && file_name.ends_with(".json"),
                "*.settings.json" => file_name.ends_with(".settings.json"),
                "kiro-globalstorage" => {
                    file_name.ends_with(".chat")
                        || file_name.ends_with(".json")
                        || path.extension().is_none()
                }
                // Kiro IDE (VS Code-based) session layout on disk:
                //   ~/.kiro/sessions/<workspace>/sess_<uuid>/session.json
                //   ~/.kiro/sessions/<workspace>/sess_<uuid>/messages.jsonl
                // Anchor discovery on `session.json` (the metadata file); the
                // parser reads the sibling `messages.jsonl` itself. Requiring a
                // `sess_*` parent keeps this from colliding with the CLI layout
                // (`~/.kiro/sessions/cli/*.json`) that shares the same tree.
                "kiro-ide-session" => {
                    file_name == "session.json"
                        && path
                            .parent()
                            .and_then(|parent| parent.file_name())
                            .and_then(|name| name.to_str())
                            .map(|name| name.starts_with("sess_"))
                            .unwrap_or(false)
                }
                "sessions.json" => file_name == "sessions.json",
                // DeepSeek Harness: one JSONL transcript per session at any
                // depth under `~/.dsh/sessions/`. The `.zstd` suffix marks the
                // physical encoding only — a backend configured with
                // `compression: none` writes the same rows to a plain
                // `session.jsonl` in the same directory — so both spellings
                // are session logs and the parser sniffs the frame magic.
                "dsh-session-log" => {
                    file_name == "session.jsonl.zstd" || file_name == "session.jsonl"
                }
                "wire.jsonl" => file_name == "wire.jsonl",
                // fx (vercel-labs/fx): one `usage-v2.json` per session
                // directory under `~/.fx/sessions/<id>/`. WalkDir recursion
                // picks up every session's snapshot.
                "usage-v2.json" => file_name == "usage-v2.json",
                "updates.jsonl" => file_name == "updates.jsonl",
                "unified.jsonl" => file_name == "unified.jsonl",
                "events.jsonl" => file_name == "events.jsonl",
                "ui_messages.json" => file_name == "ui_messages.json",
                "cline-cli-messages" => file_name.ends_with(".messages.json"),
                "session-usage.json" => file_name == "session-usage.json",
                "chat-messages.json" => file_name == "chat-messages.json",
                "workbuddy.db" => file_name == "workbuddy.db",
                "sessions.db" => file_name == "sessions.db",
                "state.db" => file_name == "state.db",
                "threads.db" => file_name == "threads.db",
                // Antigravity CLI conversation databases. `ends_with(".db")`
                // naturally rejects the `.db-wal`/`.db-shm`/`.db-journal`
                // sidecars SQLite writes alongside the main file.
                "*.db" => file_name.ends_with(".db"),
                _ => false,
            }
        })
        .map(|e| e.path().to_path_buf())
        .collect();
    // Sort for deterministic ordering. sort_unstable() is sufficient (no stability
    // requirement for PathBuf) and avoids allocation. Note: ordering is byte-lexical,
    // not case-normalized (known Windows/macOS caveat for mixed-case paths).
    paths.sort_unstable();
    paths
}

/// Parse a `TOKSCALE_EXTRA_DIRS`-formatted string into (ClientId, path) pairs.
///
/// Format: comma-separated `client:path` pairs.
/// Example: `"claude:/path/to/mac/sessions,openclaw:/other/path"`
///
/// Only returns entries whose client is present in `enabled`.
/// This is a pure function — the caller is responsible for reading the
/// environment variable and passing its value here.
pub fn parse_extra_dirs(value: &str, enabled: &HashSet<ClientId>) -> Vec<(ClientId, String)> {
    if value.is_empty() {
        return Vec::new();
    }

    value
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            let (client_str, path) = entry.split_once(':')?;
            let client_id = ClientId::from_str(client_str.trim())?;
            if !enabled.contains(&client_id) || !supports_extra_dir_scanning(client_id) {
                return None;
            }
            let path = path.trim().to_string();
            if path.is_empty() {
                return None;
            }
            Some((client_id, path))
        })
        .collect()
}

pub fn extra_scan_paths_for(
    settings: &ScannerSettings,
    enabled: &HashSet<ClientId>,
) -> Vec<(ClientId, PathBuf)> {
    settings
        .extra_scan_paths
        .iter()
        .filter_map(|(client_str, paths)| {
            let client_id = ClientId::from_str(client_str)?;
            if !enabled.contains(&client_id) || !supports_extra_dir_scanning(client_id) {
                return None;
            }
            Some(
                paths
                    .iter()
                    .filter(|path| !path.as_os_str().is_empty())
                    .cloned()
                    .map(move |path| (client_id, path)),
            )
        })
        .flatten()
        .collect()
}

/// Join `root` and a `/`-joined relative literal with native separators
/// throughout — the same spelling `ClientDef::resolve_path_with_env_strategy`
/// produces. `Path::join` only normalizes the junction, so the relative half's
/// own `/` separators would survive untouched on Windows (#1048).
fn join_native(root: &str, relative: &str) -> String {
    let mut path = std::path::PathBuf::from(root);
    for component in std::path::Path::new(relative).components() {
        path.push(component.as_os_str());
    }
    path.to_string_lossy().into_owned()
}

pub fn built_in_extra_scan_paths_for(
    home_dir: &str,
    enabled: &HashSet<ClientId>,
    use_env_roots: bool,
) -> Vec<(ClientId, PathBuf)> {
    let mut paths = Vec::new();

    if enabled.contains(&ClientId::Claude) {
        // `transcripts` is a sibling of the registered `projects` root, not a
        // fixed offset from `home_dir` — resolving through the client's own
        // `root` keeps this in sync with CLAUDE_CONFIG_DIR (#1048-adjacent:
        // this dir moves whenever the primary root does), and respecting
        // `use_env_roots` keeps parity with the `Grok` root resolved just
        // above this function's only caller.
        let claude_root = ClientId::Claude
            .data()
            .root
            .resolve_with_env_strategy(home_dir, use_env_roots);
        paths.push((
            ClientId::Claude,
            PathBuf::from(join_native(&claude_root, "transcripts")),
        ));
        paths.extend(
            crate::cc_mirror::discover_claude_project_roots(Path::new(home_dir))
                .into_iter()
                .map(|path| (ClientId::Claude, path)),
        );
    }

    if enabled.contains(&ClientId::Senpi) && use_env_roots {
        if let Some(path) =
            std::env::var_os("SENPI_CODING_AGENT_SESSION_DIR").filter(|path| !path.is_empty())
        {
            paths.push((ClientId::Senpi, PathBuf::from(path)));
        }

        if let Ok(current_dir) = std::env::current_dir() {
            paths.push((
                ClientId::Senpi,
                current_dir.join(".omo").join("senpi-task").join("children"),
            ));
        }
    }

    paths
}

/// Discover Hermes profile databases under a Hermes home directory.
///
/// Hermes stores the default profile at `<hermes-home>/state.db` and named
/// profiles at `<hermes-home>/profiles/<profile>/state.db`.
///
/// Data-isolation rule: sibling and default profiles are ONLY discovered when
/// scanning from the *root* Hermes home. When `HERMES_HOME` points at a
/// specific named profile (for example `<root>/profiles/coder`, i.e. its parent
/// directory is `profiles/`), the user has expressed intent to isolate that one
/// profile, so we scan ONLY that profile. We deliberately do NOT climb up to
/// sibling profiles under `<root>/profiles/*` or the default profile at
/// `<root>/state.db`. Auto-discovering (and therefore making uploadable via
/// `tokscale submit`) sibling/default profiles from a profile-scoped
/// `HERMES_HOME` would silently break the isolation boundary the user set up.
/// The active profile's own `state.db` is resolved separately as the primary
/// Hermes database, so this function returns no extra paths in that case.
///
/// `read_dir` keeps profile discovery intentionally shallow: each immediate
/// child of the root home's `profiles/` directory is treated as one profile
/// directory, matching Hermes' profile layout without walking arbitrary user
/// data.
pub(crate) fn discover_hermes_profile_state_dbs(hermes_home: &Path) -> Vec<PathBuf> {
    // Profile-scoped `HERMES_HOME` (parent directory is `profiles/`): isolate to
    // this single profile and perform no sibling/default discovery.
    if hermes_home
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "profiles")
    {
        return Vec::new();
    }

    // Root Hermes home: discover every named profile under `profiles/`.
    let mut dbs: Vec<PathBuf> = std::fs::read_dir(hermes_home.join("profiles"))
        .into_iter()
        .flat_map(|entries| entries.filter_map(|entry| entry.ok()))
        .filter_map(|entry| {
            let state_db = entry.path().join("state.db");
            state_db.is_file().then_some(state_db)
        })
        .collect();
    dbs.sort_unstable();
    dbs.dedup();
    dbs
}

/// Candidate Hermes home directories to scan for `state.db` and profiles.
///
/// Resolution order mirrors the Crush discovery's Windows rigor
/// ([`crush_registry_candidates`]):
/// 1. `HERMES_HOME` when set, otherwise `~/.hermes` — the `PathRoot::EnvVar`
///    strategy for [`ClientId::Hermes`].
/// 2. `%LOCALAPPDATA%\hermes` on native Windows (env roots enabled).
/// 3. `<home>/AppData/Local/hermes` — the literal Windows fallback, always
///    appended so it is exercised cross-platform (matching Crush's
///    `AppData/Local` fallback).
///
/// The native Windows roots are only consulted when `HERMES_HOME` is *not* set:
/// an explicit `HERMES_HOME` is authoritative and may be profile-scoped for data
/// isolation, so widening discovery to the default Windows home in that case
/// would reintroduce the isolation leak that the profile-scoping rule prevents.
fn hermes_home_candidates(home_dir: &str, use_env_roots: bool) -> Vec<PathBuf> {
    let mut homes = vec![PathBuf::from(
        ClientId::Hermes
            .data()
            .root
            .resolve_with_env_strategy(home_dir, use_env_roots),
    )];

    let hermes_home_set = use_env_roots
        && std::env::var("HERMES_HOME")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    if !hermes_home_set {
        if cfg!(target_os = "windows") && use_env_roots {
            if let Some(local_app_data) =
                std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())
            {
                homes.push(PathBuf::from(local_app_data).join("hermes"));
            }
        }
        homes.push(PathBuf::from(home_dir).join("AppData/Local/hermes"));
    }

    homes
}

#[derive(Debug, Deserialize, Default)]
struct CrushProjectList {
    #[serde(default)]
    projects: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct CrushProject {
    path: String,
    data_dir: String,
}

/// Discover every OpenCode SQLite database under the opencode data dir.
///
/// Matches:
/// - `opencode.db` (default, used by `latest`/`beta` channels or when
///   `OPENCODE_DISABLE_CHANNEL_DB=1` is set)
/// - `opencode-<channel>.db` where `<channel>` is the sanitized channel name
///   opencode bakes into the build (e.g. `stable`, `nightly`). Upstream
///   sanitizes channels with `/[^a-zA-Z0-9._-]/g -> "-"`, so the suffix we
///   accept here mirrors that character class exactly.
///
/// Ignores WAL/SHM sidecar files (`opencode.db-wal`, `opencode.db-shm`, etc.)
/// and anything that does not end in `.db`.
///
/// Returns a sorted, deterministic list for stable downstream behavior.
pub(crate) fn discover_opencode_dbs(data_dir: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut dbs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_file() {
                // Could be a symlink — accept it if it resolves to a file.
                if !entry.path().is_file() {
                    return None;
                }
            }
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !is_opencode_db_filename(name) {
                return None;
            }
            Some(path)
        })
        .collect();

    dbs.sort_unstable();
    dbs
}

/// Returns true if `name` matches the opencode db naming rule:
/// `opencode.db` or `opencode-<channel>.db` with `<channel>` drawn from the
/// same `[a-zA-Z0-9._-]` character class that opencode's `getChannelPath`
/// normalizes to. Sidecar files (`.db-wal`, `.db-shm`, `.db-journal`) are
/// rejected because they do not end in `.db`.
fn is_opencode_db_filename(name: &str) -> bool {
    // Strip the trailing `.db` — reject anything else so WAL/SHM sidecars
    // (e.g. `opencode.db-wal`) are ignored.
    let stem = match name.strip_suffix(".db") {
        Some(stem) => stem,
        None => return false,
    };
    if stem == "opencode" {
        return true;
    }
    let channel = match stem.strip_prefix("opencode-") {
        Some(channel) => channel,
        None => return false,
    };
    if channel.is_empty() {
        return false;
    }
    channel
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Discover MiMo Code SQLite databases under the given data directory.
///
/// Matches `mimocode.db` and `mimocode-<channel>.db` (channel names
/// sanitized with the same `[a-zA-Z0-9._-]` character class that MiMo
/// Code's `getChannelPath` normalizes to). Ignores WAL/SHM sidecar files.
pub(crate) fn discover_micode_dbs(data_dir: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut dbs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_file() && !entry.path().is_file() {
                return None;
            }
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !is_micode_db_filename(name) {
                return None;
            }
            Some(path)
        })
        .collect();

    dbs.sort_unstable();
    dbs
}

/// Collapse a stream of discovered database paths into a sorted, duplicate-free
/// list, keeping the first spelling seen of any file.
///
/// Two paths that `canonicalize` to the same file (e.g. one scan root is a
/// symlink to another) are treated as one: the row-id dedup fallback in the
/// SQLite parsers namespaces by the *path string*, so scanning a single file
/// under two spellings would otherwise double-count any message that lacks an
/// embedded id. Paths that fail to canonicalize (missing file, permissions)
/// fall back to their literal form, so an identical spelling still dedups.
fn dedup_dbs_by_canonical_path(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut dbs = Vec::new();

    for db_path in paths {
        let key = std::fs::canonicalize(&db_path).unwrap_or_else(|_| db_path.clone());
        if seen.insert(key) {
            dbs.push(db_path);
        }
    }

    dbs.sort_unstable();
    dbs
}

/// Discover MiMo Code SQLite databases across several data directories,
/// returning a single sorted list with duplicate files removed.
///
/// MiMo Code can be reached from more than one root (the XDG data dir and
/// orca's hook sandbox), so this unions `discover_micode_dbs` over each
/// directory and collapses duplicates via `dedup_dbs_by_canonical_path`.
pub(crate) fn discover_micode_dbs_in_dirs(dirs: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    dedup_dbs_by_canonical_path(dirs.into_iter().flat_map(|dir| discover_micode_dbs(&dir)))
}

/// Discover Devin CLI `sessions.db` files from the default path and any
/// configured extra scan roots. Extra roots preserve the generic scanner's
/// behavior: a root may be the database itself or a directory containing one
/// or more `sessions.db` files.
fn discover_devin_cli_dbs(roots: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    dedup_dbs_by_canonical_path(
        roots
            .into_iter()
            .flat_map(|root| scan_directory(&root.to_string_lossy(), "sessions.db")),
    )
}

/// Returns true if `name` matches the MiMo Code db naming rule:
/// `mimocode.db` or `mimocode-<channel>.db`.
fn is_micode_db_filename(name: &str) -> bool {
    let stem = match name.strip_suffix(".db") {
        Some(stem) => stem,
        None => return false,
    };
    if stem == "mimocode" {
        return true;
    }
    let channel = match stem.strip_prefix("mimocode-") {
        Some(channel) => channel,
        None => return false,
    };
    if channel.is_empty() {
        return false;
    }
    channel
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn crush_db_path(data_dir: &Path) -> Option<PathBuf> {
    let candidate = data_dir.join("crush.db");
    candidate.is_file().then_some(candidate)
}

fn resolve_crush_data_dir(project: &CrushProject) -> PathBuf {
    let data_dir = PathBuf::from(&project.data_dir);
    if data_dir.is_absolute() {
        data_dir
    } else {
        PathBuf::from(&project.path).join(data_dir)
    }
}

fn scan_crush_registry(registry_path: &Path) -> Vec<CrushDbSource> {
    let registry = match std::fs::read_to_string(registry_path) {
        Ok(contents) => contents,
        Err(_) => return Vec::new(),
    };

    let list: CrushProjectList = match serde_json::from_str(&registry) {
        Ok(list) => list,
        Err(_) => return Vec::new(),
    };

    list.projects
        .into_iter()
        .filter_map(|project| serde_json::from_value::<CrushProject>(project).ok())
        .filter_map(|project| {
            let db_path = crush_db_path(&resolve_crush_data_dir(&project))?;
            let workspace_key = normalize_workspace_key(&project.path);
            let workspace_label = workspace_key.as_deref().and_then(workspace_label_from_key);
            Some(CrushDbSource {
                db_path,
                workspace_key,
                workspace_label,
            })
        })
        .collect()
}

/// Candidate locations for Crush's `projects.json` registry, mirroring
/// Crush's own resolution order (`internal/config/load.go::GlobalConfigData`):
/// `$CRUSH_GLOBAL_DATA` first, then `$XDG_DATA_HOME/crush`, then
/// `%LOCALAPPDATA%\crush` on Windows, then `~/.local/share/crush`.
fn crush_registry_candidates(home_dir: &str, use_env_roots: bool) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if use_env_roots {
        if let Some(global_data) =
            std::env::var_os("CRUSH_GLOBAL_DATA").filter(|value| !value.is_empty())
        {
            candidates.push(PathBuf::from(global_data).join("projects.json"));
        }
    }

    candidates.push(PathBuf::from(
        ClientId::Crush
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots),
    ));

    if cfg!(target_os = "windows") && use_env_roots {
        if let Some(local_app_data) =
            std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())
        {
            candidates.push(
                PathBuf::from(local_app_data)
                    .join("crush")
                    .join("projects.json"),
            );
        }
    }
    candidates.push(PathBuf::from(home_dir).join("AppData/Local/crush/projects.json"));

    candidates
}

fn discover_crush_dbs(home_dir: &str, use_env_roots: bool) -> Vec<CrushDbSource> {
    let mut dbs = Vec::new();
    for registry_path in crush_registry_candidates(home_dir, use_env_roots) {
        dbs.extend(scan_crush_registry(&registry_path));
    }
    dbs.sort_by(|a, b| a.db_path.cmp(&b.db_path));
    dbs.dedup_by(|a, b| a.db_path == b.db_path);
    dbs
}

fn cline_additional_vscode_task_roots(home_dir: &str, use_env_roots: bool) -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from(home_dir)
        .join("Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/tasks")];

    if cfg!(target_os = "windows") && use_env_roots {
        if let Some(app_data) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
            roots.push(
                PathBuf::from(app_data)
                    .join("Code/User/globalStorage/saoudrizwan.claude-dev/tasks"),
            );
        }
    }

    roots.push(
        PathBuf::from(home_dir)
            .join("AppData/Roaming/Code/User/globalStorage/saoudrizwan.claude-dev/tasks"),
    );
    roots.push(
        PathBuf::from(home_dir)
            .join(".vscode-server/data/User/globalStorage/saoudrizwan.claude-dev/tasks"),
    );

    roots
}

fn cline_cli_session_roots(home_dir: &str, use_env_roots: bool) -> Vec<PathBuf> {
    let home_fallback = || PathBuf::from(home_dir).join(".cline/data/sessions");

    if !use_env_roots {
        return vec![home_fallback()];
    }

    let non_blank_env_path = |name: &str| {
        std::env::var_os(name)
            .filter(|value| !value.to_string_lossy().trim().is_empty())
            .map(PathBuf::from)
    };

    if let Some(path) = non_blank_env_path("CLINE_SESSION_DATA_DIR") {
        return vec![path];
    }
    if let Some(path) = non_blank_env_path("CLINE_DATA_DIR") {
        return vec![path.join("sessions")];
    }
    if let Some(path) = non_blank_env_path("CLINE_DIR") {
        return vec![path.join("data/sessions")];
    }

    vec![home_fallback()]
}

pub fn devin_desktop_additional_roots(home_dir: &str, use_env_roots: bool) -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from(home_dir).join(".config/Devin/User/acp-events"),
        PathBuf::from(home_dir).join(".config/devin/User/acp-events"),
    ];

    if cfg!(target_os = "windows") && use_env_roots {
        if let Some(app_data) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
            roots.push(PathBuf::from(app_data).join("Devin/User/acp-events"));
        }
    }

    roots.push(PathBuf::from(home_dir).join("AppData/Roaming/Devin/User/acp-events"));

    roots
}

fn supports_extra_dir_scanning(client_id: ClientId) -> bool {
    // Kilo CLI currently loads a single SQLite DB via `scan_result.kilo_db`
    // Roo/KiloCode require local + remote and server task roots, and Crush
    // discovers SQLite DBs via the project registry rather than scanned file
    // paths. Hermes/Zed profile databases are named consistently enough for
    // `scan_directory` to find them from user-provided roots.
    !matches!(
        client_id,
        ClientId::Kilo | ClientId::Crush | ClientId::Goose
    )
}

fn push_unique_scan_task(
    tasks: &mut Vec<(ClientId, String, &'static str)>,
    seen: &mut HashSet<(ClientId, PathBuf)>,
    client_id: ClientId,
    raw_path: impl Into<PathBuf>,
) {
    push_unique_scan_task_with_pattern(tasks, seen, client_id, raw_path, client_id.data().pattern);
}

fn push_unique_scan_task_with_pattern(
    tasks: &mut Vec<(ClientId, String, &'static str)>,
    seen: &mut HashSet<(ClientId, PathBuf)>,
    client_id: ClientId,
    raw_path: impl Into<PathBuf>,
    pattern: &'static str,
) {
    let raw_path = raw_path.into();
    if raw_path.as_os_str().is_empty() {
        return;
    }

    let key = std::fs::canonicalize(&raw_path).unwrap_or_else(|_| raw_path.clone());
    if seen.insert((client_id, key)) {
        tasks.push((client_id, raw_path.to_string_lossy().to_string(), pattern));
    }
}

/// Derive the Grok home directory from a scanned root path.
///
/// The primary resolution (`ClientDef::resolve_path_with_env_strategy`) returns
/// `<home>/sessions`, so a configured alternate root may point at the home
/// itself, its `sessions` directory, or any nested workspace/session directory.
/// The nearest ancestor named `sessions` (case-insensitively) is treated as the
/// home's child in all of those cases.
fn grok_home_from_scan_root(path: &Path) -> PathBuf {
    if let Some(sessions_dir) = path.ancestors().find(|candidate| {
        candidate
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case("sessions"))
            .unwrap_or(false)
    }) {
        if let Some(grok_home) = sessions_dir.parent() {
            return grok_home.to_path_buf();
        }
    }

    path.to_path_buf()
}

/// Register the dual-source scan tasks for one Grok root.
///
/// A Grok root contributes both the legacy per-session rollups
/// (`sessions/**/updates.jsonl`) and the per-inference token breakdowns
/// (`logs/unified.jsonl`). The original root is always registered for
/// `updates.jsonl` — `extraScanPaths` rows are recursive roots, so arbitrary
/// nested layouts must keep matching — and the sibling `<home>/logs/unified.jsonl`
/// task is derived from the root via [`grok_home_from_scan_root`].
///
/// Running every Grok root — the resolved primary home and each
/// `scanner.extraScanPaths.grok` entry — through this helper keeps alternate
/// roots on the same dual-source discovery as the primary home.
fn push_grok_dual_source_scan_tasks(
    tasks: &mut Vec<(ClientId, String, &'static str)>,
    seen: &mut HashSet<(ClientId, PathBuf)>,
    root: &Path,
) {
    push_unique_scan_task(tasks, seen, ClientId::Grok, root);
    let grok_home = grok_home_from_scan_root(root);
    push_unique_scan_task_with_pattern(
        tasks,
        seen,
        ClientId::Grok,
        grok_home.join("logs").join("unified.jsonl"),
        "unified.jsonl",
    );
}

fn kiro_global_storage_roots(home_dir: &str, use_env_roots: bool) -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from(format!(
            "{}/Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent",
            home_dir
        )),
        PathBuf::from(format!(
            "{}/Library/Application Support/kiro/User/globalStorage/kiro.kiroagent",
            home_dir
        )),
        PathBuf::from(format!(
            "{}/.config/Kiro/User/globalStorage/kiro.kiroagent",
            home_dir
        )),
        PathBuf::from(format!(
            "{}/.config/kiro/User/globalStorage/kiro.kiroagent",
            home_dir
        )),
    ];

    if cfg!(target_os = "windows") {
        if use_env_roots {
            if let Some(app_data) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
                roots.push(PathBuf::from(&app_data).join("Kiro/User/globalStorage/kiro.kiroagent"));
                roots.push(PathBuf::from(&app_data).join("kiro/User/globalStorage/kiro.kiroagent"));
            }
        }

        roots.push(PathBuf::from(format!(
            "{}/AppData/Roaming/Kiro/User/globalStorage/kiro.kiroagent",
            home_dir
        )));
        roots.push(PathBuf::from(format!(
            "{}/AppData/Roaming/kiro/User/globalStorage/kiro.kiroagent",
            home_dir
        )));
    }

    roots
}

/// Merge user-configured OpenCode db paths from [`ScannerSettings`] into the
/// auto-discovered list, in-place.
///
/// Rules:
/// - Non-existent paths are silently skipped so stale config never aborts a
///   scan (the config outlives any single opencode install).
/// - WAL/SHM/journal sidecars are rejected via [`is_opencode_db_filename`].
/// - Duplicates are removed by canonicalized path comparison, so a user who
///   explicitly lists an auto-discovered db in their config does not cause
///   it to be parsed twice.
///
/// Kept as a separate helper so the unit tests can exercise the merge
/// semantics without spinning up a full `scan_all_clients` run.
pub(crate) fn merge_user_opencode_db_paths(discovered: &mut Vec<PathBuf>, extra_paths: &[PathBuf]) {
    if extra_paths.is_empty() {
        return;
    }

    // Build a canonical-path set of what we already have so we can dedup
    // against auto-discovered entries. Fall back to the raw path if
    // canonicalize fails (e.g. on a filesystem that doesn't support it),
    // which preserves the pre-canonicalization behavior without silently
    // dropping entries.
    let mut seen: HashSet<PathBuf> = discovered
        .iter()
        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
        .collect();

    for raw in extra_paths {
        if !raw.is_file() {
            // Stale config or wrong path — silently skip.
            continue;
        }
        let Some(name) = raw.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !is_opencode_db_filename(name) {
            // Reject sidecars (`.db-wal`, `.db-shm`) and anything that does
            // not match the upstream channel-db naming rule.
            continue;
        }
        let canonical = std::fs::canonicalize(raw).unwrap_or_else(|_| raw.clone());
        if seen.insert(canonical) {
            discovered.push(raw.clone());
        }
    }
}

fn discover_copilot_vscode_sessions(home_dir: &str, use_env_roots: bool) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    roots.push(PathBuf::from(format!(
        "{}/Library/Application Support/Code/User/workspaceStorage",
        home_dir
    )));
    roots.push(PathBuf::from(format!(
        "{}/.config/Code/User/workspaceStorage",
        home_dir
    )));

    if cfg!(target_os = "windows") && use_env_roots {
        if let Some(app_data) = std::env::var_os("APPDATA").filter(|v| !v.is_empty()) {
            roots.push(PathBuf::from(app_data).join("Code/User/workspaceStorage"));
        }
    }
    roots.push(PathBuf::from(home_dir).join("AppData/Roaming/Code/User/workspaceStorage"));

    let mut files: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for workspace_storage in &roots {
        let hash_dirs = match std::fs::read_dir(workspace_storage) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in hash_dirs.filter_map(|e| e.ok()) {
            let chat_sessions_dir = entry.path().join("chatSessions");
            if !chat_sessions_dir.is_dir() {
                continue;
            }
            let chat_entries = match std::fs::read_dir(&chat_sessions_dir) {
                Ok(rd) => rd,
                Err(_) => continue,
            };
            for chat_entry in chat_entries.filter_map(|e| e.ok()) {
                let path = chat_entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !name.ends_with(".jsonl") {
                    continue;
                }
                if !path.is_file() {
                    continue;
                }
                let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if seen.insert(key) {
                    files.push(path);
                }
            }
        }
    }

    files.sort_unstable();
    files
}

/// Scan all session client directories in parallel, with user-controlled
/// [`ScannerSettings`] merged in.
///
/// This is the preferred entry point when you have loaded persistent
/// settings (e.g. from `~/.config/tokscale/settings.json`). Thin wrappers
/// [`scan_all_clients_with_env_strategy`] and [`scan_all_clients`] call
/// into this with `ScannerSettings::default()` for callers that don't care
/// about the persistent config.
pub fn scan_all_clients_with_scanner_settings(
    home_dir: &str,
    clients: &[String],
    use_env_roots: bool,
    scanner_settings: &ScannerSettings,
) -> ScanResult {
    scan_all_clients_with_env_strategy_inner(home_dir, clients, use_env_roots, scanner_settings)
}

/// Scan all session client directories in parallel
pub fn scan_all_clients_with_env_strategy(
    home_dir: &str,
    clients: &[String],
    use_env_roots: bool,
) -> ScanResult {
    scan_all_clients_with_scanner_settings(
        home_dir,
        clients,
        use_env_roots,
        &ScannerSettings::default(),
    )
}

/// Keep the V2 copy of a Cherry Studio session file that exists under both
/// transcript roots, filling in the V1-only leftovers. `files` carries the
/// `(is_v2_root, path_relative_to_root, absolute_path)` triples the aggregation
/// pass collects; a same-name session exists in both roots because Cherry
/// Studio migrated the active transcripts to `Data/Agents/.claude/projects`
/// (V2) while the legacy `CherryStudio/.claude/projects` (V1) root keeps the
/// untransferred history. V2 receives new writes, so it wins on a conflict.
fn is_cherrystudio_v2_root(root: &Path) -> bool {
    root.components()
        .rev()
        .zip(["projects", ".claude", "Agents", "Data"])
        .all(|(component, expected)| component.as_os_str().eq_ignore_ascii_case(expected))
        && root.components().count() >= 4
}

fn dedupe_cherrystudio_transcripts(files: Vec<(bool, String, PathBuf)>) -> Vec<PathBuf> {
    let v2_keys: HashSet<&String> = files
        .iter()
        .filter(|(is_v2, _, _)| *is_v2)
        .map(|(_, rel, _)| rel)
        .collect();
    let mut out = Vec::with_capacity(files.len());
    for (is_v2, rel, path) in &files {
        if !is_v2 && v2_keys.contains(rel) {
            continue;
        }
        out.push(path.clone());
    }
    out
}

fn scan_all_clients_with_env_strategy_inner(
    home_dir: &str,
    clients: &[String],
    use_env_roots: bool,
    scanner_settings: &ScannerSettings,
) -> ScanResult {
    let mut result = ScanResult::default();

    let include_all = clients.is_empty();
    let include_synthetic = include_all || clients.iter().any(|s| s == "synthetic");

    let enabled: HashSet<ClientId> = if include_all || include_synthetic {
        ClientId::iter().collect()
    } else {
        clients
            .iter()
            .filter_map(|s| {
                ClientId::from_str(s).or_else(|| {
                    // "9Router" is a gjc-format bridge client overseen by the
                    // 9Router bridge script. Map it to Gjc so the scanner
                    // discovers files under gjc scan roots.
                    if s.eq_ignore_ascii_case("9router") {
                        Some(ClientId::Gjc)
                    } else {
                        None
                    }
                })
            })
            .collect()
    };

    // Desktop ACP filenames need Devin CLI database titles to recover their
    // session/model/workspace metadata. Treat configured CLI roots as lookup
    // inputs for a Desktop-only scan without enabling CLI usage output.
    let mut enabled_with_devin_lookup = enabled.clone();
    if enabled.contains(&ClientId::DevinDesktop) {
        enabled_with_devin_lookup.insert(ClientId::DevinCli);
    }

    let headless_roots = headless_roots_with_env_strategy(home_dir, use_env_roots);

    // Define scan tasks
    let mut tasks: Vec<(ClientId, String, &str)> = Vec::new();
    let mut seen_scan_roots: HashSet<(ClientId, PathBuf)> = HashSet::new();
    let mut devin_cli_roots: Vec<PathBuf> = Vec::new();

    for client_id in &enabled {
        if matches!(
            client_id,
            ClientId::OpenCode
                | ClientId::Codex
                | ClientId::Mcode
                | ClientId::OpenClaw
                | ClientId::RooCode
                | ClientId::KiloCode
                | ClientId::Cline
                | ClientId::Kilo
                | ClientId::Hermes
                | ClientId::Goose
                | ClientId::Zed
                | ClientId::Crush
                | ClientId::Codebuff
                | ClientId::Freebuff
                | ClientId::Kimi
                | ClientId::Gjc
                | ClientId::MiMoCode
                | ClientId::DevinCli
                | ClientId::Grok
                | ClientId::PrimeAgent
        ) {
            continue;
        }

        let def = client_id.data();
        let path = def.resolve_path_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task(&mut tasks, &mut seen_scan_roots, *client_id, path);
    }

    if enabled.contains(&ClientId::Grok) {
        let grok_sessions = PathBuf::from(
            ClientId::Grok
                .data()
                .resolve_path_with_env_strategy(home_dir, use_env_roots),
        );
        // The resolved path is `<home>/sessions`; `push_grok_dual_source_scan_tasks`
        // keeps it registered for `updates.jsonl` and derives `logs/unified.jsonl`.
        push_grok_dual_source_scan_tasks(&mut tasks, &mut seen_scan_roots, &grok_sessions);
    }

    for (client_id, path) in extra_scan_paths_for(scanner_settings, &enabled_with_devin_lookup) {
        warn_if_escapes_home(Path::new(home_dir), client_id, &path);
        if client_id == ClientId::DevinCli {
            devin_cli_roots.push(path);
        } else if client_id == ClientId::Grok {
            push_grok_dual_source_scan_tasks(&mut tasks, &mut seen_scan_roots, &path);
        } else {
            push_unique_scan_task(&mut tasks, &mut seen_scan_roots, client_id, path);
        }
    }

    for (client_id, path) in built_in_extra_scan_paths_for(home_dir, &enabled, use_env_roots) {
        push_unique_scan_task(&mut tasks, &mut seen_scan_roots, client_id, path);
    }

    if enabled.contains(&ClientId::CodeBuddy) {
        let home_path = PathBuf::from(home_dir);
        let mut codebuddy_log_roots = vec![(
            home_path
                .join("AppData")
                .join("Local")
                .join("CodeBuddyExtension")
                .join("Logs"),
            "*.log",
        )];
        let roaming_codebuddy_roots = [
            home_path
                .join("AppData")
                .join("Roaming")
                .join("CodeBuddy CN")
                .join("logs"),
            home_path
                .join("AppData")
                .join("Roaming")
                .join("Code")
                .join("logs"),
        ];
        codebuddy_log_roots.extend(
            roaming_codebuddy_roots
                .into_iter()
                .map(|root| (root, "codebuddy-extension-log")),
        );
        if use_env_roots {
            if let Some(local_app_data) = dirs::data_local_dir() {
                codebuddy_log_roots.push((
                    local_app_data.join("CodeBuddyExtension").join("Logs"),
                    "*.log",
                ));
            }
            if let Some(roaming_app_data) = dirs::config_dir() {
                codebuddy_log_roots.push((
                    roaming_app_data.join("CodeBuddy CN").join("logs"),
                    "codebuddy-extension-log",
                ));
                codebuddy_log_roots.push((
                    roaming_app_data.join("Code").join("logs"),
                    "codebuddy-extension-log",
                ));
            }
        }

        for (log_root, pattern) in codebuddy_log_roots {
            if pattern == "*.log" {
                for root in ["CodeBuddyIDE", "VSCode"] {
                    push_unique_scan_task_with_pattern(
                        &mut tasks,
                        &mut seen_scan_roots,
                        ClientId::CodeBuddy,
                        log_root.join(root),
                        pattern,
                    );
                }
                continue;
            }

            push_unique_scan_task_with_pattern(
                &mut tasks,
                &mut seen_scan_roots,
                ClientId::CodeBuddy,
                log_root,
                pattern,
            );
        }
    }

    if enabled.contains(&ClientId::WorkBuddy) {
        push_unique_scan_task_with_pattern(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::WorkBuddy,
            PathBuf::from(home_dir).join(".workbuddy/projects"),
            "*.jsonl",
        );
    }

    // Extra scan directories are part of the caller's environment, so they are
    // intentionally ignored when an explicit --home override disables env roots.
    if use_env_roots {
        let extra_dirs_val = std::env::var("TOKSCALE_EXTRA_DIRS").unwrap_or_default();
        for (client_id, path) in parse_extra_dirs(&extra_dirs_val, &enabled_with_devin_lookup) {
            warn_if_escapes_home(Path::new(home_dir), client_id, &PathBuf::from(&path));
            if client_id == ClientId::DevinCli {
                devin_cli_roots.push(PathBuf::from(path));
            } else if client_id == ClientId::Grok {
                push_grok_dual_source_scan_tasks(
                    &mut tasks,
                    &mut seen_scan_roots,
                    &PathBuf::from(path),
                );
            } else {
                push_unique_scan_task(&mut tasks, &mut seen_scan_roots, client_id, path);
            }
        }
    }

    if enabled.contains(&ClientId::OpenCode) {
        let xdg_data = if use_env_roots {
            std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| join_native(home_dir, ".local/share"))
        } else {
            join_native(home_dir, ".local/share")
        };

        // OpenCode 1.2+: SQLite database(s) at ~/.local/share/opencode/opencode*.db
        //
        // opencode picks its db filename at build time based on the release
        // channel: `latest`/`beta` use `opencode.db`, other channels use
        // `opencode-<channel>.db` (e.g. `opencode-stable.db`). A single user
        // can run multiple channels side by side, so we pick up every match
        // under the data dir. See `getChannelPath` in
        // opencode/packages/opencode/src/storage/db.ts for the source of
        // the naming rule.
        let opencode_data_dir = PathBuf::from(join_native(&xdg_data, "opencode"));
        result.opencode_dbs = discover_opencode_dbs(&opencode_data_dir);

        // Merge user-configured `scanner.opencodeDbPaths` here, INSIDE the
        // `enabled.contains(&ClientId::OpenCode)` guard, so a request like
        // `tokscale --claude` does not pull in OpenCode dbs the user pinned
        // for unrelated reasons. Inflated OpenCode `counts` and wasted
        // SQLite parsing work otherwise sneak past the message-level
        // client filter that runs much later in the pipeline.
        merge_user_opencode_db_paths(
            &mut result.opencode_dbs,
            &scanner_settings.opencode_db_paths,
        );
        result.opencode_dbs.sort_unstable();
        result.opencode_dbs.dedup();

        // OpenCode legacy: JSON files at ~/.local/share/opencode/storage/message/*/*.json
        let opencode_path = ClientId::OpenCode
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        result.opencode_json_dir = Some(PathBuf::from(&opencode_path));
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::OpenCode,
            opencode_path,
        );
    }

    // MiMo Code: SQLite database(s). The primary location is the XDG data dir
    // (`~/.local/share/mimocode/mimocode*.db`). MiMo Code driven through orca's
    // hook sandbox additionally writes to
    // `~/Library/Application Support/orca/mimocode-hooks/shared/data/`, and that
    // copy can hold sessions the XDG copy is missing (scanning only XDG then
    // undercounts). Scan both so the totals are the union; the cross-file dedup
    // in the parse loop (keyed on the globally unique embedded message id)
    // collapses any message present in both locations, so overlapping data is
    // never double-counted.
    if enabled.contains(&ClientId::MiMoCode) {
        // Derive the primary data dir from the client metadata so the scan path
        // stays in sync with `ClientId::MiMoCode` (XdgData root + `mimocode`)
        // rather than duplicating it here.
        let micode_data_dir = PathBuf::from(
            ClientId::MiMoCode
                .data()
                .resolve_path_with_env_strategy(home_dir, use_env_roots),
        );
        let orca_data_dir = PathBuf::from(format!(
            "{}/Library/Application Support/orca/mimocode-hooks/shared/data",
            home_dir
        ));
        result.micode_dbs = discover_micode_dbs_in_dirs([micode_data_dir, orca_data_dir]);
    }

    if enabled.contains(&ClientId::Kimi) {
        // Legacy Kimi (KIMI CLI): ~/.kimi/sessions/**/wire.jsonl
        let kimi_path = ClientId::Kimi
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task(&mut tasks, &mut seen_scan_roots, ClientId::Kimi, kimi_path);

        // Kimi Code: ~/.kimi-code/sessions/**/wire.jsonl (supports KIMI_CODE_HOME).
        // Blank exports are treated as unset — matching PathRoot::EnvVar — because
        // joining "sessions" onto "" yields the root-level /sessions, which both
        // hides the real sessions and points the walker at an unrelated directory.
        // Non-blank values are passed through verbatim, leading/trailing space
        // included, since a path may legitimately contain it.
        let kimi_code_home = if use_env_roots {
            let configured = std::env::var("KIMI_CODE_HOME").unwrap_or_default();
            if configured.trim().is_empty() {
                join_native(home_dir, ".kimi-code")
            } else {
                configured
            }
        } else {
            join_native(home_dir, ".kimi-code")
        };
        let kimi_code_path = join_native(&kimi_code_home, "sessions");
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::Kimi,
            kimi_code_path,
        );
    }

    if enabled.contains(&ClientId::Codex) {
        // Codex: ~/.codex/sessions/**/*.jsonl
        let codex_home = if use_env_roots {
            std::env::var("CODEX_HOME").unwrap_or_else(|_| join_native(home_dir, ".codex"))
        } else {
            join_native(home_dir, ".codex")
        };
        let codex_path = ClientId::Codex
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::Codex,
            codex_path,
        );

        // Codex archived sessions: ~/.codex/archived_sessions/**/*.jsonl
        let codex_archived_path = join_native(&codex_home, "archived_sessions");
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::Codex,
            codex_archived_path,
        );

        // Codex headless: <headless_root>/codex/*.jsonl
        for root in &headless_roots {
            push_unique_scan_task(
                &mut tasks,
                &mut seen_scan_roots,
                ClientId::Codex,
                root.join("codex"),
            );
        }
    }

    if enabled.contains(&ClientId::Mcode) {
        // MiniMax Code headless streams: <headless_root>/mcode/*.jsonl
        for root in &headless_roots {
            push_unique_scan_task(
                &mut tasks,
                &mut seen_scan_roots,
                ClientId::Mcode,
                root.join("mcode"),
            );
        }
    }

    if enabled.contains(&ClientId::OpenClaw) {
        // OpenClaw transcripts: ~/.openclaw/agents/**/*.jsonl
        let openclaw_path = ClientId::OpenClaw
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::OpenClaw,
            openclaw_path,
        );

        // Legacy paths (Clawd -> Moltbot -> OpenClaw rebrand history)
        let clawdbot_path = join_native(home_dir, ".clawdbot/agents");
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::OpenClaw,
            clawdbot_path,
        );

        let moltbot_path = join_native(home_dir, ".moltbot/agents");
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::OpenClaw,
            moltbot_path,
        );

        let moldbot_path = join_native(home_dir, ".moldbot/agents");
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::OpenClaw,
            moldbot_path,
        );
    }

    // Oh My Pi fork (https://github.com/can1357/oh-my-pi) — same JSONL format, different root
    if enabled.contains(&ClientId::Pi) {
        let omp_path = join_native(home_dir, ".omp/agent/sessions");
        push_unique_scan_task(&mut tasks, &mut seen_scan_roots, ClientId::Pi, omp_path);
    }

    if enabled.contains(&ClientId::PrimeAgent) {
        // Prime Agent lets the session directory move independently from its
        // agent directory. Its RLM child session tree is always a sibling of
        // the effective session directory (`dirname(sessions)/session-artifacts`).
        let [sessions, artifacts] =
            prime_agent_session_roots_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task_with_pattern(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::PrimeAgent,
            sessions,
            "prime-agent-session",
        );
        push_unique_scan_task_with_pattern(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::PrimeAgent,
            artifacts,
            "prime-agent-session",
        );
    }

    if include_synthetic {
        let xdg_data = if use_env_roots {
            std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| join_native(home_dir, ".local/share"))
        } else {
            join_native(home_dir, ".local/share")
        };
        let octofriend_db_path = PathBuf::from(join_native(&xdg_data, "octofriend/sqlite.db"));
        if octofriend_db_path.exists() {
            result.synthetic_db = Some(octofriend_db_path);
        }
    }

    if enabled.contains(&ClientId::RooCode) {
        let local_path = ClientId::RooCode
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::RooCode,
            local_path,
        );

        let server_path = format!(
            "{}/.vscode-server/data/User/globalStorage/rooveterinaryinc.roo-cline/tasks",
            home_dir
        );
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::RooCode,
            server_path,
        );
    }

    if enabled.contains(&ClientId::KiloCode) {
        let local_path = ClientId::KiloCode
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::KiloCode,
            local_path,
        );

        let server_path = format!(
            "{}/.vscode-server/data/User/globalStorage/kilocode.kilo-code/tasks",
            home_dir
        );
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::KiloCode,
            server_path,
        );
    }

    if enabled.contains(&ClientId::Cline) {
        let local_path = ClientId::Cline
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::Cline,
            local_path,
        );

        for root in cline_additional_vscode_task_roots(home_dir, use_env_roots) {
            push_unique_scan_task(&mut tasks, &mut seen_scan_roots, ClientId::Cline, root);
        }

        for root in cline_cli_session_roots(home_dir, use_env_roots) {
            push_unique_scan_task_with_pattern(
                &mut tasks,
                &mut seen_scan_roots,
                ClientId::Cline,
                root,
                "cline-cli-messages",
            );
        }
    }

    if enabled.contains(&ClientId::DevinDesktop) {
        let local_path = ClientId::DevinDesktop
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::DevinDesktop,
            local_path,
        );

        for root in devin_desktop_additional_roots(home_dir, use_env_roots) {
            push_unique_scan_task(
                &mut tasks,
                &mut seen_scan_roots,
                ClientId::DevinDesktop,
                root,
            );
        }
    }

    if enabled.contains(&ClientId::Kilo) {
        let kilo_db_path = ClientId::Kilo
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        if std::path::Path::new(&kilo_db_path).exists() {
            result.kilo_db = Some(PathBuf::from(kilo_db_path));
        }
    }

    if enabled.contains(&ClientId::DevinCli) || enabled.contains(&ClientId::DevinDesktop) {
        let devin_db_path = ClientId::DevinCli
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        devin_cli_roots.push(PathBuf::from(devin_db_path));
        result.devin_dbs = discover_devin_cli_dbs(devin_cli_roots);
    }

    if enabled.contains(&ClientId::Hermes) {
        // Scan each candidate Hermes home (primary root plus native Windows
        // fallbacks). The first candidate whose `state.db` exists becomes the
        // primary `hermes_db`; every other default/profile db is collected as an
        // extra path. Profile-scoped homes contribute only their own profile
        // (see `discover_hermes_profile_state_dbs`).
        let mut extra_dbs: Vec<PathBuf> = Vec::new();
        for hermes_home in hermes_home_candidates(home_dir, use_env_roots) {
            let default_db = hermes_home.join("state.db");
            if default_db.is_file() {
                if result.hermes_db.is_none() {
                    result.hermes_db = Some(default_db);
                } else if result.hermes_db.as_ref() != Some(&default_db) {
                    extra_dbs.push(default_db);
                }
            }
            extra_dbs.extend(discover_hermes_profile_state_dbs(&hermes_home));
        }
        extra_dbs.sort_unstable();
        extra_dbs.dedup();
        result.get_mut(ClientId::Hermes).extend(extra_dbs);
    }

    if enabled.contains(&ClientId::Goose) {
        if use_env_roots {
            if let Ok(custom_root) = std::env::var("GOOSE_PATH_ROOT") {
                let trimmed = custom_root.trim();
                if !trimmed.is_empty() {
                    let custom_path = PathBuf::from(trimmed).join("data/sessions/sessions.db");
                    if custom_path.is_file() {
                        result.goose_db = Some(custom_path);
                    }
                }
            }
        }
        if result.goose_db.is_none() {
            let xdg_path = ClientId::Goose
                .data()
                .resolve_path_with_env_strategy(home_dir, use_env_roots);
            let xdg = PathBuf::from(xdg_path);
            if xdg.is_file() {
                result.goose_db = Some(xdg);
            }
        }
        if result.goose_db.is_none() {
            let macos_path = PathBuf::from(format!(
                "{}/Library/Application Support/goose/sessions/sessions.db",
                home_dir
            ));
            if macos_path.is_file() {
                result.goose_db = Some(macos_path);
            }
        }
        if result.goose_db.is_none() {
            let legacy_macos_path = PathBuf::from(format!(
                "{}/Library/Application Support/Block/goose/sessions/sessions.db",
                home_dir
            ));
            if legacy_macos_path.is_file() {
                result.goose_db = Some(legacy_macos_path);
            }
        }
        if result.goose_db.is_none() {
            let legacy_xdg_path = PathBuf::from(format!(
                "{}/.local/share/Block/goose/sessions/sessions.db",
                home_dir
            ));
            if legacy_xdg_path.is_file() {
                result.goose_db = Some(legacy_xdg_path);
            }
        }
    }

    if enabled.contains(&ClientId::Zed) {
        let zed_db_path = ClientId::Zed
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        let xdg = PathBuf::from(zed_db_path);
        if xdg.is_file() {
            result.zed_db = Some(xdg);
        }
        #[cfg(target_os = "macos")]
        if result.zed_db.is_none() {
            let macos_path = PathBuf::from(format!(
                "{}/Library/Application Support/Zed/threads/threads.db",
                home_dir
            ));
            if macos_path.is_file() {
                result.zed_db = Some(macos_path);
            }
        }
        if !use_env_roots && result.zed_db.is_none() {
            let windows_path = PathBuf::from(home_dir).join("AppData/Local/Zed/threads/threads.db");
            if windows_path.is_file() {
                result.zed_db = Some(windows_path);
            }
        }
        #[cfg(target_os = "windows")]
        if use_env_roots && result.zed_db.is_none() {
            if let Some(local_app_data) = dirs::data_local_dir() {
                let windows_path = local_app_data.join("Zed/threads/threads.db");
                if windows_path.is_file() {
                    result.zed_db = Some(windows_path);
                }
            }
        }
    }

    if enabled.contains(&ClientId::Crush) {
        result.crush_dbs = discover_crush_dbs(home_dir, use_env_roots);
    }

    if enabled.contains(&ClientId::Zcode) {
        let zcode_db_path = PathBuf::from(join_native(home_dir, ".zcode/cli/db/db.sqlite"));
        if zcode_db_path.is_file() {
            result.zcode_db = Some(zcode_db_path);
        }
    }

    if enabled.contains(&ClientId::CherryStudio) {
        let cherry_projects = ClientId::CherryStudio
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task_with_pattern(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::CherryStudio,
            cherry_projects.clone(),
            "*.jsonl",
        );
        // Cherry Studio V2 moved the Claude Code transcripts under
        // `<config_dir>/CherryStudio/Data/Agents/.claude/projects`; the legacy
        // V1 location keeps the untransferred history. Scan both roots and
        // dedupe by relative path below (V2 wins for same-name sessions).
        let cherry_v2_root = Path::new(&cherry_projects)
            .parent()
            .and_then(Path::parent)
            .map(|base| {
                base.join("Data")
                    .join("Agents")
                    .join(".claude")
                    .join("projects")
            });
        if let Some(cherry_v2) = cherry_v2_root {
            push_unique_scan_task_with_pattern(
                &mut tasks,
                &mut seen_scan_roots,
                ClientId::CherryStudio,
                cherry_v2,
                "*.jsonl",
            );
        }
    }

    if enabled.contains(&ClientId::Kiro) {
        let kiro_cli_path = ClientId::Kiro
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task_with_pattern(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::Kiro,
            kiro_cli_path,
            "*.json",
        );

        for root in kiro_global_storage_roots(home_dir, use_env_roots) {
            push_unique_scan_task_with_pattern(
                &mut tasks,
                &mut seen_scan_roots,
                ClientId::Kiro,
                root,
                "kiro-globalstorage",
            );
        }

        // Kiro IDE (VS Code-based) writes per-workspace sessions under
        // ~/.kiro/sessions/<workspace>/sess_<uuid>/ (session.json + messages.jsonl),
        // NOT the ~/.kiro/sessions/cli/*.json layout the base client path targets.
        // Scan the sessions root and match session.json inside sess_* dirs. This
        // resolves via home_dir on Windows too (Kiro IDE uses ~/.kiro there).
        let kiro_ide_sessions_root = PathBuf::from(join_native(home_dir, ".kiro/sessions"));
        push_unique_scan_task_with_pattern(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::Kiro,
            kiro_ide_sessions_root,
            "kiro-ide-session",
        );

        let xdg_path = PathBuf::from(join_native(home_dir, ".local/share/kiro-cli/data.sqlite3"));
        if xdg_path.is_file() {
            result.kiro_db = Some(xdg_path);
        }
        if result.kiro_db.is_none() {
            let macos_path = PathBuf::from(format!(
                "{}/Library/Application Support/kiro-cli/data.sqlite3",
                home_dir
            ));
            if macos_path.is_file() {
                result.kiro_db = Some(macos_path);
            }
        }
    }

    if enabled.contains(&ClientId::Codebuff) || enabled.contains(&ClientId::Freebuff) {
        // Codebuff persists per-channel chat history under
        // ~/.config/<channel>/projects/<project>/chats/<chatId>/chat-messages.json.
        // Freebuff is built on the same runtime and shares this exact directory
        // and file layout, so the two clients are fed from a single scan of the
        // same roots and the parsers partition the result. Running the scan
        // whenever either client is enabled lets a freebuff-only filter work
        // without a physical Codebuff install.
        //
        // Root resolution: an override set to a non-empty value (via
        // PathRoot::EnvVar) wins for the client that owns it; otherwise —
        // including when the env var is unset *or* set to an empty/whitespace
        // string — the three known channel roots are walked:
        //   - ~/.config/manicode (primary / legacy name — Codebuff was "Manicode")
        //   - ~/.config/manicode-dev
        //   - ~/.config/manicode-staging
        fn env_override(var: &str, use_env_roots: bool) -> Option<String> {
            if !use_env_roots {
                return None;
            }
            std::env::var(var)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        }
        // No `trim_end_matches('/')`: `PathBuf` already collapses a trailing
        // separator, and trimming empties a root of `/` — after which `push`
        // produces a cwd-relative `projects` instead of `/projects`.
        fn manicode_roots(home_dir: &str, override_root: Option<&str>) -> Vec<String> {
            match override_root {
                Some(root) => vec![join_native(root, "projects")],
                None => {
                    let config_dir = join_native(home_dir, ".config");
                    ["manicode", "manicode-dev", "manicode-staging"]
                        .iter()
                        .map(|channel| join_native(&config_dir, &format!("{channel}/projects")))
                        .collect()
                }
            }
        }

        let codebuff_override = env_override("CODEBUFF_DATA_DIR", use_env_roots);
        // Freebuff falls back to the location Codebuff resolves to, not to the
        // default channel roots. The two products share one tree, so
        // CODEBUFF_DATA_DIR redirects it for both; re-deriving the defaults
        // here would re-scan ~/.config/manicode behind a user who deliberately
        // pointed CODEBUFF_DATA_DIR elsewhere, defeating its exclusivity in any
        // run that has both clients enabled.
        let freebuff_override =
            env_override("FREEBUFF_DATA_DIR", use_env_roots).or_else(|| codebuff_override.clone());

        let mut codebuff_roots: Vec<String> = Vec::new();
        if enabled.contains(&ClientId::Codebuff) {
            codebuff_roots.extend(manicode_roots(home_dir, codebuff_override.as_deref()));
        }
        if enabled.contains(&ClientId::Freebuff) {
            codebuff_roots.extend(manicode_roots(home_dir, freebuff_override.as_deref()));
        }

        // `push_unique_scan_task` already dedups by canonicalized path, which
        // collapses the overlap between the two clients' root lists.
        for root in codebuff_roots {
            push_unique_scan_task(&mut tasks, &mut seen_scan_roots, ClientId::Codebuff, root);
        }
    }

    if enabled.contains(&ClientId::Gjc) {
        // gajae-code (gjc) persists sessions as
        // <agent-dir>/sessions/<project-slug>/*.jsonl, with depth-2 per-pass
        // sub-agent children <slug>/<session>/N-*.jsonl. scan_directory's
        // WalkDir + "*.jsonl" suffix match covers both depths.
        //
        // The agent dir is resolved under several env overrides gjc honors,
        // plus the Linux/macOS $XDG_DATA_HOME/gjc redirect (which FLATTENS the
        // `agent/` segment to `<xdg>/gjc/sessions`). Binding note N4: push
        // EVERY resolved root that exists (NOT first-match), letting the
        // cross-directory file dedup collapse overlap — first-match could read
        // a wrong empty root when the XDG redirect is the populated one.
        // Everything is gated on use_env_roots so `--home` disables overrides.
        let mut gjc_roots: Vec<PathBuf> = Vec::new();

        // (1) GJC_CODING_AGENT_DIR/sessions (the PathRoot::EnvVar default also
        // resolves here; existence-gated push + dedup keep it single).
        let agent_dir_root = ClientId::Gjc
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        gjc_roots.push(PathBuf::from(agent_dir_root));

        if use_env_roots {
            // (2) GJC_CONFIG_DIR / PI_CONFIG_DIR joined with agent/sessions.
            for var in ["GJC_CONFIG_DIR", "PI_CONFIG_DIR"] {
                if let Ok(config_dir) = std::env::var(var) {
                    let trimmed = config_dir.trim();
                    if !trimmed.is_empty() {
                        gjc_roots.push(
                            PathBuf::from(trimmed.trim_end_matches('/')).join("agent/sessions"),
                        );
                    }
                }
            }

            // (3) $XDG_DATA_HOME/gjc/sessions — the redirect flattens `agent/`.
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
                let trimmed = xdg_data.trim();
                if !trimmed.is_empty() {
                    gjc_roots
                        .push(PathBuf::from(trimmed.trim_end_matches('/')).join("gjc/sessions"));
                }
            }
        }

        // (4) ~/.gjc/agent/sessions home fallback (always available).
        gjc_roots.push(PathBuf::from(join_native(home_dir, ".gjc/agent/sessions")));

        for root in gjc_roots {
            if root.exists() {
                push_unique_scan_task(&mut tasks, &mut seen_scan_roots, ClientId::Gjc, root);
            }
        }
    }

    // Execute scans in parallel
    let scan_results: Vec<(ClientId, String, Vec<PathBuf>)> = tasks
        .into_par_iter()
        .map(|(client_id, path, pattern)| {
            let files = scan_directory(&path, pattern);
            (client_id, path, files)
        })
        .collect();

    // Aggregate results, deduplicating canonical file paths across overlapping
    // roots while preserving one copy per client. Cherry Studio's V1 and V2
    // roots intentionally retain the same relative transcript names, so defer
    // its path selection until V2 can win over a V1 counterpart.
    let mut seen: HashSet<(ClientId, PathBuf)> = HashSet::new();
    let mut cherry_files: Vec<(bool, String, PathBuf)> = Vec::new();
    for (client_id, root, files) in scan_results {
        for file in files {
            if client_id == ClientId::CherryStudio {
                let is_v2 = is_cherrystudio_v2_root(Path::new(&root));
                let rel = file
                    .strip_prefix(&root)
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .to_string();
                cherry_files.push((is_v2, rel, file));
            } else {
                let key = std::fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
                if seen.insert((client_id, key)) {
                    result.get_mut(client_id).push(file);
                }
            }
        }
    }
    for file in dedupe_cherrystudio_transcripts(cherry_files) {
        let key = std::fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
        if seen.insert((ClientId::CherryStudio, key)) {
            result.get_mut(ClientId::CherryStudio).push(file);
        }
    }

    if enabled.contains(&ClientId::Copilot) {
        let desktop_db = PathBuf::from(join_native(home_dir, ".copilot/data.db"));
        if desktop_db.is_file() {
            result.copilot_desktop_db = Some(desktop_db);
        }

        result.copilot_vscode_sessions = discover_copilot_vscode_sessions(home_dir, use_env_roots);

        if let Some(path) = copilot_exporter_path_with_env_strategy(use_env_roots) {
            let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if path.is_file() && seen.insert((ClientId::Copilot, key)) {
                let copilot_files = result.get_mut(ClientId::Copilot);
                copilot_files.push(path);
                copilot_files.sort_unstable();
            }
        }
    }

    result
}

pub fn scan_all_clients(home_dir: &str, clients: &[String]) -> ScanResult {
    scan_all_clients_with_env_strategy(home_dir, clients, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::json_path_literal;
    use crate::paths::test_env::EnvGuard;
    use serial_test::serial;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::TempDir;

    fn scan_without_extra_dirs(home_dir: &str, clients: &[String]) -> ScanResult {
        let mut extra = EnvGuard::capture(&["TOKSCALE_EXTRA_DIRS", "TOKSCALE_HEADLESS_DIR"]);
        extra.remove("TOKSCALE_EXTRA_DIRS");
        extra.remove("TOKSCALE_HEADLESS_DIR");
        scan_all_clients(home_dir, clients)
    }

    fn restore_env(var: &str, previous: Option<String>) {
        match previous {
            Some(value) => unsafe { std::env::set_var(var, value) },
            None => unsafe { std::env::remove_var(var) },
        }
    }

    fn restore_current_dir(previous: &Path) {
        std::env::set_current_dir(previous).unwrap();
    }

    struct CurrentDirGuard(PathBuf);

    impl CurrentDirGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::current_dir().unwrap();
            std::env::set_current_dir(path).unwrap();
            Self(previous)
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.0).unwrap();
        }
    }

    fn setup_mock_copilot_dir(home: &Path) {
        let sessions_dir = home.join(".copilot/otel");
        fs::create_dir_all(&sessions_dir).unwrap();
        let file_path = sessions_dir.join("copilot.jsonl");
        let mut file = File::create(file_path).unwrap();
        writeln!(file, "{{\"type\":\"span\",\"name\":\"chat gpt-5.4-mini\"}}").unwrap();
    }

    #[test]
    #[serial]
    fn test_env_guard_restores_after_unwind() {
        const KEY: &str = "TOKSCALE_SCANNER_ENV_GUARD_SELF_CHECK";
        let mut outer = EnvGuard::capture(&[KEY]);
        outer.set(KEY, "before");
        let result = std::panic::catch_unwind(|| {
            let mut inner = EnvGuard::capture(&[KEY]);
            inner.set(KEY, "during");
            panic!("exercise EnvGuard unwinding");
        });
        assert!(result.is_err());
        assert_eq!(std::env::var_os(KEY), Some("before".into()));
    }

    #[test]
    fn test_scan_result_total_files() {
        let mut result = ScanResult::default();
        result
            .get_mut(ClientId::OpenCode)
            .push(PathBuf::from("a.json"));
        result
            .get_mut(ClientId::OpenCode)
            .push(PathBuf::from("b.json"));
        result
            .get_mut(ClientId::Claude)
            .push(PathBuf::from("c.jsonl"));
        result
            .get_mut(ClientId::Gemini)
            .push(PathBuf::from("d.json"));
        result.get_mut(ClientId::Pi).push(PathBuf::from("e.jsonl"));
        assert_eq!(result.total_files(), 5);
    }

    #[test]
    fn test_scan_result_all_files() {
        let mut result = ScanResult::default();
        result
            .get_mut(ClientId::OpenCode)
            .push(PathBuf::from("a.json"));
        result
            .get_mut(ClientId::Claude)
            .push(PathBuf::from("b.jsonl"));
        result
            .get_mut(ClientId::Codex)
            .push(PathBuf::from("c.jsonl"));
        result
            .get_mut(ClientId::Gemini)
            .push(PathBuf::from("d.json"));
        result
            .get_mut(ClientId::Cursor)
            .push(PathBuf::from("e.csv"));
        result.get_mut(ClientId::Pi).push(PathBuf::from("f.jsonl"));

        let all = result.all_files();
        assert_eq!(all.len(), 6);
        assert_eq!(all[0], (ClientId::OpenCode, PathBuf::from("a.json")));
        assert_eq!(all[1], (ClientId::Claude, PathBuf::from("b.jsonl")));
        assert_eq!(all[2], (ClientId::Codex, PathBuf::from("c.jsonl")));
        assert_eq!(all[3], (ClientId::Cursor, PathBuf::from("e.csv")));
        assert_eq!(all[4], (ClientId::Gemini, PathBuf::from("d.json")));
        assert_eq!(all[5], (ClientId::Pi, PathBuf::from("f.jsonl")));
    }

    #[test]
    fn test_scan_result_empty() {
        let result = ScanResult::default();
        assert_eq!(result.total_files(), 0);
        assert!(result.all_files().is_empty());
    }

    #[test]
    fn test_overlapping_roots_do_not_suppress_independent_clients() {
        let dir = TempDir::new().unwrap();
        let session = dir.path().join("shared.jsonl");
        File::create(&session).unwrap();
        let mut settings = ScannerSettings::default();
        settings
            .extra_scan_paths
            .insert("pi".to_string(), vec![dir.path().to_path_buf()]);
        settings
            .extra_scan_paths
            .insert("senpi".to_string(), vec![dir.path().to_path_buf()]);

        let result = scan_all_clients_with_scanner_settings(
            dir.path().to_str().unwrap(),
            &["pi".to_string(), "senpi".to_string()],
            false,
            &settings,
        );

        assert_eq!(
            result.get(ClientId::Pi).as_slice(),
            std::slice::from_ref(&session)
        );
        assert_eq!(
            result.get(ClientId::Senpi).as_slice(),
            std::slice::from_ref(&session)
        );
        assert_eq!(result.all_files().len(), 2);
    }

    #[test]
    fn test_client_id_equality() {
        assert_eq!(ClientId::OpenCode, ClientId::OpenCode);
        assert_ne!(ClientId::OpenCode, ClientId::Claude);
    }

    #[test]
    fn test_scan_directory_json_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        // Create test files
        File::create(path.join("test1.json")).unwrap();
        File::create(path.join("test2.json")).unwrap();
        File::create(path.join("data.txt")).unwrap();
        File::create(path.join("other.jsonl")).unwrap();

        let json_files = scan_directory(path.to_str().unwrap(), "*.json");
        assert_eq!(json_files.len(), 2);
        assert!(json_files.iter().all(|p| p.extension().unwrap() == "json"));
    }

    #[test]
    fn test_scan_directory_json_or_jsonl_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        File::create(path.join("session.json")).unwrap();
        File::create(path.join("session.jsonl")).unwrap();
        File::create(path.join("session.txt")).unwrap();

        let session_files = scan_directory(path.to_str().unwrap(), "*.json|*.jsonl");
        assert_eq!(session_files.len(), 2);
        assert_eq!(
            session_files
                .iter()
                .map(|path| path.file_name().unwrap().to_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["session.json", "session.jsonl"]
        );
    }

    #[test]
    fn test_scan_directory_jsonl_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        File::create(path.join("session.jsonl")).unwrap();
        File::create(path.join("log.jsonl")).unwrap();
        File::create(path.join("data.json")).unwrap();

        let jsonl_files = scan_directory(path.to_str().unwrap(), "*.jsonl");
        assert_eq!(jsonl_files.len(), 2);
        assert!(jsonl_files
            .iter()
            .all(|p| p.extension().unwrap() == "jsonl"));
    }

    #[test]
    fn test_scan_directory_log_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        File::create(path.join("ide.log")).unwrap();
        File::create(path.join("vscode.log")).unwrap();
        File::create(path.join("session.jsonl")).unwrap();

        let log_files = scan_directory(path.to_str().unwrap(), "*.log");
        assert_eq!(log_files.len(), 2);
        assert!(log_files.iter().all(|p| p.extension().unwrap() == "log"));
    }

    #[test]
    fn test_scan_directory_workbuddy_db_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        File::create(path.join("workbuddy.db")).unwrap();
        File::create(path.join("workbuddy.db-wal")).unwrap();
        File::create(path.join("workbuddy.db-shm")).unwrap();

        let db_files = scan_directory(path.to_str().unwrap(), "workbuddy.db");

        assert_eq!(db_files, vec![path.join("workbuddy.db")]);
    }

    #[test]
    fn test_scan_directory_updates_jsonl_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();
        let session_dir = path.join("workspace/session-1");
        fs::create_dir_all(&session_dir).unwrap();

        File::create(session_dir.join("updates.jsonl")).unwrap();
        File::create(session_dir.join("events.jsonl")).unwrap();
        File::create(session_dir.join("updates.json")).unwrap();

        let updates_files = scan_directory(path.to_str().unwrap(), "updates.jsonl");
        assert_eq!(updates_files.len(), 1);
        assert!(updates_files[0].ends_with("updates.jsonl"));
    }

    #[test]
    fn test_scan_directory_session_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        File::create(path.join("session-001.json")).unwrap();
        File::create(path.join("session-abc.json")).unwrap();
        File::create(path.join("other.json")).unwrap();
        File::create(path.join("session.json")).unwrap(); // Shouldn't match

        let session_files = scan_directory(path.to_str().unwrap(), "session-*.json");
        assert_eq!(session_files.len(), 2);
        assert!(session_files.iter().all(|p| {
            let name = p.file_name().unwrap().to_str().unwrap();
            name.starts_with("session-") && name.ends_with(".json")
        }));
    }

    #[test]
    fn test_scan_directory_ui_messages_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        let tasks = path.join("tasks");
        fs::create_dir_all(tasks.join("task-a")).unwrap();
        fs::create_dir_all(tasks.join("task-b")).unwrap();
        fs::create_dir_all(tasks.join("task-c")).unwrap();

        File::create(tasks.join("task-a").join("ui_messages.json")).unwrap();
        File::create(tasks.join("task-b").join("ui_messages.json")).unwrap();
        File::create(tasks.join("task-c").join("api_conversation_history.json")).unwrap();

        let files = scan_directory(path.to_str().unwrap(), "ui_messages.json");
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|p| {
            p.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                == "ui_messages.json"
        }));
    }

    #[test]
    fn test_scan_directory_dsh_session_log() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        // DeepSeek Harness layout: sessions/<encoded-cwd>/<session-id>/session.jsonl.zstd
        let session_dir = path
            .join("sessions")
            .join("--E-Code-proj--")
            .join("session-abc-123");
        fs::create_dir_all(&session_dir).unwrap();
        File::create(session_dir.join("session.jsonl.zstd")).unwrap();

        // `compression: none` writes the same rows to `session.jsonl`; a
        // second session directory covers that spelling.
        let plain_dir = path
            .join("sessions")
            .join("--E-Code-proj--")
            .join("session-def-456");
        fs::create_dir_all(&plain_dir).unwrap();
        File::create(plain_dir.join("session.jsonl")).unwrap();

        // Non-matching siblings must be excluded: other zstd files and any
        // differently named file in the tree.
        File::create(path.join("sessions").join("other.jsonl.zstd")).unwrap();
        File::create(path.join("sessions").join("unrelated.txt")).unwrap();

        let files = scan_directory(path.to_str().unwrap(), "dsh-session-log");
        let names: Vec<&str> = files
            .iter()
            .filter_map(|file| file.file_name().and_then(|name| name.to_str()))
            .collect();
        // Byte-lexical path order: `session-abc-123` sorts before `session-def-456`.
        assert_eq!(names, vec!["session.jsonl.zstd", "session.jsonl"]);
    }

    #[test]
    fn test_scan_directory_cline_cli_messages_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        File::create(path.join("session.messages.json")).unwrap();
        File::create(path.join("session.json")).unwrap();
        File::create(path.join("session.messages.jsonl")).unwrap();

        let files = scan_directory(path.to_str().unwrap(), "cline-cli-messages");
        assert_eq!(files, vec![path.join("session.messages.json")]);
    }

    #[test]
    fn test_scan_directory_nested() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        // Create nested structure
        let sub1 = path.join("project1");
        let sub2 = path.join("project2");
        fs::create_dir_all(&sub1).unwrap();
        fs::create_dir_all(&sub2).unwrap();

        File::create(sub1.join("session.json")).unwrap();
        File::create(sub2.join("session.json")).unwrap();
        File::create(path.join("root.json")).unwrap();

        let files = scan_directory(path.to_str().unwrap(), "*.json");
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn test_scan_directory_csv_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        File::create(path.join("usage.csv")).unwrap();
        File::create(path.join("data.csv")).unwrap();
        File::create(path.join("other.json")).unwrap();

        let csv_files = scan_directory(path.to_str().unwrap(), "*.csv");
        assert_eq!(csv_files.len(), 2);
        assert!(csv_files.iter().all(|p| p.extension().unwrap() == "csv"));
    }

    #[test]
    fn test_scan_directory_usage_json_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();
        let archive = path.join("archive");
        fs::create_dir_all(&archive).unwrap();

        File::create(path.join("usage.json")).unwrap();
        File::create(path.join("usage.account.json")).unwrap();
        File::create(path.join("usage.backup-20240601.json")).unwrap();
        File::create(path.join("other.json")).unwrap();
        File::create(archive.join("usage.json")).unwrap();

        let usage_files = scan_directory(path.to_str().unwrap(), "usage*.json");
        let names: Vec<_> = usage_files
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap())
            .collect();

        assert_eq!(names, vec!["usage.account.json", "usage.json"]);
    }

    #[test]
    fn test_scan_directory_kiro_globalstorage_pattern() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        let root = path.join("Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent");
        let workspace = root.join("workspace-a");
        fs::create_dir_all(&workspace).unwrap();
        File::create(workspace.join("execution.chat")).unwrap();
        File::create(workspace.join("session.json")).unwrap();
        File::create(workspace.join("execution")).unwrap();
        File::create(workspace.join("index.sqlite")).unwrap();

        let files = scan_directory(root.to_str().unwrap(), "kiro-globalstorage");
        let names: Vec<_> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap())
            .collect();

        assert_eq!(names, vec!["execution", "execution.chat", "session.json"]);
    }

    #[test]
    fn test_scan_directory_kiro_ide_session_pattern() {
        let dir = TempDir::new().unwrap();
        let sessions_root = dir.path().join(".kiro/sessions");

        // IDE layout: <workspace>/sess_<uuid>/{session.json,messages.jsonl}.
        let sess_dir = sessions_root.join("workspace-a/sess_02f1c107");
        fs::create_dir_all(&sess_dir).unwrap();
        File::create(sess_dir.join("session.json")).unwrap();
        File::create(sess_dir.join("messages.jsonl")).unwrap();

        // CLI layout under the same tree must NOT be matched by this pattern
        // (it is scanned separately as *.json), and a stray session.json outside
        // a sess_* dir must be ignored.
        let cli_dir = sessions_root.join("cli");
        fs::create_dir_all(&cli_dir).unwrap();
        File::create(cli_dir.join("session-001.json")).unwrap();
        File::create(sessions_root.join("workspace-a/session.json")).unwrap();

        let files = scan_directory(sessions_root.to_str().unwrap(), "kiro-ide-session");
        let names: Vec<_> = files
            .iter()
            .map(|path| {
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap()
                    .to_string()
            })
            .collect();

        // Exactly one match: the session.json inside sess_02f1c107.
        assert_eq!(files.len(), 1);
        assert_eq!(names, vec!["sess_02f1c107"]);
    }

    #[test]
    fn test_scan_directory_nonexistent() {
        let files = scan_directory("/nonexistent/path/that/does/not/exist", "*.json");
        assert!(files.is_empty());
    }

    #[test]
    fn test_scan_all_clients_discovers_zcode_v2_sqlite() {
        let dir = TempDir::new().unwrap();
        let db_dir = dir.path().join(".zcode/cli/db");
        fs::create_dir_all(&db_dir).unwrap();
        let db_path = db_dir.join("db.sqlite");
        File::create(&db_path).unwrap();

        let result = scan_all_clients_with_env_strategy(
            dir.path().to_str().unwrap(),
            &["zcode".to_string()],
            false,
        );

        assert_eq!(result.zcode_db.as_deref(), Some(db_path.as_path()));
    }

    #[test]
    fn test_scan_all_clients_discovers_codebuddy_extension_logs() {
        let dir = TempDir::new().unwrap();
        let ide_dir = dir
            .path()
            .join("AppData")
            .join("Local")
            .join("CodeBuddyExtension")
            .join("Logs")
            .join("CodeBuddyIDE")
            .join("2026-07-01");
        let vscode_dir = dir
            .path()
            .join("AppData")
            .join("Local")
            .join("CodeBuddyExtension")
            .join("Logs")
            .join("VSCode")
            .join("2026-07-01");
        fs::create_dir_all(&ide_dir).unwrap();
        fs::create_dir_all(&vscode_dir).unwrap();
        let ide_log = ide_dir.join("ide.log");
        let vscode_log = vscode_dir.join("vscode.log");
        File::create(&ide_log).unwrap();
        File::create(&vscode_log).unwrap();

        let result = scan_all_clients_with_env_strategy(
            dir.path().to_str().unwrap(),
            &["codebuddy".to_string()],
            false,
        );

        let files = result.get(ClientId::CodeBuddy);
        assert_eq!(files.len(), 2);
        assert!(files.contains(&ide_log));
        assert!(files.contains(&vscode_log));
    }

    #[test]
    fn test_scan_all_clients_discovers_workbuddy_project_jsonl() {
        let dir = TempDir::new().unwrap();
        let project_dir = dir.path().join(".workbuddy/projects/project-a");
        fs::create_dir_all(&project_dir).unwrap();
        let session = project_dir.join("session.jsonl");
        File::create(&session).unwrap();

        let result = scan_all_clients_with_env_strategy(
            dir.path().to_str().unwrap(),
            &["workbuddy".to_string()],
            false,
        );

        let files = result.get(ClientId::WorkBuddy);
        assert_eq!(files.as_slice(), std::slice::from_ref(&session));
    }

    #[test]
    fn test_scan_directory_empty() {
        let dir = TempDir::new().unwrap();
        let files = scan_directory(dir.path().to_str().unwrap(), "*.json");
        assert!(files.is_empty());
    }

    #[test]
    fn test_scan_directory_deterministic_order() {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        for name in ["zebra.jsonl", "alpha.jsonl", "middle.jsonl", "beta.jsonl"] {
            File::create(path.join(name)).unwrap();
        }

        let first = scan_directory(path.to_str().unwrap(), "*.jsonl");
        let second = scan_directory(path.to_str().unwrap(), "*.jsonl");
        let third = scan_directory(path.to_str().unwrap(), "*.jsonl");

        assert_eq!(first, second, "Repeated scans must return identical order");
        assert_eq!(second, third, "Repeated scans must return identical order");

        let names: Vec<_> = first
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["alpha.jsonl", "beta.jsonl", "middle.jsonl", "zebra.jsonl"],
            "Results must be lexically sorted"
        );
    }

    fn setup_mock_opencode_dir(base: &std::path::Path) {
        let opencode_path = base.join(".local/share/opencode/storage/message/proj1");
        fs::create_dir_all(&opencode_path).unwrap();
        let mut file = File::create(opencode_path.join("msg_001.json")).unwrap();
        file.write_all(b"{}").unwrap();
    }

    fn setup_mock_claude_dir(base: &std::path::Path) {
        let claude_path = base.join(".claude/projects/myproject");
        fs::create_dir_all(&claude_path).unwrap();
        let mut file = File::create(claude_path.join("conversation.jsonl")).unwrap();
        file.write_all(b"").unwrap();
    }

    fn setup_mock_claude_transcripts_dir(base: &std::path::Path) -> PathBuf {
        let transcript_path = base.join(".claude/transcripts");
        fs::create_dir_all(&transcript_path).unwrap();
        let file_path = transcript_path.join("ses_123456789012345678901234567.jsonl");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"").unwrap();
        file_path
    }

    fn setup_mock_codex_dir(base: &std::path::Path) {
        let codex_path = base.join(".codex/sessions");
        fs::create_dir_all(&codex_path).unwrap();
        let mut file = File::create(codex_path.join("session.jsonl")).unwrap();
        file.write_all(b"").unwrap();
    }

    fn setup_mock_codex_archived_dir(base: &std::path::Path) {
        let archived_path = base.join(".codex/archived_sessions");
        fs::create_dir_all(&archived_path).unwrap();
        let mut file = File::create(archived_path.join("archived.jsonl")).unwrap();
        file.write_all(b"").unwrap();
    }

    fn setup_mock_gemini_dir(base: &std::path::Path) {
        let gemini_path = base.join(".gemini/tmp/123/chats");
        fs::create_dir_all(&gemini_path).unwrap();
        let mut file = File::create(gemini_path.join("session-abc.json")).unwrap();
        file.write_all(b"{}").unwrap();
    }

    fn setup_mock_pi_dir(base: &std::path::Path) {
        let pi_path = base.join(".pi/agent/sessions/--test--");
        fs::create_dir_all(&pi_path).unwrap();
        let mut file = File::create(pi_path.join("1733011200000_pi_ses_001.jsonl")).unwrap();
        file.write_all(b"{}").unwrap();
    }

    fn setup_mock_kimchi_dir(base: &std::path::Path) {
        let kimchi_path = base.join(".config/kimchi/harness/sessions/--test--");
        fs::create_dir_all(&kimchi_path).unwrap();
        let mut file =
            File::create(kimchi_path.join("2026-08-01T00-00-00Z_kimchi_ses_001.jsonl")).unwrap();
        file.write_all(b"{}").unwrap();
    }

    fn setup_mock_kiro_dir(base: &std::path::Path) {
        let kiro_path = base.join(".kiro/sessions/cli");
        fs::create_dir_all(&kiro_path).unwrap();
        File::create(kiro_path.join("session-001.json")).unwrap();
    }

    fn setup_mock_kiro_global_storage_dir(base: &std::path::Path) {
        let root = base.join("Library/Application Support/Kiro/User/globalStorage/kiro.kiroagent");
        let workspace = root.join("workspace-a");
        fs::create_dir_all(&workspace).unwrap();
        File::create(workspace.join("execution.chat")).unwrap();
        File::create(workspace.join("session.json")).unwrap();
        File::create(workspace.join("execution")).unwrap();
    }

    fn setup_mock_senpi_dir(base: &std::path::Path) {
        let senpi_path = base.join(".senpi/agent/sessions/--Users-someone-project--");
        fs::create_dir_all(&senpi_path).unwrap();
        let mut file = File::create(
            senpi_path.join("2026-07-29T15-19-53-436Z_019fae75-f35c-7b20-8d6f-e6dea8f7d9f5.jsonl"),
        )
        .unwrap();
        file.write_all(b"{}").unwrap();
    }

    fn setup_mock_omp_dir(base: &std::path::Path) {
        let omp_path = base.join(".omp/agent/sessions/--omp-test--");
        fs::create_dir_all(&omp_path).unwrap();
        let mut file =
            File::create(omp_path.join("2026-04-06T03-04-28Z_omp_ses_001.jsonl")).unwrap();
        file.write_all(b"{}").unwrap();
    }

    fn setup_mock_zed_xdg_db(base: &std::path::Path) -> PathBuf {
        let zed_db = base.join(".local/share/zed/threads/threads.db");
        fs::create_dir_all(zed_db.parent().unwrap()).unwrap();
        File::create(&zed_db).unwrap();
        zed_db
    }

    #[cfg(target_os = "macos")]
    fn setup_mock_zed_macos_db(base: &std::path::Path) -> PathBuf {
        let zed_db = base.join("Library/Application Support/Zed/threads/threads.db");
        fs::create_dir_all(zed_db.parent().unwrap()).unwrap();
        File::create(&zed_db).unwrap();
        zed_db
    }

    fn setup_mock_kimi_dir(base: &std::path::Path) {
        let kimi_session = base.join(".kimi/sessions/group1/session-uuid-1");
        fs::create_dir_all(&kimi_session).unwrap();
        let mut file = File::create(kimi_session.join("wire.jsonl")).unwrap();
        file.write_all(b"{\"type\": \"metadata\", \"protocol_version\": \"1.3\"}\n")
            .unwrap();
    }

    /// Kimi Code lays sessions out as
    /// `<root>/sessions/WORKSPACE/SESSION/agents/AGENT/wire.jsonl`, where
    /// `<root>` is `~/.kimi-code` or whatever KIMI_CODE_HOME points at.
    fn setup_mock_kimi_code_dir(root: &std::path::Path) -> PathBuf {
        let agent_dir = root.join("sessions/workspace-1/session-uuid-1/agents/main");
        fs::create_dir_all(&agent_dir).unwrap();
        let wire = agent_dir.join("wire.jsonl");
        let mut file = File::create(&wire).unwrap();
        file.write_all(b"{\"type\": \"metadata\", \"protocol_version\": \"1.3\"}\n")
            .unwrap();
        wire
    }

    fn setup_mock_grok_dir(base: &std::path::Path) {
        let grok_session = base.join(".grok/sessions/%2Ftmp%2Fproject/session-uuid-1");
        fs::create_dir_all(&grok_session).unwrap();
        let mut file = File::create(grok_session.join("updates.jsonl")).unwrap();
        file.write_all(b"{\"method\":\"session/update\"}\n")
            .unwrap();
    }

    #[test]
    #[serial]
    fn test_grok_extra_scan_path_discovers_both_sources() {
        // Regression guard: `scanner.extraScanPaths.grok` roots must receive
        // the same dual-source discovery as the primary Grok home — legacy
        // `updates.jsonl` under the configured root AND the sibling
        // `logs/unified.jsonl` derived from the Grok home. Previously the
        // unified log was only added for the resolved primary home, so an
        // alternate root contributed only the registered `updates.jsonl`
        // pattern and its inference breakdowns were silently missed.
        let mut env = EnvGuard::capture(&["GROK_HOME", "TOKSCALE_EXTRA_DIRS"]);
        env.remove("GROK_HOME");
        env.remove("TOKSCALE_EXTRA_DIRS");

        let home = TempDir::new().unwrap();
        let alt_home = TempDir::new().unwrap();

        // Alternate Grok home laid out like a real ~/.grok.
        let alt_session = alt_home
            .path()
            .join("sessions/%2Ftmp%2Fproject/session-alt");
        fs::create_dir_all(&alt_session).unwrap();
        File::create(alt_session.join("updates.jsonl")).unwrap();
        // A nested legacy update NOT under sessions/ — the configured root is
        // a recursive scan root, so nested updates.jsonl must keep matching
        // instead of being replaced by a sessions/ subdirectory task.
        let nested = alt_home.path().join("imports/nested");
        fs::create_dir_all(&nested).unwrap();
        File::create(nested.join("updates.jsonl")).unwrap();
        fs::create_dir_all(alt_home.path().join("logs")).unwrap();
        File::create(alt_home.path().join("logs/unified.jsonl")).unwrap();

        let settings: ScannerSettings = serde_json::from_value(serde_json::json!({
            "extraScanPaths": {
                "grok": [alt_home.path()]
            }
        }))
        .unwrap();

        let result = scan_all_clients_with_scanner_settings(
            home.path().to_str().unwrap(),
            &["grok".to_string()],
            true,
            &settings,
        );

        let files = result.get(ClientId::Grok);
        assert_eq!(
            files
                .iter()
                .filter(|p| p.ends_with("updates.jsonl"))
                .count(),
            2,
            "alternate Grok root must keep recursive updates.jsonl discovery: {files:?}"
        );
        assert!(
            files.iter().any(|p| p.ends_with("unified.jsonl")),
            "alternate Grok root must contribute logs/unified.jsonl: {files:?}"
        );
    }

    #[test]
    #[serial]
    fn test_grok_extra_scan_path_sessions_shape_discovers_unified_log() {
        // extraScanPaths.grok may point at the `sessions` subdirectory (the
        // shape the primary resolution returns) instead of the home itself;
        // the unified log is derived from the parent home either way.
        let mut env = EnvGuard::capture(&["GROK_HOME", "TOKSCALE_EXTRA_DIRS"]);
        env.remove("GROK_HOME");
        env.remove("TOKSCALE_EXTRA_DIRS");

        let home = TempDir::new().unwrap();
        let alt_home = TempDir::new().unwrap();

        let alt_session = alt_home
            .path()
            .join("sessions/%2Ftmp%2Fproject/session-alt");
        fs::create_dir_all(&alt_session).unwrap();
        File::create(alt_session.join("updates.jsonl")).unwrap();
        fs::create_dir_all(alt_home.path().join("logs")).unwrap();
        File::create(alt_home.path().join("logs/unified.jsonl")).unwrap();

        let settings: ScannerSettings = serde_json::from_value(serde_json::json!({
            "extraScanPaths": {
                "grok": [alt_home.path().join("sessions")]
            }
        }))
        .unwrap();

        let result = scan_all_clients_with_scanner_settings(
            home.path().to_str().unwrap(),
            &["grok".to_string()],
            true,
            &settings,
        );

        let files = result.get(ClientId::Grok);
        assert_eq!(
            files.len(),
            2,
            "expected updates.jsonl + unified.jsonl: {files:?}"
        );
        assert!(files.iter().any(|p| p.ends_with("updates.jsonl")));
        assert!(files.iter().any(|p| p.ends_with("unified.jsonl")));
    }

    #[test]
    #[serial]
    fn test_grok_extra_scan_path_nested_session_shape_discovers_unified_log() {
        // A root below a mixed-case `sessions` directory still belongs to the
        // surrounding Grok home, so its sibling logs/unified.jsonl is found.
        let mut env = EnvGuard::capture(&["GROK_HOME", "TOKSCALE_EXTRA_DIRS"]);
        env.remove("GROK_HOME");
        env.remove("TOKSCALE_EXTRA_DIRS");

        let home = TempDir::new().unwrap();
        let alt_home = TempDir::new().unwrap();

        let alt_session = alt_home
            .path()
            .join("Sessions/%2Ftmp%2Fproject/session-alt");
        fs::create_dir_all(&alt_session).unwrap();
        File::create(alt_session.join("updates.jsonl")).unwrap();
        fs::create_dir_all(alt_home.path().join("logs")).unwrap();
        File::create(alt_home.path().join("logs/unified.jsonl")).unwrap();

        let settings: ScannerSettings = serde_json::from_value(serde_json::json!({
            "extraScanPaths": {
                "grok": [alt_session]
            }
        }))
        .unwrap();

        let result = scan_all_clients_with_scanner_settings(
            home.path().to_str().unwrap(),
            &["grok".to_string()],
            true,
            &settings,
        );

        let files = result.get(ClientId::Grok);
        assert_eq!(
            files.len(),
            2,
            "expected updates.jsonl + unified.jsonl: {files:?}"
        );
        assert!(files.iter().any(|p| p.ends_with("updates.jsonl")));
        assert!(files.iter().any(|p| p.ends_with("unified.jsonl")));
    }

    fn setup_mock_jcode_dir(base: &std::path::Path) {
        let jcode_sessions = base.join(".jcode/sessions");
        fs::create_dir_all(&jcode_sessions).unwrap();
        File::create(jcode_sessions.join("session_fixture.json")).unwrap();
        File::create(jcode_sessions.join("not-a-session.json")).unwrap();
    }

    fn setup_mock_openclaw_dir(base: &std::path::Path) {
        // Mirror real OpenClaw layout: ~/.openclaw/agents/<agentId>/sessions/*.jsonl
        let openclaw_sessions = base.join(".openclaw/agents/main/sessions");
        fs::create_dir_all(&openclaw_sessions).unwrap();

        let mut transcript = File::create(openclaw_sessions.join("session-abc.jsonl")).unwrap();
        transcript.write_all(b"{}").unwrap();

        let mut archived_deleted =
            File::create(openclaw_sessions.join("session-deleted.jsonl.deleted.123")).unwrap();
        archived_deleted.write_all(b"{}").unwrap();

        let mut archived_reset =
            File::create(openclaw_sessions.join("session-reset.jsonl.reset.456")).unwrap();
        archived_reset.write_all(b"{}").unwrap();

        // Even if an index exists, we should count JSONL transcripts (not sessions.json only)
        let mut index = File::create(openclaw_sessions.join("sessions.json")).unwrap();
        index.write_all(b"{}").unwrap();
    }

    fn setup_mock_roocode_dir(base: &std::path::Path) {
        let local = base
            .join(".config/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks/task-local");
        let server = base.join(
            ".vscode-server/data/User/globalStorage/rooveterinaryinc.roo-cline/tasks/task-server",
        );
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(&server).unwrap();
        File::create(local.join("ui_messages.json")).unwrap();
        File::create(server.join("ui_messages.json")).unwrap();
    }

    fn setup_mock_kilocode_dir(base: &std::path::Path) {
        let local =
            base.join(".config/Code/User/globalStorage/kilocode.kilo-code/tasks/task-local");
        let server = base
            .join(".vscode-server/data/User/globalStorage/kilocode.kilo-code/tasks/task-server");
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(&server).unwrap();
        File::create(local.join("ui_messages.json")).unwrap();
        File::create(server.join("ui_messages.json")).unwrap();
    }

    fn setup_mock_cline_dir(base: &std::path::Path) {
        let local =
            base.join(".config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/task-local");
        let macos = base.join(
            "Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/task-macos",
        );
        let windows = base.join(
            "AppData/Roaming/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/task-windows",
        );
        let server = base.join(
            ".vscode-server/data/User/globalStorage/saoudrizwan.claude-dev/tasks/task-server",
        );
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(&macos).unwrap();
        fs::create_dir_all(&windows).unwrap();
        fs::create_dir_all(&server).unwrap();
        File::create(local.join("ui_messages.json")).unwrap();
        File::create(macos.join("ui_messages.json")).unwrap();
        File::create(windows.join("ui_messages.json")).unwrap();
        File::create(server.join("ui_messages.json")).unwrap();
    }

    fn setup_mock_cline_cli_dir(data_dir: &std::path::Path) {
        setup_mock_cline_cli_session_root(&data_dir.join("sessions"));
    }

    fn setup_mock_cline_cli_session_root(sessions_root: &std::path::Path) {
        let sessions = sessions_root.join("cli-session");
        fs::create_dir_all(&sessions).unwrap();
        File::create(sessions.join("cli-session.messages.json")).unwrap();
    }

    fn setup_mock_crush_registry(registry_path: &Path, projects_json: &str) {
        fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        fs::write(registry_path, projects_json).unwrap();
    }

    #[test]
    #[serial]
    fn test_headless_roots_default() {
        let previous = std::env::var("TOKSCALE_HEADLESS_DIR").ok();
        unsafe { std::env::remove_var("TOKSCALE_HEADLESS_DIR") };

        let home = "/tmp/tokscale-test-home";
        let roots = headless_roots(home);
        let config_root = PathBuf::from(join_native(home, ".config/tokscale/headless"));
        let mac_root = PathBuf::from(format!(
            "{}/Library/Application Support/tokscale/headless",
            home
        ));

        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&config_root));
        assert!(roots.contains(&mac_root));

        restore_env("TOKSCALE_HEADLESS_DIR", previous);
    }

    #[test]
    #[serial]
    fn test_headless_roots_override() {
        let previous = std::env::var("TOKSCALE_HEADLESS_DIR").ok();
        unsafe { std::env::set_var("TOKSCALE_HEADLESS_DIR", "/custom/headless") };

        let roots = headless_roots("/tmp/home");
        assert_eq!(roots, vec![PathBuf::from("/custom/headless")]);

        restore_env("TOKSCALE_HEADLESS_DIR", previous);
    }

    #[test]
    #[serial]
    fn test_headless_roots_ignore_env_override_when_disabled() {
        let previous = std::env::var("TOKSCALE_HEADLESS_DIR").ok();
        unsafe { std::env::set_var("TOKSCALE_HEADLESS_DIR", "/custom/headless") };

        let roots = headless_roots_with_env_strategy("/tmp/home", false);
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/tmp/home/.config/tokscale/headless"),
                PathBuf::from("/tmp/home/Library/Application Support/tokscale/headless")
            ]
        );

        restore_env("TOKSCALE_HEADLESS_DIR", previous);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_opencode() {
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_opencode_dir(home);

        // Set XDG_DATA_HOME for the test
        unsafe { std::env::set_var("XDG_DATA_HOME", home.join(".local/share")) };

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["opencode".to_string()]);
        assert_eq!(result.get(ClientId::OpenCode).len(), 1);
        assert!(result.get(ClientId::Claude).is_empty());
        assert!(result.get(ClientId::Codex).is_empty());
        assert!(result.get(ClientId::Gemini).is_empty());

        restore_env("XDG_DATA_HOME", previous_xdg);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_opencode_home_override_ignores_xdg_env() {
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path().join("target-home");
        let conflicting_xdg = dir.path().join("conflicting-xdg");
        setup_mock_opencode_dir(&home);
        fs::create_dir_all(&conflicting_xdg).unwrap();

        unsafe { std::env::set_var("XDG_DATA_HOME", &conflicting_xdg) };

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["opencode".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::OpenCode).len(), 1);
        assert_eq!(
            result.opencode_json_dir,
            Some(home.join(".local/share/opencode/storage/message"))
        );

        restore_env("XDG_DATA_HOME", previous_xdg);
    }

    #[test]
    fn test_is_opencode_db_filename_accepts_default_and_channel_variants() {
        // Default channel (`latest`/`beta`) and explicit-disable use this name.
        assert!(is_opencode_db_filename("opencode.db"));
        // Channel-suffixed dbs, drawn from opencode's `[a-zA-Z0-9._-]`
        // character class in getChannelPath.
        assert!(is_opencode_db_filename("opencode-stable.db"));
        assert!(is_opencode_db_filename("opencode-nightly.db"));
        assert!(is_opencode_db_filename("opencode-canary.db"));
        assert!(is_opencode_db_filename("opencode-local.db"));
        assert!(is_opencode_db_filename("opencode-1.2.3.db"));
        assert!(is_opencode_db_filename("opencode-pr_42.db"));
    }

    #[test]
    fn test_is_opencode_db_filename_rejects_sidecars_and_unrelated_files() {
        // WAL/SHM/journal sidecar files share the prefix — must be ignored
        // so we don't try to "parse" them.
        assert!(!is_opencode_db_filename("opencode.db-wal"));
        assert!(!is_opencode_db_filename("opencode.db-shm"));
        assert!(!is_opencode_db_filename("opencode.db-journal"));
        assert!(!is_opencode_db_filename("opencode-stable.db-wal"));
        // Unrelated / malformed names.
        assert!(!is_opencode_db_filename("opencode"));
        assert!(!is_opencode_db_filename("opencode-.db"));
        assert!(!is_opencode_db_filename("opencode_stable.db"));
        assert!(!is_opencode_db_filename("opencode-stable/beta.db"));
        assert!(!is_opencode_db_filename("auth.json"));
        assert!(!is_opencode_db_filename("other.db"));
    }

    #[test]
    fn test_is_micode_db_filename_accepts_default_and_channel_rejects_sidecars() {
        // Default and channel-suffixed db names are accepted.
        assert!(is_micode_db_filename("mimocode.db"));
        assert!(is_micode_db_filename("mimocode-stable.db"));
        assert!(is_micode_db_filename("mimocode-nightly.db"));
        // WAL/SHM sidecar files share the prefix — must be ignored.
        assert!(!is_micode_db_filename("mimocode.db-wal"));
        assert!(!is_micode_db_filename("mimocode.db-shm"));
    }

    #[test]
    fn test_discover_micode_dbs_in_dirs_unions_xdg_and_orca_roots() {
        let dir = TempDir::new().unwrap();
        // Primary XDG location.
        let xdg_dir = dir.path().join(".local/share/mimocode");
        fs::create_dir_all(&xdg_dir).unwrap();
        let xdg_db = xdg_dir.join("mimocode.db");
        fs::write(&xdg_db, b"").unwrap();

        // orca hook-sandbox location, holding both the default db and a
        // channel-suffixed one that the XDG root is missing.
        let orca_dir = dir
            .path()
            .join("Library/Application Support/orca/mimocode-hooks/shared/data");
        fs::create_dir_all(&orca_dir).unwrap();
        let orca_db = orca_dir.join("mimocode.db");
        let orca_channel_db = orca_dir.join("mimocode-nightly.db");
        fs::write(&orca_db, b"").unwrap();
        fs::write(&orca_channel_db, b"").unwrap();
        // Sidecar files must be ignored across both roots.
        fs::write(orca_dir.join("mimocode.db-wal"), b"").unwrap();

        let dbs = discover_micode_dbs_in_dirs([xdg_dir, orca_dir]);

        assert!(dbs.contains(&xdg_db), "XDG db should be discovered");
        assert!(dbs.contains(&orca_db), "orca db should be discovered");
        assert!(
            dbs.contains(&orca_channel_db),
            "orca channel db should be discovered"
        );
        assert_eq!(dbs.len(), 3, "no sidecar files, no missed dbs");
    }

    #[test]
    fn test_discover_micode_dbs_in_dirs_collapses_same_file_via_symlink() {
        // A symlink making two roots resolve to the same file must not yield the
        // db twice, or its non-embedded-id messages would be double-counted.
        let dir = TempDir::new().unwrap();
        let real_dir = dir.path().join("real/mimocode");
        fs::create_dir_all(&real_dir).unwrap();
        let real_db = real_dir.join("mimocode.db");
        fs::write(&real_db, b"").unwrap();

        let link_dir = dir.path().join("linked");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();
        #[cfg(not(unix))]
        std::os::windows::fs::symlink_dir(&real_dir, &link_dir).unwrap();

        let dbs = discover_micode_dbs_in_dirs([real_dir, link_dir]);
        assert_eq!(dbs.len(), 1, "symlinked duplicate must collapse to one db");
    }

    #[test]
    fn test_discover_opencode_dbs_finds_multiple_channels_and_skips_sidecars() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("opencode");
        fs::create_dir_all(&data_dir).unwrap();

        // Real dbs for two channels running side by side — the case from
        // junhoyeo/tokscale#387.
        File::create(data_dir.join("opencode.db")).unwrap();
        File::create(data_dir.join("opencode-stable.db")).unwrap();
        // SQLite WAL/SHM sidecars that must not be treated as dbs.
        File::create(data_dir.join("opencode.db-wal")).unwrap();
        File::create(data_dir.join("opencode.db-shm")).unwrap();
        File::create(data_dir.join("opencode-stable.db-wal")).unwrap();
        // Unrelated files that live in the same dir.
        File::create(data_dir.join("auth.json")).unwrap();

        let found = discover_opencode_dbs(&data_dir);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["opencode-stable.db", "opencode.db"]);
    }

    #[test]
    fn test_discover_opencode_dbs_returns_empty_for_missing_dir() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(discover_opencode_dbs(&missing).is_empty());
    }

    #[test]
    fn test_merge_user_opencode_db_paths_picks_up_path_outside_xdg() {
        // Simulate `OPENCODE_DB=/arbitrary/abs/path/custom.db` upstream:
        // the file is a real opencode db but lives outside
        // `~/.local/share/opencode`, so auto-discovery never sees it.
        let dir = TempDir::new().unwrap();
        let outside = dir.path().join("somewhere-else");
        fs::create_dir_all(&outside).unwrap();
        let user_db = outside.join("opencode.db");
        File::create(&user_db).unwrap();

        let mut discovered: Vec<PathBuf> = Vec::new();
        merge_user_opencode_db_paths(&mut discovered, std::slice::from_ref(&user_db));

        assert_eq!(discovered, vec![user_db]);
    }

    #[test]
    fn test_merge_user_opencode_db_paths_skips_nonexistent_and_sidecars() {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("opencode-stable.db");
        File::create(&real).unwrap();
        let wal = dir.path().join("opencode-stable.db-wal");
        File::create(&wal).unwrap();
        let missing = dir.path().join("opencode-missing.db"); // never created

        let mut discovered: Vec<PathBuf> = Vec::new();
        merge_user_opencode_db_paths(
            &mut discovered,
            &[real.clone(), wal.clone(), missing.clone()],
        );

        // Nonexistent path: silently skipped so stale config can't break a scan.
        // Sidecar path: rejected by is_opencode_db_filename.
        assert_eq!(discovered, vec![real]);
    }

    #[test]
    fn test_merge_user_opencode_db_paths_dedups_against_auto_discovered() {
        let dir = TempDir::new().unwrap();
        let shared = dir.path().join("opencode.db");
        File::create(&shared).unwrap();

        // User explicitly lists a path that auto-discovery also found —
        // must not double-parse the same sqlite file.
        let mut discovered: Vec<PathBuf> = vec![shared.clone()];
        merge_user_opencode_db_paths(&mut discovered, std::slice::from_ref(&shared));

        assert_eq!(discovered, vec![shared]);
    }

    #[test]
    fn test_scanner_settings_deserialize_from_json_camel_case() {
        // This is the contract the CLI's settings.json relies on: the
        // field is `opencodeDbPaths`, and an empty object or missing key
        // must round-trip to Default without erroring.
        let json = r#"{
            "opencodeDbPaths": ["/one/opencode.db", "/two/opencode-stable.db"]
        }"#;
        let parsed: ScannerSettings = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.opencode_db_paths.len(), 2);
        assert_eq!(
            parsed.opencode_db_paths[0],
            PathBuf::from("/one/opencode.db")
        );
        assert_eq!(
            parsed.opencode_db_paths[1],
            PathBuf::from("/two/opencode-stable.db")
        );

        let empty: ScannerSettings = serde_json::from_str("{}").unwrap();
        assert!(empty.opencode_db_paths.is_empty());
    }

    #[test]
    fn test_scanner_settings_deserialize_extra_scan_paths_camel_case() {
        let json = r#"{
            "extraScanPaths": {
                "codex": [
                    "/tmp/project-a/.codex/sessions",
                    "/tmp/project-b/.codex/archived_sessions"
                ],
                "gemini": ["/tmp/imports/gemini/tmp"]
            }
        }"#;

        let parsed: ScannerSettings = serde_json::from_str(json).unwrap();
        let serialized = serde_json::to_value(&parsed).unwrap();

        assert_eq!(
            serialized["extraScanPaths"]["codex"][0],
            serde_json::json!("/tmp/project-a/.codex/sessions")
        );
        assert_eq!(
            serialized["extraScanPaths"]["codex"][1],
            serde_json::json!("/tmp/project-b/.codex/archived_sessions")
        );
        assert_eq!(
            serialized["extraScanPaths"]["gemini"][0],
            serde_json::json!("/tmp/imports/gemini/tmp")
        );
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_merges_user_path() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        // Auto-discoverable channel db inside the default data dir.
        let data_dir = home.join(".local/share/opencode");
        fs::create_dir_all(&data_dir).unwrap();
        File::create(data_dir.join("opencode-stable.db")).unwrap();

        // User-configured db living outside XDG_DATA_HOME, the way an
        // `OPENCODE_DB=/abs/path/opencode.db` user would have it.
        let outside_dir = home.join("elsewhere");
        fs::create_dir_all(&outside_dir).unwrap();
        let outside_db = outside_dir.join("opencode.db");
        File::create(&outside_db).unwrap();

        let settings = ScannerSettings {
            opencode_db_paths: vec![outside_db.clone()],
            ..Default::default()
        };
        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["opencode".to_string()],
            false,
            &settings,
        );

        // Both paths must appear — the auto-discovered stable db and the
        // user-configured outside-XDG db.
        let names: Vec<String> = result
            .opencode_dbs
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().any(|n| n == "opencode-stable.db"),
            "expected auto-discovered opencode-stable.db, got {names:?}"
        );
        assert!(
            result.opencode_dbs.iter().any(|p| p == &outside_db),
            "expected user-configured {} in {:?}",
            outside_db.display(),
            result.opencode_dbs
        );
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_merges_settings_extra_paths() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let default_root = home.join(".codex/sessions");
        fs::create_dir_all(&default_root).unwrap();
        File::create(default_root.join("default.jsonl")).unwrap();

        let extra_root = home.join("workspace/project-a/.codex/sessions");
        fs::create_dir_all(&extra_root).unwrap();
        File::create(extra_root.join("extra.jsonl")).unwrap();

        let settings: ScannerSettings = serde_json::from_value(serde_json::json!({
            "extraScanPaths": {
                "codex": [extra_root]
            }
        }))
        .unwrap();

        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["codex".to_string()],
            false,
            &settings,
        );

        assert_eq!(result.get(ClientId::Codex).len(), 2);
    }

    #[test]
    fn test_scan_all_clients_with_scanner_settings_discovers_devin_cli_extra_databases() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let default_db = home.join(".local/share/devin/cli/sessions.db");
        fs::create_dir_all(default_db.parent().unwrap()).unwrap();
        File::create(&default_db).unwrap();

        let extra_root = home.join("imports/devin");
        let extra_db = extra_root.join("profile/sessions.db");
        fs::create_dir_all(extra_db.parent().unwrap()).unwrap();
        File::create(&extra_db).unwrap();

        let settings: ScannerSettings = serde_json::from_value(serde_json::json!({
            "extraScanPaths": {
                "devin-cli": [extra_root]
            }
        }))
        .unwrap();
        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["devin-cli".to_string()],
            false,
            &settings,
        );

        assert_eq!(result.devin_dbs, vec![default_db, extra_db]);
        assert!(
            result.get(ClientId::DevinCli).is_empty(),
            "Devin SQLite databases should use the dedicated scan result"
        );
    }

    #[test]
    fn test_devin_desktop_scan_includes_configured_cli_lookup_databases() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let extra_root = home.join("imports/devin");
        let extra_db = extra_root.join("profile/sessions.db");
        fs::create_dir_all(extra_db.parent().unwrap()).unwrap();
        File::create(&extra_db).unwrap();

        let settings: ScannerSettings = serde_json::from_value(serde_json::json!({
            "extraScanPaths": {
                "devin-cli": [extra_root]
            }
        }))
        .unwrap();
        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["devin-desktop".to_string()],
            false,
            &settings,
        );

        assert_eq!(result.devin_dbs, vec![extra_db]);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_merges_hermes_extra_profile_db() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let default_dir = home.join(".hermes");
        fs::create_dir_all(&default_dir).unwrap();
        let default_db = default_dir.join("state.db");
        File::create(&default_db).unwrap();

        let profile_dir = home.join(".hermes/profiles/director_planning");
        fs::create_dir_all(&profile_dir).unwrap();
        let profile_db = profile_dir.join("state.db");
        File::create(&profile_db).unwrap();

        let settings: ScannerSettings = serde_json::from_value(serde_json::json!({
            "extraScanPaths": {
                "hermes": [
                    profile_dir,
                    profile_db
                ]
            }
        }))
        .unwrap();

        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["hermes".to_string()],
            false,
            &settings,
        );

        assert_eq!(result.hermes_db.as_ref(), Some(&default_db));
        assert_eq!(result.hermes_db_paths(), vec![default_db, profile_db]);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_auto_discovers_hermes_profile_dbs() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let default_dir = home.join(".hermes");
        fs::create_dir_all(&default_dir).unwrap();
        let default_db = default_dir.join("state.db");
        File::create(&default_db).unwrap();

        let profile_a_dir = home.join(".hermes/profiles/director_planning");
        fs::create_dir_all(&profile_a_dir).unwrap();
        let profile_a_db = profile_a_dir.join("state.db");
        File::create(&profile_a_db).unwrap();

        let profile_b_dir = home.join(".hermes/profiles/research");
        fs::create_dir_all(&profile_b_dir).unwrap();
        let profile_b_db = profile_b_dir.join("state.db");
        File::create(&profile_b_db).unwrap();

        // Shallow discovery should not pick up arbitrary nested state.db files.
        let nested_dir = home.join(".hermes/profiles/research/archive");
        fs::create_dir_all(&nested_dir).unwrap();
        File::create(nested_dir.join("state.db")).unwrap();

        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["hermes".to_string()],
            false,
            &ScannerSettings::default(),
        );

        assert_eq!(result.hermes_db.as_ref(), Some(&default_db));
        assert_eq!(
            result.hermes_db_paths(),
            vec![default_db, profile_a_db, profile_b_db]
        );
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_auto_discovers_hermes_profiles_without_default_db(
    ) {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let profile_dir = home.join(".hermes/profiles/research");
        fs::create_dir_all(&profile_dir).unwrap();
        let profile_db = profile_dir.join("state.db");
        File::create(&profile_db).unwrap();

        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["hermes".to_string()],
            false,
            &ScannerSettings::default(),
        );

        assert_eq!(result.hermes_db, None);
        assert_eq!(result.hermes_db_paths(), vec![profile_db]);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_auto_discovers_hermes_profiles_under_env_home() {
        let mut env = EnvGuard::capture(&["HERMES_HOME", "TOKSCALE_EXTRA_DIRS"]);
        env.remove("TOKSCALE_EXTRA_DIRS");
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let hermes_home = home.join("custom-hermes-home");

        fs::create_dir_all(&hermes_home).unwrap();
        let default_db = hermes_home.join("state.db");
        File::create(&default_db).unwrap();

        let profile_dir = hermes_home.join("profiles/research");
        fs::create_dir_all(&profile_dir).unwrap();
        let profile_db = profile_dir.join("state.db");
        File::create(&profile_db).unwrap();

        env.set("HERMES_HOME", &hermes_home);
        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["hermes".to_string()],
            true,
            &ScannerSettings::default(),
        );

        assert_eq!(result.hermes_db.as_ref(), Some(&default_db));
        assert_eq!(result.hermes_db_paths(), vec![default_db, profile_db]);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_profile_scoped_hermes_home_isolates_to_own_profile(
    ) {
        // Data-isolation guarantee: a profile-scoped `HERMES_HOME` must NOT pull
        // in sibling profiles under `<root>/profiles/*` or the default profile at
        // `<root>/state.db`. Only the scoped profile's own `state.db` is scanned.
        let mut env = EnvGuard::capture(&["HERMES_HOME", "TOKSCALE_EXTRA_DIRS"]);
        env.remove("TOKSCALE_EXTRA_DIRS");
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let profile_root = home.join(".hermes/profiles");
        let default_db = home.join(".hermes/state.db");
        fs::create_dir_all(default_db.parent().unwrap()).unwrap();
        File::create(&default_db).unwrap();

        let coder_dir = profile_root.join("coder");
        fs::create_dir_all(&coder_dir).unwrap();
        let coder_db = coder_dir.join("state.db");
        File::create(&coder_db).unwrap();

        let research_dir = profile_root.join("research");
        fs::create_dir_all(&research_dir).unwrap();
        let research_db = research_dir.join("state.db");
        File::create(&research_db).unwrap();

        // Profile-scoped homes must also not scan `<active-profile>/profiles`.
        let nested_dir = coder_dir.join("profiles/archived");
        fs::create_dir_all(&nested_dir).unwrap();
        File::create(nested_dir.join("state.db")).unwrap();

        env.set("HERMES_HOME", &coder_dir);
        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["hermes".to_string()],
            true,
            &ScannerSettings::default(),
        );

        assert_eq!(result.hermes_db.as_ref(), Some(&coder_db));
        assert_eq!(result.hermes_db_paths(), vec![coder_db.clone()]);
        assert!(
            !result.hermes_db_paths().contains(&research_db),
            "profile-scoped HERMES_HOME must not discover sibling profiles"
        );
        assert!(
            !result.hermes_db_paths().contains(&default_db),
            "profile-scoped HERMES_HOME must not discover the default profile"
        );
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_discovers_hermes_windows_local_appdata_home() {
        // Native Windows root: Hermes stores its home under
        // `%LOCALAPPDATA%\hermes` (literal `<home>/AppData/Local/hermes`). Run
        // with env roots disabled so this exercises the cross-platform
        // `AppData/Local` fallback, mirroring the Crush LOCALAPPDATA tests.
        let previous_hermes_home = std::env::var("HERMES_HOME").ok();
        let previous_local_app_data = std::env::var("LOCALAPPDATA").ok();
        unsafe { std::env::remove_var("HERMES_HOME") };
        unsafe { std::env::remove_var("LOCALAPPDATA") };

        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let windows_home = home.join("AppData/Local/hermes");
        fs::create_dir_all(&windows_home).unwrap();
        let default_db = windows_home.join("state.db");
        File::create(&default_db).unwrap();

        let profile_dir = windows_home.join("profiles/research");
        fs::create_dir_all(&profile_dir).unwrap();
        let profile_db = profile_dir.join("state.db");
        File::create(&profile_db).unwrap();

        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["hermes".to_string()],
            false,
            &ScannerSettings::default(),
        );

        restore_env("HERMES_HOME", previous_hermes_home);
        restore_env("LOCALAPPDATA", previous_local_app_data);

        assert_eq!(result.hermes_db.as_ref(), Some(&default_db));
        assert_eq!(result.hermes_db_paths(), vec![default_db, profile_db]);
    }

    #[test]
    fn test_scan_all_clients_with_scanner_settings_discovers_zed_windows_local_appdata_home() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let windows_threads_dir = home.join("AppData/Local/Zed/threads");
        fs::create_dir_all(&windows_threads_dir).unwrap();
        let threads_db = windows_threads_dir.join("threads.db");
        File::create(&threads_db).unwrap();

        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["zed".to_string()],
            false,
            &ScannerSettings::default(),
        );

        assert_eq!(result.zed_db.as_ref(), Some(&threads_db));
    }

    #[test]
    fn test_scan_all_clients_with_scanner_settings_merges_zed_extra_threads_db() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let windows_threads_dir = home.join("AppData/Local/Zed/threads");
        fs::create_dir_all(&windows_threads_dir).unwrap();
        let threads_db = windows_threads_dir.join("threads.db");
        File::create(&threads_db).unwrap();

        let settings: ScannerSettings = serde_json::from_value(serde_json::json!({
            "extraScanPaths": {
                "zed": [windows_threads_dir]
            }
        }))
        .unwrap();

        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["zed".to_string()],
            false,
            &settings,
        );

        assert_eq!(result.zed_db_paths(), vec![threads_db]);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_respects_hermes_client_filter() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let profile_dir = home.join(".hermes/profiles/director_planning");
        fs::create_dir_all(&profile_dir).unwrap();
        let profile_db = profile_dir.join("state.db");
        File::create(&profile_db).unwrap();

        let settings: ScannerSettings = serde_json::from_value(serde_json::json!({
            "extraScanPaths": {
                "hermes": [profile_dir]
            }
        }))
        .unwrap();

        let claude_only = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["claude".to_string()],
            true,
            &settings,
        );
        assert!(claude_only.hermes_db_paths().is_empty());

        let hermes_only = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["hermes".to_string()],
            false,
            &settings,
        );
        assert_eq!(hermes_only.hermes_db_paths(), vec![profile_db]);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_dedups_settings_and_env_extra_paths() {
        let mut env =
            EnvGuard::capture(&["TOKSCALE_EXTRA_DIRS", "TOKSCALE_HEADLESS_DIR", "CODEX_HOME"]);
        env.remove("TOKSCALE_HEADLESS_DIR");
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        env.set("CODEX_HOME", home.join(".codex"));

        let default_root = home.join(".codex/sessions");
        fs::create_dir_all(&default_root).unwrap();
        File::create(default_root.join("default.jsonl")).unwrap();

        let extra_root = home.join("workspace/project-a/.codex/sessions");
        fs::create_dir_all(&extra_root).unwrap();
        File::create(extra_root.join("extra.jsonl")).unwrap();

        env.set(
            "TOKSCALE_EXTRA_DIRS",
            format!("codex:{}", extra_root.join("..").join("sessions").display()),
        );

        let settings: ScannerSettings = serde_json::from_value(serde_json::json!({
            "extraScanPaths": {
                "codex": [extra_root]
            }
        }))
        .unwrap();

        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["codex".to_string()],
            true,
            &settings,
        );

        assert_eq!(result.get(ClientId::Codex).len(), 2);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_respects_opencode_client_filter() {
        // Regression guard: previously the scanner unconditionally
        // merged `scanner.opencodeDbPaths` after the inner scan, which
        // bypassed the existing `enabled.contains(&ClientId::OpenCode)`
        // guard. A request like `tokscale --claude` would still pull in
        // user-pinned OpenCode dbs and inflate `parse_local_clients`
        // counts plus waste SQLite parsing work.
        //
        // The fix moves the merge inside the OpenCode-enabled block, so
        // this test exercises the four canonical filter shapes:
        //   1. ["claude"]    → opencode_dbs must be empty
        //   2. ["opencode"]  → both auto + user-configured dbs present
        //   3. ["synthetic"] → both present (synthetic enables all)
        //   4. []            → both present (empty filter = all clients)
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        // Auto-discoverable channel db inside XDG data dir.
        let data_dir = home.join(".local/share/opencode");
        fs::create_dir_all(&data_dir).unwrap();
        let auto_db = data_dir.join("opencode.db");
        File::create(&auto_db).unwrap();

        // User-configured db living outside XDG_DATA_HOME (mirrors the
        // `OPENCODE_DB=/abs/path/opencode.db` use case).
        let outside_dir = home.join("elsewhere");
        fs::create_dir_all(&outside_dir).unwrap();
        let outside_db = outside_dir.join("opencode.db");
        File::create(&outside_db).unwrap();

        let settings = ScannerSettings {
            opencode_db_paths: vec![outside_db.clone()],
            ..Default::default()
        };

        let scan = |clients: &[&str]| {
            let owned: Vec<String> = clients.iter().map(|s| s.to_string()).collect();
            scan_all_clients_with_scanner_settings(home.to_str().unwrap(), &owned, false, &settings)
        };

        // 1. clients=["claude"] — OpenCode disabled, dbs must stay empty.
        let claude_only = scan(&["claude"]);
        assert!(
            claude_only.opencode_dbs.is_empty(),
            "scanner.opencodeDbPaths must NOT leak into a Claude-only scan, \
             got {:?}",
            claude_only.opencode_dbs
        );

        // 2. clients=["opencode"] — both auto-discovered + user-configured.
        let opencode_only = scan(&["opencode"]);
        assert!(
            opencode_only.opencode_dbs.iter().any(|p| p == &auto_db),
            "expected auto-discovered {} in {:?}",
            auto_db.display(),
            opencode_only.opencode_dbs
        );
        assert!(
            opencode_only.opencode_dbs.iter().any(|p| p == &outside_db),
            "expected user-configured {} in {:?}",
            outside_db.display(),
            opencode_only.opencode_dbs
        );

        // 3. clients=["synthetic"] — synthetic enables all clients, so
        //    both dbs must be present.
        let synthetic_only = scan(&["synthetic"]);
        assert!(
            synthetic_only.opencode_dbs.iter().any(|p| p == &auto_db),
            "synthetic-only filter must enable OpenCode auto-discovery, got {:?}",
            synthetic_only.opencode_dbs
        );
        assert!(
            synthetic_only.opencode_dbs.iter().any(|p| p == &outside_db),
            "synthetic-only filter must merge user-configured paths, got {:?}",
            synthetic_only.opencode_dbs
        );

        // 4. clients=[] — empty filter = all clients = both dbs present.
        let all_clients = scan(&[]);
        assert!(
            all_clients.opencode_dbs.iter().any(|p| p == &auto_db),
            "empty client filter must enable OpenCode auto-discovery, got {:?}",
            all_clients.opencode_dbs
        );
        assert!(
            all_clients.opencode_dbs.iter().any(|p| p == &outside_db),
            "empty client filter must merge user-configured paths, got {:?}",
            all_clients.opencode_dbs
        );
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_opencode_picks_up_channel_suffixed_dbs() {
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let data_dir = home.join(".local/share/opencode");
        fs::create_dir_all(&data_dir).unwrap();

        File::create(data_dir.join("opencode.db")).unwrap();
        File::create(data_dir.join("opencode-stable.db")).unwrap();
        File::create(data_dir.join("opencode-nightly.db")).unwrap();
        // Sidecars that must be ignored.
        File::create(data_dir.join("opencode.db-wal")).unwrap();
        File::create(data_dir.join("opencode-stable.db-shm")).unwrap();

        unsafe { std::env::set_var("XDG_DATA_HOME", home.join(".local/share")) };

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["opencode".to_string()]);

        let names: Vec<String> = result
            .opencode_dbs
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "opencode-nightly.db".to_string(),
                "opencode-stable.db".to_string(),
                "opencode.db".to_string(),
            ],
            "expected all channel dbs, got {names:?}"
        );

        restore_env("XDG_DATA_HOME", previous_xdg);
    }

    #[test]
    fn test_scan_all_clients_pi() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_pi_dir(home);

        let result =
            scan_all_clients_with_env_strategy(home.to_str().unwrap(), &["pi".to_string()], false);
        assert_eq!(result.get(ClientId::Pi).len(), 1);
        assert!(result.get(ClientId::OpenCode).is_empty());
        assert!(result.get(ClientId::Claude).is_empty());
    }

    #[test]
    fn test_scan_all_clients_prime_agent_includes_root_and_rlm_child_sessions() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let root_dir = home.join(".prime/agent/sessions");
        let child_dir = home.join(".prime/agent/session-artifacts/root/sub-deadbeef");
        fs::create_dir_all(&root_dir).unwrap();
        fs::create_dir_all(&child_dir).unwrap();
        File::create(root_dir.join("root.jsonl")).unwrap();
        File::create(child_dir.join("child.jsonl")).unwrap();
        File::create(home.join(".prime/agent/session-artifacts/root/rlm-subagents.jsonl")).unwrap();

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["prime-agent".to_string()],
            false,
        );

        assert_eq!(result.get(ClientId::PrimeAgent).len(), 2);
        assert!(result
            .get(ClientId::PrimeAgent)
            .iter()
            .any(|path| path.ends_with("root.jsonl")));
        assert!(result
            .get(ClientId::PrimeAgent)
            .iter()
            .any(|path| path.ends_with("child.jsonl")));
        assert!(result.get(ClientId::Pi).is_empty());
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_prime_agent_honors_session_dir_override() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let custom_root = home.join("custom/sessions");
        let child_dir = home.join("custom/session-artifacts/root/sub-deadbeef");
        fs::create_dir_all(&custom_root).unwrap();
        fs::create_dir_all(&child_dir).unwrap();
        File::create(custom_root.join("root.jsonl")).unwrap();
        File::create(child_dir.join("child.jsonl")).unwrap();

        let mut env = EnvGuard::capture(&[
            "PRIME_AGENT_SESSION_DIR",
            "PRIME_AGENT_CODING_AGENT_SESSION_DIR",
            "PRIME_AGENT_CODING_AGENT_DIR",
        ]);
        env.set("PRIME_AGENT_SESSION_DIR", "~/custom/sessions");
        env.remove("PRIME_AGENT_CODING_AGENT_SESSION_DIR");
        env.set(
            "PRIME_AGENT_CODING_AGENT_DIR",
            home.join("unused-agent-dir"),
        );
        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["prime-agent".to_string()],
            true,
        );

        assert_eq!(result.get(ClientId::PrimeAgent).len(), 2);
        assert!(result
            .get(ClientId::PrimeAgent)
            .iter()
            .all(|path| path.starts_with(home.join("custom"))));
    }

    #[test]
    #[serial]
    fn test_prime_agent_roots_honor_agent_dir_and_legacy_session_override() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let mut env = EnvGuard::capture(&[
            "PRIME_AGENT_SESSION_DIR",
            "PRIME_AGENT_CODING_AGENT_SESSION_DIR",
            "PRIME_AGENT_CODING_AGENT_DIR",
        ]);
        env.remove("PRIME_AGENT_SESSION_DIR");
        env.remove("PRIME_AGENT_CODING_AGENT_SESSION_DIR");
        env.set("PRIME_AGENT_CODING_AGENT_DIR", "~/custom-agent");

        let roots = prime_agent_session_roots_with_env_strategy(home.to_str().unwrap(), true);
        assert_eq!(roots[0], home.join("custom-agent/sessions"));
        assert_eq!(roots[1], home.join("custom-agent/session-artifacts"));

        let legacy_sessions = home.join("legacy/sessions");
        env.set("PRIME_AGENT_CODING_AGENT_SESSION_DIR", &legacy_sessions);
        let roots = prime_agent_session_roots_with_env_strategy(home.to_str().unwrap(), true);
        assert_eq!(roots[0], legacy_sessions);
        assert_eq!(roots[1], home.join("legacy/session-artifacts"));

        // Match Prime Agent's `primary ?? legacy` environment lookup exactly:
        // an explicitly empty primary value suppresses the legacy variable,
        // then falls through to settings/default resolution.
        env.set("PRIME_AGENT_SESSION_DIR", "");
        let roots = prime_agent_session_roots_with_env_strategy(home.to_str().unwrap(), true);
        assert_eq!(roots[0], home.join("custom-agent/sessions"));
        assert_eq!(roots[1], home.join("custom-agent/session-artifacts"));
    }

    #[test]
    fn test_prime_agent_tilde_expansion_matches_upstream_forward_slash_only() {
        let home = if cfg!(windows) {
            r"C:\Users\test"
        } else {
            "/tmp/home"
        };
        assert_eq!(
            expand_tilde_path_with_home("~/sessions", home),
            PathBuf::from(home).join("sessions")
        );
        assert_eq!(
            expand_tilde_path_with_home(r"~\sessions", home),
            PathBuf::from(r"~\sessions")
        );
    }

    #[test]
    fn test_prime_agent_project_null_session_dir_resets_global_setting() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let global = home.join("global-settings.json");
        let project = home.join("project-settings.json");
        fs::write(&global, r#"{"sessionDir":"~/global-sessions"}"#).unwrap();
        fs::write(&project, r#"{"sessionDir":null}"#).unwrap();

        let setting = prime_agent_session_dir_from_settings_files(
            &global,
            Some(&project),
            home.to_str().unwrap(),
            Some(home),
        );
        assert_eq!(setting, Some(PrimeSessionDirSetting::Default));
        let sessions = match setting {
            Some(PrimeSessionDirSetting::Path(path))
            | Some(PrimeSessionDirSetting::CurrentDirectory(path)) => path,
            Some(PrimeSessionDirSetting::Default) | None => home.join("custom-agent/sessions"),
        };
        assert_eq!(sessions, home.join("custom-agent/sessions"));
    }

    #[test]
    fn test_prime_agent_empty_project_session_dir_resolves_to_current_directory() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let current_dir = home.join("project");
        let global = home.join("global-settings.json");
        let project = home.join("project-settings.json");
        fs::write(&global, r#"{"sessionDir":"~/global-sessions"}"#).unwrap();
        fs::write(&project, r#"{"sessionDir":""}"#).unwrap();

        let setting = prime_agent_session_dir_from_settings_files(
            &global,
            Some(&project),
            home.to_str().unwrap(),
            Some(&current_dir),
        );
        assert_eq!(
            setting,
            Some(PrimeSessionDirSetting::CurrentDirectory(current_dir))
        );
    }

    #[test]
    #[serial]
    fn test_prime_agent_empty_session_dir_scans_cwd_root_and_artifacts() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let project = home.join("work/project");
        fs::create_dir_all(project.join(".prime/agent")).unwrap();
        fs::write(
            project.join(".prime/agent/settings.json"),
            r#"{"sessionDir":""}"#,
        )
        .unwrap();
        let root = project.join("root.jsonl");
        let child = project.join("session-artifacts/root/sub-child/child.jsonl");
        fs::create_dir_all(child.parent().unwrap()).unwrap();
        fs::write(
            &root, "{}
",
        )
        .unwrap();
        fs::write(
            &child, "{}
",
        )
        .unwrap();
        fs::write(
            project.join("session-artifacts/root/rlm-subagents.jsonl"),
            "{}
",
        )
        .unwrap();

        let mut env = EnvGuard::capture(&[
            "PRIME_AGENT_SESSION_DIR",
            "PRIME_AGENT_CODING_AGENT_SESSION_DIR",
            "PRIME_AGENT_CODING_AGENT_DIR",
        ]);
        env.remove("PRIME_AGENT_SESSION_DIR");
        env.remove("PRIME_AGENT_CODING_AGENT_SESSION_DIR");
        env.remove("PRIME_AGENT_CODING_AGENT_DIR");
        let _current_dir = CurrentDirGuard::set(&project);
        let roots = prime_agent_session_roots_with_env_strategy(home.to_str().unwrap(), true);
        let result = scan_all_clients(home.to_str().unwrap(), &["prime-agent".to_string()]);

        let canonical_project = project.canonicalize().unwrap();
        assert_eq!(roots[0].canonicalize().unwrap(), canonical_project);
        assert_eq!(
            roots[1].canonicalize().unwrap(),
            project.join("session-artifacts").canonicalize().unwrap()
        );
        let files = result.get(ClientId::PrimeAgent);
        assert_eq!(files.len(), 2);
        let canonical_files: Vec<PathBuf> = files
            .iter()
            .map(|path| path.canonicalize().unwrap())
            .collect();
        assert!(canonical_files.contains(&root.canonicalize().unwrap()));
        assert!(canonical_files.contains(&child.canonicalize().unwrap()));
    }

    #[test]
    #[serial]
    fn test_prime_agent_roots_honor_settings_session_dir() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let agent_dir = home.join("custom-agent");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            agent_dir.join("settings.json"),
            r#"{"sessionDir":"~/settings-sessions"}"#,
        )
        .unwrap();

        let mut env = EnvGuard::capture(&[
            "PRIME_AGENT_SESSION_DIR",
            "PRIME_AGENT_CODING_AGENT_SESSION_DIR",
            "PRIME_AGENT_CODING_AGENT_DIR",
        ]);
        env.remove("PRIME_AGENT_SESSION_DIR");
        env.remove("PRIME_AGENT_CODING_AGENT_SESSION_DIR");
        env.set("PRIME_AGENT_CODING_AGENT_DIR", &agent_dir);

        let roots = prime_agent_session_roots_with_env_strategy(home.to_str().unwrap(), true);
        assert_eq!(roots[0], home.join("settings-sessions"));
        assert_eq!(roots[1], home.join("session-artifacts"));
    }

    #[test]
    fn test_scan_all_clients_kimchi() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_kimchi_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["kimchi".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Kimchi).len(), 1);
        assert!(result.get(ClientId::Kimchi)[0]
            .to_string_lossy()
            .ends_with(".jsonl"));
        assert!(result.get(ClientId::Pi).is_empty());
    }

    #[test]
    fn test_scan_all_clients_senpi() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_senpi_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["senpi".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Senpi).len(), 1);
        assert!(result.get(ClientId::Senpi)[0]
            .ends_with("2026-07-29T15-19-53-436Z_019fae75-f35c-7b20-8d6f-e6dea8f7d9f5.jsonl"));
        assert!(result.get(ClientId::Pi).is_empty());
    }

    #[test]
    fn test_scan_all_clients_senpi_is_not_scanned_as_pi() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_senpi_dir(home);

        let result =
            scan_all_clients_with_env_strategy(home.to_str().unwrap(), &["pi".to_string()], false);
        assert!(result.get(ClientId::Pi).is_empty());
        assert!(result.get(ClientId::Senpi).is_empty());
    }

    #[test]
    fn test_scan_all_clients_omp_scanned_as_pi() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_omp_dir(home);

        let result =
            scan_all_clients_with_env_strategy(home.to_str().unwrap(), &["pi".to_string()], false);
        assert_eq!(result.get(ClientId::Pi).len(), 1);
        assert!(result.get(ClientId::Pi)[0].ends_with("2026-04-06T03-04-28Z_omp_ses_001.jsonl"));
        assert!(result.get(ClientId::OpenCode).is_empty());
    }

    #[test]
    fn test_scan_all_clients_pi_from_both_paths() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_pi_dir(home);
        setup_mock_omp_dir(home);

        let result =
            scan_all_clients_with_env_strategy(home.to_str().unwrap(), &["pi".to_string()], false);
        assert_eq!(result.get(ClientId::Pi).len(), 2);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_zed_xdg_db() {
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let zed_db = setup_mock_zed_xdg_db(home);
        unsafe { std::env::set_var("XDG_DATA_HOME", home.join(".local/share")) };

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["zed".to_string()]);

        assert_eq!(result.zed_db.as_ref(), Some(&zed_db));
        restore_env("XDG_DATA_HOME", previous_xdg);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[serial]
    fn test_scan_all_clients_zed_macos_fallback() {
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let zed_db = setup_mock_zed_macos_db(home);
        unsafe { std::env::remove_var("XDG_DATA_HOME") };

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["zed".to_string()]);

        assert_eq!(result.zed_db.as_ref(), Some(&zed_db));
        restore_env("XDG_DATA_HOME", previous_xdg);
    }

    #[test]
    fn test_scan_all_clients_claude() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_claude_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["claude".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Claude).len(), 1);
        assert!(result.get(ClientId::OpenCode).is_empty());
    }

    /// Regression for #815: nested-layout subagent/workflow transcripts
    /// (`<session>/subagents/workflows/<wf>/agent-*.jsonl`) must be discovered by
    /// the recursive project-dir walk, so their usage is counted. The sibling
    /// `journal.jsonl` orchestration metadata is discovered too, but the parser
    /// drops it (covered in the claudecode parser tests).
    #[test]
    fn test_scan_all_clients_claude_nested_workflow_agents() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let wf = home.join(".claude/projects/myproject/sess-uuid/subagents/workflows/wf_abc");
        fs::create_dir_all(&wf).unwrap();
        let agent = wf.join("agent-a123.jsonl");
        File::create(&agent).unwrap().write_all(b"{}\n").unwrap();
        File::create(wf.join("journal.jsonl"))
            .unwrap()
            .write_all(b"{}\n")
            .unwrap();

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["claude".to_string()],
            false,
        );
        assert!(
            result.get(ClientId::Claude).iter().any(|p| p == &agent),
            "nested workflow agent transcript must be discovered, got {:?}",
            result.get(ClientId::Claude)
        );
    }

    #[test]
    fn test_scan_all_clients_claude_transcripts() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_claude_dir(home);
        let transcript = setup_mock_claude_transcripts_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["claude".to_string()],
            false,
        );

        assert_eq!(result.get(ClientId::Claude).len(), 2);
        assert!(
            result
                .get(ClientId::Claude)
                .iter()
                .any(|path| path == &transcript),
            "expected Claude transcript {} in {:?}",
            transcript.display(),
            result.get(ClientId::Claude)
        );
        assert!(result.get(ClientId::OpenCode).is_empty());
    }

    #[test]
    fn test_scan_all_clients_claude_transcripts_without_projects_dir() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let transcript = setup_mock_claude_transcripts_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["claude".to_string()],
            false,
        );

        assert_eq!(result.get(ClientId::Claude), &vec![transcript]);
        assert!(result.get(ClientId::OpenCode).is_empty());
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_claude_honors_claude_config_dir() {
        let mut env = EnvGuard::capture(&["CLAUDE_CONFIG_DIR"]);

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        // A stray ~/.claude/projects entry must be ignored once
        // CLAUDE_CONFIG_DIR redirects the client elsewhere.
        setup_mock_claude_dir(home);

        let custom_root = home.join("custom-claude-config");
        let custom_projects = custom_root.join("projects").join("myproject");
        fs::create_dir_all(&custom_projects).unwrap();
        let custom_conversation = custom_projects.join("conversation.jsonl");
        File::create(&custom_conversation)
            .unwrap()
            .write_all(b"")
            .unwrap();
        let custom_transcripts = custom_root.join("transcripts");
        fs::create_dir_all(&custom_transcripts).unwrap();
        let custom_transcript = custom_transcripts.join("ses_123456789012345678901234567.jsonl");
        File::create(&custom_transcript)
            .unwrap()
            .write_all(b"")
            .unwrap();

        env.set("CLAUDE_CONFIG_DIR", custom_root);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["claude".to_string()],
            true,
        );

        assert_eq!(result.get(ClientId::Claude).len(), 2);
        assert!(
            result
                .get(ClientId::Claude)
                .iter()
                .any(|p| p == &custom_conversation),
            "expected {} in {:?}",
            custom_conversation.display(),
            result.get(ClientId::Claude)
        );
        assert!(
            result
                .get(ClientId::Claude)
                .iter()
                .any(|p| p == &custom_transcript),
            "expected {} in {:?}",
            custom_transcript.display(),
            result.get(ClientId::Claude)
        );
    }

    #[test]
    fn test_scan_all_clients_claude_discovers_cc_mirror_variant_projects() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_claude_dir(home);

        let variant_dir = home.join(".cc-mirror/kimi-code");
        let config_dir = variant_dir.join("config");
        let project_dir = config_dir.join("projects/project-one");
        fs::create_dir_all(&project_dir).unwrap();
        let variant_file = variant_dir.join("variant.json");
        fs::write(
            &variant_file,
            format!(
                r#"{{"name":"kimi-code","provider":"kimi","configDir":{}}}"#,
                json_path_literal(&config_dir)
            ),
        )
        .unwrap();
        let variant_session = project_dir.join("variant-session.jsonl");
        File::create(&variant_session).unwrap();

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["claude".to_string()],
            false,
        );

        assert_eq!(result.get(ClientId::Claude).len(), 2);
        assert!(
            result
                .get(ClientId::Claude)
                .iter()
                .any(|path| path == &variant_session),
            "expected cc-mirror session {} in {:?}",
            variant_session.display(),
            result.get(ClientId::Claude)
        );
    }

    #[test]
    fn test_scan_all_clients_claude_dedups_cc_mirror_config_dir_pointing_at_normal_claude() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_claude_dir(home);

        let normal_claude_dir = home.join(".claude");
        let variant_dir = home.join(".cc-mirror/plain-mirror");
        fs::create_dir_all(&variant_dir).unwrap();
        fs::write(
            variant_dir.join("variant.json"),
            format!(
                r#"{{"name":"plain-mirror","provider":"mirror","configDir":{}}}"#,
                json_path_literal(&normal_claude_dir)
            ),
        )
        .unwrap();

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["claude".to_string()],
            false,
        );

        assert_eq!(
            result.get(ClientId::Claude).len(),
            1,
            "cc-mirror variants pointing at ~/.claude must not duplicate normal Claude files"
        );
    }

    #[test]
    fn test_scan_all_clients_gemini() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_gemini_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["gemini".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Gemini).len(), 1);
        assert!(result.get(ClientId::OpenCode).is_empty());
    }

    #[test]
    fn test_scan_all_clients_gemini_jsonl_session() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let gemini_path = home.join(".gemini/tmp/123/chats");
        fs::create_dir_all(&gemini_path).unwrap();
        File::create(gemini_path.join("session-abc.jsonl")).unwrap();

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["gemini".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Gemini).len(), 1);
        assert!(result.get(ClientId::Gemini)[0].ends_with("session-abc.jsonl"));
    }

    #[test]
    fn test_scan_all_clients_copilot() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_copilot_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["copilot".to_string()],
            false,
        );

        assert_eq!(result.get(ClientId::Copilot).len(), 1);
        assert!(result.get(ClientId::Copilot)[0].ends_with("copilot.jsonl"));
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_copilot_includes_explicit_exporter_file() {
        let previous = std::env::var("COPILOT_OTEL_FILE_EXPORTER_PATH").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let explicit_dir = home.join("otel-export");
        fs::create_dir_all(&explicit_dir).unwrap();
        let explicit_file = explicit_dir.join("copilot-explicit.jsonl");
        File::create(&explicit_file).unwrap();

        unsafe { std::env::set_var("COPILOT_OTEL_FILE_EXPORTER_PATH", &explicit_file) };

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["copilot".to_string()]);

        assert_eq!(result.get(ClientId::Copilot), &vec![explicit_file]);

        restore_env("COPILOT_OTEL_FILE_EXPORTER_PATH", previous);
    }

    #[test]
    fn test_scan_all_clients_openclaw_jsonl_only() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_openclaw_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["openclaw".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::OpenClaw).len(), 3);
        assert!(result
            .get(ClientId::OpenClaw)
            .iter()
            .any(|path| path.ends_with("session-abc.jsonl")));
        assert!(result
            .get(ClientId::OpenClaw)
            .iter()
            .any(|path| path.ends_with("session-deleted.jsonl.deleted.123")));
        assert!(result
            .get(ClientId::OpenClaw)
            .iter()
            .any(|path| path.ends_with("session-reset.jsonl.reset.456")));
    }

    #[test]
    fn test_scan_all_clients_openclaw_deleted_transcript() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let openclaw_sessions = home.join(".openclaw/agents/main/sessions");
        fs::create_dir_all(&openclaw_sessions).unwrap();
        File::create(openclaw_sessions.join("session-archived.jsonl.deleted.1700000000000"))
            .unwrap();

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["openclaw".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::OpenClaw).len(), 1);
        assert!(result.get(ClientId::OpenClaw)[0]
            .ends_with("session-archived.jsonl.deleted.1700000000000"));
    }

    #[test]
    fn test_scan_all_clients_multiple() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        setup_mock_claude_dir(home);
        setup_mock_gemini_dir(home);

        // use_env_roots=false to avoid interference from TOKSCALE_EXTRA_DIRS
        // set by parallel tests
        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["claude".to_string(), "gemini".to_string()],
            false,
        );

        assert_eq!(result.get(ClientId::Claude).len(), 1);
        assert_eq!(result.get(ClientId::Gemini).len(), 1);
        assert!(result.get(ClientId::OpenCode).is_empty());
        assert!(result.get(ClientId::Codex).is_empty());
    }

    #[test]
    fn test_scan_all_clients_kiro_includes_cli_and_global_storage() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_kiro_dir(home);
        setup_mock_kiro_global_storage_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["kiro".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Kiro).len(), 4);
        assert!(result
            .get(ClientId::Kiro)
            .iter()
            .any(|p| p.ends_with("session-001.json")));
        assert!(result
            .get(ClientId::Kiro)
            .iter()
            .any(|p| p.ends_with("execution.chat")));
        assert!(result
            .get(ClientId::Kiro)
            .iter()
            .any(|p| p.ends_with("execution")));
    }

    #[test]
    fn test_scan_all_clients_kiro_includes_ide_sessions() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let sess_dir = home.join(".kiro/sessions/workspace-a/sess_02f1c107");
        fs::create_dir_all(&sess_dir).unwrap();
        File::create(sess_dir.join("session.json")).unwrap();
        File::create(sess_dir.join("messages.jsonl")).unwrap();

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["kiro".to_string()],
            false,
        );
        assert!(result
            .get(ClientId::Kiro)
            .iter()
            .any(|p| p.ends_with("sess_02f1c107/session.json")));
        // The sibling messages.jsonl is read by the parser, not scanned directly.
        assert!(!result
            .get(ClientId::Kiro)
            .iter()
            .any(|p| p.ends_with("messages.jsonl")));
    }

    /// The `workspace_key` a Crush fixture path should produce, spelled out
    /// here rather than obtained from the production normalizer.
    ///
    /// `scan_crush_registry` keys a workspace with `normalize_workspace_key`,
    /// which folds `\` to `/` on purpose so one workspace reached under two
    /// separator spellings is one key. That fold is the claim these tests
    /// carry on Windows, and a raw `display()` expectation could not state it —
    /// it only agreed with the normalizer on Unix, where there is nothing to
    /// fold.
    ///
    /// Calling `normalize_workspace_key` in the expectation states it, but
    /// states it self-referentially: production applies the same function to
    /// the same input, so the assertion would hold no matter what that function
    /// did, including nothing. The fold is one line to write out, so write it
    /// out — this expectation is wrong whenever the normalizer stops folding,
    /// which is the entire point of having it.
    ///
    /// The single `replace` is the whole rule for these inputs. The normalizer
    /// also collapses repeated separators and trims a trailing one; a
    /// `TempDir`-rooted `join` produces neither, so nothing else applies. (It
    /// agrees on a UNC root too: `\\srv\share\p` folds to `//srv/share/p` with
    /// no doubled separator left inside to collapse.)
    fn expected_workspace_key(path: &Path) -> Option<String> {
        Some(path.to_string_lossy().replace('\\', "/"))
    }

    #[test]
    fn test_scan_crush_registry_resolves_relative_and_absolute_data_dirs() {
        let dir = TempDir::new().unwrap();
        let project_a = dir.path().join("project-a");
        let project_b_data = dir.path().join("project-b-data");
        fs::create_dir_all(project_a.join(".crush")).unwrap();
        fs::create_dir_all(&project_b_data).unwrap();
        File::create(project_a.join(".crush").join("crush.db")).unwrap();
        File::create(project_b_data.join("crush.db")).unwrap();

        let registry_path = dir.path().join("projects.json");
        let projects_json = format!(
            r#"{{
  "projects": [
    {{ "path": {}, "data_dir": ".crush" }},
    {{ "path": {}, "data_dir": {} }},
    {{ "path": {}, "data_dir": ".crush" }}
  ]
}}"#,
            json_path_literal(&project_a),
            json_path_literal(&dir.path().join("project-b")),
            json_path_literal(&project_b_data),
            json_path_literal(&dir.path().join("missing-project")),
        );
        setup_mock_crush_registry(&registry_path, &projects_json);

        let result = scan_crush_registry(&registry_path);
        assert_eq!(
            result,
            vec![
                CrushDbSource {
                    db_path: project_a.join(".crush").join("crush.db"),
                    workspace_key: expected_workspace_key(&project_a),
                    workspace_label: Some("project-a".to_string()),
                },
                CrushDbSource {
                    db_path: project_b_data.join("crush.db"),
                    workspace_key: expected_workspace_key(&dir.path().join("project-b")),
                    workspace_label: Some("project-b".to_string()),
                },
            ]
        );
    }

    #[test]
    fn test_scan_crush_registry_skips_malformed_project_entries() {
        let dir = TempDir::new().unwrap();
        let valid_project = dir.path().join("valid-project");
        fs::create_dir_all(valid_project.join(".crush")).unwrap();
        File::create(valid_project.join(".crush").join("crush.db")).unwrap();

        let registry_path = dir.path().join("projects.json");
        let projects_json = format!(
            r#"{{
  "projects": [
    {{ "path": {}, "data_dir": ".crush" }},
    {{ "path": 123, "data_dir": ".crush" }},
    {{ "data_dir": ".crush" }},
    "not-an-object"
  ]
}}"#,
            json_path_literal(&valid_project)
        );
        setup_mock_crush_registry(&registry_path, &projects_json);

        let result = scan_crush_registry(&registry_path);
        assert_eq!(
            result,
            vec![CrushDbSource {
                db_path: valid_project.join(".crush").join("crush.db"),
                workspace_key: expected_workspace_key(&valid_project),
                workspace_label: Some("valid-project".to_string()),
            }]
        );
    }

    #[test]
    #[serial]
    fn test_discover_crush_dbs_ignores_cwd_without_override() {
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();
        let previous_dir = std::env::current_dir().unwrap();

        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let project = dir.path().join("workspace");
        let nested = project.join("src/subdir");
        let xdg = dir.path().join("xdg");

        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(xdg.join("crush")).unwrap();
        fs::create_dir_all(project.join(".crush")).unwrap();
        File::create(project.join(".crush").join("crush.db")).unwrap();
        fs::write(
            xdg.join("crush").join("projects.json"),
            r#"{"projects":[]}"#,
        )
        .unwrap();

        unsafe { std::env::set_var("XDG_DATA_HOME", &xdg) };
        std::env::set_current_dir(&nested).unwrap();

        let result = discover_crush_dbs(home.to_str().unwrap(), false);
        assert!(result.is_empty());

        restore_current_dir(&previous_dir);
        restore_env("XDG_DATA_HOME", previous_xdg);
    }

    #[test]
    #[serial]
    fn test_discover_crush_dbs_honors_crush_global_data_env() {
        let previous_global = std::env::var("CRUSH_GLOBAL_DATA").ok();
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();
        let previous_local_app_data = std::env::var("LOCALAPPDATA").ok();
        unsafe { std::env::remove_var("LOCALAPPDATA") };

        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let global_data = dir.path().join("crush-global");
        let project = dir.path().join("project");
        fs::create_dir_all(project.join(".crush")).unwrap();
        File::create(project.join(".crush").join("crush.db")).unwrap();

        let projects_json = format!(
            r#"{{ "projects": [ {{ "path": {}, "data_dir": ".crush" }} ] }}"#,
            json_path_literal(&project)
        );
        setup_mock_crush_registry(&global_data.join("projects.json"), &projects_json);

        unsafe { std::env::set_var("CRUSH_GLOBAL_DATA", &global_data) };
        unsafe { std::env::remove_var("XDG_DATA_HOME") };

        let result = discover_crush_dbs(home.to_str().unwrap(), true);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].db_path, project.join(".crush").join("crush.db"));

        let without_env_roots = discover_crush_dbs(home.to_str().unwrap(), false);
        assert!(
            without_env_roots.is_empty(),
            "CRUSH_GLOBAL_DATA must be ignored when env roots are disabled"
        );

        restore_env("CRUSH_GLOBAL_DATA", previous_global);
        restore_env("XDG_DATA_HOME", previous_xdg);
        restore_env("LOCALAPPDATA", previous_local_app_data);
    }

    #[test]
    #[serial]
    fn test_discover_crush_dbs_scans_windows_local_appdata_under_home() {
        let previous_global = std::env::var("CRUSH_GLOBAL_DATA").ok();
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();
        unsafe { std::env::remove_var("CRUSH_GLOBAL_DATA") };
        unsafe { std::env::remove_var("XDG_DATA_HOME") };

        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let project = dir.path().join("project");
        fs::create_dir_all(project.join(".crush")).unwrap();
        File::create(project.join(".crush").join("crush.db")).unwrap();

        let projects_json = format!(
            r#"{{ "projects": [ {{ "path": {}, "data_dir": ".crush" }} ] }}"#,
            json_path_literal(&project)
        );
        setup_mock_crush_registry(
            &home.join("AppData/Local/crush/projects.json"),
            &projects_json,
        );

        let result = discover_crush_dbs(home.to_str().unwrap(), false);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].db_path, project.join(".crush").join("crush.db"));

        restore_env("CRUSH_GLOBAL_DATA", previous_global);
        restore_env("XDG_DATA_HOME", previous_xdg);
    }

    #[test]
    #[serial]
    fn test_discover_crush_dbs_dedups_across_registry_candidates() {
        let previous_global = std::env::var("CRUSH_GLOBAL_DATA").ok();
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();
        let previous_local_app_data = std::env::var("LOCALAPPDATA").ok();
        unsafe { std::env::remove_var("LOCALAPPDATA") };

        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let xdg = dir.path().join("xdg");
        let project = dir.path().join("project");
        fs::create_dir_all(project.join(".crush")).unwrap();
        File::create(project.join(".crush").join("crush.db")).unwrap();

        let projects_json = format!(
            r#"{{ "projects": [ {{ "path": {}, "data_dir": ".crush" }} ] }}"#,
            json_path_literal(&project)
        );
        setup_mock_crush_registry(&xdg.join("crush/projects.json"), &projects_json);
        setup_mock_crush_registry(
            &home.join("AppData/Local/crush/projects.json"),
            &projects_json,
        );

        unsafe { std::env::remove_var("CRUSH_GLOBAL_DATA") };
        unsafe { std::env::set_var("XDG_DATA_HOME", &xdg) };

        let result = discover_crush_dbs(home.to_str().unwrap(), true);
        assert_eq!(
            result.len(),
            1,
            "same crush.db reachable via multiple registries must be deduplicated"
        );

        restore_env("CRUSH_GLOBAL_DATA", previous_global);
        restore_env("XDG_DATA_HOME", previous_xdg);
        restore_env("LOCALAPPDATA", previous_local_app_data);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_crush_populates_crush_db_paths() {
        let mut env = EnvGuard::capture(&["XDG_DATA_HOME", "CRUSH_GLOBAL_DATA", "LOCALAPPDATA"]);
        env.remove("CRUSH_GLOBAL_DATA");
        env.remove("LOCALAPPDATA");

        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let xdg = dir.path().join("xdg");
        let project = dir.path().join("project");
        let data_dir = project.join(".crush");

        fs::create_dir_all(xdg.join("crush")).unwrap();
        fs::create_dir_all(&data_dir).unwrap();
        File::create(data_dir.join("crush.db")).unwrap();

        let registry_path = xdg.join("crush").join("projects.json");
        let projects_json = format!(
            r#"{{
  "projects": [
    {{ "path": {}, "data_dir": ".crush" }}
  ]
}}"#,
            json_path_literal(&project)
        );
        setup_mock_crush_registry(&registry_path, &projects_json);

        env.set("XDG_DATA_HOME", &xdg);

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["crush".to_string()]);
        assert_eq!(
            result.crush_dbs,
            vec![CrushDbSource {
                db_path: data_dir.join("crush.db"),
                workspace_key: expected_workspace_key(&project),
                workspace_label: Some("project".to_string()),
            }]
        );
        assert!(result.get(ClientId::Crush).is_empty());
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_headless_paths() {
        let mut env = EnvGuard::capture(&[
            "TOKSCALE_HEADLESS_DIR",
            "TOKSCALE_EXTRA_DIRS",
            "CODEX_HOME",
            "GEMINI_CLI_HOME",
        ]);
        env.remove("TOKSCALE_HEADLESS_DIR");
        env.remove("TOKSCALE_EXTRA_DIRS");
        env.remove("CODEX_HOME");
        env.remove("GEMINI_CLI_HOME");

        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let mac_root = home
            .join("Library")
            .join("Application Support")
            .join("tokscale")
            .join("headless");

        fs::create_dir_all(mac_root.join("codex")).unwrap();
        File::create(mac_root.join("codex").join("codex.jsonl")).unwrap();
        fs::create_dir_all(mac_root.join("mcode")).unwrap();
        File::create(mac_root.join("mcode").join("mcode.jsonl")).unwrap();

        let result = scan_all_clients(
            home.to_str().unwrap(),
            &[
                "claude".to_string(),
                "codex".to_string(),
                "gemini".to_string(),
                "mcode".to_string(),
            ],
        );

        assert!(result.get(ClientId::Claude).is_empty());
        assert_eq!(result.get(ClientId::Codex).len(), 1);
        assert!(result.get(ClientId::Gemini).is_empty());
        assert_eq!(result.get(ClientId::Mcode).len(), 1);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_codex_with_env() {
        let previous_codex = std::env::var("CODEX_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_codex_dir(home);

        // Set CODEX_HOME environment variable
        unsafe { std::env::set_var("CODEX_HOME", home.join(".codex")) };

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["codex".to_string()]);
        assert_eq!(result.get(ClientId::Codex).len(), 1);

        restore_env("CODEX_HOME", previous_codex);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_codex_home_override_ignores_codex_home_env() {
        let previous_codex = std::env::var("CODEX_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path().join("target-home");
        let conflicting = dir.path().join("conflicting-codex-home");
        setup_mock_codex_dir(&home);
        fs::create_dir_all(&conflicting).unwrap();

        unsafe { std::env::set_var("CODEX_HOME", &conflicting) };

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["codex".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Codex).len(), 1);
        assert!(result.get(ClientId::Codex)[0].ends_with("session.jsonl"));
        assert!(result.get(ClientId::Codex)[0].starts_with(home.join(".codex")));

        restore_env("CODEX_HOME", previous_codex);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_codex_archived_sessions() {
        let previous_codex = std::env::var("CODEX_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_codex_archived_dir(home);

        unsafe { std::env::set_var("CODEX_HOME", home.join(".codex")) };

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["codex".to_string()]);
        assert_eq!(result.get(ClientId::Codex).len(), 1);
        assert!(result.get(ClientId::Codex)[0].ends_with("archived.jsonl"));

        restore_env("CODEX_HOME", previous_codex);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_codex_sessions_and_archived() {
        let previous_codex = std::env::var("CODEX_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_codex_dir(home);
        setup_mock_codex_archived_dir(home);

        unsafe { std::env::set_var("CODEX_HOME", home.join(".codex")) };

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["codex".to_string()]);
        assert_eq!(result.get(ClientId::Codex).len(), 2);

        restore_env("CODEX_HOME", previous_codex);
    }

    #[test]
    fn test_scan_all_clients_kimi() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_kimi_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["kimi".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Kimi).len(), 1);
        assert!(result.get(ClientId::Kimi)[0].ends_with("wire.jsonl"));
        assert!(result.get(ClientId::OpenCode).is_empty());
        assert!(result.get(ClientId::Claude).is_empty());
    }

    /// An explicit KIMI_CODE_HOME moves Kimi Code discovery to that root.
    #[test]
    #[serial]
    fn test_scan_all_clients_kimi_code_home_override() {
        let mut env = EnvGuard::capture(&["KIMI_CODE_HOME"]);
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let custom_root = dir.path().join("custom-kimi-code");
        let wire = setup_mock_kimi_code_dir(&custom_root);
        env.set("KIMI_CODE_HOME", &custom_root);

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["kimi".to_string()]);

        assert_eq!(result.get(ClientId::Kimi), &vec![wire]);
    }

    /// Only *blank* values are reinterpreted. A non-blank value is used
    /// verbatim, so a root whose name carries surrounding whitespace still
    /// resolves — trimming the value here would silently miss it.
    #[test]
    #[serial]
    // Unix-only because the fixture cannot exist on Windows: a directory name
    // ending in a space is not addressable there. `CreateDirectoryW` strips the
    // trailing space, so `<tmp>\ padded-kimi-code ` becomes
    // `<tmp>\ padded-kimi-code`, and the very next call — which carries that
    // component in the middle of a longer path, where no stripping happens —
    // fails with ERROR_PATH_NOT_FOUND. The claim being made here (a padded
    // KIMI_CODE_HOME is honored verbatim rather than trimmed) is also one
    // Windows cannot violate: the OS trims the name before tokscale sees a
    // directory at all.
    #[cfg(unix)]
    fn test_scan_all_clients_kimi_code_home_override_is_not_trimmed() {
        let mut env = EnvGuard::capture(&["KIMI_CODE_HOME"]);
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let padded_root = dir.path().join(" padded-kimi-code ");
        let wire = setup_mock_kimi_code_dir(&padded_root);
        env.set("KIMI_CODE_HOME", &padded_root);

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["kimi".to_string()]);

        assert_eq!(result.get(ClientId::Kimi), &vec![wire]);
    }

    /// A blank KIMI_CODE_HOME — empty or whitespace-only, which is how
    /// shells export an optional variable that was never given a value — means
    /// "unset". Without this, `format!("{}/sessions", "")` walks the
    /// root-level `/sessions` and the user's real sessions go unreported.
    #[test]
    #[serial]
    fn test_scan_all_clients_kimi_code_home_blank_falls_back_to_home() {
        for blank in ["", "   ", "\t\n"] {
            let mut env = EnvGuard::capture(&["KIMI_CODE_HOME"]);
            let dir = TempDir::new().unwrap();
            let home = dir.path();
            let wire = setup_mock_kimi_code_dir(&home.join(".kimi-code"));
            env.set("KIMI_CODE_HOME", blank);

            let result = scan_without_extra_dirs(home.to_str().unwrap(), &["kimi".to_string()]);

            assert_eq!(
                result.get(ClientId::Kimi),
                &vec![wire],
                "KIMI_CODE_HOME={:?} must fall back to <home>/.kimi-code",
                blank
            );
        }
    }

    /// use_env_roots=false keeps `--home` authoritative: KIMI_CODE_HOME is
    /// never consulted, blank or not.
    #[test]
    #[serial]
    fn test_scan_all_clients_kimi_code_home_ignored_without_env_roots() {
        let mut env = EnvGuard::capture(&["KIMI_CODE_HOME"]);
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let conflicting = dir.path().join("conflicting-kimi-code");
        let wire = setup_mock_kimi_code_dir(&home.join(".kimi-code"));
        setup_mock_kimi_code_dir(&conflicting);
        env.set("KIMI_CODE_HOME", &conflicting);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["kimi".to_string()],
            false,
        );

        assert_eq!(result.get(ClientId::Kimi), &vec![wire]);
    }

    #[test]
    fn test_scan_all_clients_grok() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_grok_dir(home);

        let unified_dir = home.join(".grok/logs");
        fs::create_dir_all(&unified_dir).unwrap();
        let unified_log = unified_dir.join("unified.jsonl");
        File::create(&unified_log).unwrap();
        let nested_dir = unified_dir.join("archive");
        fs::create_dir_all(&nested_dir).unwrap();
        let nested_log = nested_dir.join("unified.jsonl");
        File::create(&nested_log).unwrap();

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["grok".to_string()],
            false,
        );
        let grok_files = result.get(ClientId::Grok);
        assert_eq!(grok_files.len(), 2);
        assert!(grok_files
            .iter()
            .any(|path| path.ends_with("updates.jsonl")));
        assert!(grok_files.iter().any(|path| path == &unified_log));
        assert!(!grok_files.iter().any(|path| path == &nested_log));
        assert!(result.get(ClientId::OpenCode).is_empty());
        assert!(result.get(ClientId::Claude).is_empty());
    }

    #[test]
    fn test_scan_all_clients_jcode() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_jcode_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["jcode".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Jcode).len(), 1);
        assert!(result.get(ClientId::Jcode)[0].ends_with("session_fixture.json"));
        assert!(result.get(ClientId::OpenCode).is_empty());
        assert!(result.get(ClientId::Claude).is_empty());
    }

    #[test]
    fn test_scan_all_clients_roocode() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_roocode_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["roocode".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::RooCode).len(), 2);
        assert!(result
            .get(ClientId::RooCode)
            .iter()
            .all(|p| p.ends_with("ui_messages.json")));
    }

    #[test]
    fn test_scan_all_clients_kilocode() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_kilocode_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["kilocode".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::KiloCode).len(), 2);
        assert!(result
            .get(ClientId::KiloCode)
            .iter()
            .all(|p| p.ends_with("ui_messages.json")));
    }

    #[test]
    fn test_scan_all_clients_cline() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_cline_dir(home);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["cline".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Cline).len(), 4);
        assert!(result
            .get(ClientId::Cline)
            .iter()
            .all(|p| p.ends_with("ui_messages.json")));
    }

    #[test]
    fn test_scan_all_clients_cline_cli() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_cline_cli_dir(&home.join(".cline/data"));

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["cline".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Cline).len(), 1);
        assert!(result.get(ClientId::Cline)[0]
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".messages.json")));
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_cline_cli_session_data_dir_takes_precedence() {
        let mut env = EnvGuard::capture(&["CLINE_SESSION_DATA_DIR", "CLINE_DATA_DIR", "CLINE_DIR"]);
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let session_data_dir = dir.path().join("custom-cline-sessions");
        let data_dir = dir.path().join("custom-cline-data");
        let cline_dir = dir.path().join("custom-cline");

        setup_mock_cline_cli_session_root(&session_data_dir);
        setup_mock_cline_cli_dir(&data_dir);
        setup_mock_cline_cli_dir(&cline_dir.join("data"));
        setup_mock_cline_cli_dir(&home.join(".cline/data"));
        env.set("CLINE_SESSION_DATA_DIR", &session_data_dir);
        env.set("CLINE_DATA_DIR", &data_dir);
        env.set("CLINE_DIR", &cline_dir);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["cline".to_string()],
            true,
        );
        let expected = session_data_dir.join("cli-session/cli-session.messages.json");

        assert_eq!(result.get(ClientId::Cline), &vec![expected]);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_cline_cli_uses_data_dir_override() {
        let mut env = EnvGuard::capture(&["CLINE_SESSION_DATA_DIR", "CLINE_DATA_DIR", "CLINE_DIR"]);
        env.remove("CLINE_SESSION_DATA_DIR");
        env.remove("CLINE_DIR");
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let data_dir = dir.path().join("custom-cline-data");
        let cline_dir = dir.path().join("custom-cline");

        setup_mock_cline_cli_dir(&data_dir);
        setup_mock_cline_cli_dir(&cline_dir.join("data"));
        setup_mock_cline_cli_dir(&home.join(".cline/data"));
        env.set("CLINE_DATA_DIR", &data_dir);
        env.set("CLINE_DIR", &cline_dir);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["cline".to_string()],
            true,
        );
        let expected = data_dir.join("sessions/cli-session/cli-session.messages.json");

        assert_eq!(result.get(ClientId::Cline), &vec![expected]);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_cline_cli_uses_cline_dir_override() {
        let mut env = EnvGuard::capture(&["CLINE_SESSION_DATA_DIR", "CLINE_DATA_DIR", "CLINE_DIR"]);
        env.remove("CLINE_SESSION_DATA_DIR");
        env.remove("CLINE_DATA_DIR");
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let cline_dir = dir.path().join("custom-cline");

        setup_mock_cline_cli_dir(&cline_dir.join("data"));
        setup_mock_cline_cli_dir(&home.join(".cline/data"));
        env.set("CLINE_DIR", &cline_dir);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["cline".to_string()],
            true,
        );
        let expected = cline_dir.join("data/sessions/cli-session/cli-session.messages.json");

        assert_eq!(result.get(ClientId::Cline), &vec![expected]);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_cline_cli_ignores_env_roots_when_disabled() {
        let mut env = EnvGuard::capture(&["CLINE_SESSION_DATA_DIR", "CLINE_DATA_DIR", "CLINE_DIR"]);
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let session_data_dir = dir.path().join("custom-cline-sessions");
        let data_dir = dir.path().join("custom-cline-data");
        let cline_dir = dir.path().join("custom-cline");

        setup_mock_cline_cli_session_root(&session_data_dir);
        setup_mock_cline_cli_dir(&data_dir);
        setup_mock_cline_cli_dir(&cline_dir.join("data"));
        setup_mock_cline_cli_dir(&home.join(".cline/data"));
        env.set("CLINE_SESSION_DATA_DIR", &session_data_dir);
        env.set("CLINE_DATA_DIR", &data_dir);
        env.set("CLINE_DIR", &cline_dir);

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["cline".to_string()],
            false,
        );
        let expected = home.join(".cline/data/sessions/cli-session/cli-session.messages.json");

        assert_eq!(result.get(ClientId::Cline), &vec![expected]);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_cline_cli_whitespace_data_dir_uses_default() {
        let mut env = EnvGuard::capture(&["CLINE_SESSION_DATA_DIR", "CLINE_DATA_DIR", "CLINE_DIR"]);
        env.remove("CLINE_SESSION_DATA_DIR");
        env.remove("CLINE_DIR");
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_cline_cli_dir(&home.join(".cline/data"));
        env.set("CLINE_DATA_DIR", " \t ");

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["cline".to_string()],
            true,
        );
        let expected = home.join(".cline/data/sessions/cli-session/cli-session.messages.json");

        assert_eq!(result.get(ClientId::Cline), &vec![expected]);
    }

    #[test]
    fn test_parse_extra_dirs_basic() {
        let enabled: HashSet<ClientId> = [ClientId::Claude, ClientId::OpenClaw]
            .iter()
            .copied()
            .collect();
        let dirs = parse_extra_dirs("claude:/tmp/mac-sessions,openclaw:/tmp/oc-extra", &enabled);
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[0].0, ClientId::Claude);
        assert_eq!(dirs[0].1, "/tmp/mac-sessions");
        assert_eq!(dirs[1].0, ClientId::OpenClaw);
        assert_eq!(dirs[1].1, "/tmp/oc-extra");
    }

    #[test]
    fn test_parse_extra_dirs_filters_disabled_clients() {
        let enabled: HashSet<ClientId> = [ClientId::Claude].iter().copied().collect();
        let dirs = parse_extra_dirs(
            "claude:/tmp/mac-sessions,gemini:/tmp/gemini-extra",
            &enabled,
        );
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].0, ClientId::Claude);
    }

    #[test]
    fn test_parse_extra_dirs_skips_unsupported_clients() {
        let enabled: HashSet<ClientId> =
            [ClientId::Claude, ClientId::Kilo].iter().copied().collect();
        let dirs = parse_extra_dirs("claude:/tmp/mac-sessions,kilo:/tmp/kilo", &enabled);
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].0, ClientId::Claude);
        assert_eq!(dirs[0].1, "/tmp/mac-sessions");
    }

    #[test]
    fn test_parse_extra_dirs_empty_string() {
        let enabled: HashSet<ClientId> = ClientId::iter().collect();
        let dirs = parse_extra_dirs("", &enabled);
        assert!(dirs.is_empty());
    }

    #[test]
    fn test_parse_extra_dirs_invalid_client() {
        let enabled: HashSet<ClientId> = ClientId::iter().collect();
        let dirs = parse_extra_dirs("nonexistent:/tmp/foo", &enabled);
        assert!(dirs.is_empty());
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_extra_dirs() {
        let previous = std::env::var("TOKSCALE_EXTRA_DIRS").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();

        // Setup default Claude dir
        setup_mock_claude_dir(home);

        // Setup extra dir with additional session files
        let extra_dir = TempDir::new().unwrap();
        let extra_project = extra_dir.path().join("mac-project");
        fs::create_dir_all(&extra_project).unwrap();
        File::create(extra_project.join("extra-session.jsonl")).unwrap();

        unsafe {
            std::env::set_var(
                "TOKSCALE_EXTRA_DIRS",
                format!("claude:{}", extra_dir.path().to_string_lossy()),
            )
        };

        let result = scan_all_clients(home.to_str().unwrap(), &["claude".to_string()]);
        // 1 from default path + 1 from extra dir
        assert_eq!(result.get(ClientId::Claude).len(), 2);

        restore_env("TOKSCALE_EXTRA_DIRS", previous);
    }

    fn setup_mock_codebuff_chat(base: &Path, channel: &str, chat_id: &str) -> PathBuf {
        let chat_dir = base
            .join(".config")
            .join(channel)
            .join("projects")
            .join("sandbox")
            .join("chats")
            .join(chat_id);
        fs::create_dir_all(&chat_dir).unwrap();
        let file_path = chat_dir.join("chat-messages.json");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "[]").unwrap();
        file_path
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_codebuff_walks_all_three_channels_by_default() {
        let previous = std::env::var("CODEBUFF_DATA_DIR").ok();
        unsafe { std::env::remove_var("CODEBUFF_DATA_DIR") };

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_codebuff_chat(home, "manicode", "2025-12-14T10-00-00.000Z");
        setup_mock_codebuff_chat(home, "manicode-dev", "2025-12-14T11-00-00.000Z");
        setup_mock_codebuff_chat(home, "manicode-staging", "2025-12-14T12-00-00.000Z");

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["codebuff".to_string()]);
        assert_eq!(result.get(ClientId::Codebuff).len(), 3);

        restore_env("CODEBUFF_DATA_DIR", previous);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_codebuff_empty_env_var_falls_back_to_default_channels() {
        let previous = std::env::var("CODEBUFF_DATA_DIR").ok();
        // Regression: a whitespace-only override used to produce zero scan
        // roots because the `Some(_)` branch was taken and then skipped.
        unsafe { std::env::set_var("CODEBUFF_DATA_DIR", "   ") };

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_codebuff_chat(home, "manicode", "2025-12-14T10-00-00.000Z");
        setup_mock_codebuff_chat(home, "manicode-dev", "2025-12-14T11-00-00.000Z");

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["codebuff".to_string()]);
        assert_eq!(result.get(ClientId::Codebuff).len(), 2);

        restore_env("CODEBUFF_DATA_DIR", previous);
    }

    #[test]
    fn join_native_preserves_an_absolute_root() {
        // `PathBuf::push` on an empty buffer emits no leading separator, so a
        // caller that strips the trailing `/` from a root of `/` turns an
        // absolute scan root into a cwd-relative one. Nothing about the join
        // needs that strip: a trailing separator is collapsed either way.
        #[cfg(unix)]
        {
            assert_eq!(join_native("/", "projects"), "/projects");
            assert_eq!(join_native("/foo/", "projects"), "/foo/projects");
            assert_eq!(join_native("/foo", "projects"), "/foo/projects");
        }
        // An empty root is the shape that produced the relative path.
        assert_eq!(join_native("", "projects"), "projects");
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_codebuff_override_root_may_end_in_a_separator() {
        let previous = std::env::var("CODEBUFF_DATA_DIR").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        let override_root = dir.path().join("custom-codebuff");
        let override_chat_dir = override_root
            .join("projects")
            .join("sandbox")
            .join("chats")
            .join("2025-12-14T11-00-00.000Z");
        fs::create_dir_all(&override_chat_dir).unwrap();
        File::create(override_chat_dir.join("chat-messages.json")).unwrap();

        // Trailing separator: the reason the call site used to trim.
        unsafe {
            std::env::set_var(
                "CODEBUFF_DATA_DIR",
                format!("{}/", override_root.to_string_lossy()),
            )
        };

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["codebuff".to_string()]);
        assert_eq!(result.get(ClientId::Codebuff).len(), 1);

        restore_env("CODEBUFF_DATA_DIR", previous);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_codebuff_honours_explicit_env_override() {
        let previous = std::env::var("CODEBUFF_DATA_DIR").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        // Default-channel data that should NOT be picked up when the env is set.
        setup_mock_codebuff_chat(home, "manicode", "2025-12-14T10-00-00.000Z");
        // Override target (lives OUTSIDE ~/.config to prove the override wins).
        let override_root = dir.path().join("custom-codebuff");
        let override_chat_dir = override_root
            .join("projects")
            .join("sandbox")
            .join("chats")
            .join("2025-12-14T11-00-00.000Z");
        fs::create_dir_all(&override_chat_dir).unwrap();
        File::create(override_chat_dir.join("chat-messages.json")).unwrap();

        unsafe {
            std::env::set_var(
                "CODEBUFF_DATA_DIR",
                override_root.to_string_lossy().as_ref(),
            )
        };

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["codebuff".to_string()]);
        assert_eq!(result.get(ClientId::Codebuff).len(), 1);
        assert!(result.get(ClientId::Codebuff)[0]
            .to_string_lossy()
            .contains("custom-codebuff"));

        restore_env("CODEBUFF_DATA_DIR", previous);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_freebuff_enables_shared_manicode_scan() {
        let mut env = EnvGuard::capture(&["FREEBUFF_DATA_DIR", "CODEBUFF_DATA_DIR"]);
        env.remove("FREEBUFF_DATA_DIR");
        env.remove("CODEBUFF_DATA_DIR");

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_codebuff_chat(home, "manicode", "2025-12-14T10-00-00.000Z");

        // Freebuff shares Codebuff's manicode layout, so enabling it alone must
        // still walk the shared root (populating the Codebuff scan vector) for
        // the estimated Freebuff parser in lib.rs to have files to read.
        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["freebuff".to_string()]);
        assert_eq!(result.get(ClientId::Codebuff).len(), 1);
    }

    #[test]
    #[serial]
    fn test_freebuff_fallback_does_not_defeat_codebuff_data_dir_exclusivity() {
        // CODEBUFF_DATA_DIR is documented as exclusive: point it somewhere and
        // the default ~/.config/manicode channels are not scanned. Freebuff
        // writes into that same tree, so with only CODEBUFF_DATA_DIR set it has
        // to follow the redirect. Falling back to the default channel roots for
        // the Freebuff half of an all-clients run would re-scan the very
        // directory the user redirected away from.
        let mut env = EnvGuard::capture(&["FREEBUFF_DATA_DIR", "CODEBUFF_DATA_DIR"]);
        env.remove("FREEBUFF_DATA_DIR");

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        // The default location the user redirected away from.
        setup_mock_codebuff_chat(home, "manicode", "2025-12-14T10-00-00.000Z");

        // The redirect target, with its own chat.
        let override_home = TempDir::new().unwrap();
        let override_root = override_home.path().join("elsewhere");
        let redirected_chat = override_root
            .join("projects")
            .join("sandbox")
            .join("chats")
            .join("2025-12-14T11-00-00.000Z");
        fs::create_dir_all(&redirected_chat).unwrap();
        writeln!(
            File::create(redirected_chat.join("chat-messages.json")).unwrap(),
            "[]"
        )
        .unwrap();
        env.set(
            "CODEBUFF_DATA_DIR",
            override_root.to_string_lossy().as_ref(),
        );

        let result = scan_without_extra_dirs(
            home.to_str().unwrap(),
            &["codebuff".to_string(), "freebuff".to_string()],
        );

        let found = result.get(ClientId::Codebuff);
        assert_eq!(
            found.len(),
            1,
            "only the redirected root should be scanned, got {found:?}"
        );
        assert!(
            !found[0].to_string_lossy().contains("manicode"),
            "the default manicode root must not be re-scanned via the Freebuff \
             fallback, got {found:?}"
        );
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_codebuff_ignores_freebuff_override_when_not_enabled() {
        let mut env = EnvGuard::capture(&["FREEBUFF_DATA_DIR", "CODEBUFF_DATA_DIR"]);
        env.remove("CODEBUFF_DATA_DIR");
        let override_dir = TempDir::new().unwrap();
        // The Freebuff override must NOT redirect a codebuff-only scan: each
        // client resolves only its own override (see the shared-manicode scan).
        unsafe {
            std::env::set_var(
                "FREEBUFF_DATA_DIR",
                override_dir.path().to_string_lossy().as_ref(),
            )
        };

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_codebuff_chat(home, "manicode", "2025-12-14T10-00-00.000Z");

        let result = scan_without_extra_dirs(home.to_str().unwrap(), &["codebuff".to_string()]);
        assert_eq!(result.get(ClientId::Codebuff).len(), 1);
        // Compare against a natively joined suffix: scan roots are built with
        // `join_native`, so this path is `manicode\projects` on Windows and a
        // hardcoded `manicode/projects` never matches there.
        let found = result.get(ClientId::Codebuff)[0]
            .to_string_lossy()
            .into_owned();
        assert!(
            found.contains(&join_native("manicode", "projects")),
            "expected the default manicode root, got {found}"
        );
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_ignores_extra_dirs_when_env_roots_disabled() {
        let previous = std::env::var("TOKSCALE_EXTRA_DIRS").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_claude_dir(home);

        let extra_dir = TempDir::new().unwrap();
        let extra_project = extra_dir.path().join("mac-project");
        fs::create_dir_all(&extra_project).unwrap();
        File::create(extra_project.join("extra-session.jsonl")).unwrap();

        unsafe {
            std::env::set_var(
                "TOKSCALE_EXTRA_DIRS",
                format!("claude:{}", extra_dir.path().to_string_lossy()),
            )
        };

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["claude".to_string()],
            false,
        );
        assert_eq!(result.get(ClientId::Claude).len(), 1);

        restore_env("TOKSCALE_EXTRA_DIRS", previous);
    }

    /// Verify that an extra scan path outside $HOME does not abort the scan.
    /// `warn_if_escapes_home` must only warn, never block.
    #[test]
    #[serial]
    fn test_extra_scan_path_outside_home_does_not_block_scan() {
        let fake_home = TempDir::new().unwrap();
        let outside_home = TempDir::new().unwrap();
        let outside_path = outside_home.path();
        assert!(!outside_path.starts_with(fake_home.path()));

        // Populate with a valid session file so the scanner has something to find.
        let session_dir = outside_path.join("sessions");
        fs::create_dir_all(&session_dir).unwrap();
        File::create(session_dir.join("session-abc123.json")).unwrap();

        // Set TOKSCALE_EXTRA_DIRS to point claude at the outside path.
        let previous = std::env::var("TOKSCALE_EXTRA_DIRS").ok();
        unsafe {
            std::env::set_var(
                "TOKSCALE_EXTRA_DIRS",
                format!("claude:{}", outside_path.to_string_lossy()),
            )
        };

        // The scan must complete without panicking.
        let _result = scan_all_clients_with_env_strategy(
            fake_home.path().to_str().unwrap(),
            &["claude".to_string()],
            true, // use_env_roots = true so TOKSCALE_EXTRA_DIRS is picked up
        );

        restore_env("TOKSCALE_EXTRA_DIRS", previous);
        // No assertion on result.get(ClientId::Claude) — the outside dir might
        // not match the expected file patterns. The test goal is only liveness:
        // the scan must not panic when an extra path escapes $HOME.
    }
    /// Write a gjc session JSONL file at
    /// <home>/.gjc/agent/sessions/<slug>/<name> and return its path.
    fn setup_mock_gjc_session(home: &Path, slug: &str, name: &str) -> PathBuf {
        let dir = home.join(".gjc/agent/sessions").join(slug);
        fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join(name);
        File::create(&file_path).unwrap();
        file_path
    }

    #[test]
    #[serial]
    fn test_gjc_discovery_recursive_glob_depth1_and_depth2() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        // depth 1: <slug>/<id>.jsonl
        setup_mock_gjc_session(home, "--work--proj--", "sess-001.jsonl");
        // depth 2: <slug>/<session>/N-Pass.jsonl
        let depth2 = home.join(".gjc/agent/sessions/--work--proj--/sess-001");
        fs::create_dir_all(&depth2).unwrap();
        File::create(depth2.join("0-Pass.jsonl")).unwrap();

        let result =
            scan_all_clients_with_env_strategy(home.to_str().unwrap(), &["gjc".to_string()], false);
        assert_eq!(result.get(ClientId::Gjc).len(), 2);
    }

    #[test]
    #[serial]
    fn test_gjc_discovery_home_fallback_when_env_disabled() {
        let previous = std::env::var("GJC_CODING_AGENT_DIR").ok();
        // Even with the env var set, use_env_roots=false must ignore it and
        // read only the home fallback.
        let other = TempDir::new().unwrap();
        unsafe {
            std::env::set_var(
                "GJC_CODING_AGENT_DIR",
                other.path().to_string_lossy().as_ref(),
            )
        };

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_gjc_session(home, "slug", "a.jsonl");

        let result =
            scan_all_clients_with_env_strategy(home.to_str().unwrap(), &["gjc".to_string()], false);
        assert_eq!(result.get(ClientId::Gjc).len(), 1);

        restore_env("GJC_CODING_AGENT_DIR", previous);
    }

    #[test]
    #[serial]
    fn test_gjc_discovery_env_override() {
        let mut env = EnvGuard::capture(&[
            "GJC_CODING_AGENT_DIR",
            "GJC_CONFIG_DIR",
            "PI_CONFIG_DIR",
            "XDG_DATA_HOME",
            "TOKSCALE_EXTRA_DIRS",
        ]);
        env.remove("GJC_CONFIG_DIR");
        env.remove("PI_CONFIG_DIR");
        env.remove("XDG_DATA_HOME");
        env.remove("TOKSCALE_EXTRA_DIRS");

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        // Override target lives OUTSIDE ~/.gjc to prove the override is read.
        let agent_dir = dir.path().join("custom-gjc-agent");
        let override_sessions = agent_dir.join("sessions").join("slug");
        fs::create_dir_all(&override_sessions).unwrap();
        File::create(override_sessions.join("o.jsonl")).unwrap();

        env.set("GJC_CODING_AGENT_DIR", &agent_dir);

        let result = scan_all_clients(home.to_str().unwrap(), &["gjc".to_string()]);
        assert!(result
            .get(ClientId::Gjc)
            .iter()
            .any(|p| p.to_string_lossy().contains("custom-gjc-agent")));
    }

    #[test]
    #[serial]
    fn test_gjc_discovery_multi_root_files_dedup_to_one() {
        // When GJC_CODING_AGENT_DIR points at the same on-disk location the
        // home fallback also resolves, the file must be counted ONCE.
        let mut env = EnvGuard::capture(&[
            "GJC_CODING_AGENT_DIR",
            "GJC_CONFIG_DIR",
            "PI_CONFIG_DIR",
            "XDG_DATA_HOME",
            "TOKSCALE_EXTRA_DIRS",
        ]);
        env.remove("GJC_CONFIG_DIR");
        env.remove("PI_CONFIG_DIR");
        env.remove("XDG_DATA_HOME");
        env.remove("TOKSCALE_EXTRA_DIRS");

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_gjc_session(home, "slug", "dup.jsonl");

        // Point the env var at <home>/.gjc/agent so root (1) and root (4)
        // resolve to the same directory.
        let agent_dir = home.join(".gjc/agent");
        env.set("GJC_CODING_AGENT_DIR", &agent_dir);

        let result = scan_all_clients(home.to_str().unwrap(), &["gjc".to_string()]);
        assert_eq!(result.get(ClientId::Gjc).len(), 1);
    }

    // -----------------------------------------------------------------------
    // Adversarial discovery tests for the gjc block
    // -----------------------------------------------------------------------

    /// (a) GJC_CONFIG_DIR set → <config>/agent/sessions/<slug>/x.jsonl discovered.
    #[test]
    #[serial]
    fn test_gjc_discovery_gjc_config_dir() {
        let prev_agent = std::env::var("GJC_CODING_AGENT_DIR").ok();
        let prev_config = std::env::var("GJC_CONFIG_DIR").ok();
        let prev_pi = std::env::var("PI_CONFIG_DIR").ok();
        let prev_xdg = std::env::var("XDG_DATA_HOME").ok();

        // Clear all interfering env vars; we only want root (2) via GJC_CONFIG_DIR.
        unsafe {
            std::env::remove_var("GJC_CODING_AGENT_DIR");
            std::env::remove_var("PI_CONFIG_DIR");
            std::env::remove_var("XDG_DATA_HOME");
        }

        let home_dir = TempDir::new().unwrap();
        let config_dir = TempDir::new().unwrap();

        // Seed a file under the config-dir root.
        let sessions = config_dir.path().join("agent/sessions/my-slug");
        fs::create_dir_all(&sessions).unwrap();
        File::create(sessions.join("x.jsonl")).unwrap();

        unsafe {
            std::env::set_var(
                "GJC_CONFIG_DIR",
                config_dir.path().to_string_lossy().as_ref(),
            )
        };

        let result =
            scan_without_extra_dirs(home_dir.path().to_str().unwrap(), &["gjc".to_string()]);
        assert!(
            !result.get(ClientId::Gjc).is_empty(),
            "expected at least 1 file from GJC_CONFIG_DIR root, got {:?}",
            result.get(ClientId::Gjc)
        );
        assert!(
            result
                .get(ClientId::Gjc)
                .iter()
                .any(|p| p.to_string_lossy().contains("my-slug")),
            "discovered files should include the GJC_CONFIG_DIR session path"
        );

        restore_env("GJC_CODING_AGENT_DIR", prev_agent);
        restore_env("GJC_CONFIG_DIR", prev_config);
        restore_env("PI_CONFIG_DIR", prev_pi);
        restore_env("XDG_DATA_HOME", prev_xdg);
    }

    /// (b) PI_CONFIG_DIR set with GJC_CODING_AGENT_DIR and GJC_CONFIG_DIR unset →
    ///     <pi-config>/agent/sessions/<slug>/x.jsonl discovered.
    #[test]
    #[serial]
    fn test_gjc_discovery_pi_config_dir() {
        let prev_agent = std::env::var("GJC_CODING_AGENT_DIR").ok();
        let prev_config = std::env::var("GJC_CONFIG_DIR").ok();
        let prev_pi = std::env::var("PI_CONFIG_DIR").ok();
        let prev_xdg = std::env::var("XDG_DATA_HOME").ok();

        unsafe {
            std::env::remove_var("GJC_CODING_AGENT_DIR");
            std::env::remove_var("GJC_CONFIG_DIR");
            std::env::remove_var("XDG_DATA_HOME");
        }

        let home_dir = TempDir::new().unwrap();
        let pi_config = TempDir::new().unwrap();

        let sessions = pi_config.path().join("agent/sessions/pi-slug");
        fs::create_dir_all(&sessions).unwrap();
        File::create(sessions.join("x.jsonl")).unwrap();

        unsafe { std::env::set_var("PI_CONFIG_DIR", pi_config.path().to_string_lossy().as_ref()) };

        let result =
            scan_without_extra_dirs(home_dir.path().to_str().unwrap(), &["gjc".to_string()]);
        assert!(
            !result.get(ClientId::Gjc).is_empty(),
            "expected at least 1 file from PI_CONFIG_DIR root, got {:?}",
            result.get(ClientId::Gjc)
        );
        assert!(
            result
                .get(ClientId::Gjc)
                .iter()
                .any(|p| p.to_string_lossy().contains("pi-slug")),
            "discovered files should include the PI_CONFIG_DIR session path"
        );

        restore_env("GJC_CODING_AGENT_DIR", prev_agent);
        restore_env("GJC_CONFIG_DIR", prev_config);
        restore_env("PI_CONFIG_DIR", prev_pi);
        restore_env("XDG_DATA_HOME", prev_xdg);
    }

    /// (c) XDG_DATA_HOME redirect — flattened path <xdg>/gjc/sessions/<slug>/x.jsonl
    ///     is discovered (the `agent/` segment is NOT present).
    #[test]
    #[serial]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn test_gjc_discovery_xdg_data_home_flattened() {
        let prev_agent = std::env::var("GJC_CODING_AGENT_DIR").ok();
        let prev_config = std::env::var("GJC_CONFIG_DIR").ok();
        let prev_pi = std::env::var("PI_CONFIG_DIR").ok();
        let prev_xdg = std::env::var("XDG_DATA_HOME").ok();

        unsafe {
            std::env::remove_var("GJC_CODING_AGENT_DIR");
            std::env::remove_var("GJC_CONFIG_DIR");
            std::env::remove_var("PI_CONFIG_DIR");
        }

        let home_dir = TempDir::new().unwrap();
        let xdg_data = TempDir::new().unwrap();

        // The XDG redirect flattens the `agent/` segment.
        let sessions = xdg_data.path().join("gjc/sessions/xdg-slug");
        fs::create_dir_all(&sessions).unwrap();
        File::create(sessions.join("x.jsonl")).unwrap();

        unsafe { std::env::set_var("XDG_DATA_HOME", xdg_data.path().to_string_lossy().as_ref()) };

        let result =
            scan_without_extra_dirs(home_dir.path().to_str().unwrap(), &["gjc".to_string()]);
        assert!(
            !result.get(ClientId::Gjc).is_empty(),
            "expected at least 1 file from XDG_DATA_HOME/gjc/sessions, got {:?}",
            result.get(ClientId::Gjc)
        );
        assert!(
            result
                .get(ClientId::Gjc)
                .iter()
                .any(|p| p.to_string_lossy().contains("xdg-slug")),
            "XDG redirect path must be discovered (flattened, no agent/ segment)"
        );

        restore_env("GJC_CODING_AGENT_DIR", prev_agent);
        restore_env("GJC_CONFIG_DIR", prev_config);
        restore_env("PI_CONFIG_DIR", prev_pi);
        restore_env("XDG_DATA_HOME", prev_xdg);
    }

    /// (d) Multi-root N4: home fallback file + XDG redirect file (DIFFERENT files,
    ///     different slugs) → count == 2.
    #[test]
    #[serial]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn test_gjc_discovery_multi_root_home_and_xdg_both_counted() {
        let prev_agent = std::env::var("GJC_CODING_AGENT_DIR").ok();
        let prev_config = std::env::var("GJC_CONFIG_DIR").ok();
        let prev_pi = std::env::var("PI_CONFIG_DIR").ok();
        let prev_xdg = std::env::var("XDG_DATA_HOME").ok();

        unsafe {
            std::env::remove_var("GJC_CODING_AGENT_DIR");
            std::env::remove_var("GJC_CONFIG_DIR");
            std::env::remove_var("PI_CONFIG_DIR");
        }

        let home_dir = TempDir::new().unwrap();
        let xdg_data = TempDir::new().unwrap();

        // Home fallback file.
        setup_mock_gjc_session(home_dir.path(), "home-slug", "home.jsonl");

        // XDG redirect file (different slug → distinct on-disk path, no dedup).
        let xdg_sessions = xdg_data.path().join("gjc/sessions/xdg-slug");
        fs::create_dir_all(&xdg_sessions).unwrap();
        File::create(xdg_sessions.join("xdg.jsonl")).unwrap();

        unsafe { std::env::set_var("XDG_DATA_HOME", xdg_data.path().to_string_lossy().as_ref()) };

        let result =
            scan_without_extra_dirs(home_dir.path().to_str().unwrap(), &["gjc".to_string()]);
        assert_eq!(
            result.get(ClientId::Gjc).len(),
            2,
            "both roots must contribute; files should NOT be collapsed to 1 (N4 push-all, not first-match). got {:?}",
            result.get(ClientId::Gjc)
        );

        restore_env("GJC_CODING_AGENT_DIR", prev_agent);
        restore_env("GJC_CONFIG_DIR", prev_config);
        restore_env("PI_CONFIG_DIR", prev_pi);
        restore_env("XDG_DATA_HOME", prev_xdg);
    }

    /// (e) use_env_roots=false ignores GJC_CONFIG_DIR and XDG_DATA_HOME even when
    ///     set, reading only the home fallback.
    #[test]
    #[serial]
    fn test_gjc_discovery_use_env_roots_false_ignores_config_and_xdg() {
        let prev_agent = std::env::var("GJC_CODING_AGENT_DIR").ok();
        let prev_config = std::env::var("GJC_CONFIG_DIR").ok();
        let prev_pi = std::env::var("PI_CONFIG_DIR").ok();
        let prev_xdg = std::env::var("XDG_DATA_HOME").ok();

        let home_dir = TempDir::new().unwrap();
        let config_dir = TempDir::new().unwrap();
        let xdg_data = TempDir::new().unwrap();

        // Seed a home-fallback file.
        setup_mock_gjc_session(home_dir.path(), "home-slug", "home.jsonl");

        // Seed a GJC_CONFIG_DIR file — must be ignored.
        let config_sessions = config_dir.path().join("agent/sessions/cfg-slug");
        fs::create_dir_all(&config_sessions).unwrap();
        File::create(config_sessions.join("cfg.jsonl")).unwrap();

        // Seed an XDG file — must be ignored.
        let xdg_sessions = xdg_data.path().join("gjc/sessions/xdg-slug");
        fs::create_dir_all(&xdg_sessions).unwrap();
        File::create(xdg_sessions.join("xdg.jsonl")).unwrap();

        unsafe {
            std::env::remove_var("GJC_CODING_AGENT_DIR");
            std::env::set_var(
                "GJC_CONFIG_DIR",
                config_dir.path().to_string_lossy().as_ref(),
            );
            std::env::set_var("XDG_DATA_HOME", xdg_data.path().to_string_lossy().as_ref());
        }

        let result = scan_all_clients_with_env_strategy(
            home_dir.path().to_str().unwrap(),
            &["gjc".to_string()],
            false, // use_env_roots = false
        );

        assert_eq!(
            result.get(ClientId::Gjc).len(),
            1,
            "use_env_roots=false must suppress GJC_CONFIG_DIR and XDG_DATA_HOME, yielding only the home fallback. got {:?}",
            result.get(ClientId::Gjc)
        );
        assert!(
            result
                .get(ClientId::Gjc)
                .iter()
                .any(|p| p.to_string_lossy().contains("home-slug")),
            "the sole discovered file must be from the home fallback"
        );

        restore_env("GJC_CODING_AGENT_DIR", prev_agent);
        restore_env("GJC_CONFIG_DIR", prev_config);
        restore_env("PI_CONFIG_DIR", prev_pi);
        restore_env("XDG_DATA_HOME", prev_xdg);
    }

    /// (f) Nonexistent GJC_CODING_AGENT_DIR does not panic and yields only the
    ///     home fallback file.
    #[test]
    #[serial]
    fn test_gjc_discovery_nonexistent_agent_dir_no_panic() {
        let prev_agent = std::env::var("GJC_CODING_AGENT_DIR").ok();
        let prev_config = std::env::var("GJC_CONFIG_DIR").ok();
        let prev_pi = std::env::var("PI_CONFIG_DIR").ok();
        let prev_xdg = std::env::var("XDG_DATA_HOME").ok();

        let home_dir = TempDir::new().unwrap();

        // Point GJC_CODING_AGENT_DIR at a path that does not exist.
        unsafe {
            std::env::set_var(
                "GJC_CODING_AGENT_DIR",
                "/nonexistent/path/that/does/not/exist",
            );
            std::env::remove_var("GJC_CONFIG_DIR");
            std::env::remove_var("PI_CONFIG_DIR");
            std::env::remove_var("XDG_DATA_HOME");
        }

        // Seed a home-fallback file so there is something to discover.
        setup_mock_gjc_session(home_dir.path(), "slug", "a.jsonl");

        // Must not panic.
        let result =
            scan_without_extra_dirs(home_dir.path().to_str().unwrap(), &["gjc".to_string()]);

        assert_eq!(
            result.get(ClientId::Gjc).len(),
            1,
            "nonexistent GJC_CODING_AGENT_DIR should be silently skipped, home fallback must still be found. got {:?}",
            result.get(ClientId::Gjc)
        );

        restore_env("GJC_CODING_AGENT_DIR", prev_agent);
        restore_env("GJC_CONFIG_DIR", prev_config);
        restore_env("PI_CONFIG_DIR", prev_pi);
        restore_env("XDG_DATA_HOME", prev_xdg);
    }

    fn setup_mock_senpi_omo_child(project_dir: &Path) -> PathBuf {
        let child_session = project_dir
            .join(".omo/senpi-task/children/task-123/sessions/task-123")
            .join("child.jsonl");
        fs::create_dir_all(child_session.parent().unwrap()).unwrap();
        File::create(&child_session).unwrap();
        child_session.canonicalize().unwrap()
    }

    #[cfg(windows)]
    #[test]
    #[serial]
    fn test_senpi_omo_task_children_root_uses_native_separators() {
        let project_dir = TempDir::new().unwrap();
        let mut env = EnvGuard::capture(&["SENPI_CODING_AGENT_SESSION_DIR"]);
        env.remove("SENPI_CODING_AGENT_SESSION_DIR");
        let _current_dir = CurrentDirGuard::set(project_dir.path());
        let enabled = HashSet::from([ClientId::Senpi]);

        let paths = built_in_extra_scan_paths_for("C:\\Users\\test", &enabled, true);
        let omo_root = paths
            .into_iter()
            .find_map(|(client_id, path)| (client_id == ClientId::Senpi).then_some(path))
            .expect("Senpi OmO child root must be registered");

        assert_eq!(
            omo_root,
            project_dir
                .path()
                .join(".omo")
                .join("senpi-task")
                .join("children")
        );
        assert!(
            !omo_root.to_string_lossy().contains('/'),
            "Windows scan root must not contain non-native separators: {}",
            omo_root.display()
        );
    }

    fn explicit_home_app_data_root(home: &Path) -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            home.join("AppData").join("Roaming")
        }
        #[cfg(target_os = "macos")]
        {
            home.join("Library").join("Application Support")
        }
        #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
        {
            home.join(".config")
        }
    }

    #[test]
    fn test_cherrystudio_dual_root_dedup_ignores_hostile_home_ancestor_names() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("data-and-agents-ancestor/home");
        let app_data = explicit_home_app_data_root(&home);
        let v1_root = app_data.join("CherryStudio/.claude/projects");
        let v2_root = app_data.join("CherryStudio/Data/Agents/.claude/projects");
        let relative_session = Path::new("workspace/session.jsonl");
        let v1_session = v1_root.join(relative_session);
        let v2_session = v2_root.join(relative_session);
        let transcript = "{\"type\":\"assistant\"}\n";

        fs::create_dir_all(v1_session.parent().unwrap()).unwrap();
        fs::create_dir_all(v2_session.parent().unwrap()).unwrap();
        fs::write(&v1_session, transcript).unwrap();
        fs::write(&v2_session, transcript).unwrap();

        let result = scan_all_clients_with_env_strategy(
            home.to_str().unwrap(),
            &["cherrystudio".to_string()],
            false,
        );

        assert_eq!(
            result.get(ClientId::CherryStudio),
            &vec![v2_session],
            "the V2 copy must win and the V1 copy must not be counted when the --home path has arbitrary data/agents substrings"
        );
    }

    #[test]
    fn test_cherrystudio_dual_root_dedup_prefers_v2() {
        let files = vec![
            (
                false,
                "ws/session-a.jsonl".to_string(),
                PathBuf::from("v1/ws/session-a.jsonl"),
            ),
            (
                true,
                "ws/session-a.jsonl".to_string(),
                PathBuf::from("v2/ws/session-a.jsonl"),
            ),
            (
                true,
                "ws/session-b.jsonl".to_string(),
                PathBuf::from("v2/ws/session-b.jsonl"),
            ),
            (
                false,
                "ws/session-c.jsonl".to_string(),
                PathBuf::from("v1/ws/session-c.jsonl"),
            ),
        ];
        let out = dedupe_cherrystudio_transcripts(files);
        assert_eq!(out.len(), 3, "same-name session collapses to the V2 copy");
        assert!(out.contains(&PathBuf::from("v2/ws/session-a.jsonl")));
        assert!(!out.contains(&PathBuf::from("v1/ws/session-a.jsonl")));
        assert!(out.contains(&PathBuf::from("v2/ws/session-b.jsonl")));
        assert!(out.contains(&PathBuf::from("v1/ws/session-c.jsonl")));
    }

    #[test]
    fn test_cherrystudio_dual_root_dedup_keeps_v2_only_files() {
        let files = vec![(
            true,
            "ws/session-x.jsonl".to_string(),
            PathBuf::from("v2/ws/session-x.jsonl"),
        )];
        let out = dedupe_cherrystudio_transcripts(files);
        assert_eq!(out, vec![PathBuf::from("v2/ws/session-x.jsonl")]);
    }

    #[test]
    fn test_cherrystudio_dual_root_dedup_empty_input() {
        let out = dedupe_cherrystudio_transcripts(Vec::new());
        assert!(out.is_empty());
    }

    #[test]
    #[serial]
    fn test_senpi_discovers_omo_task_children_in_current_project() {
        let home_dir = TempDir::new().unwrap();
        let project_dir = TempDir::new().unwrap();
        let child_session = setup_mock_senpi_omo_child(project_dir.path());
        let mut env = EnvGuard::capture(&["SENPI_CODING_AGENT_SESSION_DIR"]);
        env.remove("SENPI_CODING_AGENT_SESSION_DIR");
        let _current_dir = CurrentDirGuard::set(project_dir.path());

        let result =
            scan_without_extra_dirs(home_dir.path().to_str().unwrap(), &["senpi".to_string()]);

        let files = result.get(ClientId::Senpi);
        assert_eq!(
            files.len(),
            1,
            "current-project OmO child sessions must be auto-discovered"
        );
        assert_eq!(files[0].canonicalize().unwrap(), child_session);
    }

    #[test]
    #[serial]
    fn test_senpi_honors_independent_coding_agent_session_dir() {
        let home_dir = TempDir::new().unwrap();
        let project_dir = TempDir::new().unwrap();
        let child_session = setup_mock_senpi_omo_child(project_dir.path());
        let redirected_sessions = TempDir::new().unwrap();
        let redirected_session = redirected_sessions.path().join("redirected.jsonl");
        File::create(&redirected_session).unwrap();
        let mut env = EnvGuard::capture(&["SENPI_CODING_AGENT_SESSION_DIR"]);
        env.set("SENPI_CODING_AGENT_SESSION_DIR", redirected_sessions.path());
        let _current_dir = CurrentDirGuard::set(project_dir.path());

        let result =
            scan_without_extra_dirs(home_dir.path().to_str().unwrap(), &["senpi".to_string()]);

        let canonical_files: HashSet<PathBuf> = result
            .get(ClientId::Senpi)
            .iter()
            .map(|path| path.canonicalize().unwrap())
            .collect();
        assert_eq!(canonical_files.len(), 2);
        assert!(canonical_files.contains(&child_session));
        assert!(canonical_files.contains(&redirected_session.canonicalize().unwrap()));
    }

    #[test]
    #[serial]
    fn test_senpi_overlapping_session_roots_return_each_file_once() {
        let home_dir = TempDir::new().unwrap();
        let project_dir = TempDir::new().unwrap();
        let child_session = setup_mock_senpi_omo_child(project_dir.path());
        let children_dir = project_dir.path().join(".omo/senpi-task/children");
        let task_dir = children_dir.join("task-123");
        let exact_session_dir = task_dir.join("sessions/task-123");
        let mut env = EnvGuard::capture(&["SENPI_CODING_AGENT_SESSION_DIR"]);
        let _current_dir = CurrentDirGuard::set(project_dir.path());

        for env_root in [
            project_dir.path(),
            children_dir.as_path(),
            task_dir.as_path(),
            exact_session_dir.as_path(),
            child_session.as_path(),
        ] {
            env.set("SENPI_CODING_AGENT_SESSION_DIR", env_root);
            let result =
                scan_without_extra_dirs(home_dir.path().to_str().unwrap(), &["senpi".to_string()]);
            let files = result.get(ClientId::Senpi);
            assert_eq!(
                files.len(),
                1,
                "overlapping env root {} must not duplicate the child session",
                env_root.display()
            );
            assert_eq!(files[0].canonicalize().unwrap(), child_session);
        }
    }

    #[cfg(unix)]
    #[test]
    #[serial]
    fn test_senpi_symlinked_overlapping_session_root_returns_each_file_once() {
        use std::os::unix::fs::symlink;

        let home_dir = TempDir::new().unwrap();
        let project_dir = TempDir::new().unwrap();
        let child_session = setup_mock_senpi_omo_child(project_dir.path());
        let symlink_root = project_dir.path().join("redirected-sessions");
        symlink(
            project_dir.path().join(".omo/senpi-task/children"),
            &symlink_root,
        )
        .unwrap();
        let mut env = EnvGuard::capture(&["SENPI_CODING_AGENT_SESSION_DIR"]);
        env.set("SENPI_CODING_AGENT_SESSION_DIR", &symlink_root);
        let _current_dir = CurrentDirGuard::set(project_dir.path());

        let result =
            scan_without_extra_dirs(home_dir.path().to_str().unwrap(), &["senpi".to_string()]);

        let files = result.get(ClientId::Senpi);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].canonicalize().unwrap(), child_session);
    }
}
