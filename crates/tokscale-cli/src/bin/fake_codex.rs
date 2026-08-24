//! A stand-in for the `codex` binary, used by the `headless_capture_*` tests in
//! `tests/cli_tests.rs`.
//!
//! Those tests assert three things about `tokscale headless <command>`: that the
//! child's exit code is passed through verbatim, that a fast-exiting child is
//! not waited on until the timeout, and that a hung child is killed and reported
//! as 124. None of that is platform-specific — `run_capture_command` is ordinary
//! `std::process` code — so the coverage is worth having everywhere.
//!
//! The fixture used to be a `#!/bin/sh` script written at test time, which is
//! the one part that could not cross platforms: Windows has no shebang, and
//! `Command::new("codex")` there resolves a bare program name by appending
//! `.exe` only, so an extensionless file is never even probed. Building the
//! stand-in as a real binary lets cargo produce a native executable on each
//! platform, and the test copies it onto PATH under the name the production code
//! looks for.
//!
//! Behaviour is selected with `TOKSCALE_FAKE_CODEX_MODE` and must match the sh
//! script it replaces, in particular writing `captured ok` / `captured fail`
//! with **no trailing newline** — the tests compare stdout byte for byte.
//!
//! In `slow` mode the sleep must outlast the parent's
//! `TOKSCALE_NATIVE_TIMEOUT_MS`, or the child would exit on its own and the test
//! would stop testing the timeout at all. It has to outlast it by a wide margin,
//! not merely exceed it: the test's upper bound sits between the parent's
//! deadline and this sleep, and every second of that gap it does not use is
//! runner noise it cannot absorb. The 20s sleep this replaces left an 8s gap
//! against a 10s deadline, and CI overshot it (run 31196968982). See
//! `FAKE_CODEX_SLOW_SLEEP_SECS` below and the comment on
//! `headless_capture_slow_command_times_out` in `tests/cli_tests.rs`.

use std::io::Write;

/// How long `slow` mode sleeps.
///
/// Twelve times the 10s `TOKSCALE_NATIVE_TIMEOUT_MS` the `slow` test sets, so
/// "the parent killed the child at its deadline" (~10s) and "the parent outlived
/// the child" (~120s) are far enough apart that no plausible runner slowness can
/// turn one into the other. The parent always kills this process; reaching the
/// end of the sleep is not expected, and only happens when the behaviour under
/// test is broken — in which case the test should, and does, take that long to
/// say so.
const FAKE_CODEX_SLOW_SLEEP_SECS: u64 = 120;

fn emit(text: &str) {
    let mut stdout = std::io::stdout();
    // `print!` alone would leave this in the buffer, and `process::exit` does
    // not run stdout's flush-on-drop.
    write!(stdout, "{text}").expect("failed to write to stdout");
    stdout.flush().expect("failed to flush stdout");
}

/// Spawns a grandchild that inherits stdout and outlives this process, so the
/// write end of the parent's pipe stays open after this process is gone.
///
/// Deliberately never waited on -- reaping it would close the write end and
/// destroy the condition under test. The PID is published to
/// `TOKSCALE_FAKE_CODEX_PIDFILE` when set so the test can reap it in teardown
/// rather than leaving it to age out on its own.
fn spawn_stdout_holder() {
    let exe = std::env::current_exe().expect("current_exe");
    #[allow(clippy::zombie_processes)]
    let holder = std::process::Command::new(exe)
        .env("TOKSCALE_FAKE_CODEX_MODE", "slow")
        .env_remove("TOKSCALE_FAKE_CODEX_PIDFILE")
        .stdout(std::process::Stdio::inherit())
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn stdout holder");

    if let Ok(path) = std::env::var("TOKSCALE_FAKE_CODEX_PIDFILE") {
        let _ = std::fs::write(path, holder.id().to_string());
    }
}

fn main() {
    let mode = std::env::var("TOKSCALE_FAKE_CODEX_MODE").unwrap_or_default();

    match mode.as_str() {
        "success" => emit("captured ok"),
        "args" => emit(&std::env::args().skip(1).collect::<Vec<_>>().join("\n")),
        "fail" => {
            emit("captured fail");
            std::process::exit(17);
        }
        // Far longer than the `TOKSCALE_NATIVE_TIMEOUT_MS` the `slow` test sets,
        // so the parent's timeout always wins by a margin the runner cannot eat
        // into. See `FAKE_CODEX_SLOW_SLEEP_SECS`.
        "slow" => std::thread::sleep(std::time::Duration::from_secs(FAKE_CODEX_SLOW_SLEEP_SECS)),
        // #1049's shape. A grandchild inherits the stdout pipe and outlives the
        // direct child, so killing the child does not close the write end and
        // the parent's pump never reaches EOF. The grandchild must outlast the
        // test's own upper bound, not merely the parent's deadline: a parent
        // that waits for EOF unboundedly has to look *slower than the bound*,
        // or the test cannot tell it apart from a parent that drained promptly.
        // The direct child is killed at the deadline while the holder keeps the
        // pipe open: the timeout branch of the drain.
        "descendant" => {
            spawn_stdout_holder();
            std::thread::sleep(std::time::Duration::from_secs(FAKE_CODEX_SLOW_SLEEP_SECS));
        }
        // The direct child exits *successfully and immediately* while the holder
        // keeps the pipe open, so `try_wait` observes a normal exit and
        // `timed_out` stays false: the non-timeout branch of the drain. The
        // parent has no deadline to fall back on here, which is what makes this
        // shape distinct from `descendant`.
        "descendant-exit" => {
            spawn_stdout_holder();
            emit("captured partial");
        }
        _ => {
            eprintln!("unknown TOKSCALE_FAKE_CODEX_MODE");
            std::process::exit(2);
        }
    }
}
