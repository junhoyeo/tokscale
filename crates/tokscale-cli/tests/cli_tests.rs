use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

// ── Fixture helpers ────────────────────────────────────────────────────────

fn prime_pricing_cache(base: &Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs();
    let payload = format!(r#"{{"timestamp":{},"data":{{}}}}"#, now);

    for dir in [
        base.join("Library/Caches/tokscale"),
        base.join(".cache/tokscale"),
        base.join(".config/tokscale/cache"),
    ] {
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("pricing-litellm.json"), &payload).unwrap();
        fs::write(dir.join("pricing-openrouter.json"), &payload).unwrap();
    }
}

fn prime_override_pricing_cache(config_dir: &Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs();
    let payload = format!(r#"{{"timestamp":{},"data":{{}}}}"#, now);

    let cache_dir = config_dir.join("cache");
    fs::create_dir_all(&cache_dir).unwrap();
    fs::write(cache_dir.join("pricing-litellm.json"), &payload).unwrap();
    fs::write(cache_dir.join("pricing-openrouter.json"), &payload).unwrap();
}

/// Create a temporary directory with minimal OpenCode fixture data.
///
/// Layout:
///   <tmp>/.local/share/opencode/storage/message/session1/msg_a.json  (2024-06-15, claude-sonnet-4-20250514, anthropic)
///   <tmp>/.local/share/opencode/storage/message/session1/msg_b.json  (2024-06-15, claude-sonnet-4-20250514, anthropic)
///   <tmp>/.local/share/opencode/storage/message/session2/msg_c.json  (2025-01-10, gpt-4o, openai)
fn create_temp_fixture_dir_with_pricing_cache(with_pricing_cache: bool) -> TempDir {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();
    if with_pricing_cache {
        prime_pricing_cache(base);
    }

    // Session 1: two messages on 2024-06-15 using claude-sonnet-4
    let session1 = base.join(".local/share/opencode/storage/message/session1");
    fs::create_dir_all(&session1).unwrap();

    // 2024-06-15 12:00:00 UTC = 1718452800000 ms
    let msg_a = r#"{
        "id": "msg_a",
        "sessionID": "session1",
        "role": "assistant",
        "modelID": "claude-sonnet-4-20250514",
        "providerID": "anthropic",
        "cost": 0.05,
        "tokens": {
            "input": 1000,
            "output": 500,
            "reasoning": 0,
            "cache": { "read": 200, "write": 50 }
        },
        "time": { "created": 1718452800000.0 }
    }"#;
    fs::write(session1.join("msg_a.json"), msg_a).unwrap();

    // Same session, a bit later on the same day
    let msg_b = r#"{
        "id": "msg_b",
        "sessionID": "session1",
        "role": "assistant",
        "modelID": "claude-sonnet-4-20250514",
        "providerID": "anthropic",
        "cost": 0.03,
        "tokens": {
            "input": 800,
            "output": 300,
            "reasoning": 0,
            "cache": { "read": 150, "write": 30 }
        },
        "time": { "created": 1718456400000.0 }
    }"#;
    fs::write(session1.join("msg_b.json"), msg_b).unwrap();

    // Session 2: one message on 2025-01-10 using gpt-4o
    let session2 = base.join(".local/share/opencode/storage/message/session2");
    fs::create_dir_all(&session2).unwrap();

    // 2025-01-10 12:00:00 UTC = 1736510400000 ms
    let msg_c = r#"{
        "id": "msg_c",
        "sessionID": "session2",
        "role": "assistant",
        "modelID": "gpt-4o",
        "providerID": "openai",
        "cost": 0.02,
        "tokens": {
            "input": 600,
            "output": 200,
            "reasoning": 0,
            "cache": { "read": 100, "write": 20 }
        },
        "time": { "created": 1736510400000.0 }
    }"#;
    fs::write(session2.join("msg_c.json"), msg_c).unwrap();

    tmp
}

fn create_temp_fixture_dir() -> TempDir {
    create_temp_fixture_dir_with_pricing_cache(true)
}

fn create_temp_fixture_dir_without_pricing_cache() -> TempDir {
    create_temp_fixture_dir_with_pricing_cache(false)
}

/// Create an empty fixture dir with no session data.
fn create_empty_fixture_dir() -> TempDir {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();
    prime_pricing_cache(base);
    let opencode_dir = base.join(".local/share/opencode/storage/message");
    fs::create_dir_all(opencode_dir).unwrap();
    tmp
}

