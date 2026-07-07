use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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
        "time": { "created": 1718452800000.0, "completed": 1718452803500.0 }
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
        "time": { "created": 1718456400000.0, "completed": 1718456402560.0 }
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
        "time": { "created": 1736510400000.0, "completed": 1736510400920.0 }
    }"#;
    fs::write(session2.join("msg_c.json"), msg_c).unwrap();

    tmp
}

fn create_temp_fixture_dir() -> TempDir {
    create_temp_fixture_dir_with_pricing_cache(true)
}

fn create_fake_codex_bin() -> TempDir {
    let tmp = TempDir::new().expect("failed to create fake codex dir");
    let codex_path = tmp.path().join("codex");
    fs::write(
        &codex_path,
        r#"#!/bin/sh
case "$TOKSCALE_FAKE_CODEX_MODE" in
  success)
    printf 'captured ok'
    exit 0
    ;;
  fail)
    printf 'captured fail'
    exit 17
    ;;
  slow)
    exec sleep 20
    ;;
  *)
    echo "unknown TOKSCALE_FAKE_CODEX_MODE" >&2
    exit 2
    ;;
esac
"#,
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&codex_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&codex_path, permissions).unwrap();
    }

    tmp
}

fn headless_capture_command(fake_bin: &Path, output_path: &Path, mode: &str) -> Command {
    let mut cmd = cargo_bin_cmd!("tokscale");
    let path = std::env::var_os("PATH").unwrap_or_default();
    let joined_path = std::env::join_paths(
        std::iter::once(fake_bin.to_path_buf()).chain(std::env::split_paths(&path)),
    )
    .unwrap();

    cmd.env("HOME", fake_bin)
        .env("TOKSCALE_FAKE_CODEX_MODE", mode)
        .env("TOKSCALE_NATIVE_TIMEOUT_MS", "10000")
        .env("PATH", joined_path)
        .args([
            "headless",
            "--output",
            output_path.to_str().unwrap(),
            "--no-auto-flags",
            "codex",
        ]);

    cmd
}

#[test]
fn headless_capture_fast_success_does_not_wait_for_timeout() {
    let fake_bin = create_fake_codex_bin();
    let output_path = fake_bin.path().join("success.jsonl");

    let started = Instant::now();
    headless_capture_command(fake_bin.path(), &output_path, "success")
        .assert()
        .success();
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(8),
        "fast success waited too long: {elapsed:?}"
    );
    assert_eq!(fs::read_to_string(output_path).unwrap(), "captured ok");
}

#[test]
fn headless_capture_fast_nonzero_preserves_exit_code() {
    let fake_bin = create_fake_codex_bin();
    let output_path = fake_bin.path().join("fail.jsonl");

    let started = Instant::now();
    headless_capture_command(fake_bin.path(), &output_path, "fail")
        .assert()
        .failure()
        .code(17);
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(8),
        "fast failure waited too long: {elapsed:?}"
    );
    assert_eq!(fs::read_to_string(output_path).unwrap(), "captured fail");
}

#[test]
fn headless_capture_slow_command_times_out() {
    let fake_bin = create_fake_codex_bin();
    let output_path = fake_bin.path().join("slow.jsonl");

    let started = Instant::now();
    headless_capture_command(fake_bin.path(), &output_path, "slow")
        .assert()
        .failure()
        .code(124);
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_secs(10) && elapsed < Duration::from_secs(14),
        "slow command timeout duration was unexpected: {elapsed:?}"
    );
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

fn create_codex_workspace_fixture_dir() -> TempDir {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();
    prime_pricing_cache(base);

    let sessions_dir = base.join(".codex/sessions");
    fs::create_dir_all(&sessions_dir).unwrap();
    fs::write(
        sessions_dir.join("workspace-session.jsonl"),
        concat!(
            r#"{"type":"session_meta","payload":{"source":"chat","cwd":"/Users/alice/codex-workspace"}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.4"}}"#,
            "\n",
            r#"{"timestamp":"2026-01-01T00:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":120,"cached_input_tokens":20,"output_tokens":30}}}}"#,
            "\n"
        ),
    )
    .unwrap();

    tmp
}

fn create_opencode_workspace_fixture_dir() -> TempDir {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();
    prime_pricing_cache(base);

    let session = base.join(".local/share/opencode/storage/message/workspace-session");
    fs::create_dir_all(&session).unwrap();

    let msg = r#"{
        "id": "workspace_msg",
        "sessionID": "workspace-session",
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
        "time": { "created": 1718452800000.0 },
        "path": { "root": "/Users/alice/opencode-workspace" }
    }"#;
    fs::write(session.join("workspace_msg.json"), msg).unwrap();

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

