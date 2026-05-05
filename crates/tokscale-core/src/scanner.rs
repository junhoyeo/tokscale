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
    pub synthetic_db: Option<PathBuf>,
    pub kilo_db: Option<PathBuf>,
    pub hermes_db: Option<PathBuf>,
    pub goose_db: Option<PathBuf>,
    pub crush_dbs: Vec<CrushDbSource>,
    /// Path to the OpenCode legacy JSON directory (for migration cache stat checks)
    pub opencode_json_dir: Option<PathBuf>,
}

impl Default for ScanResult {
    fn default() -> Self {
        Self {
            files: std::array::from_fn(|_| Vec::new()),
            opencode_dbs: Vec::new(),
            synthetic_db: None,
            kilo_db: None,
            hermes_db: None,
            goose_db: None,
            crush_dbs: Vec::new(),
            opencode_json_dir: None,
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
}

pub fn headless_roots_with_env_strategy(home_dir: &str, use_env_roots: bool) -> Vec<PathBuf> {
    if use_env_roots {
        if let Ok(path) = std::env::var("TOKSCALE_HEADLESS_DIR") {
            return vec![PathBuf::from(path)];
        }
    }

    let mut roots = Vec::new();
    roots.push(PathBuf::from(format!(
        "{}/.config/tokscale/headless",
        home_dir
    )));

    let mac_root = PathBuf::from(format!(
        "{}/Library/Application Support/tokscale/headless",
        home_dir
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
            if !path.is_file() {
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
                "session-*.json" => {
                    file_name.starts_with("session-") && file_name.ends_with(".json")
                }
                "T-*.json" => file_name.starts_with("T-") && file_name.ends_with(".json"),
                "*.settings.json" => file_name.ends_with(".settings.json"),
                "sessions.json" => file_name == "sessions.json",
                "wire.jsonl" => file_name == "wire.jsonl",
                "ui_messages.json" => file_name == "ui_messages.json",
                "session-usage.json" => file_name == "session-usage.json",
                "chat-messages.json" => file_name == "chat-messages.json",
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

pub fn built_in_extra_scan_paths_for(
    home_dir: &str,
    enabled: &HashSet<ClientId>,
) -> Vec<(ClientId, PathBuf)> {
    let mut paths = Vec::new();

    if enabled.contains(&ClientId::Claude) {
        paths.push((
            ClientId::Claude,
            PathBuf::from(format!("{}/.claude/transcripts", home_dir)),
        ));
    }

    paths
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

fn discover_crush_dbs(home_dir: &str, use_env_roots: bool) -> Vec<CrushDbSource> {
    let registry_path = PathBuf::from(
        ClientId::Crush
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots),
    );
    let mut dbs = scan_crush_registry(&registry_path);
    dbs.sort_by(|a, b| a.db_path.cmp(&b.db_path));
    dbs.dedup_by(|a, b| a.db_path == b.db_path);
    dbs
}

fn supports_extra_dir_scanning(client_id: ClientId) -> bool {
    // Kilo CLI currently loads a single SQLite DB via `scan_result.kilo_db`
    // Kilo CLI and Hermes use SQLite database paths, Roo/KiloCode require local + remote
    // and server task roots, and Crush discovers SQLite DBs via the project
    // registry rather than scanned file paths.
    !matches!(
        client_id,
        ClientId::Kilo | ClientId::Crush | ClientId::Hermes | ClientId::Goose
    )
}

fn push_unique_scan_task(
    tasks: &mut Vec<(ClientId, String, &'static str)>,
    seen: &mut HashSet<(ClientId, PathBuf)>,
    client_id: ClientId,
    raw_path: impl Into<PathBuf>,
) {
    let raw_path = raw_path.into();
    if raw_path.as_os_str().is_empty() {
        return;
    }

    let key = std::fs::canonicalize(&raw_path).unwrap_or_else(|_| raw_path.clone());
    if seen.insert((client_id, key)) {
        let pattern = client_id.data().pattern;
        tasks.push((client_id, raw_path.to_string_lossy().to_string(), pattern));
    }
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
            .filter_map(|s| ClientId::from_str(s))
            .collect()
    };

    let headless_roots = headless_roots_with_env_strategy(home_dir, use_env_roots);

    // Define scan tasks
    let mut tasks: Vec<(ClientId, String, &str)> = Vec::new();
    let mut seen_scan_roots: HashSet<(ClientId, PathBuf)> = HashSet::new();

    for client_id in &enabled {
        if matches!(
            client_id,
            ClientId::OpenCode
                | ClientId::Codex
                | ClientId::OpenClaw
                | ClientId::RooCode
                | ClientId::KiloCode
                | ClientId::Kilo
                | ClientId::Hermes
                | ClientId::Goose
                | ClientId::Crush
                | ClientId::Codebuff
        ) {
            continue;
        }

        let def = client_id.data();
        let path = def.resolve_path_with_env_strategy(home_dir, use_env_roots);
        push_unique_scan_task(&mut tasks, &mut seen_scan_roots, *client_id, path);
    }

    for (client_id, path) in extra_scan_paths_for(scanner_settings, &enabled) {
        push_unique_scan_task(&mut tasks, &mut seen_scan_roots, client_id, path);
    }

    for (client_id, path) in built_in_extra_scan_paths_for(home_dir, &enabled) {
        push_unique_scan_task(&mut tasks, &mut seen_scan_roots, client_id, path);
    }

    // Extra scan directories are part of the caller's environment, so they are
    // intentionally ignored when an explicit --home override disables env roots.
    if use_env_roots {
        let extra_dirs_val = std::env::var("TOKSCALE_EXTRA_DIRS").unwrap_or_default();
        for (client_id, path) in parse_extra_dirs(&extra_dirs_val, &enabled) {
            push_unique_scan_task(&mut tasks, &mut seen_scan_roots, client_id, path);
        }
    }

    if enabled.contains(&ClientId::OpenCode) {
        let xdg_data = if use_env_roots {
            std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{}/.local/share", home_dir))
        } else {
            format!("{}/.local/share", home_dir)
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
        let opencode_data_dir = PathBuf::from(format!("{}/opencode", xdg_data));
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

    if enabled.contains(&ClientId::Codex) {
        // Codex: ~/.codex/sessions/**/*.jsonl
        let codex_home = if use_env_roots {
            std::env::var("CODEX_HOME").unwrap_or_else(|_| format!("{}/.codex", home_dir))
        } else {
            format!("{}/.codex", home_dir)
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
        let codex_archived_path = format!("{}/archived_sessions", codex_home);
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
        let clawdbot_path = format!("{}/.clawdbot/agents", home_dir);
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::OpenClaw,
            clawdbot_path,
        );

        let moltbot_path = format!("{}/.moltbot/agents", home_dir);
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::OpenClaw,
            moltbot_path,
        );

        let moldbot_path = format!("{}/.moldbot/agents", home_dir);
        push_unique_scan_task(
            &mut tasks,
            &mut seen_scan_roots,
            ClientId::OpenClaw,
            moldbot_path,
        );
    }

    // Oh My Pi fork (https://github.com/can1357/oh-my-pi) — same JSONL format, different root
    if enabled.contains(&ClientId::Pi) {
        let omp_path = format!("{}/.omp/agent/sessions", home_dir);
        push_unique_scan_task(&mut tasks, &mut seen_scan_roots, ClientId::Pi, omp_path);
    }

    if include_synthetic {
        let xdg_data = if use_env_roots {
            std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{}/.local/share", home_dir))
        } else {
            format!("{}/.local/share", home_dir)
        };
        let octofriend_db_path = PathBuf::from(format!("{}/octofriend/sqlite.db", xdg_data));
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

    if enabled.contains(&ClientId::Kilo) {
        let kilo_db_path = ClientId::Kilo
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        if std::path::Path::new(&kilo_db_path).exists() {
            result.kilo_db = Some(PathBuf::from(kilo_db_path));
        }
    }

    if enabled.contains(&ClientId::Hermes) {
        let hermes_db_path = ClientId::Hermes
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        if std::path::Path::new(&hermes_db_path).exists() {
            result.hermes_db = Some(PathBuf::from(hermes_db_path));
        }
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

    if enabled.contains(&ClientId::Crush) {
        result.crush_dbs = discover_crush_dbs(home_dir, use_env_roots);
    }

    if enabled.contains(&ClientId::Codebuff) {
        // Codebuff persists per-channel chat history under
        // ~/.config/<channel>/projects/<project>/chats/<chatId>/chat-messages.json.
        // When CODEBUFF_DATA_DIR is set to a non-empty value (via
        // PathRoot::EnvVar), scan only that root; otherwise — including when
        // the env var is unset *or* set to an empty/whitespace string — walk
        // the three known channel roots:
        //   - ~/.config/manicode (primary / legacy name — Codebuff was "Manicode")
        //   - ~/.config/manicode-dev
        //   - ~/.config/manicode-staging
        let trimmed_override = if use_env_roots {
            std::env::var("CODEBUFF_DATA_DIR")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        } else {
            None
        };

        let mut codebuff_roots: Vec<String> = Vec::new();
        if let Some(root) = trimmed_override {
            codebuff_roots.push(format!("{}/projects", root.trim_end_matches('/')));
        } else {
            let config_dir = format!("{}/.config", home_dir);
            for channel in ["manicode", "manicode-dev", "manicode-staging"] {
                codebuff_roots.push(format!("{}/{}/projects", config_dir, channel));
            }
        }

        for root in codebuff_roots {
            push_unique_scan_task(&mut tasks, &mut seen_scan_roots, ClientId::Codebuff, root);
        }
    }

    // Execute scans in parallel
    let scan_results: Vec<(ClientId, Vec<PathBuf>)> = tasks
        .into_par_iter()
        .map(|(client_id, path, pattern)| {
            let files = scan_directory(&path, pattern);
            (client_id, files)
        })
        .collect();

    // Aggregate results, deduplicating file paths across overlapping directories
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for (client_id, files) in scan_results {
        for file in files {
            if seen.insert(file.clone()) {
                result.get_mut(client_id).push(file);
            }
        }
    }

    if enabled.contains(&ClientId::Copilot) {
        if let Some(path) = copilot_exporter_path_with_env_strategy(use_env_roots) {
            if path.is_file() && seen.insert(path.clone()) {
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
    use serial_test::serial;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::TempDir;

    fn restore_env(var: &str, previous: Option<String>) {
        match previous {
            Some(value) => unsafe { std::env::set_var(var, value) },
            None => unsafe { std::env::remove_var(var) },
        }
    }

    fn restore_current_dir(previous: &Path) {
        std::env::set_current_dir(previous).unwrap();
    }

    fn setup_mock_copilot_dir(home: &Path) {
        let sessions_dir = home.join(".copilot/otel");
        fs::create_dir_all(&sessions_dir).unwrap();
        let file_path = sessions_dir.join("copilot.jsonl");
        let mut file = File::create(file_path).unwrap();
        writeln!(file, "{{\"type\":\"span\",\"name\":\"chat gpt-5.4-mini\"}}").unwrap();
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
    fn test_scan_directory_nonexistent() {
        let files = scan_directory("/nonexistent/path/that/does/not/exist", "*.json");
        assert!(files.is_empty());
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

    fn setup_mock_omp_dir(base: &std::path::Path) {
        let omp_path = base.join(".omp/agent/sessions/--omp-test--");
        fs::create_dir_all(&omp_path).unwrap();
        let mut file =
            File::create(omp_path.join("2026-04-06T03-04-28Z_omp_ses_001.jsonl")).unwrap();
        file.write_all(b"{}").unwrap();
    }

    fn setup_mock_kimi_dir(base: &std::path::Path) {
        let kimi_session = base.join(".kimi/sessions/group1/session-uuid-1");
        fs::create_dir_all(&kimi_session).unwrap();
        let mut file = File::create(kimi_session.join("wire.jsonl")).unwrap();
        file.write_all(b"{\"type\": \"metadata\", \"protocol_version\": \"1.3\"}\n")
            .unwrap();
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
        let config_root = PathBuf::from(format!("{}/.config/tokscale/headless", home));
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

        let result = scan_all_clients(home.to_str().unwrap(), &["opencode".to_string()]);
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
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        // Auto-discoverable channel db inside XDG data dir.
        let data_dir = home.join(".local/share/opencode");
        fs::create_dir_all(&data_dir).unwrap();
        File::create(data_dir.join("opencode-stable.db")).unwrap();

        // User-configured db living outside XDG_DATA_HOME, the way an
        // `OPENCODE_DB=/abs/path/opencode.db` user would have it.
        let outside_dir = home.join("elsewhere");
        fs::create_dir_all(&outside_dir).unwrap();
        let outside_db = outside_dir.join("opencode.db");
        File::create(&outside_db).unwrap();

        unsafe { std::env::set_var("XDG_DATA_HOME", home.join(".local/share")) };

        let settings = ScannerSettings {
            opencode_db_paths: vec![outside_db.clone()],
            ..Default::default()
        };
        let result = scan_all_clients_with_scanner_settings(
            home.to_str().unwrap(),
            &["opencode".to_string()],
            true,
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

        restore_env("XDG_DATA_HOME", previous_xdg);
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
            true,
            &settings,
        );

        assert_eq!(result.get(ClientId::Codex).len(), 2);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_with_scanner_settings_dedups_settings_and_env_extra_paths() {
        let previous = std::env::var("TOKSCALE_EXTRA_DIRS").ok();
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let default_root = home.join(".codex/sessions");
        fs::create_dir_all(&default_root).unwrap();
        File::create(default_root.join("default.jsonl")).unwrap();

        let extra_root = home.join("workspace/project-a/.codex/sessions");
        fs::create_dir_all(&extra_root).unwrap();
        File::create(extra_root.join("extra.jsonl")).unwrap();

        unsafe {
            std::env::set_var(
                "TOKSCALE_EXTRA_DIRS",
                format!("codex:{}", extra_root.join("..").join("sessions").display()),
            )
        };

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
        restore_env("TOKSCALE_EXTRA_DIRS", previous);
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
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();

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

        unsafe { std::env::set_var("XDG_DATA_HOME", home.join(".local/share")) };

        let settings = ScannerSettings {
            opencode_db_paths: vec![outside_db.clone()],
            ..Default::default()
        };

        let scan = |clients: &[&str]| {
            let owned: Vec<String> = clients.iter().map(|s| s.to_string()).collect();
            scan_all_clients_with_scanner_settings(home.to_str().unwrap(), &owned, true, &settings)
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

        restore_env("XDG_DATA_HOME", previous_xdg);
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

        let result = scan_all_clients(home.to_str().unwrap(), &["opencode".to_string()]);

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

        let result = scan_all_clients(home.to_str().unwrap(), &["pi".to_string()]);
        assert_eq!(result.get(ClientId::Pi).len(), 1);
        assert!(result.get(ClientId::OpenCode).is_empty());
        assert!(result.get(ClientId::Claude).is_empty());
    }

    #[test]
    fn test_scan_all_clients_omp_scanned_as_pi() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_omp_dir(home);

        let result = scan_all_clients(home.to_str().unwrap(), &["pi".to_string()]);
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

        let result = scan_all_clients(home.to_str().unwrap(), &["pi".to_string()]);
        assert_eq!(result.get(ClientId::Pi).len(), 2);
    }

    #[test]
    fn test_scan_all_clients_claude() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_claude_dir(home);

        let result = scan_all_clients(home.to_str().unwrap(), &["claude".to_string()]);
        assert_eq!(result.get(ClientId::Claude).len(), 1);
        assert!(result.get(ClientId::OpenCode).is_empty());
    }

    #[test]
    fn test_scan_all_clients_claude_transcripts() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_claude_dir(home);
        let transcript = setup_mock_claude_transcripts_dir(home);

        let result = scan_all_clients(home.to_str().unwrap(), &["claude".to_string()]);

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

        let result = scan_all_clients(home.to_str().unwrap(), &["claude".to_string()]);

        assert_eq!(result.get(ClientId::Claude), &vec![transcript]);
        assert!(result.get(ClientId::OpenCode).is_empty());
    }

    #[test]
    fn test_scan_all_clients_gemini() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_gemini_dir(home);

        let result = scan_all_clients(home.to_str().unwrap(), &["gemini".to_string()]);
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

        let result = scan_all_clients(home.to_str().unwrap(), &["gemini".to_string()]);
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

        let result = scan_all_clients(home.to_str().unwrap(), &["copilot".to_string()]);

        assert_eq!(result.get(ClientId::Copilot), &vec![explicit_file]);

        restore_env("COPILOT_OTEL_FILE_EXPORTER_PATH", previous);
    }

    #[test]
    fn test_scan_all_clients_openclaw_jsonl_only() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_openclaw_dir(home);

        let result = scan_all_clients(home.to_str().unwrap(), &["openclaw".to_string()]);
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

        let result = scan_all_clients(home.to_str().unwrap(), &["openclaw".to_string()]);
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
    {{ "path": "{}", "data_dir": ".crush" }},
    {{ "path": "{}", "data_dir": "{}" }},
    {{ "path": "{}", "data_dir": ".crush" }}
  ]
}}"#,
            project_a.display(),
            dir.path().join("project-b").display(),
            project_b_data.display(),
            dir.path().join("missing-project").display(),
        );
        setup_mock_crush_registry(&registry_path, &projects_json);

        let result = scan_crush_registry(&registry_path);
        assert_eq!(
            result,
            vec![
                CrushDbSource {
                    db_path: project_a.join(".crush").join("crush.db"),
                    workspace_key: Some(project_a.display().to_string()),
                    workspace_label: Some("project-a".to_string()),
                },
                CrushDbSource {
                    db_path: project_b_data.join("crush.db"),
                    workspace_key: Some(dir.path().join("project-b").display().to_string()),
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
    {{ "path": "{}", "data_dir": ".crush" }},
    {{ "path": 123, "data_dir": ".crush" }},
    {{ "data_dir": ".crush" }},
    "not-an-object"
  ]
}}"#,
            valid_project.display()
        );
        setup_mock_crush_registry(&registry_path, &projects_json);

        let result = scan_crush_registry(&registry_path);
        assert_eq!(
            result,
            vec![CrushDbSource {
                db_path: valid_project.join(".crush").join("crush.db"),
                workspace_key: Some(valid_project.display().to_string()),
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
    fn test_scan_all_clients_crush_populates_crush_db_paths() {
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();

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
    {{ "path": "{}", "data_dir": ".crush" }}
  ]
}}"#,
            project.display()
        );
        setup_mock_crush_registry(&registry_path, &projects_json);

        unsafe { std::env::set_var("XDG_DATA_HOME", &xdg) };

        let result = scan_all_clients(home.to_str().unwrap(), &["crush".to_string()]);
        assert_eq!(
            result.crush_dbs,
            vec![CrushDbSource {
                db_path: data_dir.join("crush.db"),
                workspace_key: Some(project.display().to_string()),
                workspace_label: Some("project".to_string()),
            }]
        );
        assert!(result.get(ClientId::Crush).is_empty());

        restore_env("XDG_DATA_HOME", previous_xdg);
    }

    #[test]
    #[serial]
    fn test_scan_all_clients_headless_paths() {
        let previous_headless = std::env::var("TOKSCALE_HEADLESS_DIR").ok();
        unsafe { std::env::remove_var("TOKSCALE_HEADLESS_DIR") };

        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let mac_root = home
            .join("Library")
            .join("Application Support")
            .join("tokscale")
            .join("headless");

        fs::create_dir_all(mac_root.join("codex")).unwrap();
        File::create(mac_root.join("codex").join("codex.jsonl")).unwrap();

        let result = scan_all_clients(
            home.to_str().unwrap(),
            &[
                "claude".to_string(),
                "codex".to_string(),
                "gemini".to_string(),
            ],
        );

        assert!(result.get(ClientId::Claude).is_empty());
        assert_eq!(result.get(ClientId::Codex).len(), 1);
        assert!(result.get(ClientId::Gemini).is_empty());

        restore_env("TOKSCALE_HEADLESS_DIR", previous_headless);
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

        let result = scan_all_clients(home.to_str().unwrap(), &["codex".to_string()]);
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

        let result = scan_all_clients(home.to_str().unwrap(), &["codex".to_string()]);
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

        let result = scan_all_clients(home.to_str().unwrap(), &["codex".to_string()]);
        assert_eq!(result.get(ClientId::Codex).len(), 2);

        restore_env("CODEX_HOME", previous_codex);
    }

    #[test]
    fn test_scan_all_clients_kimi() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_kimi_dir(home);

        let result = scan_all_clients(home.to_str().unwrap(), &["kimi".to_string()]);
        assert_eq!(result.get(ClientId::Kimi).len(), 1);
        assert!(result.get(ClientId::Kimi)[0].ends_with("wire.jsonl"));
        assert!(result.get(ClientId::OpenCode).is_empty());
        assert!(result.get(ClientId::Claude).is_empty());
    }

    #[test]
    fn test_scan_all_clients_roocode() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_roocode_dir(home);

        let result = scan_all_clients(home.to_str().unwrap(), &["roocode".to_string()]);
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

        let result = scan_all_clients(home.to_str().unwrap(), &["kilocode".to_string()]);
        assert_eq!(result.get(ClientId::KiloCode).len(), 2);
        assert!(result
            .get(ClientId::KiloCode)
            .iter()
            .all(|p| p.ends_with("ui_messages.json")));
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

        let result = scan_all_clients(home.to_str().unwrap(), &["codebuff".to_string()]);
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

        let result = scan_all_clients(home.to_str().unwrap(), &["codebuff".to_string()]);
        assert_eq!(result.get(ClientId::Codebuff).len(), 2);

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

        let result = scan_all_clients(home.to_str().unwrap(), &["codebuff".to_string()]);
        assert_eq!(result.get(ClientId::Codebuff).len(), 1);
        assert!(result.get(ClientId::Codebuff)[0]
            .to_string_lossy()
            .contains("custom-codebuff"));

        restore_env("CODEBUFF_DATA_DIR", previous);
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
}