fn create_timezone_boundary_fixture_dir() -> TempDir {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();
    prime_pricing_cache(base);

    let session = base.join(".local/share/opencode/storage/message/session1");
    fs::create_dir_all(&session).unwrap();

    // 2026-03-02 18:00:00 UTC = 2026-03-02 10:00:00 in America/Los_Angeles
    let msg_a = r#"{
        "id": "msg_a",
        "sessionID": "session1",
        "role": "assistant",
        "modelID": "claude-sonnet-4-20250514",
        "providerID": "anthropic",
        "cost": 0.05,
        "tokens": {
            "input": 1000,
            "output": 500,
            "reasoning": 0,
            "cache": { "read": 200, "write": 50 }
        },
        "time": { "created": 1772474400000.0 }
    }"#;
    fs::write(session.join("msg_a.json"), msg_a).unwrap();

    // 2026-03-03 04:30:00 UTC = 2026-03-02 20:30:00 in America/Los_Angeles
    let msg_b = r#"{
        "id": "msg_b",
        "sessionID": "session1",
        "role": "assistant",
        "modelID": "claude-sonnet-4-20250514",
        "providerID": "anthropic",
        "cost": 0.03,
        "tokens": {
            "input": 800,
            "output": 300,
            "reasoning": 0,
            "cache": { "read": 150, "write": 30 }
        },
        "time": { "created": 1772512200000.0 }
    }"#;
    fs::write(session.join("msg_b.json"), msg_b).unwrap();

    tmp
}

fn create_qwen_workspace_fixture_dir() -> TempDir {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();
    prime_pricing_cache(base);

    let session = base.join(".qwen/projects/demo-workspace/chats");
    fs::create_dir_all(&session).unwrap();

    let msg = r#"{"type":"assistant","model":"qwen3.5-plus","timestamp":"2026-02-23T14:24:56.857Z","sessionId":"demo-session","usageMetadata":{"promptTokenCount":12414,"candidatesTokenCount":76,"thoughtsTokenCount":39,"cachedContentTokenCount":0}}"#;
    fs::write(session.join("session-1.jsonl"), msg).unwrap();

    tmp
}

fn create_codex_fixture_dir() -> TempDir {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();
    prime_pricing_cache(base);

    let sessions_dir = base.join(".codex/sessions");
    fs::create_dir_all(&sessions_dir).unwrap();
    fs::write(
        sessions_dir.join("session-1.jsonl"),
        concat!(
            r#"{"type":"turn_context","payload":{"model":"gpt-4o-mini"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":120,"cached_input_tokens":20,"output_tokens":30}}}}"#,
            "\n"
        ),
    )
    .unwrap();

    tmp
}

fn create_conflicting_opencode_fixture_dir() -> TempDir {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();
    prime_pricing_cache(base);

    let session = base.join(".local/share/opencode/storage/message/conflicting-session");
    fs::create_dir_all(&session).unwrap();

    let msg = r#"{
        "id": "conflict_msg",
        "sessionID": "conflicting-session",
        "role": "assistant",
        "modelID": "gemini-2.5-pro",
        "providerID": "google",
        "cost": 0.11,
        "tokens": {
            "input": 111,
            "output": 222,
            "reasoning": 0,
            "cache": { "read": 0, "write": 0 }
        },
        "time": { "created": 1736510400000.0 }
    }"#;
    fs::write(session.join("conflict_msg.json"), msg).unwrap();

    tmp
}

fn create_conflicting_codex_fixture_dir() -> TempDir {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();
    prime_pricing_cache(base);

    let sessions_dir = base.join(".codex/sessions");
    fs::create_dir_all(&sessions_dir).unwrap();
    fs::write(
        sessions_dir.join("conflicting-session.jsonl"),
        concat!(
            r#"{"type":"turn_context","payload":{"model":"gpt-5"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":900,"cached_input_tokens":90,"output_tokens":45}}}}"#,
            "\n"
        ),
    )
    .unwrap();

    tmp
}

/// Build a Command pointing HOME at the given temp dir, with --no-spinner and --opencode flags.
fn cmd_with_home(tmp: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.env("HOME", tmp)
        .env("XDG_CONFIG_HOME", tmp.join(".config"))
        .env("XDG_DATA_HOME", tmp.join(".local/share"))
        .env("XDG_CACHE_HOME", tmp.join(".cache"))
        .env("TOKSCALE_PRICING_CACHE_ONLY", "1");
    cmd
}

fn cmd_with_conflicting_env(tmp: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.env("HOME", tmp)
        .env("XDG_CONFIG_HOME", tmp.join(".config"))
        .env("XDG_DATA_HOME", tmp.join(".local/share"))
        .env("XDG_CACHE_HOME", tmp.join(".cache"));
    cmd
}

fn offline_cmd_with_home(tmp: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("tokscale");
    // Pin every XDG_* var so the cache resolvers stay inside the sandbox.
    // Without XDG_CONFIG_HOME the post-#470 cache root can leak to the
    // host's $XDG_CONFIG_HOME (set globally on some CI runners) and
    // either find pricing data outside the fixture or write to the
    // host filesystem. Mirrors what cmd_with_home does.
    cmd.env("HOME", tmp)
        .env("XDG_CONFIG_HOME", tmp.join(".config"))
        .env("XDG_DATA_HOME", tmp.join(".local/share"))
        .env("XDG_CACHE_HOME", tmp.join(".cache"))
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9");
    cmd
}

