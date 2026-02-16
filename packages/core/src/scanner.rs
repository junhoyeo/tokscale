//! Parallel file scanner for session directories
//!
//! Uses walkdir with rayon for parallel directory traversal.

use rayon::prelude::*;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Session source type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionType {
    OpenCode,
    Claude,
    Codex,
    Gemini,
    Cursor,
    Amp,
    Droid,
    OpenClaw,
    Pi,
}

/// Result of scanning all session directories
#[derive(Debug, Default)]
pub struct ScanResult {
    pub opencode_files: Vec<PathBuf>,
    pub opencode_db: Option<PathBuf>,
    pub claude_files: Vec<PathBuf>,
    pub codex_files: Vec<PathBuf>,
    pub gemini_files: Vec<PathBuf>,
    pub cursor_files: Vec<PathBuf>,
    pub amp_files: Vec<PathBuf>,
    pub droid_files: Vec<PathBuf>,
    pub openclaw_files: Vec<PathBuf>,
    pub pi_files: Vec<PathBuf>,
}

impl ScanResult {
    /// Get total number of files found
    pub fn total_files(&self) -> usize {
        self.opencode_files.len()
            + self.claude_files.len()
            + self.codex_files.len()
            + self.gemini_files.len()
            + self.cursor_files.len()
            + self.amp_files.len()
            + self.droid_files.len()
            + self.openclaw_files.len()
            + self.pi_files.len()
    }

    /// Get all files as a single vector
    pub fn all_files(&self) -> Vec<(SessionType, PathBuf)> {
        let mut result = Vec::with_capacity(self.total_files());

        for path in &self.opencode_files {
            result.push((SessionType::OpenCode, path.clone()));
        }
        for path in &self.claude_files {
            result.push((SessionType::Claude, path.clone()));
        }
        for path in &self.codex_files {
            result.push((SessionType::Codex, path.clone()));
        }
        for path in &self.gemini_files {
            result.push((SessionType::Gemini, path.clone()));
        }
        for path in &self.cursor_files {
            result.push((SessionType::Cursor, path.clone()));
        }
        for path in &self.amp_files {
            result.push((SessionType::Amp, path.clone()));
        }
        for path in &self.droid_files {
            result.push((SessionType::Droid, path.clone()));
        }
        for path in &self.openclaw_files {
            result.push((SessionType::OpenClaw, path.clone()));
        }
        for path in &self.pi_files {
            result.push((SessionType::Pi, path.clone()));
        }

        result
    }
}