/// Build a Command pointing HOME and the XDG dirs at the given temp dir for
/// hermetic test runs (no flags are added; callers append their own).
fn cmd_with_home(tmp: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.env("HOME", tmp)
        .env("XDG_CONFIG_HOME", tmp.join(".config"))
        .env("XDG_DATA_HOME", tmp.join(".local/share"))
        .env("XDG_CACHE_HOME", tmp.join(".cache"))
        .env("TOKSCALE_PRICING_CACHE_ONLY", "1")
        // Clear scan-path overrides inherited from the dev's shell, otherwise a
        // developer who exports e.g. TOKSCALE_EXTRA_DIRS=~/.codex/sessions (for
        // codefuse mirror tracking) makes the scanner read real session data
        // and breaks fixture-count assertions. Hermetic on CI either way.
        .env_remove("TOKSCALE_EXTRA_DIRS")
        .env_remove("TOKSCALE_HEADLESS_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("COPILOT_OTEL_FILE_EXPORTER_PATH")
        .env_remove("GOOSE_PATH_ROOT")
        .env_remove("CODEBUFF_DATA_DIR")
        .env_remove("GEMINI_CLI_HOME")
        .env_remove("HERMES_HOME")
        .env_remove("TOKSCALE_CONFIG_DIR");
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
        .env("ALL_PROXY", "http://127.0.0.1:9")
        // Clear scan-path overrides (mirrors cmd_with_home)
        .env_remove("TOKSCALE_EXTRA_DIRS")
        .env_remove("TOKSCALE_HEADLESS_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("COPILOT_OTEL_FILE_EXPORTER_PATH")
        .env_remove("GOOSE_PATH_ROOT")
        .env_remove("CODEBUFF_DATA_DIR")
        .env_remove("GEMINI_CLI_HOME")
        .env_remove("HERMES_HOME")
        .env_remove("TOKSCALE_CONFIG_DIR");
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

/// Add an assistant message with NO embedded cost to the OpenCode fixture.
///
/// Provider-reported OpenCode costs are preserved verbatim (never repriced),
/// so the stale-pricing-cache tests need an uncosted message to prove the
/// cache is actually consulted: 1000 input * 0.0000025 + 400 output *
/// 0.00001 = 0.0065 on top of the 0.10 embedded total.
fn add_uncosted_opencode_message(base: &Path) {
    let session2 = base.join(".local/share/opencode/storage/message/session2");
    fs::create_dir_all(&session2).unwrap();

    // Same hour as msg_c (2025-01-10 12:01 UTC) so hourly bucket counts hold
    let msg_d = r#"{
        "id": "msg_d",
        "sessionID": "session2",
        "role": "assistant",
        "modelID": "gpt-4o",
        "providerID": "openai",
        "tokens": {
            "input": 1000,
            "output": 400,
            "reasoning": 0,
            "cache": { "read": 0, "write": 0 }
        },
        "time": { "created": 1736510460000.0 }
    }"#;
    fs::write(session2.join("msg_d.json"), msg_d).unwrap();
}

fn write_fireworks_pricing_cache(base: &Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs();
    let litellm = serde_json::json!({
        "timestamp": now,
        "data": {
            "fireworks_ai/accounts/fireworks/models/deepseek-r1-0528-distill-qwen3-8b": {
                "input_cost_per_token": 0.0000002,
                "output_cost_per_token": 0.0000002
            }
        }
    });
    let openrouter = serde_json::json!({
        "timestamp": now,
        "data": {
            "deepseek/deepseek-v4-pro": {
                "input_cost_per_token": 0.000001,
                "output_cost_per_token": 0.000002
            }
        }
    });

    for dir in [
        base.join(".config/tokscale/cache"),
        base.join("Library/Caches/tokscale"),
        base.join(".cache/tokscale"),
    ] {
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("pricing-litellm.json"),
            serde_json::to_vec(&litellm).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("pricing-openrouter.json"),
            serde_json::to_vec(&openrouter).unwrap(),
        )
        .unwrap();
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
    let path = settings_json_path(base);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn settings_json_path(base: &Path) -> std::path::PathBuf {
    if cfg!(target_os = "windows") {
        base.join("AppData")
            .join("Roaming")
            .join("tokscale")
            .join("settings.json")
    } else {
        base.join(".config").join("tokscale").join("settings.json")
    }
}

fn write_codex_token_session(dir: &Path, name: &str, model: &str, input: i64, output: i64) {
    fs::create_dir_all(dir).unwrap();
    let turn_context = serde_json::json!({
        "type": "turn_context",
        "payload": {
            "model": model
        }
    });
    let token_count = serde_json::json!({
        "timestamp": "2026-01-01T00:00:01Z",
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "last_token_usage": {
                    "input_tokens": input,
                    "cached_input_tokens": 0,
                    "output_tokens": output
                }
            }
        }
    });
    fs::write(
        dir.join(name),
        format!("{}\n{}\n", turn_context, token_count),
    )
    .unwrap();
}

fn write_jcode_session(base: &Path) {
    let sessions_dir = base.join(".jcode/sessions");
    fs::create_dir_all(&sessions_dir).unwrap();
    fs::write(
        sessions_dir.join("session_cli_fixture.json"),
        r#"{
  "id":"session_cli_fixture",
  "provider_key":"cliproxyapi",
  "model":"jcode-cli-model",
  "working_dir":"/work/cli-fixture",
  "messages":[
    {"id":"assistant_1","role":"assistant","timestamp":"2026-01-01T00:00:01Z","token_usage":{"input_tokens":1000,"output_tokens":250,"cache_read_input_tokens":400,"cache_creation_input_tokens":50,"reasoning_output_tokens":25}}
  ]
}"#,
    )
    .unwrap();
}

