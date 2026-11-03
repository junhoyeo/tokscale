//! OpenCode's `claude-agent` provider drives the Claude Agent SDK, which writes
//! an ordinary Claude Code transcript under `~/.claude/projects` for every turn.
//! OpenCode records the same turns (with usage) in its own database and tags
//! each content part with `native.sessionID`, the Claude transcript's session
//! ID. Counting both would report the usage twice and credit Claude Code for
//! work done in OpenCode, so the scanner drops Claude transcripts whose session
//! ID an OpenCode database claims.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::utils::open_readonly_sqlite_opt;

/// Collect every Claude Agent SDK session ID referenced by OpenCode messages.
pub(crate) fn claimed_claude_session_ids(db_paths: &[PathBuf]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for db_path in db_paths {
        collect_from_db(db_path, &mut ids);
    }
    ids
}

fn collect_from_db(db_path: &Path, ids: &mut HashSet<String>) {
    let Some(conn) = open_readonly_sqlite_opt(db_path) else {
        return;
    };
    // Only the v2 `session_message` table carries native SDK ids. A missing
    // table (older OpenCode) simply claims nothing.
    let Ok(mut stmt) =
        conn.prepare("SELECT data FROM session_message WHERE data LIKE '%\"native\"%sessionID%'")
    else {
        return;
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
        return;
    };
    for data in rows.flatten() {
        if let Ok(value) = serde_json::from_str::<Value>(&data) {
            collect_native_session_ids(&value, ids);
        }
    }
}

fn collect_native_session_ids(value: &Value, ids: &mut HashSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(id) = map
                .get("native")
                .and_then(|native| native.get("sessionID"))
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            {
                ids.insert(id.to_string());
            }
            for child in map.values() {
                collect_native_session_ids(child, ids);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_native_session_ids(item, ids);
            }
        }
        _ => {}
    }
}

/// Session ID a Claude Code transcript path belongs to: the file stem for a
/// main transcript, or the owning session directory for a subagent sidechain
/// (`<project>/<session>/subagents/agent-*.jsonl`).
pub(crate) fn claude_transcript_session_id(path: &Path) -> Option<&str> {
    let parent = path.parent()?;
    if parent.file_name().and_then(|n| n.to_str()) == Some("subagents") {
        return parent.parent()?.file_name()?.to_str();
    }
    path.file_stem()?.to_str()
}

/// Drop Claude transcripts that OpenCode already accounts for.
pub(crate) fn drop_claimed_claude_paths(paths: &mut Vec<PathBuf>, opencode_dbs: &[PathBuf]) {
    if paths.is_empty() || opencode_dbs.is_empty() {
        return;
    }
    let claimed = claimed_claude_session_ids(opencode_dbs);
    if claimed.is_empty() {
        return;
    }
    paths.retain(|path| {
        claude_transcript_session_id(path).is_none_or(|session| !claimed.contains(session))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    const SDK_SESSION: &str = "67ef217f-3444-4f81-a770-69fd1d2930d3";

    fn opencode_db(dir: &Path) -> PathBuf {
        let db_path = dir.join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT, type TEXT, data TEXT);",
        )
        .unwrap();
        let data = serde_json::json!({
            "model": {"id": "opus", "providerID": "claude-agent"},
            "content": [{"type": "text", "text": "hi",
                "native": {"sessionID": SDK_SESSION, "messageID": "m1"}}],
            "tokens": {"input": 1, "output": 2}
        });
        conn.execute(
            "INSERT INTO session_message VALUES ('msg_1', 'ses_1', 'assistant', ?1)",
            [data.to_string()],
        )
        .unwrap();
        db_path
    }

    #[test]
    fn drops_main_and_subagent_transcripts_claimed_by_opencode() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = opencode_db(dir.path());
        let project = dir.path().join("projects/-home-me-repo");
        let mut paths = vec![
            project.join(format!("{SDK_SESSION}.jsonl")),
            project.join(format!("{SDK_SESSION}/subagents/agent-abc.jsonl")),
            project.join("d0559909-68b7-4e89-92da-1ee9e95c7726.jsonl"),
        ];
        drop_claimed_claude_paths(&mut paths, &[db]);
        assert_eq!(
            paths,
            vec![project.join("d0559909-68b7-4e89-92da-1ee9e95c7726.jsonl")]
        );
    }

    #[test]
    fn scanner_drops_sdk_transcripts_for_full_and_claude_only_scans() {
        let home = tempfile::TempDir::new().unwrap();
        let opencode_dir = home.path().join(".local/share/opencode");
        std::fs::create_dir_all(&opencode_dir).unwrap();
        opencode_db(&opencode_dir);
        let project = home.path().join(".claude/projects/-home-me-repo");
        std::fs::create_dir_all(&project).unwrap();
        let sdk = project.join(format!("{SDK_SESSION}.jsonl"));
        let cli = project.join("d0559909-68b7-4e89-92da-1ee9e95c7726.jsonl");
        std::fs::write(&sdk, "{}\n").unwrap();
        std::fs::write(&cli, "{}\n").unwrap();

        let home_str = home.path().to_str().unwrap();
        for clients in [vec![], vec!["claude".to_string()]] {
            let result =
                crate::scanner::scan_all_clients_with_env_strategy(home_str, &clients, false);
            let claude = result.get(crate::clients::ClientId::Claude);
            assert!(claude.contains(&cli), "clients={clients:?}");
            assert!(!claude.contains(&sdk), "clients={clients:?}");
        }
    }

    #[test]
    fn keeps_everything_without_session_message_table() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("opencode.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch("CREATE TABLE message (id TEXT, data TEXT);")
            .unwrap();
        let mut paths = vec![dir.path().join(format!("p/{SDK_SESSION}.jsonl"))];
        drop_claimed_claude_paths(&mut paths, &[db_path]);
        assert_eq!(paths.len(), 1);
    }
}