fn write_pricing_cache(base: &Path, timestamp: u64) {
    let litellm = format!(
        r#"{{"timestamp":{},"data":{{"gpt-4o":{{"input_cost_per_token":0.0000025,"output_cost_per_token":0.00001}},"claude-sonnet-4-20250514":{{"input_cost_per_token":0.000003,"output_cost_per_token":0.000015}}}}}}"#,
        timestamp
    );
    let openrouter = format!(r#"{{"timestamp":{},"data":{{}}}}"#, timestamp);

    // Seed all three locations so the test exercises the same fallback
    // chain the binary uses post-#470: canonical
    // <config_dir>/cache/, then legacy dirs::cache_dir()/tokscale, then
    // ~/.cache/tokscale. Without the canonical path seeded, CI runners
    // where dirs::cache_dir() resolves outside the sandboxed HOME (e.g.
    // some Linux runners with XDG_CACHE_HOME set globally) miss the
    // pricing cache entirely and the report falls back to embedded
    // source costs.
    for dir in [
        base.join(".config/tokscale/cache"),
        base.join("Library/Caches/tokscale"),
        base.join(".cache/tokscale"),
    ] {
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("pricing-litellm.json"), &litellm).unwrap();
        fs::write(dir.join("pricing-openrouter.json"), &openrouter).unwrap();
    }
}

fn write_fake_credentials(base: &Path) {
    let creds_dir = base.join(".config/tokscale");
    fs::create_dir_all(&creds_dir).unwrap();
    fs::write(
        creds_dir.join("credentials.json"),
        r#"{"token":"fake","username":"testuser","createdAt":"2024-01-01T00:00:00Z"}"#,
    )
    .unwrap();
}

fn write_settings_json(base: &Path, body: &str) {
    for dir in [
        base.join(".config/tokscale"),
        base.join("Library/Application Support/tokscale"),
    ] {
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.json"), body).unwrap();
    }
}

// ── Existing tests ─────────────────────────────────────────────────────────

#[test]
fn test_help_command() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("AI token usage analytics"));
}

#[test]
fn test_help_short_flag() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("-h")
        .assert()
        .success()
        .stdout(predicate::str::contains("AI token usage analytics"));
}

#[test]
fn test_version_flag() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "tokscale {}",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn test_models_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("models")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Show model usage report"));
}

#[test]
fn test_monthly_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("monthly")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Show monthly usage report"));
}

#[test]
fn test_pricing_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("pricing")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Show pricing for a model"));
}

#[test]
fn test_clients_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("clients")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Show local scan locations"));
}

#[test]
fn test_graph_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("graph")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Export contribution graph data"));
}

#[test]
fn test_tui_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("tui")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Launch interactive TUI"));
}

#[test]
fn test_headless_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("headless")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Capture subprocess output"));
}

#[test]
fn test_login_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("login")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Login to Tokscale"));
}

#[test]
fn test_logout_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("logout")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Logout from Tokscale"));
}

#[test]
fn test_whoami_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("whoami")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Show current logged in user"));
}

#[test]
fn test_invalid_command() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("invalid-command").assert().failure();
}

#[test]
fn test_invalid_subcommand() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("models").arg("invalid-flag").assert().failure();
}

#[test]
fn test_pricing_command_missing_model() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("pricing").assert().failure();
}

#[test]
fn test_headless_command_missing_client() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("headless").assert().failure();
}

#[test]
fn test_headless_command_invalid_client() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("headless")
        .arg("invalid-client")
        .arg("test")
        .assert()
        .failure();
}

#[test]
fn test_models_with_invalid_date_format() {
    let tmp = create_empty_fixture_dir();
    cmd_with_home(tmp.path())
        .arg("models")
        .arg("--light")
        .arg("--opencode")
        .arg("--no-spinner")
        .arg("--since")
        .arg("invalid-date")
        .assert()
        .success();
}

#[test]
fn test_models_with_invalid_year() {
    let tmp = create_empty_fixture_dir();
    cmd_with_home(tmp.path())
        .arg("models")
        .arg("--light")
        .arg("--opencode")
        .arg("--no-spinner")
        .arg("--year")
        .arg("not-a-year")
        .assert()
        .success();
}

#[test]
fn test_global_theme_flag() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("--theme")
        .arg("blue")
        .arg("--help")
        .assert()
        .success();
}

#[test]
fn test_global_debug_flag() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("--debug").arg("--help").assert().success();
}

// ── Date filtering tests ───────────────────────────────────────────────────

