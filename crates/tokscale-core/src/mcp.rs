//! MCP server discovery — collects configured server *names* only (no secrets/paths).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct McpServerEntry {
    pub name: String,
    pub source: McpSource,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpSource {
    ClaudeCode,
    ClaudeDesktop,
    Cursor,
    Kiro,
    OpenCodeSkill,
}

pub fn discover_mcp_server_names(home_dir: Option<&Path>) -> Vec<String> {
    let home = match home_dir.map(PathBuf::from).or_else(dirs::home_dir) {
        Some(h) => h,
        None => return Vec::new(),
    };

    let mut names: BTreeSet<String> = BTreeSet::new();

    collect_mcp_server_keys(&home.join(".claude").join(".mcp.json"), &mut names);

    #[cfg(target_os = "macos")]
    {
        collect_mcp_server_keys(
            &home
                .join("Library")
                .join("Application Support")
                .join("Claude")
                .join("claude_desktop_config.json"),
            &mut names,
        );
    }

    collect_mcp_server_keys(&home.join(".cursor").join("mcp.json"), &mut names);

    collect_mcp_server_keys(
        &home.join(".kiro").join("settings").join("mcp.json"),
        &mut names,
    );

    collect_skill_mcp_names(
        &home.join(".config").join("opencode").join("skills"),
        &mut names,
    );
    collect_skill_mcp_names(&home.join(".opencode").join("skills"), &mut names);

    names.into_iter().collect()
}

fn collect_mcp_server_keys(path: &Path, names: &mut BTreeSet<String>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    if let Some(servers) = value.get("mcpServers").and_then(|v| v.as_object()) {
        for key in servers.keys() {
            if !key.is_empty() {
                names.insert(key.clone());
            }
        }
    }
}

fn collect_skill_mcp_names(skills_dir: &Path, names: &mut BTreeSet<String>) {
    let dir = match std::fs::read_dir(skills_dir) {
        Ok(d) => d,
        Err(_) => return,
    };

    for entry in dir.flatten() {
        let path = entry.path();

        if path.is_dir() {
            let skill_file = path.join("SKILL.md");
            if skill_file.is_file() {
                extract_mcp_names_from_skill_md(&skill_file, names);
            }
        } else if path.extension().is_some_and(|ext| ext == "md") {
            extract_mcp_names_from_skill_md(&path, names);
        }
    }
}

/// Line-based YAML frontmatter `mcp:` key extractor (avoids a full YAML dependency).
fn extract_mcp_names_from_skill_md(path: &Path, names: &mut BTreeSet<String>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let frontmatter = match extract_yaml_frontmatter(&content) {
        Some(fm) => fm,
        None => return,
    };

    let mut in_mcp_section = false;
    let mut mcp_indent: usize = 0;

    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = line.len() - line.trim_start().len();

        if !in_mcp_section {
            if trimmed == "mcp:" || trimmed.starts_with("mcp:") {
                in_mcp_section = true;
                mcp_indent = indent;
            }
        } else {
            if indent <= mcp_indent && !trimmed.is_empty() {
                break;
            }

            if indent == mcp_indent + 2 || (mcp_indent == 0 && indent == 2) {
                if let Some(key) = trimmed
                    .strip_suffix(':')
                    .or_else(|| trimmed.split(':').next())
                {
                    let key = key.trim();
                    if !key.is_empty() && !key.starts_with('-') {
                        names.insert(key.to_string());
                    }
                }
            }
        }
    }
}

