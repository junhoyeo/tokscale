//! Amp thread sync.
//!
//! Amp keeps threads server-side, so there are no local thread files to scan.
//! `tokscale amp sync` exports them through the user's installed and
//! authenticated `amp` CLI into `<config>/amp-cache/threads/`, which the core
//! scanner then parses like any other local client. Credentials stay owned by
//! the Amp CLI; Tokscale never reads or stores them here.

use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const AMP_SYNC_DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// Implicit syncs block a report, the TUI, or a submit, so they get a short
/// budget; an unfinished sync continues on the next run.
const AMP_AUTO_SYNC_TIMEOUT: Duration = Duration::from_secs(10);
/// Implicit syncs are skipped while the last attempt is younger than this, so
/// scripted report loops do not spawn `amp` on every run.
const AMP_AUTO_SYNC_FRESHNESS: Duration = Duration::from_secs(5 * 60);
const LIST_PAGE_SIZE: usize = 200;
const MAX_CONCURRENT_EXPORTS: usize = 4;
/// Bump when the cached thread projection changes; a mismatched index makes
/// every thread pending again.
const INDEX_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AmpSyncIndex {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    last_attempt_at: Option<String>,
    /// Thread id -> `updated` value reported by `amp threads list` when the
    /// thread was last exported.
    #[serde(default)]
    threads: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct AmpThreadSummary {
    id: String,
    updated: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncAmpResult {
    /// The thread list was fetched. Individual exports may still have failed
    /// or been cut off by the timeout; see `error`.
    pub synced: bool,
    pub listed: usize,
    pub exported: usize,
    pub unchanged: usize,
    pub failed: usize,
    /// Threads not attempted because the sync timeout elapsed.
    pub remaining: usize,
    pub error: Option<String>,
}

pub fn get_amp_cache_dir() -> PathBuf {
    crate::paths::get_config_dir().join("amp-cache")
}

fn threads_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("threads")
}

fn index_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("index.json")
}

fn load_index(cache_dir: &Path) -> AmpSyncIndex {
    fs::read_to_string(index_path(cache_dir))
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn write_index(cache_dir: &Path, index: &AmpSyncIndex) -> Result<()> {
    let json = serde_json::to_vec_pretty(index)?;
    crate::commands::usage::helpers::atomic_write_secret(&index_path(cache_dir), &json)?;
    Ok(())
}

fn ensure_private_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

fn amp_binary_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["amp.exe", "amp.cmd"]
    } else {
        &["amp"]
    }
}

/// Resolve the Amp CLI: `PATH` first, then the standalone installer locations.
pub fn find_amp_binary() -> Option<PathBuf> {
    let names = amp_binary_names();
    let on_path = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
            .find(|candidate| candidate.is_file())
    });
    on_path.or_else(|| {
        let home = crate::paths::home_dir()?;
        [".amp/bin", ".local/share/amp/bin"]
            .iter()
            .flat_map(|dir| {
                let dir = home.join(dir);
                names.iter().map(move |name| dir.join(name))
            })
            .find(|candidate| candidate.is_file())
    })
}

