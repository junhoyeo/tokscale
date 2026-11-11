//! Token-scoped request state. No credentials are serialized here.
use super::{UsageFreshness, UsageOutput};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

const FRESH_SAMPLE_SECS: u64 = 60;
const MAX_STALE_SECS: u64 = 3600;

#[derive(Clone, Default, Deserialize, Serialize)]
pub(super) struct State {
    version: u8,
    pub observed_at: u64,
    // Request serialization orders mutations even within the same clock second.
    // Old state files predate the counter and start at zero.
    #[serde(default)]
    revision: u64,
    pub sample: Option<UsageOutput>,
    pub failures: u32,
    pub until: u64,
}
static MEMORY: OnceLock<Mutex<HashMap<PathBuf, State>>> = OnceLock::new();
type RequestLocks = HashMap<PathBuf, Arc<Mutex<()>>>;
static REQUESTS: OnceLock<Mutex<RequestLocks>> = OnceLock::new();

pub(super) fn path(root: &Path, key: &str) -> PathBuf {
    root.join("claude-samples").join(format!("{key}.json"))
}
pub(super) fn memory_lock(path: &Path) -> Arc<Mutex<()>> {
    REQUESTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(path.into())
        .or_default()
        .clone()
}
pub(super) fn disk_lock(path: &Path) -> std::io::Result<fs::File> {
    fs::create_dir_all(path.parent().unwrap())?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("request.lock"))?;
    for _ in 0..10 {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.raw_os_error().is_some_and(|code| {
                        Some(code) == fs2::lock_contended_error().raw_os_error()
                    }) =>
            {
                // Windows reports ERROR_LOCK_VIOLATION as PermissionDenied.
                // Match fs2's exact contention code without masking access errors.
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::WouldBlock,
        "Claude usage request already in progress",
    ))
}
pub(super) fn load(path: &Path, now: u64) -> State {
    let memory = MEMORY
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .cloned();
    let disk = fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<State>(&b).ok());
    let valid = |s: &State| {
        s.version == 1
            && s.observed_at <= now
            && s.until.saturating_sub(s.observed_at) <= MAX_STALE_SECS
            && s.sample.as_ref().is_none_or(|output| {
                output.provider == "Claude"
                    && output
                        .freshness
                        .as_ref()
                        .is_some_and(|f| f.fetched_at <= s.observed_at)
            })
    };
    memory
        .into_iter()
        .chain(disk)
        .filter(valid)
        .max_by_key(|s| (s.observed_at, s.revision, s.sample.is_none()))
        .unwrap_or_default()
}
pub(super) fn save(path: &Path, state: &mut State, now: u64) {
    state.version = 1;
    state.observed_at = now;
    state.revision = state.revision.saturating_add(1);
    MEMORY
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.into(), state.clone());
    // Request lock serializes writers. Random staging names also tolerate an
    // interrupted previous writer without removing another process's file.
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        fs::create_dir_all(path.parent().unwrap())?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&serde_json::to_vec(state)?)?;
        file.sync_all()?;
        tokscale_core::fs_atomic::replace_file(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
}
impl State {
    pub fn sample(&self, now: u64, stale: bool) -> Option<UsageOutput> {
        let mut output = self.sample.clone()?;
        let fetched = output.freshness.as_ref()?.fetched_at;
        if fetched > now || now - fetched > MAX_STALE_SECS {
            return None;
        }
        output.freshness = Some(UsageFreshness {
            fetched_at: fetched,
            stale,
        });
        Some(output)
    }
    pub fn fresh(&self, now: u64) -> Option<UsageOutput> {
        self.sample(now, false)
            .filter(|s| now - s.freshness.as_ref().unwrap().fetched_at < FRESH_SAMPLE_SECS)
    }
    /// `Retry-After` can only lengthen the progressive backoff: the usage
    /// endpoint answers some 429s with `Retry-After: 0`, and honoring that
    /// would re-request on every refresh while the limit is still in force.
    pub fn limited(&mut self, retry_after: Option<u64>, now: u64) -> u64 {
        self.failures = self.failures.saturating_add(1);
        let backoff =
            FRESH_SAMPLE_SECS.saturating_mul(1u64 << self.failures.saturating_sub(1).min(6));
        let wait = retry_after
            .unwrap_or_default()
            .max(backoff)
            .min(MAX_STALE_SECS);
        self.until = now.saturating_add(wait);
        wait
    }
}

#[cfg(test)]
pub(super) fn forget_memory(path: &Path) {
    MEMORY
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .remove(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_zero_retry_after_uses_progressive_backoff() {
        let mut state = State::default();
        assert_eq!(state.limited(Some(0), 1000), 60);
        assert_eq!(state.until, 1060);
        assert_eq!(state.limited(Some(0), 1060), 120);
        assert_eq!(state.until, 1180);
        assert_eq!(state.limited(Some(0), 1180), 240);
    }

    #[test]
    fn limited_short_retry_after_is_raised_to_backoff() {
        let mut state = State::default();
        assert_eq!(state.limited(Some(30), 1000), 60);
    }

    #[test]
    fn limited_longer_retry_after_is_honored() {
        let mut state = State::default();
        assert_eq!(state.limited(Some(300), 1000), 300);
        assert_eq!(state.limited(Some(10_000), 1300), 3600);
    }

    #[test]
    fn limited_repeated_zero_caps_at_one_hour() {
        let mut state = State::default();
        let mut now = 1000;
        let waits: Vec<u64> = (0..8)
            .map(|_| {
                let wait = state.limited(Some(0), now);
                now += wait;
                wait
            })
            .collect();
        assert_eq!(waits, [60, 120, 240, 480, 960, 1920, 3600, 3600]);
    }

    #[test]
    fn disk_lock_reports_contention_as_would_block_on_every_platform() {
        let temp = tempfile::tempdir().unwrap();
        let path = path(temp.path(), "test-account");
        let _held = disk_lock(&path).unwrap();
        let started = std::time::Instant::now();
        let error = disk_lock(&path).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