fn extract_yaml_frontmatter(content: &str) -> Option<&str> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    let after_first = &trimmed[3..];
    let after_first = after_first.trim_start_matches(['\r', '\n']);

    let end = after_first.find("\n---")?;
    Some(&after_first[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_collect_mcp_server_keys() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("mcp.json");
        fs::write(
            &path,
            r#"{"mcpServers":{"slack":{"command":"npx"},"github":{"command":"npx"}}}"#,
        )
        .unwrap();

        let mut names = BTreeSet::new();
        collect_mcp_server_keys(&path, &mut names);
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            vec!["github", "slack"]
        );
    }

    #[test]
    fn test_collect_mcp_server_keys_missing_file() {
        let mut names = BTreeSet::new();
        collect_mcp_server_keys(Path::new("/nonexistent/mcp.json"), &mut names);
        assert!(names.is_empty());
    }

    #[test]
    fn test_extract_mcp_names_from_skill_md() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.md");
        fs::write(
            &path,
            r#"---
name: playwright
description: "Browser automation"
mcp:
  playwright:
    command: npx
    args:
      - "@playwright/mcp@latest"
---

# Content here
"#,
        )
        .unwrap();

        let mut names = BTreeSet::new();
        extract_mcp_names_from_skill_md(&path, &mut names);
        assert_eq!(names.into_iter().collect::<Vec<_>>(), vec!["playwright"]);
    }

    #[test]
    fn test_extract_mcp_names_from_skill_md_multiple() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("multi.md");
        fs::write(
            &path,
            r#"---
mcp:
  server-a:
    command: npx
    args: ["-y", "server-a"]
  server-b:
    command: bunx
    args: ["server-b"]
---
"#,
        )
        .unwrap();

        let mut names = BTreeSet::new();
        extract_mcp_names_from_skill_md(&path, &mut names);
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            vec!["server-a", "server-b"]
        );
    }

    #[test]
    fn test_skill_directory_layout() {
        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path().join("skills");

        // Subdirectory layout
        let tmap_dir = skills_dir.join("tmap");
        fs::create_dir_all(&tmap_dir).unwrap();
        fs::write(
            tmap_dir.join("SKILL.md"),
            "---\ndescription: \"TMAP\"\n---\n# TMAP MCP\n",
        )
        .unwrap();

        // Flat file with mcp section
        fs::write(
            skills_dir.join("playwright.md"),
            "---\nmcp:\n  playwright:\n    command: npx\n---\n# PW\n",
        )
        .unwrap();

        let mut names = BTreeSet::new();
        collect_skill_mcp_names(&skills_dir, &mut names);
        // tmap SKILL.md has no mcp: section, so only playwright is found
        assert_eq!(names.into_iter().collect::<Vec<_>>(), vec!["playwright"]);
    }

    #[test]
    fn test_discover_mcp_server_names_integration() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();

        // Create Claude .mcp.json
        let claude_dir = home.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            claude_dir.join(".mcp.json"),
            r#"{"mcpServers":{"slack":{},"tmap":{}}}"#,
        )
        .unwrap();

        // Create Cursor mcp.json
        let cursor_dir = home.join(".cursor");
        fs::create_dir_all(&cursor_dir).unwrap();
        fs::write(
            cursor_dir.join("mcp.json"),
            r#"{"mcpServers":{"github":{},"slack":{}}}"#,
        )
        .unwrap();

        // Create Kiro mcp.json
        let kiro_dir = home.join(".kiro").join("settings");
        fs::create_dir_all(&kiro_dir).unwrap();
        fs::write(kiro_dir.join("mcp.json"), r#"{"mcpServers":{"tmap":{}}}"#).unwrap();

        // Create skill
        let skills_dir = home.join(".opencode").join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(
            skills_dir.join("apple-mcp.md"),
            "---\nmcp:\n  apple-mcp:\n    command: bunx\n---\n",
        )
        .unwrap();

        let result = discover_mcp_server_names(Some(home));
        // Deduplicated and sorted
        assert_eq!(result, vec!["apple-mcp", "github", "slack", "tmap"]);
    }

    #[test]
    fn test_extract_yaml_frontmatter() {
        let content = "---\nname: test\nmcp:\n  foo:\n    cmd: x\n---\n# Body";
        let fm = extract_yaml_frontmatter(content).unwrap();
        assert!(fm.contains("mcp:"));
        assert!(!fm.contains("# Body"));
    }

    #[test]
    fn test_no_frontmatter() {
        assert_eq!(extract_yaml_frontmatter("# Just a heading"), None);
    }
}