pub(crate) fn run_amp(binary: &Path, args: &[&str], timeout: Duration) -> Result<Vec<u8>> {
    let mut cmd = Command::new(binary);
    // A logged-out CLI must fail instead of waiting on an interactive prompt.
    cmd.args(args).stdin(Stdio::null());
    let output = crate::commands::report::run_command_with_timeout(cmd, timeout, None)
        .with_context(|| format!("`amp {}` failed", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("no error output");
        anyhow::bail!(
            "`amp {}` exited with {}: {}",
            args.join(" "),
            output.status,
            detail
        );
    }
    Ok(output.stdout)
}

/// Thread ids become file names, so only accept Amp's `T-<uuid>` shape.
fn is_valid_thread_id(id: &str) -> bool {
    id.len() > 2
        && id.starts_with("T-")
        && id[2..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Keep only the fields the core Amp parser reads. Full exports carry the
/// whole conversation (often hundreds of KB per thread); the cache needs
/// usage, not prompts and tool output.
fn project_thread_export(bytes: &[u8], expected_id: &str) -> Result<Vec<u8>> {
    let thread: Value = serde_json::from_slice(bytes).context("thread export is not valid JSON")?;
    let id = thread.get("id").and_then(Value::as_str);
    if id != Some(expected_id) {
        anyhow::bail!("thread export has unexpected id {:?}", id);
    }

    let mut projected = Map::new();
    for key in ["id", "created", "usageLedger"] {
        if let Some(value) = thread.get(key) {
            projected.insert(key.to_string(), value.clone());
        }
    }
    let messages: Vec<Value> = thread
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|message| message.get("usage").is_some_and(|usage| !usage.is_null()))
        .map(|message| {
            let mut kept = Map::new();
            for key in ["role", "messageId", "usage"] {
                if let Some(value) = message.get(key) {
                    kept.insert(key.to_string(), value.clone());
                }
            }
            Value::Object(kept)
        })
        .collect();
    projected.insert("messages".to_string(), Value::Array(messages));
    Ok(serde_json::to_vec(&Value::Object(projected))?)
}

fn list_threads<F>(run: &F, deadline: Instant) -> Result<Vec<AmpThreadSummary>>
where
    F: Fn(&[&str], Duration) -> Result<Vec<u8>>,
{
    let limit = LIST_PAGE_SIZE.to_string();
    let mut threads = Vec::new();
    loop {
        let offset = threads.len().to_string();
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("timed out while listing Amp threads");
        }
        let args = [
            "threads",
            "list",
            "--json",
            "--include-archived",
            "--limit",
            &limit,
            "--offset",
            &offset,
        ];
        let stdout = run(&args, remaining)?;
        let page: Vec<AmpThreadSummary> = serde_json::from_slice(&stdout)
            .context("`amp threads list --json` returned unexpected output")?;
        let page_len = page.len();
        threads.extend(page);
        if page_len < LIST_PAGE_SIZE {
            return Ok(threads);
        }
    }
}

/// Sync core with an injectable `amp` runner. `run(args, timeout)` returns
/// the command's stdout or an error.
fn sync_amp_cache_with<F>(cache_dir: &Path, timeout: Duration, run: F) -> SyncAmpResult
where
    F: Fn(&[&str], Duration) -> Result<Vec<u8>> + Sync,
{
    let deadline = Instant::now() + timeout;
    let mut index = load_index(cache_dir);
    if index.version != INDEX_VERSION {
        // Cached exports use an older projection. Clearing the map keeps
        // them pending until each is re-exported, even across a failed
        // listing or a timed-out sync.
        index.threads.clear();
        index.version = INDEX_VERSION;
    }
    index.last_attempt_at = Some(chrono::Utc::now().to_rfc3339());

    let mut result = SyncAmpResult::default();
    let finish = |index: &AmpSyncIndex, mut result: SyncAmpResult| {
        if let Err(error) =
            ensure_private_dir(cache_dir).and_then(|_| write_index(cache_dir, index))
        {
            let message = format!("failed to write Amp sync index: {error}");
            result.error = Some(match result.error.take() {
                Some(existing) => format!("{existing}; {message}"),
                None => message,
            });
        }
        result
    };

    let threads = match list_threads(&run, deadline) {
        Ok(threads) => threads,
        Err(error) => {
            result.error = Some(format!("{error:#}"));
            return finish(&index, result);
        }
    };
    result.synced = true;
    result.listed = threads.len();

    let threads_dir = threads_dir(cache_dir);
    let pending: Vec<AmpThreadSummary> = threads
        .into_iter()
        .filter(|thread| is_valid_thread_id(&thread.id))
        .filter(|thread| {
            index.threads.get(&thread.id) != Some(&thread.updated)
                || !threads_dir.join(format!("{}.json", thread.id)).is_file()
        })
        .collect();
    result.unchanged = result.listed - pending.len();

    if let Err(error) = ensure_private_dir(cache_dir).and_then(|_| ensure_private_dir(&threads_dir))
    {
        result.error = Some(format!("failed to create Amp cache directory: {error}"));
        return finish(&index, result);
    }

    let next = AtomicUsize::new(0);
    let timed_out = AtomicUsize::new(0);
    let exported: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    let failures: Mutex<Vec<String>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..pending.len().min(MAX_CONCURRENT_EXPORTS) {
            scope.spawn(|| {
                while let Some(thread) = pending.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        timed_out.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    let outcome = run(&["threads", "export", &thread.id], remaining)
                        .and_then(|bytes| project_thread_export(&bytes, &thread.id))
                        .and_then(|bytes| {
                            let path = threads_dir.join(format!("{}.json", thread.id));
                            crate::commands::usage::helpers::atomic_write_secret(&path, &bytes)
                                .with_context(|| format!("failed to write {}", path.display()))
                        });
                    match outcome {
                        Ok(()) => exported
                            .lock()
                            .unwrap()
                            .push((thread.id.clone(), thread.updated.clone())),
                        // Cut off by the sync deadline: retry next run, not a failure.
                        Err(_) if Instant::now() >= deadline => {
                            timed_out.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(error) => failures
                            .lock()
                            .unwrap()
                            .push(format!("{}: {error:#}", thread.id)),
                    }
                }
            });
        }
    });

    let exported = exported.into_inner().unwrap();
    let failures = failures.into_inner().unwrap();
    result.exported = exported.len();
    result.failed = failures.len();
    result.remaining = timed_out.into_inner();
    index.threads.extend(exported);

    let mut problems = Vec::new();
    if let Some(first) = failures.first() {
        problems.push(format!(
            "{} thread export(s) failed (first: {first})",
            failures.len()
        ));
    }
    if result.remaining > 0 {
        problems.push(format!(
            "timed out after {}s with {} thread(s) left; run `tokscale amp sync` again to continue",
            timeout.as_secs(),
            result.remaining
        ));
    }
    if !problems.is_empty() {
        result.error = Some(problems.join("; "));
    }
    finish(&index, result)
}