#[test]
fn test_models_with_since_until_filter() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .args(["--since", "2024-06-01", "--until", "2024-06-30"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-sonnet-4"))
        .stdout(predicate::str::contains("gpt-4o").not());
}

#[test]
fn test_models_with_year_filter() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .args(["--year", "2024"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-sonnet-4"))
        .stdout(predicate::str::contains("gpt-4o").not());
}

#[test]
fn test_monthly_with_date_filters() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["monthly", "--json", "--opencode", "--no-spinner"])
        .args(["--since", "2025-01-01", "--until", "2025-12-31"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2025-01"));
}

#[test]
fn test_models_home_override_ignores_conflicting_xdg_env() {
    let real_home = create_temp_fixture_dir();
    let conflicting_home = create_conflicting_opencode_fixture_dir();

    let output = cmd_with_conflicting_env(conflicting_home.path())
        .args([
            "models",
            "--json",
            "--opencode",
            "--no-spinner",
            "--home",
            real_home.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["totalMessages"].as_i64().unwrap(), 3);
    assert_eq!(json["totalInput"].as_i64().unwrap(), 2400);
    assert_eq!(json["totalOutput"].as_i64().unwrap(), 1000);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("gemini-2.5-pro"));
}

#[test]
fn test_monthly_home_override_ignores_conflicting_xdg_env() {
    let real_home = create_temp_fixture_dir();
    let conflicting_home = create_conflicting_opencode_fixture_dir();

    let output = cmd_with_conflicting_env(conflicting_home.path())
        .args([
            "monthly",
            "--json",
            "--opencode",
            "--no-spinner",
            "--home",
            real_home.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|entry| entry["month"] == "2024-06"));
    assert!(entries.iter().any(|entry| entry["month"] == "2025-01"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("gemini-2.5-pro"));
}

#[test]
fn test_graph_home_override_ignores_conflicting_xdg_env() {
    let real_home = create_temp_fixture_dir();
    let conflicting_home = create_conflicting_opencode_fixture_dir();

    let output = cmd_with_conflicting_env(conflicting_home.path())
        .args([
            "graph",
            "--opencode",
            "--no-spinner",
            "--home",
            real_home.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let contributions = json["contributions"].as_array().unwrap();
    assert_eq!(contributions.len(), 2);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("gemini-2.5-pro"));
}

#[test]
fn test_models_home_override_ignores_conflicting_codex_home_env() {
    let real_home = create_codex_fixture_dir();
    let conflicting_home = create_conflicting_codex_fixture_dir();

    let output = cmd_with_conflicting_env(conflicting_home.path())
        .env("CODEX_HOME", conflicting_home.path().join(".codex"))
        .args([
            "models",
            "--json",
            "--codex",
            "--no-spinner",
            "--home",
            real_home.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["totalMessages"].as_i64().unwrap(), 1);
    assert_eq!(json["totalInput"].as_i64().unwrap(), 100);
    assert_eq!(json["totalOutput"].as_i64().unwrap(), 30);
    assert_eq!(json["totalCacheRead"].as_i64().unwrap(), 20);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("\"gpt-5\""));
}

#[test]
fn test_tui_rejects_home_override() {
    let tmp = TempDir::new().unwrap();

    cargo_bin_cmd!("tokscale")
        .args(["--home", tmp.path().to_str().unwrap(), "tui"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--home is currently supported for local report commands only",
        ));
}

#[test]
fn test_clients_rejects_home_override() {
    let tmp = TempDir::new().unwrap();

    cargo_bin_cmd!("tokscale")
        .args(["--home", tmp.path().to_str().unwrap(), "clients"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "It is not supported for `clients`",
        ));
}

#[test]
fn test_models_with_since_only() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .args(["--since", "2025-01-01"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gpt-4o"))
        .stdout(predicate::str::contains("anthropic").not());
}

#[test]
fn test_models_with_until_only() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .args(["--until", "2024-12-31"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-sonnet-4"))
        .stdout(predicate::str::contains("gpt-4o").not());
}

#[test]
fn test_models_with_no_matching_date() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .args(["--since", "2099-01-01", "--until", "2099-12-31"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"].as_array().unwrap();
    assert!(
        entries.is_empty(),
        "No entries expected for future date range"
    );
}

#[test]
fn test_graph_single_day_filter_uses_local_timezone_boundaries() {
    let tmp = create_timezone_boundary_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .env("TZ", "America/Los_Angeles")
        .args(["graph", "--opencode", "--no-spinner"])
        .args(["--since", "2026-03-02", "--until", "2026-03-02"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let contributions = json["contributions"].as_array().unwrap();
    assert_eq!(
        contributions.len(),
        1,
        "expected a single local-day bucket, got {:?}",
        contributions
    );
    assert_eq!(contributions[0]["date"].as_str().unwrap(), "2026-03-02");
    assert_eq!(contributions[0]["totals"]["messages"].as_i64().unwrap(), 2);
}

#[test]
fn test_graph_with_year_filter() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["graph", "--opencode", "--no-spinner"])
        .args(["--year", "2024"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let contributions = json["contributions"].as_array().unwrap();
    for c in contributions {
        let date = c["date"].as_str().unwrap();
        assert!(
            date.starts_with("2024-"),
            "Expected 2024 dates, got {}",
            date
        );
    }
}

// ── Client filtering tests ─────────────────────────────────────────────────

#[test]
fn test_models_with_client_filter_opencode() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"].as_array().unwrap();
    for entry in entries {
        assert_eq!(entry["client"].as_str().unwrap(), "opencode");
    }
}

#[test]
fn test_models_with_client_filter_multiple() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--claude", "--no-spinner"])
        .assert()
        .success();
}

#[test]
fn test_models_with_all_client_flags() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args([
            "models",
            "--json",
            "--no-spinner",
            "--opencode",
            "--claude",
            "--codex",
            "--gemini",
            "--cursor",
            "--amp",
            "--droid",
            "--openclaw",
            "--pi",
        ])
        .assert()
        .success();
}

#[test]
fn test_models_client_and_date_combined() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .args(["--year", "2025"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gpt-4o"))
        .stdout(predicate::str::contains("anthropic").not());
}

// ── JSON output validation tests ───────────────────────────────────────────

#[test]
fn test_models_json_output() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(json.get("groupBy").is_some(), "Missing groupBy field");
    assert!(json.get("entries").is_some(), "Missing entries field");
    assert!(json.get("totalInput").is_some(), "Missing totalInput");
    assert!(json.get("totalOutput").is_some(), "Missing totalOutput");
    assert!(
        json.get("totalCacheRead").is_some(),
        "Missing totalCacheRead"
    );
    assert!(
        json.get("totalCacheWrite").is_some(),
        "Missing totalCacheWrite"
    );
    assert!(json.get("totalMessages").is_some(), "Missing totalMessages");
    assert!(json.get("totalCost").is_some(), "Missing totalCost");
    assert!(
        json.get("processingTimeMs").is_some(),
        "Missing processingTimeMs"
    );

    let entries = json["entries"].as_array().unwrap();
    assert!(!entries.is_empty(), "Should have entries from fixture data");
    let first = &entries[0];
    assert!(first.get("client").is_some());
    assert!(first.get("model").is_some());
    assert!(first.get("provider").is_some());
    assert!(first.get("input").is_some());
    assert!(first.get("output").is_some());
    assert!(first.get("cacheRead").is_some());
    assert!(first.get("cacheWrite").is_some());
    assert!(first.get("cost").is_some());
}

#[test]
fn test_models_json_offline_without_pricing_cache_still_succeeds() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    let output = offline_cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["totalInput"].as_i64().unwrap(), 2400);
    assert_eq!(json["totalOutput"].as_i64().unwrap(), 1000);
    assert_eq!(json["totalMessages"].as_i64().unwrap(), 3);
    assert_eq!(json["entries"].as_array().unwrap().len(), 2);
    // Without pricing, embedded source costs are preserved (0.05 + 0.03 + 0.02)
    let total_cost = json["totalCost"].as_f64().unwrap();
    assert!(
        (total_cost - 0.10).abs() < 1e-9,
        "unexpected totalCost without pricing: {total_cost}"
    );
}

#[test]
fn test_monthly_json_offline_without_pricing_cache_still_succeeds() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    let output = offline_cmd_with_home(tmp.path())
        .args(["monthly", "--json", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["month"].as_str().unwrap(), "2024-06");
    assert_eq!(entries[1]["month"].as_str().unwrap(), "2025-01");
    // Without pricing, embedded source costs are preserved
    let total_cost = json["totalCost"].as_f64().unwrap();
    assert!(
        (total_cost - 0.10).abs() < 1e-9,
        "unexpected totalCost without pricing: {total_cost}"
    );
}

#[test]
fn test_graph_offline_without_pricing_cache_still_succeeds() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    let output = offline_cmd_with_home(tmp.path())
        .args(["graph", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["summary"]["totalTokens"].as_i64().unwrap(), 3950);
    assert_eq!(json["summary"]["activeDays"].as_i64().unwrap(), 2);
    assert_eq!(json["contributions"].as_array().unwrap().len(), 2);
    // Without pricing, embedded source costs are preserved
    let total_cost = json["summary"]["totalCost"].as_f64().unwrap();
    assert!(
        (total_cost - 0.10).abs() < 1e-9,
        "unexpected totalCost without pricing: {total_cost}"
    );
}

#[test]
fn test_models_json_offline_uses_stale_pricing_cache_when_available() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    write_pricing_cache(tmp.path(), 1);

    let output = offline_cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let total_cost = json["totalCost"].as_f64().unwrap();
    assert!(
        (total_cost - 0.0209).abs() < 1e-9,
        "unexpected totalCost: {total_cost}"
    );
}

#[test]
fn test_monthly_json_offline_uses_stale_pricing_cache_when_available() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    write_pricing_cache(tmp.path(), 1);

    let output = offline_cmd_with_home(tmp.path())
        .args(["monthly", "--json", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let total_cost = json["totalCost"].as_f64().unwrap();
    assert!(
        (total_cost - 0.0209).abs() < 1e-9,
        "unexpected totalCost: {total_cost}"
    );
}

#[test]
fn test_graph_offline_uses_stale_pricing_cache_when_available() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    write_pricing_cache(tmp.path(), 1);

    let output = offline_cmd_with_home(tmp.path())
        .args(["graph", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let total_cost = json["summary"]["totalCost"].as_f64().unwrap();
    assert!(
        (total_cost - 0.0209).abs() < 1e-9,
        "unexpected totalCost: {total_cost}"
    );
}

#[test]
fn test_models_json_total_consistency() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let entries = json["entries"].as_array().unwrap();
    let sum_input: i64 = entries.iter().map(|e| e["input"].as_i64().unwrap()).sum();
    let sum_output: i64 = entries.iter().map(|e| e["output"].as_i64().unwrap()).sum();
    let total_input = json["totalInput"].as_i64().unwrap();
    let total_output = json["totalOutput"].as_i64().unwrap();

    assert_eq!(
        sum_input, total_input,
        "Sum of entry inputs must match totalInput"
    );
    assert_eq!(
        sum_output, total_output,
        "Sum of entry outputs must match totalOutput"
    );
}

#[test]
fn test_monthly_json_output() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["monthly", "--json", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(json.get("entries").is_some(), "Missing entries field");
    assert!(json.get("totalCost").is_some(), "Missing totalCost field");
    assert!(
        json.get("processingTimeMs").is_some(),
        "Missing processingTimeMs"
    );

    let entries = json["entries"].as_array().unwrap();
    assert!(
        !entries.is_empty(),
        "Should have monthly entries from fixture data"
    );
    let first = &entries[0];
    assert!(first.get("month").is_some());
    assert!(first.get("models").is_some());
    assert!(first.get("input").is_some());
    assert!(first.get("output").is_some());
    assert!(first.get("cacheRead").is_some());
    assert!(first.get("cacheWrite").is_some());
    assert!(first.get("messageCount").is_some());
    assert!(first.get("cost").is_some());
}

#[test]
fn test_monthly_json_with_client_filter() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["monthly", "--json", "--opencode", "--no-spinner"])
        .args(["--year", "2024"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"].as_array().unwrap();
    for entry in entries {
        let month = entry["month"].as_str().unwrap();
        assert!(
            month.starts_with("2024-"),
            "Expected 2024 months only, got {}",
            month
        );
    }
}

#[test]
fn test_graph_json_output() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["graph", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(json.get("meta").is_some(), "Missing meta field");
    assert!(json.get("summary").is_some(), "Missing summary field");
    assert!(json.get("years").is_some(), "Missing years field");
    assert!(
        json.get("contributions").is_some(),
        "Missing contributions field"
    );
}

#[test]
fn test_graph_json_has_meta() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["graph", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let meta = &json["meta"];
    assert!(
        meta.get("generatedAt").is_some(),
        "Missing meta.generatedAt"
    );
    assert!(meta.get("version").is_some(), "Missing meta.version");
    assert!(meta.get("dateRange").is_some(), "Missing meta.dateRange");
}

#[test]
fn test_graph_json_has_summary() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["graph", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let summary = &json["summary"];
    assert!(
        summary.get("totalTokens").is_some(),
        "Missing summary.totalTokens"
    );
    assert!(
        summary.get("totalCost").is_some(),
        "Missing summary.totalCost"
    );
    assert!(
        summary.get("totalDays").is_some(),
        "Missing summary.totalDays"
    );
    assert!(
        summary.get("activeDays").is_some(),
        "Missing summary.activeDays"
    );
    assert!(summary.get("clients").is_some(), "Missing summary.clients");
    assert!(summary.get("models").is_some(), "Missing summary.models");
}

// ── Group-by strategy tests ────────────────────────────────────────────────

#[test]
fn test_models_group_by_default() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["groupBy"].as_str().unwrap(), "client,model");
}

#[test]
fn test_models_group_by_model() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .args(["--group-by", "model"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["groupBy"].as_str().unwrap(), "model");

    let entries = json["entries"].as_array().unwrap();
    let models: Vec<&str> = entries
        .iter()
        .map(|e| e["model"].as_str().unwrap())
        .collect();
    let unique_models: std::collections::HashSet<&&str> = models.iter().collect();
    assert_eq!(
        models.len(),
        unique_models.len(),
        "group-by model should produce unique model entries"
    );
}

#[test]
fn test_models_group_by_client_provider_model() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .args(["--group-by", "client,provider,model"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["groupBy"].as_str().unwrap(), "client,provider,model");

    let entries = json["entries"].as_array().unwrap();
    for entry in entries {
        assert!(entry.get("client").is_some(), "Entry must have client");
        assert!(entry.get("provider").is_some(), "Entry must have provider");
        assert!(entry.get("model").is_some(), "Entry must have model");
    }
}

#[test]
fn test_models_json_with_group_by_model() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .args(["--group-by", "model"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"].as_array().unwrap();
    for entry in entries {
        assert!(
            entry.get("mergedClients").is_some(),
            "group-by model entries should have mergedClients field"
        );
        assert!(
            entry.get("workspaceKey").is_none(),
            "group-by model entries should not expose workspaceKey"
        );
        assert!(
            entry.get("workspaceLabel").is_none(),
            "group-by model entries should not expose workspaceLabel"
        );
    }
}

#[test]
fn test_models_group_by_workspace_model_uses_unknown_bucket_for_unsupported_clients() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .args(["--group-by", "workspace,model"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["groupBy"].as_str().unwrap(), "workspace,model");

    let entries = json["entries"].as_array().unwrap();
    assert!(!entries.is_empty());
    for entry in entries {
        assert!(
            entry.get("workspaceKey").is_some(),
            "workspace grouping entries should always expose workspaceKey"
        );
        assert!(entry["workspaceKey"].is_null());
        assert!(
            entry.get("workspaceLabel").is_some(),
            "workspace grouping entries should always expose workspaceLabel"
        );
        assert_eq!(
            entry["workspaceLabel"].as_str().unwrap(),
            "Unknown workspace"
        );
    }
}

#[test]
fn test_models_group_by_workspace_model_surfaces_workspace_fields_for_qwen() {
    let tmp = create_qwen_workspace_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--qwen", "--no-spinner"])
        .args(["--group-by", "workspace-model"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["groupBy"].as_str().unwrap(), "workspace,model");

    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0]["workspaceKey"].as_str().unwrap(),
        "demo-workspace"
    );
    assert_eq!(
        entries[0]["workspaceLabel"].as_str().unwrap(),
        "demo-workspace"
    );
    assert_eq!(entries[0]["model"].as_str().unwrap(), "qwen3.5-plus");
}

// ── Pricing command tests ──────────────────────────────────────────────────

#[test]
fn test_pricing_command_success() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.args(["pricing", "claude-sonnet-4-20250514", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Pricing for"))
        .stdout(predicate::str::contains("Input"))
        .stdout(predicate::str::contains("Output"));
}

#[test]
fn test_pricing_command_json() {
    let output = cargo_bin_cmd!("tokscale")
        .args([
            "pricing",
            "claude-sonnet-4-20250514",
            "--json",
            "--no-spinner",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("modelId").is_some(), "Missing modelId");
    assert!(json.get("matchedKey").is_some(), "Missing matchedKey");
    assert!(json.get("source").is_some(), "Missing source");
    assert!(json.get("pricing").is_some(), "Missing pricing");

    let pricing = &json["pricing"];
    assert!(pricing.get("inputCostPerToken").is_some());
    assert!(pricing.get("outputCostPerToken").is_some());
}

#[test]
fn test_pricing_command_with_provider() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.args([
        "pricing",
        "claude-sonnet-4-20250514",
        "--provider",
        "litellm",
        "--no-spinner",
    ])
    .assert()
    .success();
}

#[test]
fn test_pricing_command_invalid_provider() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.args([
        "pricing",
        "claude-sonnet-4-20250514",
        "--provider",
        "invalid-provider",
        "--no-spinner",
    ])
    .assert()
    .failure();
}

// ── Clients command tests ──────────────────────────────────────────────────

#[test]
fn test_clients_command() {
    let tmp = create_empty_fixture_dir();
    cmd_with_home(tmp.path())
        .arg("clients")
        .assert()
        .success()
        .stdout(predicate::str::contains("OpenCode").or(predicate::str::contains("opencode")))
        .stdout(predicate::str::contains("Claude").or(predicate::str::contains("claude")));
}

#[test]
fn test_clients_json() {
    let tmp = create_empty_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["clients", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.is_object(), "Clients JSON should be an object");
    assert!(json.get("clients").is_some(), "Should have 'clients' field");
    assert!(
        json.get("headlessRoots").is_some(),
        "Should have 'headlessRoots' field"
    );
    assert!(json.get("note").is_some(), "Should have 'note' field");

    let arr = json["clients"].as_array().unwrap();
    assert!(!arr.is_empty(), "Should list at least one client");

    let first = &arr[0];
    assert!(
        first.get("client").is_some(),
        "Client entry should have 'client' field"
    );
    assert!(
        first.get("label").is_some(),
        "Client entry should have 'label' field"
    );
    assert!(
        first.get("sessionsPath").is_some(),
        "Client entry should have 'sessionsPath' field"
    );
    assert!(
        first.get("messageCount").is_some(),
        "Client entry should have 'messageCount' field"
    );
}

#[test]
fn test_clients_json_includes_settings_extra_paths() {
    let tmp = create_empty_fixture_dir();
    write_settings_json(
        tmp.path(),
        r#"{
            "scanner": {
                "extraScanPaths": {
                    "codex": ["/tmp/project-a/.codex/sessions"]
                }
            }
        }"#,
    );

    let output = cmd_with_home(tmp.path())
        .args(["clients", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let codex = json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["client"] == "codex")
        .unwrap();

    assert_eq!(
        codex["extraPaths"][0]["path"],
        serde_json::json!("/tmp/project-a/.codex/sessions")
    );
    assert_eq!(
        codex["extraPaths"][0]["source"],
        serde_json::json!("settings")
    );
}

#[test]
fn test_clients_command_includes_settings_extra_paths_text() {
    let tmp = create_empty_fixture_dir();
    write_settings_json(
        tmp.path(),
        r#"{
            "scanner": {
                "extraScanPaths": {
                    "codex": ["/tmp/project-a/.codex/sessions"]
                }
            }
        }"#,
    );

    cmd_with_home(tmp.path())
        .arg("clients")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "extra (settings): /tmp/project-a/.codex/sessions ✗",
        ));
}

