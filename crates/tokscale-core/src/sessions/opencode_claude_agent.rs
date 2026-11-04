//! OpenCode's `claude-agent` provider drives the Claude Agent SDK, which writes
//! an ordinary Claude Code transcript under `~/.claude/projects` for every turn.
//! OpenCode records the same turns (with usage) in its own database and tags
//! each content part with `native.sessionID`, the Claude transcript's session
//! ID. Counting both would report the usage twice and credit Claude Code for
//! work done in OpenCode, so the scanner drops Claude transcripts whose session
//! ID an OpenCode database has actually counted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::opencode_schema::{
    opencode_payload_is_countable_usage, OPENCODE_SCHEMA_PARSER_BASE_VERSION,
};
use super::utils::open_readonly_sqlite_opt;
use crate::message_cache::{FingerprintStatus, SourceFingerprint};

/// Bump when the claim rules in this module change. A cached set from an older
/// version is discarded even if the database file is unchanged.
const CLAIM_LOGIC_VERSION: u32 = 1;

/// Claims depend on both this module and the shared OpenCode driver's
/// countable-usage gate, so a cached set is valid only when both match.
const CLAIM_CACHE_VERSION: (u32, u32) = (CLAIM_LOGIC_VERSION, OPENCODE_SCHEMA_PARSER_BASE_VERSION);

const CLAIM_QUERY: &str = "\
SELECT data FROM session_message
WHERE type = 'assistant'
  AND json_extract(data, '$.tokens') IS NOT NULL
  AND data LIKE '%\"native\"%sessionID%'";

/// Collect every Claude Agent SDK session ID whose usage OpenCode counted.
pub(crate) fn claimed_claude_session_ids(db_paths: &[PathBuf]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for db_path in db_paths {
        ids.extend(claims_for_db(db_path));
    }
    ids
}

fn claims_for_db(db_path: &Path) -> HashSet<String> {
    if let Some(cached) = read_claim_cache(db_path) {
        return cached;
    }
    let fingerprint = match SourceFingerprint::check_sqlite_path(db_path, None) {
        Some(FingerprintStatus::Changed(fingerprint)) => Some(fingerprint),
        _ => None,
    };
    let mut ids = HashSet::new();
    // Cache only a finished scan. A locked or unreadable file fails closed
    // with no cache entry, so the next report tries again. A missing
    // `session_message` table is finished: it claims nothing, and creating
    // that table later changes the file fingerprint.
    if collect_from_db(db_path, &mut ids) {
        if let Some(fingerprint) = fingerprint.as_ref() {
            if matches!(
                SourceFingerprint::check_sqlite_path(db_path, Some(fingerprint)),
                Some(FingerprintStatus::Unchanged)
            ) {
                write_claim_cache(db_path, fingerprint, &ids);
            }
        }
    }
    ids
}

/// `true` when every qualifying row was read. `false` leaves the set partial
/// and tells the caller not to cache it.
fn collect_from_db(db_path: &Path, ids: &mut HashSet<String>) -> bool {
    let Some(conn) = open_readonly_sqlite_opt(db_path) else {
        return false;
    };
    // Only the v2 `session_message` table carries native SDK ids. A missing
    // table (older OpenCode) is a finished scan that claims nothing.
    let mut stmt = match conn.prepare(CLAIM_QUERY) {
        Ok(stmt) => stmt,
        Err(error) => return error.to_string().contains("no such table"),
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
        Ok(rows) => rows,
        Err(_) => return false,
    };
    // Stream rows instead of buffering every payload: OpenCode histories can
    // be large. A fetch error (e.g. SQLITE_BUSY mid-scan) ends the step loop;
    // keep what was claimed so far but report the scan as incomplete.
    let mut complete = true;
    for row in rows {
        let Ok(data) = row else {
            complete = false;
            break;
        };
        if !opencode_payload_is_countable_usage(&data) {
            continue;
        }
        match serde_json::from_str::<Value>(&data) {
            Ok(value) => collect_native_session_ids(&value, ids),
            Err(_) => complete = false,
        }
    }
    complete
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

#[derive(Serialize, Deserialize)]
struct ClaimCacheFile {
    version: (u32, u32),
    fingerprint: SourceFingerprint,
    session_ids: Vec<String>,
}

fn claim_cache_path(db_path: &Path) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(db_path.as_os_str().as_encoded_bytes());
    let digest = hasher.finalize();
    let mut name = String::with_capacity(digest.len() * 2);
    for byte in digest {
        name.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        name.push(char::from(b"0123456789abcdef"[(byte & 0xf) as usize]));
    }
    name.push_str(".bin");
    crate::paths::get_cache_dir()
        .join("opencode-claude-agent-claims")
        .join(name)
}