pub fn headless_roots(home_dir: &str) -> Vec<PathBuf> {
    if let Ok(path) = std::env::var("TOKSCALE_HEADLESS_DIR") {
        return vec![PathBuf::from(path)];
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

/// Scan a single directory for session files
pub fn scan_directory(root: &str, pattern: &str) -> Vec<PathBuf> {
    if !std::path::Path::new(root).exists() {
        return Vec::new();
    }

    WalkDir::new(root)
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
                _ => false,
            }
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

/// Scan all session source directories in parallel
pub fn scan_all_sources(home_dir: &str, sources: &[String]) -> ScanResult {
    let mut result = ScanResult::default();

    let include_all = sources.is_empty();
    let include_opencode = include_all || sources.iter().any(|s| s == "opencode");
    let include_claude = include_all || sources.iter().any(|s| s == "claude");
    let include_codex = include_all || sources.iter().any(|s| s == "codex");
    let include_gemini = include_all || sources.iter().any(|s| s == "gemini");
    let include_cursor = include_all || sources.iter().any(|s| s == "cursor");
    let include_amp = include_all || sources.iter().any(|s| s == "amp");
    let include_droid = include_all || sources.iter().any(|s| s == "droid");
    let include_openclaw = include_all || sources.iter().any(|s| s == "openclaw");
    let include_pi = include_all || sources.iter().any(|s| s == "pi");

    let headless_roots = headless_roots(home_dir);

    // Define scan tasks
    let mut tasks: Vec<(SessionType, String, &str)> = Vec::new();

    if include_opencode {
        let xdg_data =
            std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{}/.local/share", home_dir));

        // OpenCode 1.2+: SQLite database at ~/.local/share/opencode/opencode.db
        let opencode_db_path = PathBuf::from(format!("{}/opencode/opencode.db", xdg_data));
        if opencode_db_path.exists() {
            result.opencode_db = Some(opencode_db_path);
        }

        // OpenCode legacy: JSON files at ~/.local/share/opencode/storage/message/*/*.json
        let opencode_path = format!("{}/opencode/storage/message", xdg_data);
        tasks.push((SessionType::OpenCode, opencode_path, "*.json"));
    }

    if include_claude {
        // Claude: ~/.claude/projects/**/*.jsonl
        let claude_path = format!("{}/.claude/projects", home_dir);
        tasks.push((SessionType::Claude, claude_path, "*.jsonl"));
    }

    if include_codex {
        // Codex: ~/.codex/sessions/**/*.jsonl
        let codex_home =
            std::env::var("CODEX_HOME").unwrap_or_else(|_| format!("{}/.codex", home_dir));
        let codex_path = format!("{}/sessions", codex_home);
        tasks.push((SessionType::Codex, codex_path, "*.jsonl"));

        // Codex headless: <headless_root>/codex/*.jsonl
        for root in &headless_roots {
            let codex_headless_path = root.join("codex");
            let path = codex_headless_path.to_string_lossy().to_string();
            tasks.push((SessionType::Codex, path, "*.jsonl"));
        }
    }

    if include_gemini {
        // Gemini: ~/.gemini/tmp/*/chats/session-*.json
        let gemini_path = format!("{}/.gemini/tmp", home_dir);
        tasks.push((SessionType::Gemini, gemini_path, "session-*.json"));
    }

    if include_cursor {
        // Cursor: ~/.config/tokscale/cursor-cache/*.csv (migrated from ~/.tokscale)
        let cursor_path = format!("{}/.config/tokscale/cursor-cache", home_dir);
        // Only scan Cursor usage CSVs to avoid counting unrelated CSVs.
        tasks.push((SessionType::Cursor, cursor_path, "usage*.csv"));
    }

    if include_amp {
        // Amp: ~/.local/share/amp/threads/T-*.json
        let xdg_data =
            std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{}/.local/share", home_dir));
        let amp_path = format!("{}/amp/threads", xdg_data);
        tasks.push((SessionType::Amp, amp_path, "T-*.json"));
    }

    if include_droid {
        // Droid: ~/.factory/sessions/*.settings.json
        let droid_path = format!("{}/.factory/sessions", home_dir);
        tasks.push((SessionType::Droid, droid_path, "*.settings.json"));
    }

    if include_openclaw {
        // OpenClaw transcripts: ~/.openclaw/agents/**/*.jsonl
        let openclaw_path = format!("{}/.openclaw/agents", home_dir);
        tasks.push((SessionType::OpenClaw, openclaw_path, "*.jsonl"));

        // Legacy paths (Clawd -> Moltbot -> OpenClaw rebrand history)
        let clawdbot_path = format!("{}/.clawdbot/agents", home_dir);
        tasks.push((SessionType::OpenClaw, clawdbot_path, "*.jsonl"));

        let moltbot_path = format!("{}/.moltbot/agents", home_dir);
        tasks.push((SessionType::OpenClaw, moltbot_path, "*.jsonl"));

        let moldbot_path = format!("{}/.moldbot/agents", home_dir);
        tasks.push((SessionType::OpenClaw, moldbot_path, "*.jsonl"));
    }

    if include_pi {
        // Pi (badlogic/pi-mono): ~/.pi/agent/sessions/**/*.jsonl
        let pi_path = format!("{}/.pi/agent/sessions", home_dir);
        tasks.push((SessionType::Pi, pi_path, "*.jsonl"));
    }

    // Execute scans in parallel
    let scan_results: Vec<(SessionType, Vec<PathBuf>)> = tasks
        .into_par_iter()
        .map(|(session_type, path, pattern)| {
            let files = scan_directory(&path, pattern);
            (session_type, files)
        })
        .collect();

    // Aggregate results
    for (session_type, files) in scan_results {
        match session_type {
            SessionType::OpenCode => result.opencode_files.extend(files),
            SessionType::Claude => result.claude_files.extend(files),
            SessionType::Codex => result.codex_files.extend(files),
            SessionType::Gemini => result.gemini_files.extend(files),
            SessionType::Cursor => result.cursor_files.extend(files),
            SessionType::Amp => result.amp_files.extend(files),
            SessionType::Droid => result.droid_files.extend(files),
            SessionType::OpenClaw => result.openclaw_files.extend(files),
            SessionType::Pi => result.pi_files.extend(files),
        }
    }

    result
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
            Some(value) => std::env::set_var(var, value),
            None => std::env::remove_var(var),
        }
    }

    #[test]
    fn test_scan_result_total_files() {
        let result = ScanResult {
            opencode_files: vec![PathBuf::from("a.json"), PathBuf::from("b.json")],
            opencode_db: None,
            claude_files: vec![PathBuf::from("c.jsonl")],
            codex_files: vec![],
            gemini_files: vec![PathBuf::from("d.json")],
            cursor_files: vec![],
            amp_files: vec![],
            droid_files: vec![],
            openclaw_files: vec![],
            pi_files: vec![PathBuf::from("e.jsonl")],
        };
        assert_eq!(result.total_files(), 5);
    }

    #[test]
    fn test_scan_result_all_files() {
        let result = ScanResult {
            opencode_files: vec![PathBuf::from("a.json")],
            opencode_db: None,
            claude_files: vec![PathBuf::from("b.jsonl")],
            codex_files: vec![PathBuf::from("c.jsonl")],
            gemini_files: vec![PathBuf::from("d.json")],
            cursor_files: vec![PathBuf::from("e.csv")],
            amp_files: vec![],
            droid_files: vec![],
            openclaw_files: vec![],
            pi_files: vec![PathBuf::from("f.jsonl")],
        };

        let all = result.all_files();
        assert_eq!(all.len(), 6);
        assert_eq!(all[0], (SessionType::OpenCode, PathBuf::from("a.json")));
        assert_eq!(all[1], (SessionType::Claude, PathBuf::from("b.jsonl")));
        assert_eq!(all[2], (SessionType::Codex, PathBuf::from("c.jsonl")));
        assert_eq!(all[3], (SessionType::Gemini, PathBuf::from("d.json")));
        assert_eq!(all[4], (SessionType::Cursor, PathBuf::from("e.csv")));
        assert_eq!(all[5], (SessionType::Pi, PathBuf::from("f.jsonl")));
    }

    #[test]
    fn test_scan_result_empty() {
        let result = ScanResult::default();
        assert_eq!(result.total_files(), 0);
        assert!(result.all_files().is_empty());
    }

    #[test]
    fn test_session_type_equality() {
        assert_eq!(SessionType::OpenCode, SessionType::OpenCode);
        assert_ne!(SessionType::OpenCode, SessionType::Claude);
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

    fn setup_mock_openclaw_dir(base: &std::path::Path) {
        // Mirror real OpenClaw layout: ~/.openclaw/agents/<agentId>/sessions/*.jsonl
        let openclaw_sessions = base.join(".openclaw/agents/main/sessions");
        fs::create_dir_all(&openclaw_sessions).unwrap();

        let mut transcript = File::create(openclaw_sessions.join("session-abc.jsonl")).unwrap();
        transcript.write_all(b"{}").unwrap();

        // Even if an index exists, we should count JSONL transcripts (not sessions.json only)
        let mut index = File::create(openclaw_sessions.join("sessions.json")).unwrap();
        index.write_all(b"{}").unwrap();
    }

    #[test]
    #[serial]
    fn test_headless_roots_default() {
        let previous = std::env::var("TOKSCALE_HEADLESS_DIR").ok();
        std::env::remove_var("TOKSCALE_HEADLESS_DIR");

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
        std::env::set_var("TOKSCALE_HEADLESS_DIR", "/custom/headless");

        let roots = headless_roots("/tmp/home");
        assert_eq!(roots, vec![PathBuf::from("/custom/headless")]);

        restore_env("TOKSCALE_HEADLESS_DIR", previous);
    }

    #[test]
    #[serial]
    fn test_scan_all_sources_opencode() {
        let previous_xdg = std::env::var("XDG_DATA_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_opencode_dir(home);

        // Set XDG_DATA_HOME for the test
        std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));

        let result = scan_all_sources(home.to_str().unwrap(), &["opencode".to_string()]);
        assert_eq!(result.opencode_files.len(), 1);
        assert!(result.claude_files.is_empty());
        assert!(result.codex_files.is_empty());
        assert!(result.gemini_files.is_empty());

        restore_env("XDG_DATA_HOME", previous_xdg);
    }

    #[test]
    fn test_scan_all_sources_pi() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_pi_dir(home);

        let result = scan_all_sources(home.to_str().unwrap(), &["pi".to_string()]);
        assert_eq!(result.pi_files.len(), 1);
        assert!(result.opencode_files.is_empty());
        assert!(result.claude_files.is_empty());
    }

    #[test]
    fn test_scan_all_sources_claude() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_claude_dir(home);

        let result = scan_all_sources(home.to_str().unwrap(), &["claude".to_string()]);
        assert_eq!(result.claude_files.len(), 1);
        assert!(result.opencode_files.is_empty());
    }

    #[test]
    fn test_scan_all_sources_gemini() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_gemini_dir(home);

        let result = scan_all_sources(home.to_str().unwrap(), &["gemini".to_string()]);
        assert_eq!(result.gemini_files.len(), 1);
        assert!(result.opencode_files.is_empty());
    }

    #[test]
    fn test_scan_all_sources_openclaw_jsonl_only() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_openclaw_dir(home);

        let result = scan_all_sources(home.to_str().unwrap(), &["openclaw".to_string()]);
        assert_eq!(result.openclaw_files.len(), 1);
        assert!(result.openclaw_files[0].ends_with("session-abc.jsonl"));
    }

    #[test]
    fn test_scan_all_sources_multiple() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();

        setup_mock_claude_dir(home);
        setup_mock_gemini_dir(home);

        let result = scan_all_sources(
            home.to_str().unwrap(),
            &["claude".to_string(), "gemini".to_string()],
        );

        assert_eq!(result.claude_files.len(), 1);
        assert_eq!(result.gemini_files.len(), 1);
        assert!(result.opencode_files.is_empty());
        assert!(result.codex_files.is_empty());
    }

    #[test]
    #[serial]
    fn test_scan_all_sources_headless_paths() {
        let previous_headless = std::env::var("TOKSCALE_HEADLESS_DIR").ok();
        std::env::remove_var("TOKSCALE_HEADLESS_DIR");

        let dir = TempDir::new().unwrap();
        let home = dir.path();

        let mac_root = home
            .join("Library")
            .join("Application Support")
            .join("tokscale")
            .join("headless");

        fs::create_dir_all(mac_root.join("codex")).unwrap();
        File::create(mac_root.join("codex").join("codex.jsonl")).unwrap();

        let result = scan_all_sources(
            home.to_str().unwrap(),
            &[
                "claude".to_string(),
                "codex".to_string(),
                "gemini".to_string(),
            ],
        );

        assert!(result.claude_files.is_empty());
        assert_eq!(result.codex_files.len(), 1);
        assert!(result.gemini_files.is_empty());

        restore_env("TOKSCALE_HEADLESS_DIR", previous_headless);
    }

    #[test]
    #[serial]
    fn test_scan_all_sources_codex_with_env() {
        let previous_codex = std::env::var("CODEX_HOME").ok();

        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_codex_dir(home);

        // Set CODEX_HOME environment variable
        std::env::set_var("CODEX_HOME", home.join(".codex"));

        let result = scan_all_sources(home.to_str().unwrap(), &["codex".to_string()]);
        assert_eq!(result.codex_files.len(), 1);

        restore_env("CODEX_HOME", previous_codex);
    }
}
