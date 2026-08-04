//! Exclusive locking shared by the client sync commands.
//!
//! Ownership is the kernel's exclusive lock on a file, not the bytes inside
//! it. The protocol this replaces created a lock file, wrote its pid, and on a
//! collision read that pid, probed whether it was alive, and unlinked and
//! retried when it was not — a read-decide-unlink sequence that is not atomic
//! (#1010).

use anyhow::{Context, Result};
use fs2::FileExt;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::process_liveness::pid_is_alive;

/// Wording a sync command uses when reporting on its own lock.
#[derive(Clone, Copy)]
pub(crate) struct SyncLockLabels {
    /// Sentence for a lock somebody else holds, without trailing punctuation,
    /// e.g. `"Another tokscale antigravity sync is in progress"`.
    pub(crate) busy: &'static str,
    /// Noun phrase for I/O error context, e.g. `"Antigravity sync lock"`.
    pub(crate) subject: &'static str,
}

/// Holds the sync lock for as long as it is alive.
///
/// The lock file is deliberately left on disk when the guard drops. Unlinking
/// it would let a contender create a fresh file and lock that instead, which
/// is the same hole the pid-file protocol had in a different shape.
#[derive(Debug)]
pub(crate) struct SyncLockGuard {
    _file: std::fs::File,
}

impl SyncLockGuard {
    pub(crate) fn acquire(cache_dir: &Path, labels: SyncLockLabels) -> Result<Self> {
        if !cache_dir.exists() {
            std::fs::create_dir_all(cache_dir).with_context(|| {
                format!(
                    "failed to create {} at {}",
                    labels.subject,
                    cache_dir.display()
                )
            })?;
        }

        let lock_path = cache_dir.join("sync.lock");
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| {
                format!(
                    "failed to open {} at {}",
                    labels.subject,
                    lock_path.display()
                )
            })?;

        // Read the recorded owner BEFORE locking. A binary from before this
        // change owns the sync through this file alone and holds no kernel
        // lock, so during a rolling upgrade the lock below would be free while
        // that sync is still writing the manifest. Reading first also keeps
        // this working on Windows, which refuses reads of a range once we hold
        // it ourselves.
        let recorded_owner = read_sync_lock(&lock_path);

        match file.try_lock_exclusive() {
            Ok(()) => {}
            Err(err) if crate::commands::autosubmit::is_lock_contention(&err) => {
                let owner = recorded_owner
                    .filter(|(pid, _)| pid_is_alive(*pid))
                    .map(|(pid, _)| format!(" (pid {pid})"))
                    .unwrap_or_default();
                anyhow::bail!("{}{owner}; aborting", labels.busy);
            }
            Err(err) => {
                return Err(anyhow::Error::new(err)
                    .context(format!("failed to acquire {}", labels.subject)));
            }
        }

        if let Some(pid) =
            pid_file_owner_still_running(recorded_owner, std::process::id(), pid_is_alive)
        {
            anyhow::bail!("{} (pid {pid}); aborting", labels.busy);
        }

        // Recorded for diagnostics, and so a binary from before this change
        // still sees an owner. The lock above is what excludes.
        let pid = std::process::id();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = file.set_len(0);
        let _ = writeln!(file, "{pid} {timestamp}");

        Ok(SyncLockGuard { _file: file })
    }
}

/// Whether a pre-change binary still owns this sync through the pid file
/// alone, which the kernel lock cannot detect because that binary never took
/// one. Returns the owner's pid when it does.
///
/// Our own pid is skipped: after a successful sync the file still names this
/// process, and a genuine second sync from the same process is already refused
/// by the exclusive lock.
fn pid_file_owner_still_running(
    recorded: Option<(u32, u64)>,
    self_pid: u32,
    is_alive: impl Fn(u32) -> bool,
) -> Option<u32> {
    let (pid, _) = recorded?;
    (pid != self_pid && is_alive(pid)).then_some(pid)
}