// ── Light mode tests ───────────────────────────────────────────────────────

#[test]
fn test_models_light_output() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--light", "--opencode", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Token Usage Report by Model"));
}

#[test]
fn test_monthly_light_output() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["monthly", "--light", "--opencode", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Monthly Token Usage Report"));
}

#[test]
fn test_models_light_with_client_filter() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--light", "--opencode", "--no-spinner"])
        .args(["--year", "2024"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2024"));
}

// ── Benchmark flag tests ───────────────────────────────────────────────────

#[test]
fn test_models_benchmark_flag() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args([
            "models",
            "--light",
            "--opencode",
            "--no-spinner",
            "--benchmark",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Processing time"));
}

#[test]
fn test_monthly_benchmark_flag() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args([
            "monthly",
            "--light",
            "--opencode",
            "--no-spinner",
            "--benchmark",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Processing time"));
}

// ── Empty fixture tests ────────────────────────────────────────────────────

#[test]
fn test_models_empty_fixture() {
    let tmp = create_empty_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"].as_array().unwrap();
    assert!(
        entries.is_empty(),
        "Empty fixture should produce no entries"
    );
    assert_eq!(json["totalInput"].as_i64().unwrap(), 0);
    assert_eq!(json["totalOutput"].as_i64().unwrap(), 0);
}

#[test]
fn test_graph_empty_contributions() {
    let tmp = create_empty_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["graph", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let contributions = json["contributions"].as_array().unwrap();
    assert!(
        contributions.is_empty(),
        "Empty fixture should produce no contributions"
    );
}

// ── No-spinner flag tests ──────────────────────────────────────────────────

#[test]
fn test_models_no_spinner_flag() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--light", "--opencode", "--no-spinner"])
        .assert()
        .success();
}

#[test]
fn test_graph_no_spinner_flag() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["graph", "--opencode", "--no-spinner"])
        .assert()
        .success();
}

// ── Graph with client filter tests ─────────────────────────────────────────

#[test]
fn test_graph_with_client_filter() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["graph", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let contributions = json["contributions"].as_array().unwrap();
    for c in contributions {
        let clients = c["clients"].as_array().unwrap();
        for cl in clients {
            assert_eq!(
                cl["client"].as_str().unwrap(),
                "opencode",
                "All contributions should be from opencode"
            );
        }
    }
}

// ── Graph output file test ─────────────────────────────────────────────────

#[test]
fn test_graph_output_to_file() {
    let tmp = create_temp_fixture_dir();
    let output_file = tmp.path().join("graph-output.json");
    cmd_with_home(tmp.path())
        .args(["graph", "--opencode", "--no-spinner"])
        .args(["--output", output_file.to_str().unwrap()])
        .assert()
        .success();
    assert!(output_file.exists(), "Output file should be created");
    let content = fs::read_to_string(&output_file).unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(json.get("meta").is_some());
    assert!(json.get("contributions").is_some());
}

// ── Root command tests (no subcommand) ─────────────────────────────────────

#[test]
fn test_root_json_output() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["--json", "--opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("entries").is_some());
    assert!(json.get("totalCost").is_some());
}

#[test]
fn test_root_light_output() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["--light", "--opencode", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Token Usage Report by Model"));
}

#[test]
fn light_with_write_cache_writes_to_canonical_path() {
    let tmp = create_temp_fixture_dir();
    let config_dir = tmp.path().join("custom-config-root");
    prime_override_pricing_cache(&config_dir);

    cmd_with_home(tmp.path())
        .env("TOKSCALE_CONFIG_DIR", &config_dir)
        .args(["--light", "--opencode", "--write-cache", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Token Usage Report by Model"));

    assert!(
        config_dir.join("cache/tui-data-cache.json").exists(),
        "--write-cache should populate the canonical cache path"
    );
}

#[test]
fn test_root_with_date_filter() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["--json", "--opencode", "--no-spinner"])
        .args(["--year", "2025"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gpt-4o"));
}

#[test]
fn test_root_with_group_by() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["--json", "--opencode", "--no-spinner"])
        .args(["--group-by", "model"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["groupBy"].as_str().unwrap(), "model");
}

#[test]
fn test_submit_offline_without_pricing_cache_fails() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    write_fake_credentials(tmp.path());

    let output = offline_cmd_with_home(tmp.path())
        .args(["submit", "--opencode", "--dry-run"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "submit should fail when pricing is unavailable; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    // Verify failure is from pricing fetch, not from auth or argument errors
    assert!(
        !stderr.contains("Not logged in"),
        "submit failed due to auth, not pricing: {stderr}"
    );
    assert!(
        stderr.contains("error") || stderr.contains("Error"),
        "stderr should contain a pricing/network error: {stderr}"
    );
}