fn write_cursor_usage_cache(base: &Path) {
    let cache_dir = base.join(".config/tokscale/cursor-cache");
    fs::create_dir_all(&cache_dir).unwrap();
    fs::write(cache_dir.join("usage.csv"), "Date,Model\n").unwrap();
}

fn write_cursor_credentials(base: &Path) {
    let config_dir = base.join(".config/tokscale");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("cursor-credentials.json"),
        serde_json::json!({
            "version": 1,
            "activeAccountId": "active-account",
            "accounts": {
                "active-account": {
                    "sessionToken": "test-session-token",
                    "userId": "active-account",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "label": "work"
                }
            }
        })
        .to_string(),
    )
    .unwrap();
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
fn test_codex_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.arg("codex")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Codex account integration commands",
        ));
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
fn test_codex_accounts_empty_json() {
    let tmp = TempDir::new().expect("failed to create temp home");
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.env("HOME", tmp.path())
        .env_remove("CODEX_HOME")
        .args(["codex", "accounts", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""accounts": []"#));
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
        .args(["--client", "opencode"])
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
        .args(["--client", "opencode"])
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
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["monthly", "--json", "--client", "opencode", "--no-spinner"])
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
            "--client",
            "opencode",
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
            "--client",
            "opencode",
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
            "--client",
            "opencode",
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
            "--client",
            "codex",
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
fn test_clients_home_override_uses_explicit_home_for_json() {
    let real_home = create_codex_fixture_dir();
    let conflicting_home = create_conflicting_codex_fixture_dir();
    write_codex_token_session(
        &real_home.path().join(".codex/sessions"),
        "session-2.jsonl",
        "gpt-4o-mini",
        80,
        20,
    );

    let output = cmd_with_conflicting_env(conflicting_home.path())
        .env("CODEX_HOME", conflicting_home.path().join(".codex"))
        .args([
            "--home",
            real_home.path().to_str().unwrap(),
            "clients",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let codex = json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["client"] == "codex")
        .unwrap();
    assert_eq!(
        codex["sessionsPath"],
        serde_json::json!(real_home.path().join(".codex/sessions"))
    );
    assert_eq!(codex["messageCount"].as_i64().unwrap(), 2);
}

#[test]
fn test_clients_home_override_ignores_copilot_exporter_env() {
    let real_home = create_empty_fixture_dir();
    let conflicting_home = create_empty_fixture_dir();
    let exporter_file = conflicting_home.path().join("copilot-host.jsonl");
    fs::write(&exporter_file, "{}").unwrap();

    let output = cmd_with_conflicting_env(conflicting_home.path())
        .env("COPILOT_OTEL_FILE_EXPORTER_PATH", &exporter_file)
        .args([
            "--home",
            real_home.path().to_str().unwrap(),
            "clients",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let copilot = json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["client"] == "copilot")
        .unwrap();
    assert!(
        copilot.get("exporterStatus").is_none(),
        "explicit --home diagnostics must not report host COPILOT_OTEL_FILE_EXPORTER_PATH: {copilot:#?}"
    );
}

#[test]
fn test_models_with_since_only() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["graph", "--client", "opencode", "--no-spinner"])
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
        .args(["graph", "--client", "opencode", "--no-spinner"])
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
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args([
            "models",
            "--json",
            "--client",
            "opencode",
            "--client",
            "claude",
            "--no-spinner",
        ])
        .assert()
        .success();
}

#[test]
fn test_models_with_client_filter_jcode() {
    let tmp = create_empty_fixture_dir();
    write_jcode_session(tmp.path());

    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "jcode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry["client"].as_str().unwrap(), "jcode");
    assert_eq!(entry["provider"].as_str().unwrap(), "cliproxyapi");
    assert_eq!(entry["model"].as_str().unwrap(), "jcode-cli-model");
    assert_eq!(entry["input"].as_i64().unwrap(), 1000);
    assert_eq!(entry["cacheRead"].as_i64().unwrap(), 400);
    assert_eq!(entry["cacheWrite"].as_i64().unwrap(), 50);
    assert_eq!(entry["output"].as_i64().unwrap(), 250);
    assert_eq!(entry["reasoning"].as_i64().unwrap(), 25);
}

fn assert_cursor_setup_warning(json: &serde_json::Value) {
    let warnings = json["warnings"]
        .as_array()
        .expect("explicit Cursor report should expose setup warnings");
    assert!(
        warnings
            .iter()
            .any(|warning| warning.as_str().is_some_and(|text| text
                .contains("tokscale cursor login")
                && text.contains("tokscale cursor sync --json")
                && text.contains("cursor-cache/usage*.csv")
                && text.contains("Tokscale does not parse local `~/.cursor`"))),
        "warnings did not explain Cursor setup: {warnings:?}"
    );
}

#[test]
fn test_models_cursor_explicit_missing_cache_reports_setup_warning_json() {
    let tmp = create_empty_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "cursor", "--no-spinner"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_cursor_setup_warning(&json);
}

#[test]
fn test_models_cursor_explicit_local_cursor_state_still_reports_setup_warning_json() {
    let tmp = create_empty_fixture_dir();
    fs::create_dir_all(
        tmp.path()
            .join(".cursor/projects/demo/agent-transcripts/session"),
    )
    .unwrap();
    fs::write(
        tmp.path()
            .join(".cursor/projects/demo/agent-transcripts/session/session.jsonl"),
        r#"{"role":"user","content":"hello"}"#,
    )
    .unwrap();

    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "cursor", "--no-spinner"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_cursor_setup_warning(&json);
}

#[test]
fn test_monthly_cursor_explicit_missing_cache_reports_setup_warning_json() {
    let tmp = create_empty_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["monthly", "--json", "--client", "cursor", "--no-spinner"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_cursor_setup_warning(&json);
}

#[test]
fn test_hourly_cursor_explicit_missing_cache_reports_setup_warning_json() {
    let tmp = create_empty_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["hourly", "--json", "--client", "cursor", "--no-spinner"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_cursor_setup_warning(&json);
}

#[test]
fn test_models_cursor_explicit_home_override_reports_fixture_cache_path() {
    let tmp = create_empty_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args([
            "--home",
            tmp.path().to_str().unwrap(),
            "models",
            "--json",
            "--client",
            "cursor",
            "--no-spinner",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let warnings = json["warnings"]
        .as_array()
        .expect("explicit Cursor --home report should expose setup warnings");
    assert!(
        warnings
            .iter()
            .any(|warning| warning.as_str().is_some_and(|text| text
                .contains(tmp.path().to_str().unwrap())
                && text.contains("tokscale cursor login")
                && text.contains("tokscale cursor sync --json")
                && text.contains("cursor-cache/usage*.csv"))),
        "warnings did not explain Cursor --home setup: {warnings:?}"
    );
}

#[test]
fn test_models_cursor_explicit_missing_cache_reports_setup_warning_text() {
    let tmp = create_empty_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--client", "cursor", "--no-spinner"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Cursor usage requires"))
        .stderr(predicate::str::contains("tokscale cursor login"))
        .stderr(predicate::str::contains("tokscale cursor sync --json"))
        .stderr(predicate::str::contains(
            "Tokscale does not parse local `~/.cursor`",
        ));
}

#[test]
fn test_models_default_missing_cursor_cache_does_not_emit_setup_warning_json() {
    let tmp = create_empty_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--no-spinner"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json.get("warnings")
            .and_then(serde_json::Value::as_array)
            .is_none_or(Vec::is_empty),
        "default all-client report should not warn about unrequested Cursor setup"
    );
}

#[test]
fn test_models_cursor_explicit_existing_cache_suppresses_setup_warning_json() {
    let tmp = create_empty_fixture_dir();
    write_cursor_usage_cache(tmp.path());

    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "cursor", "--no-spinner"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json.get("warnings")
            .and_then(serde_json::Value::as_array)
            .is_none_or(Vec::is_empty),
        "existing Cursor cache should suppress setup warnings"
    );
}

#[test]
fn test_models_cursor_logged_in_missing_cache_suggests_sync_only_json() {
    let tmp = create_empty_fixture_dir();
    write_cursor_credentials(tmp.path());

    let output = cmd_with_home(tmp.path())
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .args(["models", "--json", "--client", "cursor", "--no-spinner"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let warnings = json["warnings"].as_array().unwrap();
    let warning = warnings[0].as_str().unwrap();
    assert!(warning.contains("tokscale cursor sync --json"));
    assert!(
        !warning.contains("tokscale cursor login"),
        "logged-in users with no cache should be told to sync, not log in again: {warning}"
    );
}

#[test]
fn test_time_metrics_cursor_explicit_missing_cache_reports_setup_warning_json() {
    let tmp = create_empty_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args([
            "time-metrics",
            "--json",
            "--client",
            "cursor",
            "--no-spinner",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_cursor_setup_warning(&json);
}

#[test]
fn test_graph_cursor_explicit_missing_cache_reports_setup_warning_text() {
    let tmp = create_empty_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["graph", "--client", "cursor", "--no-spinner"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Cursor usage requires"))
        .stderr(predicate::str::contains("tokscale cursor login"));
}

#[test]
fn test_graph_fresh_cursor_cache_skips_auto_sync_warning() {
    let tmp = create_empty_fixture_dir();
    write_cursor_credentials(tmp.path());
    write_cursor_usage_cache(tmp.path());

    let output = cmd_with_home(tmp.path())
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .args(["graph", "--client", "cursor", "--no-spinner"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Cursor sync failed") && !stderr.contains("Cursor sync warning"),
        "fresh Cursor cache should skip implicit graph sync; stderr: {stderr}"
    );
}

#[test]
fn test_submit_cursor_explicit_missing_cache_reports_setup_warning_text() {
    let tmp = create_empty_fixture_dir();
    cmd_with_home(tmp.path())
        .env("TOKSCALE_API_TOKEN", "test-token")
        .args(["submit", "--client", "cursor", "--dry-run"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Cursor usage requires"))
        .stderr(predicate::str::contains("tokscale cursor login"));
}

#[test]
fn test_models_with_all_client_flags() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args([
            "models",
            "--json",
            "--no-spinner",
            "--client",
            "opencode",
            "--client",
            "claude",
            "--client",
            "codex",
            "--client",
            "gemini",
            "--client",
            "cursor",
            "--client",
            "amp",
            "--client",
            "droid",
            "--client",
            "openclaw",
            "--client",
            "pi",
        ])
        .assert()
        .success();
}

#[test]
fn test_models_client_and_date_combined() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
    let performance = first
        .get("performance")
        .expect("Missing performance")
        .as_object()
        .expect("performance should be an object");
    assert!(performance.contains_key("msPer1KTokens"));
    assert!(performance.contains_key("totalDurationMs"));
    assert!(performance.contains_key("timedTokens"));
    assert!(performance.contains_key("sampleCount"));
    assert!(performance.contains_key("tokenCoverage"));
    assert!(performance["msPer1KTokens"].as_f64().unwrap() > 0.0);
}

#[test]
fn test_models_json_offline_without_pricing_cache_still_succeeds() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    let output = offline_cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["monthly", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["graph", "--client", "opencode", "--no-spinner"])
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
fn test_hourly_json_offline_without_pricing_cache_still_succeeds() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    let output = offline_cmd_with_home(tmp.path())
        .args(["hourly", "--json", "--client", "opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry["input"].as_i64().unwrap())
            .sum::<i64>(),
        2400
    );
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry["output"].as_i64().unwrap())
            .sum::<i64>(),
        1000
    );
    let total_cost = json["totalCost"].as_f64().unwrap();
    assert!(
        (total_cost - 0.10).abs() < 1e-9,
        "unexpected totalCost without pricing: {total_cost}"
    );
}

#[test]
fn test_models_json_offline_uses_stale_pricing_cache_when_available() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    write_pricing_cache(tmp.path(), 1);
    add_uncosted_opencode_message(tmp.path());

    let output = offline_cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        (total_cost - 0.1065).abs() < 1e-9,
        "unexpected totalCost: {total_cost}"
    );
}

#[test]
fn test_monthly_json_offline_uses_stale_pricing_cache_when_available() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    write_pricing_cache(tmp.path(), 1);
    add_uncosted_opencode_message(tmp.path());

    let output = offline_cmd_with_home(tmp.path())
        .args(["monthly", "--json", "--client", "opencode", "--no-spinner"])
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
        (total_cost - 0.1065).abs() < 1e-9,
        "unexpected totalCost: {total_cost}"
    );
}

#[test]
fn test_graph_offline_uses_stale_pricing_cache_when_available() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    write_pricing_cache(tmp.path(), 1);
    add_uncosted_opencode_message(tmp.path());

    let output = offline_cmd_with_home(tmp.path())
        .args(["graph", "--client", "opencode", "--no-spinner"])
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
        (total_cost - 0.1065).abs() < 1e-9,
        "unexpected totalCost: {total_cost}"
    );
}