fn read_claim_cache(db_path: &Path) -> Option<HashSet<String>> {
    let path = claim_cache_path(db_path);
    let cached: ClaimCacheFile = bincode::deserialize(&std::fs::read(path).ok()?).ok()?;
    if cached.version != CLAIM_CACHE_VERSION {
        return None;
    }
    match SourceFingerprint::check_sqlite_path(db_path, Some(&cached.fingerprint))? {
        FingerprintStatus::Unchanged => Some(cached.session_ids.into_iter().collect()),
        FingerprintStatus::Changed(_) => None,
    }
}

fn write_claim_cache(db_path: &Path, fingerprint: &SourceFingerprint, ids: &HashSet<String>) {
    let path = claim_cache_path(db_path);
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let mut session_ids: Vec<String> = ids.iter().cloned().collect();
    session_ids.sort();
    let cached = ClaimCacheFile {
        version: CLAIM_CACHE_VERSION,
        fingerprint: fingerprint.clone(),
        session_ids,
    };
    let Ok(bytes) = bincode::serialize(&cached) else {
        return;
    };
    // Concurrent reports may write the same entry, so each writer needs its
    // own temp file; a shared name lets one truncate it while another renames.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = parent.join(format!(
        ".{}.{}.{nanos:x}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("claims"),
        std::process::id(),
    ));
    let written =
        std::fs::write(&tmp, bytes).and_then(|()| crate::fs_atomic::replace_file(&tmp, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn isolated_config() -> (tempfile::TempDir, crate::paths::test_env::EnvGuard) {
        let dir = tempfile::TempDir::new().unwrap();
        let mut env = crate::paths::test_env::EnvGuard::capture(&["TOKSCALE_CONFIG_DIR"]);
        env.set("TOKSCALE_CONFIG_DIR", dir.path());
        (dir, env)
    }

    const SDK_SESSION: &str = "67ef217f-3444-4f81-a770-69fd1d2930d3";
    const OTHER_SESSION: &str = "d0559909-68b7-4e89-92da-1ee9e95c7726";

    fn message_data(session_id: &str, countable: bool) -> String {
        let mut data = serde_json::json!({
            "model": {"id": "opus", "providerID": "claude-agent"},
            "content": [{"type": "text", "text": "hi",
                "native": {"sessionID": session_id, "messageID": "m1"}}],
            "tokens": {"input": 1, "output": 2}
        });
        if countable {
            data["tokens"]["cache"] = serde_json::json!({"read": 3, "write": 4});
            data["time"] = serde_json::json!({"created": 1_700_000_000_000u64});
        }
        data.to_string()
    }

    fn insert_message(conn: &Connection, id: &str, kind: &str, data: &str) {
        conn.execute(
            "INSERT INTO session_message VALUES (?1, 'ses_1', ?2, ?3)",
            rusqlite::params![id, kind, data],
        )
        .unwrap();
    }

    fn opencode_db(dir: &Path) -> PathBuf {
        let db_path = dir.join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT, type TEXT, data TEXT);",
        )
        .unwrap();
        insert_message(
            &conn,
            "msg_1",
            "assistant",
            &message_data(SDK_SESSION, true),
        );
        db_path
    }

    #[test]
    #[serial_test::serial]
    fn drops_main_and_subagent_transcripts_claimed_by_opencode() {
        let (_config_dir, _env) = isolated_config();
        let dir = tempfile::TempDir::new().unwrap();
        let db = opencode_db(dir.path());
        assert_eq!(
            crate::sessions::opencode::parse_opencode_sqlite(&db).len(),
            1,
            "the claiming row has to be usage OpenCode actually counts"
        );
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
    #[serial_test::serial]
    fn scanner_drops_sdk_transcripts_for_full_and_claude_only_scans() {
        let (_config_dir, _env) = isolated_config();
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
    #[serial_test::serial]
    fn keeps_everything_without_session_message_table() {
        let (_config_dir, _env) = isolated_config();
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

    #[test]
    #[serial_test::serial]
    fn keeps_claude_transcript_when_opencode_does_not_count_the_row() {
        let (_config_dir, _env) = isolated_config();
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT, type TEXT, data TEXT);",
        )
        .unwrap();
        insert_message(
            &conn,
            "msg_partial",
            "assistant",
            &message_data(SDK_SESSION, false),
        );
        insert_message(
            &conn,
            "msg_user",
            "user",
            &message_data(OTHER_SESSION, true),
        );
        drop(conn);
        assert!(
            crate::sessions::opencode::parse_opencode_sqlite(&db_path).is_empty(),
            "neither row is counted OpenCode usage"
        );

        let project = dir.path().join("projects");
        let mut paths = vec![
            project.join(format!("{SDK_SESSION}.jsonl")),
            project.join(format!("{OTHER_SESSION}.jsonl")),
        ];
        drop_claimed_claude_paths(&mut paths, &[db_path]);
        assert_eq!(paths.len(), 2);
    }

    #[test]
    #[serial_test::serial]
    fn reuses_claim_cache_until_the_database_changes() {
        let (_config_dir, _env) = isolated_config();
        let dir = tempfile::TempDir::new().unwrap();
        let db = opencode_db(dir.path());
        let project = dir.path().join("projects");
        let sdk = project.join(format!("{SDK_SESSION}.jsonl"));
        let other = project.join(format!("{OTHER_SESSION}.jsonl"));
        let mut paths = vec![sdk.clone(), other.clone()];
        drop_claimed_claude_paths(&mut paths, std::slice::from_ref(&db));
        assert_eq!(paths, vec![other.clone()]);

        let cache_path = claim_cache_path(&db);
        let mut cached: ClaimCacheFile =
            bincode::deserialize(&std::fs::read(&cache_path).unwrap()).unwrap();
        cached.session_ids = vec![OTHER_SESSION.to_string()];
        std::fs::write(&cache_path, bincode::serialize(&cached).unwrap()).unwrap();

        let mut paths = vec![sdk.clone(), other.clone()];
        drop_claimed_claude_paths(&mut paths, std::slice::from_ref(&db));
        assert_eq!(
            paths,
            vec![sdk.clone()],
            "an unchanged database must reuse the cached claim set"
        );

        // A cache written under another OpenCode driver version is stale even
        // though the database is unchanged.
        cached.version.1 = OPENCODE_SCHEMA_PARSER_BASE_VERSION + 1;
        std::fs::write(&cache_path, bincode::serialize(&cached).unwrap()).unwrap();
        let mut paths = vec![sdk.clone(), other.clone()];
        drop_claimed_claude_paths(&mut paths, std::slice::from_ref(&db));
        assert_eq!(paths, vec![other.clone()]);

        let conn = Connection::open(&db).unwrap();
        insert_message(
            &conn,
            "msg_2",
            "assistant",
            &message_data(SDK_SESSION, true),
        );
        drop(conn);

        let mut paths = vec![sdk.clone(), other.clone()];
        drop_claimed_claude_paths(&mut paths, &[db]);
        assert_eq!(paths, vec![other]);
    }
}
