//! Cross-platform "is this PID still running" probe.
//!
//! The Antigravity and Trae sync locks both record the owning PID and evict a
//! lock only once that owner is provably dead. A wrong answer in the
//! permissive direction lets two syncs run against the same manifest and
//! delete each other's session artifacts, so every uncertain result here
//! resolves to "alive" and merely defers a sync.
//!
//! Each platform backend answers with a [`Probe`] rather than a bool, and only
//! `Probe::Absent` — the OS positively reporting an unused PID — releases a
//! lock. Routing every backend through one verdict is what stops the
//! fail-alive policy from being re-decided, differently, inside each `cfg`
//! block.
//!
//! The classification is split from the FFI so it can be unit-tested
//! off-platform. CI runs no Windows job, so a Windows-only `pid_is_alive`
//! would otherwise ship with its policy never executed; the pure classifiers
//! below are compiled and asserted on every host that runs the suite.
//!
//! The two callers previously carried separate copies of this probe that had
//! already drifted — Antigravity's comment defended the no-overlap invariant
//! while Trae's waived it — and both were Unix-only. One module keeps the
//! policy in a single place.

/// Whether `pid` names a process that is currently running.
///
/// Returns `true` whenever liveness cannot be established. A false "dead"
/// answer costs correctness (two syncs overlap and corrupt the manifest); a
/// false "alive" answer only postpones a sync, which the caller reports and
/// the next run retries.
pub fn pid_is_alive(pid: u32) -> bool {
    // PID 0 is the kernel/idle process on every platform tokscale ships to,
    // and is also what a truncated or zero-filled lock file parses to, so it
    // never denotes a live sync.
    if pid == 0 {
        return false;
    }
    imp::pid_is_alive(pid)
}

/// What the OS said about a PID, in the only three categories the lock policy
/// distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Probe {
    /// The OS confirmed something holds this PID.
    Exists,
    /// The OS confirmed nothing holds this PID. The sole verdict that evicts a
    /// lock, so a backend may only return it for a code meaning exactly "no
    /// such process" — never as a catch-all for "the call failed".
    Absent,
    /// The probe failed for a reason that says nothing about the PID: out of
    /// memory, handle exhaustion, a permission wall, an unrecognised errno.
    Unknown,
}

impl Probe {
    fn is_alive(self) -> bool {
        !matches!(self, Probe::Absent)
    }
}

#[cfg(unix)]
mod imp {
    use super::Probe;

    /// `errno` values for `kill`, identical across the Unix targets tokscale
    /// builds.
    const EPERM: i32 = 1;
    const ESRCH: i32 = 3;

    extern "C" {
        #[link_name = "kill"]
        fn libc_kill(pid: i32, sig: i32) -> i32;
    }

    /// Classify a `kill(pid, 0)` return value plus `errno`.
    ///
    /// Split out from the FFI so the policy is asserted directly rather than
    /// only through whatever the host happens to report.
    fn classify_kill(result: i32, raw_os_error: Option<i32>) -> Probe {
        if result == 0 {
            return Probe::Exists;
        }
        match raw_os_error {
            // EPERM proves the process exists; it only says we may not signal
            // it.
            Some(EPERM) => Probe::Exists,
            Some(ESRCH) => Probe::Absent,
            // POSIX defines no other failure for signal 0, so anything else is
            // the platform behaving in a way this code has not accounted for.
            // Reading it as death is what would let two syncs overlap.
            _ => Probe::Unknown,
        }
    }

    pub(super) fn pid_is_alive(pid: u32) -> bool {
        // Signal 0 runs `kill`'s existence and permission checks without
        // delivering anything.
        let result = unsafe { libc_kill(pid as i32, 0) };
        classify_kill(result, std::io::Error::last_os_error().raw_os_error()).is_alive()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn kill_success_is_alive() {
            assert_eq!(classify_kill(0, None), Probe::Exists);
        }

        #[test]
        fn eperm_still_proves_existence() {
            assert_eq!(classify_kill(-1, Some(EPERM)), Probe::Exists);
        }

        #[test]
        fn only_esrch_reports_death() {
            assert_eq!(classify_kill(-1, Some(ESRCH)), Probe::Absent);
        }

        /// An errno `kill` is not documented to produce for signal 0 must not
        /// count as proof of death — that is the permissive direction this
        /// module never guesses in.
        #[test]
        fn unrecognised_errno_is_not_death() {
            for errno in [4, 12, 22, 24] {
                let probe = classify_kill(-1, Some(errno));
                assert_eq!(probe, Probe::Unknown, "errno {errno}");
                assert!(probe.is_alive(), "errno {errno}");
            }
            assert!(classify_kill(-1, None).is_alive());
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::windows_policy::{classify_exit_code, classify_open_failure};
    use std::ffi::c_void;

    /// Narrower than `PROCESS_QUERY_INFORMATION`, so the probe still succeeds
    /// against a process running at a higher integrity level instead of
    /// mistaking "may not inspect" for "does not exist".
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;
        fn GetExitCodeProcess(process: *mut c_void, exit_code: *mut u32) -> i32;
        fn CloseHandle(object: *mut c_void) -> i32;
    }

    pub(super) fn pid_is_alive(pid: u32) -> bool {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return classify_open_failure(std::io::Error::last_os_error().raw_os_error())
                .is_alive();
        }

        let mut exit_code: u32 = 0;
        let queried = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
        unsafe {
            CloseHandle(handle);
        }

        classify_exit_code(queried != 0, exit_code).is_alive()
    }
}