#[test]
fn test_hourly_json_offline_uses_stale_pricing_cache_when_available() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    write_pricing_cache(tmp.path(), 1);
    add_uncosted_opencode_message(tmp.path());

    let output = offline_cmd_with_home(tmp.path())
        .args(["hourly", "--json", "--client", "opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry["input"].as_i64().unwrap())
            .sum::<i64>(),
        3400
    );
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry["output"].as_i64().unwrap())
            .sum::<i64>(),
        1400
    );
    let total_cost = json["totalCost"].as_f64().unwrap();
    assert!(
        (total_cost - 0.1065).abs() < 1e-9,
        "unexpected totalCost: {total_cost}"
    );
}

#[test]
fn test_models_json_total_consistency() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["monthly", "--json", "--client", "opencode", "--no-spinner"])
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
fn test_hourly_home_override_uses_explicit_home_scanner_settings() {
    let real_home = create_empty_fixture_dir();
    let conflicting_home = create_conflicting_codex_fixture_dir();
    let extra_home = TempDir::new().unwrap();
    let extra_sessions = extra_home.path().join("portable-codex/sessions");
    write_codex_token_session(
        &extra_sessions,
        "settings-session.jsonl",
        "gpt-4o-mini",
        210,
        40,
    );
    write_settings_json(
        real_home.path(),
        &format!(
            r#"{{
                "scanner": {{
                    "extraScanPaths": {{
                        "codex": [{}]
                    }}
                }}
            }}"#,
            serde_json::to_string(extra_sessions.to_str().unwrap()).unwrap()
        ),
    );

    let output = cmd_with_conflicting_env(conflicting_home.path())
        .env("TOKSCALE_PRICING_CACHE_ONLY", "1")
        .env("CODEX_HOME", conflicting_home.path().join(".codex"))
        .args([
            "hourly",
            "--json",
            "--client",
            "codex",
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
    assert_eq!(json["entries"].as_array().unwrap().len(), 1);
    assert_eq!(json["entries"][0]["input"].as_i64().unwrap(), 210);
    assert_eq!(json["entries"][0]["output"].as_i64().unwrap(), 40);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("gpt-5"));
}

