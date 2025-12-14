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
}

/// Result of scanning all session directories
#[derive(Debug, Default)]
pub struct ScanResult {
    pub opencode_files: Vec<PathBuf>,
    pub claude_files: Vec<PathBuf>,
    pub codex_files: Vec<PathBuf>,
    pub gemini_files: Vec<PathBuf>,
    pub cursor_files: Vec<PathBuf>,
}

impl ScanResult {
    /// Get total number of files found
    pub fn total_files(&self) -> usize {
        self.opencode_files.len()
            + self.claude_files.len()
            + self.codex_files.len()
            + self.gemini_files.len()
            + self.cursor_files.len()
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

        result
    }
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

            match pattern {
                "*.json" => file_name.ends_with(".json"),
                "*.jsonl" => file_name.ends_with(".jsonl"),
                "*.csv" => file_name.ends_with(".csv"),
                "session-*.json" => {
                    file_name.starts_with("session-") && file_name.ends_with(".json")
                }
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

    // Define scan tasks
    let mut tasks: Vec<(SessionType, String, &str)> = Vec::new();

    if include_opencode {
        // OpenCode: ~/.local/share/opencode/storage/message/*/*.json
        let xdg_data =
            std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{}/.local/share", home_dir));
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
    }

    if include_gemini {
        // Gemini: ~/.gemini/tmp/*/chats/session-*.json
        let gemini_path = format!("{}/.gemini/tmp", home_dir);
        tasks.push((SessionType::Gemini, gemini_path, "session-*.json"));
    }

    if include_cursor {
        // Cursor: ~/.config/tokscale/cursor-cache/*.csv (migrated from ~/.tokscale)
        let cursor_path = format!("{}/.config/tokscale/cursor-cache", home_dir);
        tasks.push((SessionType::Cursor, cursor_path, "*.csv"));
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
            SessionType::OpenCode => result.opencode_files = files,
            SessionType::Claude => result.claude_files = files,
            SessionType::Codex => result.codex_files = files,
            SessionType::Gemini => result.gemini_files = files,
            SessionType::Cursor => result.cursor_files = files,
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_scan_result_total_files() {
        let result = ScanResult {
            opencode_files: vec![PathBuf::from("a.json"), PathBuf::from("b.json")],
            claude_files: vec![PathBuf::from("c.jsonl")],
            codex_files: vec![],
            gemini_files: vec![PathBuf::from("d.json")],
            cursor_files: vec![],
        };
        assert_eq!(result.total_files(), 4);
    }

    #[test]
    fn test_scan_result_all_files() {
        let result = ScanResult {
            opencode_files: vec![PathBuf::from("a.json")],
            claude_files: vec![PathBuf::from("b.jsonl")],
            codex_files: vec![PathBuf::from("c.jsonl")],
            gemini_files: vec![PathBuf::from("d.json")],
            cursor_files: vec![PathBuf::from("e.csv")],
        };

        let all = result.all_files();
        assert_eq!(all.len(), 5);
        assert_eq!(all[0], (SessionType::OpenCode, PathBuf::from("a.json")));
        assert_eq!(all[1], (SessionType::Claude, PathBuf::from("b.jsonl")));
        assert_eq!(all[2], (SessionType::Codex, PathBuf::from("c.jsonl")));
        assert_eq!(all[3], (SessionType::Gemini, PathBuf::from("d.json")));
        assert_eq!(all[4], (SessionType::Cursor, PathBuf::from("e.csv")));
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

    #[test]
    fn test_scan_all_sources_opencode() {
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
    fn test_scan_all_sources_codex_with_env() {
        let dir = TempDir::new().unwrap();
        let home = dir.path();
        setup_mock_codex_dir(home);

        // Set CODEX_HOME environment variable
        std::env::set_var("CODEX_HOME", home.join(".codex"));

        let result = scan_all_sources(home.to_str().unwrap(), &["codex".to_string()]);
        assert_eq!(result.codex_files.len(), 1);
    }
}