fn sync_amp_cache(timeout: Duration) -> SyncAmpResult {
    let Some(binary) = find_amp_binary() else {
        return SyncAmpResult {
            error: Some(
                "Amp CLI not found. Install it from https://ampcode.com and run `amp login`."
                    .to_string(),
            ),
            ..SyncAmpResult::default()
        };
    };
    sync_amp_cache_with(&get_amp_cache_dir(), timeout, |args, timeout| {
        run_amp(&binary, args, timeout)
    })
}

/// True when no sync attempt is recorded within [`AMP_AUTO_SYNC_FRESHNESS`].
fn auto_sync_is_due(cache_dir: &Path) -> bool {
    let Some(last_attempt) = load_index(cache_dir).last_attempt_at else {
        return true;
    };
    let Ok(last_attempt) = chrono::DateTime::parse_from_rfc3339(&last_attempt) else {
        return true;
    };
    let age = chrono::Utc::now().signed_duration_since(last_attempt);
    // A future timestamp (clock skew) counts as fresh rather than re-syncing
    // on every run while the clock recovers.
    age.to_std().is_ok_and(|age| age >= AMP_AUTO_SYNC_FRESHNESS)
}

fn has_cached_threads(cache_dir: &Path) -> bool {
    fs::read_dir(threads_dir(cache_dir)).is_ok_and(|mut entries| {
        entries.any(|entry| {
            entry.is_ok_and(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with("T-") && name.ends_with(".json")
            })
        })
    })
}

/// Implicit incremental sync before Amp data is read (reports, TUI, submit,
/// wrapped). Skipped when the Amp CLI is missing or the last attempt is
/// recent. Returns a warning when the sync failed or was incomplete; callers
/// then read the existing cache.
pub fn auto_sync() -> Option<String> {
    let cache_dir = get_amp_cache_dir();
    if find_amp_binary().is_none() || !auto_sync_is_due(&cache_dir) {
        return None;
    }
    let had_cache = has_cached_threads(&cache_dir);
    let result = sync_amp_cache(AMP_AUTO_SYNC_TIMEOUT);
    let error = result.error?;
    let prefix = if result.synced {
        "Amp sync warning"
    } else if had_cache {
        "Amp sync failed; using cached data"
    } else {
        "Amp sync failed"
    };
    Some(format!("  {prefix}: {error}"))
}

fn manual_sync_completion(result: &SyncAmpResult) -> Result<()> {
    if !result.synced {
        anyhow::bail!(
            "Amp sync failed: {}",
            result.error.as_deref().unwrap_or("unknown error")
        );
    }
    if result.failed != 0 || result.remaining != 0 {
        anyhow::bail!(
            "Amp sync completed incompletely: {} failed, {} remaining",
            result.failed,
            result.remaining
        );
    }
    Ok(())
}