/// Compiled on every host under `cfg(test)` so the suite exercises the Windows
/// policy even though CI has no Windows job. Nothing here touches the Win32
/// API, so running it off-platform is meaningful rather than a stub.
#[cfg(any(windows, test))]
mod windows_policy {
    use super::Probe;

    /// What `OpenProcess` reports for a PID no process holds — the only
    /// failure that proves death.
    const ERROR_INVALID_PARAMETER: i32 = 87;
    /// `GetExitCodeProcess` reports this for a running process and,
    /// indistinguishably, for one that exited with 259.
    const STILL_ACTIVE: u32 = 259;

    /// Classify a null `OpenProcess` handle from `GetLastError`.
    ///
    /// Everything except the nonexistent-PID code is `Unknown`. Recognising
    /// `ERROR_ACCESS_DENIED` alone — as an earlier revision did — silently
    /// read handle exhaustion, `ERROR_NOT_ENOUGH_MEMORY` and every unforeseen
    /// failure as proof of death.
    pub(super) fn classify_open_failure(raw_os_error: Option<i32>) -> Probe {
        if raw_os_error == Some(ERROR_INVALID_PARAMETER) {
            Probe::Absent
        } else {
            Probe::Unknown
        }
    }

    /// Classify a `GetExitCodeProcess` result on an open handle.
    ///
    /// A process that exited with 259 reads as alive: that status is
    /// indistinguishable from `STILL_ACTIVE`, and erring toward "alive" is the
    /// direction the lock policy wants.
    pub(super) fn classify_exit_code(queried: bool, exit_code: u32) -> Probe {
        if !queried {
            return Probe::Unknown;
        }
        if exit_code == STILL_ACTIVE {
            return Probe::Exists;
        }
        Probe::Absent
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn only_the_nonexistent_pid_code_reports_death() {
            assert_eq!(
                classify_open_failure(Some(ERROR_INVALID_PARAMETER)),
                Probe::Absent
            );
        }

        /// `ERROR_ACCESS_DENIED` (5) means the process exists but sits in a
        /// context we may not open — the same distinction the Unix branch
        /// draws for `EPERM`.
        #[test]
        fn access_denied_is_alive() {
            assert!(classify_open_failure(Some(5)).is_alive());
        }

        /// The regression this pins: a resource failure is not evidence of
        /// death. `ERROR_NOT_ENOUGH_MEMORY` (8), `ERROR_OUTOFMEMORY` (14),
        /// `ERROR_NO_SYSTEM_RESOURCES` (1450) and anything unmapped must all
        /// keep the lock held.
        #[test]
        fn resource_failures_are_not_death() {
            for code in [8, 14, 1450, 1816] {
                let probe = classify_open_failure(Some(code));
                assert_eq!(probe, Probe::Unknown, "error {code}");
                assert!(probe.is_alive(), "error {code}");
            }
            assert!(classify_open_failure(None).is_alive());
        }

        #[test]
        fn exit_code_query_classifies_liveness() {
            assert_eq!(classify_exit_code(true, STILL_ACTIVE), Probe::Exists);
            assert_eq!(classify_exit_code(true, 0), Probe::Absent);
            assert_eq!(classify_exit_code(true, 1), Probe::Absent);
        }

        /// A failed query leaves liveness unknown, so report alive rather than
        /// hand the lock to a second sync on a guess.
        #[test]
        fn failed_exit_code_query_is_alive() {
            let probe = classify_exit_code(false, 0);
            assert_eq!(probe, Probe::Unknown);
            assert!(probe.is_alive());
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::Probe;

    pub(super) fn pid_is_alive(_pid: u32) -> bool {
        // No liveness probe on this platform, so nothing here can prove death.
        // Reporting "alive" blocks a sync until the lock file is removed by
        // hand; reporting "dead" would silently allow the overlapping syncs
        // this module exists to prevent. Unreachable for every target
        // tokscale releases — all of them are Unix or Windows.
        Probe::Unknown.is_alive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn pid_zero_is_never_alive() {
        assert!(!pid_is_alive(0));
    }

    /// Runs on Windows too. It was previously `#[cfg(unix)]` in `trae.rs`,
    /// which is precisely why the always-false Windows stub went unnoticed.
    #[test]
    fn current_process_is_alive() {
        assert!(pid_is_alive(std::process::id()));
    }

    #[test]
    fn exited_process_is_not_alive() {
        #[cfg(windows)]
        let mut child = Command::new("cmd")
            .args(["/C", "exit"])
            .spawn()
            .expect("spawn a process that exits immediately");
        #[cfg(unix)]
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .expect("spawn a process that exits immediately");

        let pid = child.id();
        child.wait().expect("child exits");

        // `child` is deliberately still in scope: on Windows an open process
        // handle pins the PID, so it cannot be recycled by an unrelated
        // process between the wait and the probe.
        assert!(!pid_is_alive(pid));
        drop(child);
    }
}
