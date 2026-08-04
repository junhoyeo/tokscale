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

use std::io::Write;

fn emit(text: &str) {
    let mut stdout = std::io::stdout();
    // `print!` alone would leave this in the buffer, and `process::exit` does
    // not run stdout's flush-on-drop.
    write!(stdout, "{text}").expect("failed to write to stdout");
    stdout.flush().expect("failed to flush stdout");
}

fn main() {
    let mode = std::env::var("TOKSCALE_FAKE_CODEX_MODE").unwrap_or_default();

    match mode.as_str() {
        "success" => emit("captured ok"),
        "fail" => {
            emit("captured fail");
            std::process::exit(17);
        }
        // Longer than the 10s `TOKSCALE_NATIVE_TIMEOUT_MS` the tests set, so the
        // parent's timeout always wins. The parent kills this process; it is not
        // expected to reach the end of the sleep.
        "slow" => std::thread::sleep(std::time::Duration::from_secs(20)),
        _ => {
            eprintln!("unknown TOKSCALE_FAKE_CODEX_MODE");
            std::process::exit(2);
        }
    }
}