#[test]
fn test_monthly_json_with_client_filter() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["monthly", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["graph", "--client", "opencode", "--no-spinner"])
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
        .args(["graph", "--client", "opencode", "--no-spinner"])
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
        .args(["graph", "--client", "opencode", "--no-spinner"])
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
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        assert!(
            entry.get("sessionId").is_none(),
            "group-by model entries should not expose sessionId"
        );
    }
}

#[test]
fn test_models_group_by_session_emits_session_id_per_entry() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
        .args(["--group-by", "session,model"])
        .output()
        .unwrap();
    assert!(output.status.success(), "command failed: {:?}", output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["groupBy"].as_str().unwrap(), "session,model");

    let entries = json["entries"].as_array().unwrap();
    assert!(!entries.is_empty(), "expected at least one entry");

    let mut session_ids: Vec<&str> = entries
        .iter()
        .map(|e| {
            e.get("sessionId")
                .and_then(|v| v.as_str())
                .expect("session,model entries must include sessionId")
        })
        .collect();
    session_ids.sort();
    session_ids.dedup();
    // Fixture has two sessions ("session1", "session2"); expect both to appear.
    assert!(
        session_ids.contains(&"session1") && session_ids.contains(&"session2"),
        "expected both fixture sessions to appear in output, got {:?}",
        session_ids
    );

    for entry in entries {
        assert!(
            entry.get("workspaceKey").is_none(),
            "session grouping should not expose workspaceKey"
        );
        assert!(entry.get("model").is_some());
        assert!(entry.get("provider").is_some());
        assert!(entry.get("cost").is_some());
    }
}