pub fn run_amp_sync(json: bool, timeout_secs: Option<u64>) -> Result<()> {
    let timeout = timeout_secs
        .map(Duration::from_secs)
        .unwrap_or(AMP_SYNC_DEFAULT_TIMEOUT);
    let result = sync_amp_cache(timeout);
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if result.synced {
        println!(
            "{}",
            format!(
                "  Amp: exported {} thread(s), {} unchanged ({} listed)",
                result.exported, result.unchanged, result.listed
            )
            .green()
        );
        if let Some(error) = &result.error {
            eprintln!("{}", format!("  Amp sync warning: {error}").yellow());
        }
    }
    manual_sync_completion(&result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn summary(id: &str, updated: &str) -> Value {
        serde_json::json!({ "id": id, "title": "t", "updated": updated, "messageCount": 2 })
    }

    fn export(id: &str) -> Vec<u8> {
        serde_json::json!({
            "id": id,
            "created": 1_791_882_483_264_i64,
            "title": "secret title",
            "messages": [
                { "role": "user", "messageId": 1, "usage": null, "content": [{ "text": "secret prompt" }] },
                {
                    "role": "assistant",
                    "messageId": 2,
                    "content": [{ "text": "secret answer" }],
                    "usage": { "model": "claude-sonnet-4-0", "timestamp": "2026-04-04T12:00:00Z", "inputTokens": 1 }
                }
            ]
        })
        .to_string()
        .into_bytes()
    }

    /// Fake `amp` runner: one list page plus per-thread export results.
    /// Records every exported id.
    struct FakeAmp {
        list: Vec<Value>,
        failing: Vec<&'static str>,
        exported: Mutex<Vec<String>>,
    }

    impl FakeAmp {
        fn new(list: Vec<Value>) -> Self {
            Self {
                list,
                failing: Vec::new(),
                exported: Mutex::new(Vec::new()),
            }
        }

        fn run(&self, args: &[&str], _timeout: Duration) -> Result<Vec<u8>> {
            match args {
                ["threads", "list", ..] => Ok(serde_json::to_vec(&self.list)?),
                ["threads", "export", id] => {
                    self.exported.lock().unwrap().push(id.to_string());
                    if self.failing.contains(id) {
                        anyhow::bail!("export exploded");
                    }
                    Ok(export(id))
                }
                other => anyhow::bail!("unexpected args {other:?}"),
            }
        }

        fn exported(&self) -> Vec<String> {
            let mut ids = self.exported.lock().unwrap().clone();
            ids.sort();
            ids
        }
    }

    fn sync(cache: &Path, fake: &FakeAmp) -> SyncAmpResult {
        sync_amp_cache_with(cache, AMP_SYNC_DEFAULT_TIMEOUT, |args, timeout| {
            fake.run(args, timeout)
        })
    }

    fn index_threads(cache: &Path) -> HashMap<String, String> {
        load_index(cache).threads.into_iter().collect()
    }

    #[test]
    fn incremental_sync_exports_only_new_or_updated_threads() {
        let temp = tempfile::TempDir::new().unwrap();
        let cache = temp.path();

        let first = FakeAmp::new(vec![summary("T-a", "1"), summary("T-b", "1")]);
        let result = sync(cache, &first);
        assert!(result.synced && result.error.is_none(), "{result:?}");
        assert_eq!(first.exported(), vec!["T-a", "T-b"]);

        // T-a unchanged, T-b updated, T-c new.
        let second = FakeAmp::new(vec![
            summary("T-a", "1"),
            summary("T-b", "2"),
            summary("T-c", "1"),
        ]);
        let result = sync(cache, &second);
        assert_eq!(second.exported(), vec!["T-b", "T-c"]);
        assert_eq!((result.exported, result.unchanged), (2, 1));
        assert_eq!(
            index_threads(cache),
            HashMap::from([
                ("T-a".to_string(), "1".to_string()),
                ("T-b".to_string(), "2".to_string()),
                ("T-c".to_string(), "1".to_string()),
            ])
        );
    }

    #[test]
    fn failed_export_does_not_stop_others_and_is_retried_next_sync() {
        let temp = tempfile::TempDir::new().unwrap();
        let cache = temp.path();

        let mut fake = FakeAmp::new(vec![
            summary("T-a", "1"),
            summary("T-bad", "1"),
            summary("T-c", "1"),
        ]);
        fake.failing = vec!["T-bad"];
        let result = sync(cache, &fake);
        assert!(result.synced);
        assert_eq!((result.exported, result.failed), (2, 1));
        assert!(result.error.as_deref().unwrap().contains("T-bad"));
        assert!(threads_dir(cache).join("T-a.json").is_file());
        assert!(threads_dir(cache).join("T-c.json").is_file());
        assert!(!threads_dir(cache).join("T-bad.json").exists());
        assert!(!index_threads(cache).contains_key("T-bad"));

        let retry = FakeAmp::new(vec![
            summary("T-a", "1"),
            summary("T-bad", "1"),
            summary("T-c", "1"),
        ]);
        sync(cache, &retry);
        assert_eq!(retry.exported(), vec!["T-bad"]);
    }

    #[test]
    fn empty_or_failed_listing_keeps_existing_cache() {
        let temp = tempfile::TempDir::new().unwrap();
        let cache = temp.path();
        sync(cache, &FakeAmp::new(vec![summary("T-a", "1")]));

        let result = sync(cache, &FakeAmp::new(Vec::new()));
        assert!(result.synced);
        assert!(threads_dir(cache).join("T-a.json").is_file());
        assert!(index_threads(cache).contains_key("T-a"));

        let result = sync_amp_cache_with(cache, AMP_SYNC_DEFAULT_TIMEOUT, |_, _| {
            anyhow::bail!("not logged in")
        });
        assert!(!result.synced);
        assert!(result.error.as_deref().unwrap().contains("not logged in"));
        assert!(threads_dir(cache).join("T-a.json").is_file());
        assert!(index_threads(cache).contains_key("T-a"));
    }

    #[test]
    fn projection_change_re_exports_every_thread_after_a_failed_listing() {
        let temp = tempfile::TempDir::new().unwrap();
        let cache = temp.path();
        sync(cache, &FakeAmp::new(vec![summary("T-a", "1")]));
        let mut index = load_index(cache);
        index.version = INDEX_VERSION - 1;
        write_index(cache, &index).unwrap();

        let result = sync_amp_cache_with(cache, AMP_SYNC_DEFAULT_TIMEOUT, |_, _| {
            anyhow::bail!("offline")
        });
        assert!(!result.synced);

        let fake = FakeAmp::new(vec![summary("T-a", "1")]);
        sync(cache, &fake);
        assert_eq!(fake.exported(), vec!["T-a"]);
    }

    #[test]
    fn cached_thread_keeps_usage_and_drops_conversation_content() {
        let temp = tempfile::TempDir::new().unwrap();
        let cache = temp.path();
        sync(cache, &FakeAmp::new(vec![summary("T-a", "1")]));

        let cached = fs::read_to_string(threads_dir(cache).join("T-a.json")).unwrap();
        assert!(!cached.contains("secret"), "{cached}");
        let cached: Value = serde_json::from_str(&cached).unwrap();
        assert_eq!(cached["id"], "T-a");
        assert_eq!(cached["created"], 1_791_882_483_264_i64);
        assert_eq!(cached["messages"].as_array().unwrap().len(), 1);
        assert_eq!(
            cached["messages"][0]["usage"]["timestamp"],
            "2026-04-04T12:00:00Z"
        );
    }

    #[test]
    fn listing_paginates_until_a_short_page() {
        let temp = tempfile::TempDir::new().unwrap();
        let offsets = Mutex::new(Vec::new());
        let result = sync_amp_cache_with(
            temp.path(),
            AMP_SYNC_DEFAULT_TIMEOUT,
            |args, _| match args {
                ["threads", "list", .., "--offset", offset] => {
                    let offset: usize = offset.parse()?;
                    offsets.lock().unwrap().push(offset);
                    let count = if offset == 0 { LIST_PAGE_SIZE } else { 3 };
                    let page: Vec<Value> = (offset..offset + count)
                        .map(|i| summary(&format!("T-{i}"), "1"))
                        .collect();
                    Ok(serde_json::to_vec(&page)?)
                }
                ["threads", "export", id] => Ok(export(id)),
                other => anyhow::bail!("unexpected args {other:?}"),
            },
        );
        assert_eq!(*offsets.lock().unwrap(), vec![0, LIST_PAGE_SIZE]);
        assert_eq!(result.listed, LIST_PAGE_SIZE + 3);
        assert_eq!(result.exported, LIST_PAGE_SIZE + 3);
    }

    #[test]
    fn rejects_thread_ids_that_are_not_safe_file_names() {
        assert!(is_valid_thread_id("T-01a13484-d595-75dc-8b07-5dcca2bef897"));
        assert!(!is_valid_thread_id("T-../../etc/passwd"));
        assert!(!is_valid_thread_id("T-"));
        assert!(!is_valid_thread_id("thread-1"));
    }

    #[test]
    fn manual_sync_rejects_partial_export_results() {
        let result = SyncAmpResult {
            synced: true,
            failed: 1,
            ..SyncAmpResult::default()
        };
        assert!(manual_sync_completion(&result).is_err());
        let result = SyncAmpResult {
            synced: true,
            remaining: 1,
            ..SyncAmpResult::default()
        };
        assert!(manual_sync_completion(&result).is_err());
        assert!(manual_sync_completion(&SyncAmpResult {
            synced: true,
            ..SyncAmpResult::default()
        })
        .is_ok());
    }
}
