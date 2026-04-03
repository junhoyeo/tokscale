//! Parallel file scanner for session directories
//!
//! Uses walkdir with rayon for parallel directory traversal.

use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::clients::ClientId;
use crate::sessions::{normalize_workspace_key, workspace_label_from_key};
use serde::Deserialize;
use serde_json::Value;

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
    pub opencode_db: Option<PathBuf>,
    pub synthetic_db: Option<PathBuf>,
    pub kilo_db: Option<PathBuf>,
    pub hermes_db: Option<PathBuf>,
    pub crush_dbs: Vec<CrushDbSource>,
    /// Path to the OpenCode legacy JSON directory (for migration cache stat checks)
    pub opencode_json_dir: Option<PathBuf>,
}

impl Default for ScanResult {
    fn default() -> Self {
        Self {
            files: std::array::from_fn(|_| Vec::new()),
            opencode_db: None,
            synthetic_db: None,
            kilo_db: None,
            hermes_db: None,
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
        ClientId::Kilo | ClientId::Crush | ClientId::Hermes
    )
}

/// Scan all session client directories in parallel
pub fn scan_all_clients_with_env_strategy(
    home_dir: &str,
    clients: &[String],
    use_env_roots: bool,
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
                | ClientId::Crush
        ) {
            continue;
        }

        let def = client_id.data();
        let path = def.resolve_path_with_env_strategy(home_dir, use_env_roots);
        tasks.push((*client_id, path, def.pattern));
    }

    // Extra scan directories are part of the caller's environment, so they are
    // intentionally ignored when an explicit --home override disables env roots.
    if use_env_roots {
        let extra_dirs_val = std::env::var("TOKSCALE_EXTRA_DIRS").unwrap_or_default();
        for (client_id, path) in parse_extra_dirs(&extra_dirs_val, &enabled) {
            let pattern = client_id.data().pattern;
            tasks.push((client_id, path, pattern));
        }
    }

    if enabled.contains(&ClientId::OpenCode) {
        let xdg_data = if use_env_roots {
            std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{}/.local/share", home_dir))
        } else {
            format!("{}/.local/share", home_dir)
        };

        // OpenCode 1.2+: SQLite database at ~/.local/share/opencode/opencode.db
        let opencode_db_path = PathBuf::from(format!("{}/opencode/opencode.db", xdg_data));
        if opencode_db_path.exists() {
            result.opencode_db = Some(opencode_db_path);
        }

        // OpenCode legacy: JSON files at ~/.local/share/opencode/storage/message/*/*.json
        let opencode_path = ClientId::OpenCode
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        result.opencode_json_dir = Some(PathBuf::from(&opencode_path));
        tasks.push((
            ClientId::OpenCode,
            opencode_path,
            ClientId::OpenCode.data().pattern,
        ));
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
        tasks.push((ClientId::Codex, codex_path, ClientId::Codex.data().pattern));

        // Codex archived sessions: ~/.codex/archived_sessions/**/*.jsonl
        let codex_archived_path = format!("{}/archived_sessions", codex_home);
        tasks.push((
            ClientId::Codex,
            codex_archived_path,
            ClientId::Codex.data().pattern,
        ));

        // Codex headless: <headless_root>/codex/*.jsonl
        for root in &headless_roots {
            let codex_headless_path = root.join("codex");
            let path = codex_headless_path.to_string_lossy().to_string();
            tasks.push((ClientId::Codex, path, ClientId::Codex.data().pattern));
        }
    }

    if enabled.contains(&ClientId::OpenClaw) {
        // OpenClaw transcripts: ~/.openclaw/agents/**/*.jsonl
        let openclaw_path = ClientId::OpenClaw
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        tasks.push((
            ClientId::OpenClaw,
            openclaw_path,
            ClientId::OpenClaw.data().pattern,
        ));

        // Legacy paths (Clawd -> Moltbot -> OpenClaw rebrand history)
        let clawdbot_path = format!("{}/.clawdbot/agents", home_dir);
        tasks.push((
            ClientId::OpenClaw,
            clawdbot_path,
            ClientId::OpenClaw.data().pattern,
        ));

        let moltbot_path = format!("{}/.moltbot/agents", home_dir);
        tasks.push((
            ClientId::OpenClaw,
            moltbot_path,
            ClientId::OpenClaw.data().pattern,
        ));

        let moldbot_path = format!("{}/.moldbot/agents", home_dir);
        tasks.push((
            ClientId::OpenClaw,
            moldbot_path,
            ClientId::OpenClaw.data().pattern,
        ));
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
        tasks.push((
            ClientId::RooCode,
            local_path,
            ClientId::RooCode.data().pattern,
        ));

        let server_path = format!(
            "{}/.vscode-server/data/User/globalStorage/rooveterinaryinc.roo-cline/tasks",
            home_dir
        );
        tasks.push((
            ClientId::RooCode,
            server_path,
            ClientId::RooCode.data().pattern,
        ));
    }

    if enabled.contains(&ClientId::KiloCode) {
        let local_path = ClientId::KiloCode
            .data()
            .resolve_path_with_env_strategy(home_dir, use_env_roots);
        tasks.push((
            ClientId::KiloCode,
            local_path,
            ClientId::KiloCode.data().pattern,
        ));

        let server_path = format!(
            "{}/.vscode-server/data/User/globalStorage/kilocode.kilo-code/tasks",
            home_dir
        );
        tasks.push((
            ClientId::KiloCode,
            server_path,
            ClientId::KiloCode.data().pattern,
        ));
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

    if enabled.contains(&ClientId::Crush) {
        result.crush_dbs = discover_crush_dbs(home_dir, use_env_roots);
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
    fn test_scan_all_clients_claude() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_claude_dir(home);

        let result = scan_all_clients(home.to_str().unwrap(), &["claude".to_string()]);
        assert_eq!(result.get(ClientId::Claude).len(), 1);
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

        let result = scan_all_clients(
            home.to_str().unwrap(),
            &["claude".to_string(), "gemini".to_string()],
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