#[test]
fn test_models_group_by_client_session_includes_client_and_session() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
        .args(["--group-by", "client,session,model"])
        .output()
        .unwrap();
    assert!(output.status.success(), "command failed: {:?}", output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["groupBy"].as_str().unwrap(), "client,session,model");

    let entries = json["entries"].as_array().unwrap();
    assert!(!entries.is_empty());
    for entry in entries {
        assert!(entry.get("sessionId").and_then(|v| v.as_str()).is_some());
        assert!(entry.get("client").and_then(|v| v.as_str()).is_some());
        assert!(entry.get("model").is_some());
    }
}

#[test]
fn test_models_group_by_workspace_model_uses_unknown_bucket_for_unsupported_clients() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["models", "--json", "--client", "qwen", "--no-spinner"])
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

#[test]
fn test_models_group_by_workspace_model_surfaces_workspace_fields_for_codex() {
    let tmp = create_codex_workspace_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "codex", "--no-spinner"])
        .args(["--group-by", "workspace,model"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["groupBy"].as_str().unwrap(), "workspace,model");

    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0]["workspaceKey"].as_str().unwrap(),
        "/Users/alice/codex-workspace"
    );
    assert_eq!(
        entries[0]["workspaceLabel"].as_str().unwrap(),
        "codex-workspace"
    );
    assert_eq!(entries[0]["model"].as_str().unwrap(), "gpt-5.4");
}

#[test]
fn test_models_group_by_workspace_model_surfaces_workspace_fields_for_opencode() {
    let tmp = create_opencode_workspace_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
        .args(["--group-by", "workspace,model"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["groupBy"].as_str().unwrap(), "workspace,model");

    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0]["workspaceKey"].as_str().unwrap(),
        "/Users/alice/opencode-workspace"
    );
    assert_eq!(
        entries[0]["workspaceLabel"].as_str().unwrap(),
        "opencode-workspace"
    );
    assert_eq!(entries[0]["model"].as_str().unwrap(), "claude-sonnet-4");
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

#[test]
fn test_pricing_command_does_not_fuzzy_match_provider_scoped_fireworks_model() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    write_fireworks_pricing_cache(tmp.path());

    let output = cmd_with_home(tmp.path())
        .args([
            "pricing",
            "accounts/fireworks/models/deepseek-v4-pro",
            "--no-spinner",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Model not found: accounts/fireworks/models/deepseek-v4-pro"),
        "expected a not-found message, got: {stdout}"
    );
    assert!(
        !stdout.contains("deepseek-r1-0528-distill-qwen3-8b"),
        "provider-scoped pricing lookup must not report the wrong Fireworks match: {stdout}"
    );
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
fn test_clients_json_includes_claude_transcripts_path() {
    let tmp = create_empty_fixture_dir();
    fs::create_dir_all(tmp.path().join(".claude/transcripts")).unwrap();

    let output = cmd_with_home(tmp.path())
        .args(["clients", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let claude = json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["client"] == "claude")
        .unwrap();

    assert_eq!(
        claude["additionalPaths"][0]["path"],
        serde_json::json!(tmp.path().join(".claude/transcripts"))
    );
    assert_eq!(claude["additionalPaths"][0]["exists"], true);
}

#[test]
fn test_clients_command_includes_claude_transcripts_text() {
    let tmp = create_empty_fixture_dir();
    fs::create_dir_all(tmp.path().join(".claude/transcripts")).unwrap();

    cmd_with_home(tmp.path())
        .arg("clients")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "additional: ~/.claude/transcripts ✓",
        ));
}