pub(crate) fn read_sync_lock(path: &Path) -> Option<(u32, u64)> {
    let contents = std::fs::read_to_string(path).ok()?;
    let mut parts = contents.split_whitespace();
    let pid = parts.next()?.parse::<u32>().ok()?;
    let timestamp = parts.next()?.parse::<u64>().ok()?;
    Some((pid, timestamp))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABELS: SyncLockLabels = SyncLockLabels {
        busy: "Another sync is in progress",
        subject: "test sync lock",
    };

    /// Regression (#1010 review): during a rolling upgrade a sync started by a
    /// binary from before this change owns the lock file but holds no kernel
    /// lock, so the exclusive lock alone would let both versions into the
    /// critical section.
    #[test]
    fn a_live_pid_file_owner_from_the_old_protocol_still_blocks() {
        assert_eq!(
            pid_file_owner_still_running(Some((4321, 1)), 1234, |_| true),
            Some(4321)
        );
    }

    #[test]
    fn a_dead_pid_file_owner_does_not_block() {
        assert_eq!(
            pid_file_owner_still_running(Some((4321, 1)), 1234, |_| false),
            None
        );
    }

    /// Our own pid is left behind by every successful sync, so it must never
    /// block the next one.
    #[test]
    fn our_own_recorded_pid_does_not_block() {
        assert_eq!(
            pid_file_owner_still_running(Some((1234, 1)), 1234, |_| true),
            None
        );
    }

    #[test]
    fn an_absent_or_unreadable_record_does_not_block() {
        assert_eq!(pid_file_owner_still_running(None, 1234, |_| true), None);
    }

    #[test]
    fn read_sync_lock_parses_pid_and_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sync.lock");
        std::fs::write(&path, "12345 1776000000\n").unwrap();
        assert_eq!(read_sync_lock(&path), Some((12345, 1776000000)));
    }

    #[test]
    fn read_sync_lock_returns_none_on_malformed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sync.lock");
        std::fs::write(&path, "not a pid\n").unwrap();
        assert!(read_sync_lock(&path).is_none());
    }

    /// A second sync is refused for as long as the first guard lives, and
    /// succeeds once it is dropped.
    #[test]
    fn a_second_acquire_is_refused_until_the_first_guard_drops() {
        let tmp = tempfile::tempdir().unwrap();
        let guard = SyncLockGuard::acquire(tmp.path(), LABELS).unwrap();

        let err = SyncLockGuard::acquire(tmp.path(), LABELS).unwrap_err();
        assert!(
            err.to_string().contains("Another sync is in progress"),
            "got: {err:#}"
        );

        drop(guard);
        SyncLockGuard::acquire(tmp.path(), LABELS)
            .expect("the lock is free once the guard is dropped");
    }

    /// Regression (#1010): the old protocol read ownership out of the file, so
    /// a lock held by a live process that had not yet written its pid looked
    /// unowned and was evicted.
    #[test]
    fn a_held_lock_without_a_written_pid_is_never_evicted() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_path = tmp.path().join("sync.lock");
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        holder.try_lock_exclusive().unwrap();

        let err = SyncLockGuard::acquire(tmp.path(), LABELS).unwrap_err();
        assert!(
            err.to_string().contains("Another sync is in progress"),
            "a held lock must never be evicted, got: {err:#}"
        );

        FileExt::unlock(&holder).unwrap();
    }

    /// A lock file left behind by a crashed run must not block anyone: the
    /// kernel drops the lock when its owner dies, so the leftover bytes are
    /// inert. Pid 0 is reserved and never a live user-space process.
    #[test]
    fn a_lock_file_nobody_holds_is_taken_over() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_path = tmp.path().join("sync.lock");
        std::fs::write(&lock_path, "0 1\n").unwrap();

        drop(SyncLockGuard::acquire(tmp.path(), LABELS).unwrap());

        // Read back only once the guard is gone: Windows refuses reads of a
        // range another handle has locked.
        assert!(lock_path.exists(), "the lock file outlives the guard");
        assert_eq!(read_sync_lock(&lock_path).unwrap().0, std::process::id());
    }
}
