#![cfg(target_os = "macos")]

fn probe(case: &str) {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/helpers/tui_startup_probe.py");
    let output = std::process::Command::new("python3")
        .arg("-I")
        .arg(script)
        .arg("--binary")
        .arg(env!("CARGO_BIN_EXE_tokscale"))
        .arg("--case")
        .arg(case)
        .output()
        .expect("start isolated PTY probe");
    assert!(
        output.status.success(),
        "case={case}\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn offline_startup_renders_and_quits_without_keychain() {
    probe("offline-empty");
}

#[test]
fn local_tab_startup_does_not_query_keychain() {
    probe("online-local");
}

#[test]
fn usage_keychain_wait_does_not_block_quit() {
    probe("online-usage-blocked");
}

#[test]
fn offline_startup_does_not_show_unverified_claude_cache() {
    probe("offline-untrusted-cache");
}