#[test]
fn test_clients_json_includes_claude_desktop_diagnostic() {
    let tmp = create_empty_fixture_dir();
    fs::create_dir_all(tmp.path().join("Library/Application Support/Claude")).unwrap();

    let output = cmd_with_home(tmp.path())
        .args(["clients", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let claude = json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["client"] == "claude")
        .unwrap();
    let diagnostics = claude["diagnostics"].as_array().unwrap();

    assert!(diagnostics.iter().any(|item| {
        item["code"] == "claude_desktop_not_scanned"
            && item["severity"] == "warning"
            && item["message"]
                .as_str()
                .unwrap()
                .contains("Claude Desktop app data was detected")
    }));
}

#[test]
fn test_clients_command_includes_claude_desktop_diagnostic_text() {
    let tmp = create_empty_fixture_dir();
    fs::create_dir_all(tmp.path().join("Library/Application Support/Claude")).unwrap();

    cmd_with_home(tmp.path())
        .arg("clients")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Claude Desktop app data was detected",
        ))
        .stdout(predicate::str::contains(
            "Claude Code JSONL transcripts only",
        ));
}

#[test]
fn test_models_json_includes_claude_desktop_diagnostic_for_empty_explicit_claude_report() {
    let tmp = create_empty_fixture_dir();
    fs::create_dir_all(tmp.path().join("Library/Application Support/Claude")).unwrap();

    let output = cmd_with_home(tmp.path())
        .args(["models", "--client", "claude", "--json", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostics = json["diagnostics"].as_array().unwrap();

    assert!(diagnostics.iter().any(|item| {
        item["code"] == "claude_desktop_not_scanned"
            && item["message"]
                .as_str()
                .unwrap()
                .contains("Tokscale counts Claude Code JSONL transcripts")
    }));
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
fn test_clients_json_includes_hermes_settings_extra_profile_path() {
    let tmp = create_empty_fixture_dir();
    let hermes_profile = tmp.path().join(".hermes/profiles/director_planning");
    fs::create_dir_all(&hermes_profile).unwrap();
    let hermes_profile_json = serde_json::to_string(&hermes_profile).unwrap();
    write_settings_json(
        tmp.path(),
        &format!(
            r#"{{
            "scanner": {{
                "extraScanPaths": {{
                    "hermes": [{hermes_profile_json}]
                }}
            }}
        }}"#
        ),
    );

    let output = cmd_with_home(tmp.path())
        .args(["clients", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let hermes = json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["client"] == "hermes")
        .unwrap();

    assert_eq!(
        hermes["extraPaths"][0]["path"],
        serde_json::json!(hermes_profile)
    );
    assert_eq!(
        hermes["extraPaths"][0]["source"],
        serde_json::json!("settings")
    );
    assert_eq!(hermes["extraPaths"][0]["exists"], true);
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
        .args(["models", "--light", "--client", "opencode", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Token Usage Report by Model"))
        .stdout(predicate::str::contains("ms/1K"));
}

#[test]
fn test_monthly_light_output() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["monthly", "--light", "--client", "opencode", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Monthly Token Usage Report"));
}

#[test]
fn test_models_light_with_client_filter() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["models", "--light", "--client", "opencode", "--no-spinner"])
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
            "--client",
            "opencode",
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
            "--client",
            "opencode",
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
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
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
        .args(["graph", "--client", "opencode", "--no-spinner"])
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
        .args(["models", "--light", "--client", "opencode", "--no-spinner"])
        .assert()
        .success();
}

#[test]
fn test_graph_no_spinner_flag() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .args(["graph", "--client", "opencode", "--no-spinner"])
        .assert()
        .success();
}

// ── Graph with client filter tests ─────────────────────────────────────────

#[test]
fn test_graph_with_client_filter() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["graph", "--client", "opencode", "--no-spinner"])
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
        .args(["graph", "--client", "opencode", "--no-spinner"])
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
        .args(["--json", "--client", "opencode", "--no-spinner"])
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
        .args(["--light", "--client", "opencode", "--no-spinner"])
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
        .args([
            "--light",
            "--client",
            "opencode",
            "--write-cache",
            "--no-spinner",
        ])
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
        .args(["--json", "--client", "opencode", "--no-spinner"])
        .args(["--year", "2025"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gpt-4o"));
}

#[test]
fn test_root_with_group_by() {
    let tmp = create_temp_fixture_dir();
    let output = cmd_with_home(tmp.path())
        .args(["--json", "--client", "opencode", "--no-spinner"])
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
        .args(["submit", "--client", "opencode", "--dry-run"])
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
// ── gjc client filter tests ────────────────────────────────────────────────

/// Write a gjc session JSONL file at
/// <home>/.gjc/agent/sessions/<slug>/sess.jsonl
/// with one assistant message: model claude-sonnet-4, provider anthropic,
/// input 1000 / output 500, usage.cost.total 0.5.
fn write_gjc_session_fixture(base: &Path) {
    let session_dir = base.join(".gjc/agent/sessions/test-project");
    fs::create_dir_all(&session_dir).unwrap();
    let jsonl = concat!(
        r#"{"type":"session","id":"gjc_e2e_session","timestamp":"2025-06-15T12:00:00.000Z","cwd":"/work/test-project"}"#,
        "\n",
        r#"{"type":"message","id":"gjc_e2e_msg_1","parentId":null,"timestamp":"2025-06-15T12:00:01.000Z","message":{"role":"assistant","model":"claude-sonnet-4","provider":"anthropic","api":"anthropic","timestamp":1750082401000,"usage":{"input":1000,"output":500,"cacheRead":0,"cacheWrite":0,"totalTokens":1500,"cost":{"input":0.3,"output":0.2,"cacheRead":0.0,"cacheWrite":0.0,"total":0.5}}}}"#,
        "\n"
    );
    fs::write(session_dir.join("sess.jsonl"), jsonl).unwrap();
}

/// Build a Command that uses HOME=tmp AND removes gjc-related env overrides
/// so the scanner uses only the home-derived ~/.gjc/agent/sessions path.
fn gjc_cmd_with_home(tmp: &Path) -> Command {
    let mut cmd = cmd_with_home(tmp);
    cmd.env_remove("GJC_CODING_AGENT_DIR")
        .env_remove("GJC_CONFIG_DIR")
        .env_remove("PI_CONFIG_DIR");
    cmd
}

#[test]
fn test_models_with_client_filter_gjc() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    prime_pricing_cache(tmp.path());
    write_gjc_session_fixture(tmp.path());

    let output = gjc_cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "gjc", "--no-spinner"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "command failed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"]
        .as_array()
        .expect("entries must be an array");

    assert!(
        !entries.is_empty(),
        "expected gjc entries but got none; full JSON: {json}"
    );

    // Every returned entry must be from the gjc client.
    for entry in entries {
        assert_eq!(
            entry["client"].as_str().unwrap_or(""),
            "gjc",
            "unexpected client in entry: {entry}"
        );
    }

    // The fixture model claude-sonnet-4 must appear.
    let has_sonnet = entries.iter().any(|e| {
        e["model"]
            .as_str()
            .unwrap_or("")
            .contains("claude-sonnet-4")
    });
    assert!(
        has_sonnet,
        "expected claude-sonnet-4 in gjc entries; got: {entries:?}"
    );
}

#[test]
fn test_client_filter_gjc_empty_is_clean() {
    // No gjc fixture data on disk — command must still exit successfully
    // and return an empty (zero-entry) result without panicking.
    let tmp = TempDir::new().expect("failed to create temp dir");
    prime_pricing_cache(tmp.path());

    let output = gjc_cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "gjc", "--no-spinner"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "command failed with no gjc data; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"]
        .as_array()
        .expect("entries must be an array");
    assert!(
        entries.is_empty(),
        "expected zero entries for empty gjc fixture, got: {entries:?}"
    );
}

#[test]
fn test_client_filter_gjc_isolation() {
    // Write gjc fixture, then query with --client claude (NOT gjc).
    // The gjc model must NOT appear in the output (filter isolation).
    let tmp = TempDir::new().expect("failed to create temp dir");
    prime_pricing_cache(tmp.path());
    write_gjc_session_fixture(tmp.path());

    let output = gjc_cmd_with_home(tmp.path())
        .args(["models", "--json", "--client", "claude", "--no-spinner"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "command failed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = json["entries"]
        .as_array()
        .expect("entries must be an array");

    // No gjc entry should leak through when filtering for claude.
    for entry in entries {
        assert_ne!(
            entry["client"].as_str().unwrap_or(""),
            "gjc",
            "gjc entry leaked into --client claude output: {entry}"
        );
    }
}

#[test]
fn report_no_summarize_json_empty_home_emits_valid_json_without_panic() {
    // Smoke test for the non-LLM `report` path: against an empty home it must
    // exit 0, never panic (UTF-8 truncation / NaN sort / div-by-zero guards),
    // and emit a parseable JSON array of entries.
    let tmp = create_empty_fixture_dir();

    let output = cmd_with_home(tmp.path())
        .arg("report")
        .arg("--no-summarize")
        .arg("--json")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "report --no-summarize --json failed against empty home; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("report --json must emit valid JSON");
    let entries = json
        .as_array()
        .expect("report --json output must be a JSON array of entries");
    assert!(
        entries.is_empty(),
        "expected zero entries for empty home, got: {entries:?}"
    );
}
