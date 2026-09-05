use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

// ── Fixture helpers ────────────────────────────────────────────────────────

fn write_canonical_pricing_cache_files(
    base: &Path,
    litellm_payload: &str,
    openrouter_payload: &str,
    models_dev_payload: &str,
) {
    let dir = base.join(".config/tokscale/cache");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("pricing-litellm.json"), litellm_payload).unwrap();
    fs::write(dir.join("pricing-openrouter.json"), openrouter_payload).unwrap();
    fs::write(dir.join("pricing-models-dev.json"), models_dev_payload).unwrap();
}

fn prime_pricing_cache(base: &Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs();
    let payload = format!(r#"{{"timestamp":{},"data":{{}}}}"#, now);
    let models_dev_payload = format!(
        r#"{{"timestamp":{},"data":{{"fixture/unused-model":{{"input_cost_per_token":0.000001,"output_cost_per_token":0.000002}}}}}}"#,
        now
    );
    write_canonical_pricing_cache_files(base, &payload, &payload, &models_dev_payload);
}

// @keep: the sentinel models.dev row in prime_pricing_cache marks the dataset
// as loaded without pricing any model used by the generic report fixtures.
/// Prime the cache with an actual priced model for submission tests.
///
/// The generic fixture deliberately has no matching published model, so tests
/// that need "pricing loaded fine, it just does not cover *this* model" must
/// prime a non-empty dataset with this helper.
fn prime_pricing_cache_with_a_priced_model(base: &Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs();
    let payload = format!(
        r#"{{"timestamp":{},"data":{{"gpt-4o":{{"input_cost_per_token":0.0000025,"output_cost_per_token":0.00001,"cache_read_input_token_cost":0.00000125,"cache_creation_input_token_cost":0.000003125}}}}}}"#,
        now
    );

    write_canonical_pricing_cache_files(base, &payload, &payload, &payload);
}

/// Prime a deterministic canonical Sonnet catalog for offline pricing command tests.
fn prime_canonical_sonnet_pricing_cache(base: &Path, model: &str) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs();
    let pricing = serde_json::json!({
        "input_cost_per_token": 0.000003,
        "output_cost_per_token": 0.000015,
        "cache_read_input_token_cost": 0.0000003,
        "cache_creation_input_token_cost": 0.00000375,
    });
    let payload = serde_json::to_string(&serde_json::json!({
        "timestamp": now,
        "data": serde_json::Map::from_iter([(model.to_owned(), pricing.clone())]),
    }))
    .unwrap();
    let models_dev_payload = serde_json::to_string(&serde_json::json!({
        "timestamp": now,
        "data": serde_json::Map::from_iter([(format!("anthropic/{model}"), pricing)]),
    }))
    .unwrap();
    write_canonical_pricing_cache_files(base, &payload, &payload, &models_dev_payload);
}

#[test]
fn prime_canonical_sonnet_pricing_cache_escapes_arbitrary_model_names() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let model = "claude-\"quoted\\model\nnext-line";

    prime_canonical_sonnet_pricing_cache(tmp.path(), model);

    let cache_dir = tmp.path().join(".config/tokscale/cache");
    for (file, expected_model) in [
        ("pricing-litellm.json", model.to_owned()),
        ("pricing-openrouter.json", model.to_owned()),
        ("pricing-models-dev.json", format!("anthropic/{model}")),
    ] {
        let payload: serde_json::Value =
            serde_json::from_slice(&fs::read(cache_dir.join(file)).unwrap()).unwrap();
        assert!(payload["data"].get(&expected_model).is_some());
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
    fs::write(cache_dir.join("pricing-models-dev.json"), &payload).unwrap();
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

/// Put a stand-in `codex` on PATH for the three `headless_capture_*` tests.
///
/// The stand-in is `src/bin/fake_codex.rs`, built by cargo as a real binary for
/// whatever platform the tests are running on, and copied here under the name
/// `run_capture_command` looks for. `CARGO_BIN_EXE_<name>` is set by cargo for
/// every binary in this package when it compiles this package's integration
/// tests, so the helper is guaranteed to exist and there is no target directory
/// to guess at.
///
/// It used to be a `#!/bin/sh` script written at test time, which is why these
/// tests could not run on Windows: there is no shebang there, and
/// `Command::new("codex")` resolves a bare program name by appending `.exe`
/// only — an extensionless `codex` is never even probed, so the child reported
/// `Failed to spawn 'codex': program not found`. That was the fixture failing,
/// not the feature.
///
/// The extension matters, so it is chosen per platform rather than assumed.
/// `std::fs::copy` carries the source's mode on Unix, which is why there is no
/// longer an explicit chmod.
fn create_fake_codex_bin() -> TempDir {
    let tmp = TempDir::new().expect("failed to create fake codex dir");
    let codex_path = tmp
        .path()
        .join(if cfg!(windows) { "codex.exe" } else { "codex" });

    fs::copy(Path::new(env!("CARGO_BIN_EXE_fake_codex")), &codex_path)
        .expect("failed to install the fake codex onto PATH");

    tmp
}

fn create_fake_mcode_bin() -> TempDir {
    let tmp = TempDir::new().expect("failed to create fake mcode dir");
    let mcode_path = tmp
        .path()
        .join(if cfg!(windows) { "mcode.exe" } else { "mcode" });

    fs::copy(Path::new(env!("CARGO_BIN_EXE_fake_codex")), &mcode_path)
        .expect("failed to install the fake mcode onto PATH");

    tmp
}

/// Build the `tokscale headless codex` invocation for one `headless_capture_*`
/// test.
///
/// `timeout_ms` is the parent's `TOKSCALE_NATIVE_TIMEOUT_MS` and is per-test on
/// purpose: the two `fast` tests and the `slow` test need the parent's deadline
/// on opposite sides of the child's runtime, so a single shared constant cannot
/// serve both. It is passed through `Settings::get_native_timeout`, which clamps
/// to `[5_000, 3_600_000]` ms — keep any value here inside that range or the
/// test will silently run against a different deadline than it asserts on.
fn headless_capture_command(
    fake_bin: &Path,
    output_path: &Path,
    mode: &str,
    timeout_ms: u64,
) -> Command {
    let mut cmd = cargo_bin_cmd!("tokscale");
    let path = std::env::var_os("PATH").unwrap_or_default();
    let joined_path = std::env::join_paths(
        std::iter::once(fake_bin.to_path_buf()).chain(std::env::split_paths(&path)),
    )
    .unwrap();

    cmd.env("HOME", fake_bin)
        .env("TOKSCALE_FAKE_CODEX_MODE", mode)
        .env("TOKSCALE_NATIVE_TIMEOUT_MS", timeout_ms.to_string())
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
fn headless_capture_mcode_injects_stream_json_after_exec() {
    let fake_bin = create_fake_mcode_bin();
    let output_dir = TempDir::new().expect("failed to create output dir");
    let output_path = output_dir.path().join("mcode.jsonl");
    let path = std::env::var_os("PATH").unwrap_or_default();
    let joined_path = std::env::join_paths(
        std::iter::once(fake_bin.path().to_path_buf()).chain(std::env::split_paths(&path)),
    )
    .unwrap();

    cargo_bin_cmd!("tokscale")
        .env("HOME", fake_bin.path())
        .env("TOKSCALE_FAKE_CODEX_MODE", "args")
        .env("PATH", joined_path)
        .args([
            "headless",
            "--output",
            output_path.to_str().unwrap(),
            "mcode",
            "exec",
            "review this change",
        ])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(output_path).unwrap(),
        "exec\n--output-format\nstream-json\nreview this change"
    );
}

/// The parent deadline the two `fast` tests give `tokscale`, and the elapsed
/// bound they assert against it.
///
/// These tests time a whole `tokscale` process spawn, so the measurement always
/// includes startup, and the useful question is not "how tight can the bound be"
/// but "is the gap between the two outcomes bigger than the noise". The two
/// outcomes are: the parent notices its child already exited and returns at once,
/// or it wrongly sits on its deadline. So the gap *is* the timeout, and the bound
/// only has to land somewhere inside it.
///
/// With the previous constants — a 10s deadline and an 8s bound — the gap was 10s
/// and the noise was larger than that. All three of these tests failed on the
/// `windows-latest` leg of run 31196968982 (job 93170461116) with correct
/// behaviour and wrong timings: `fast failure waited too long: 11.0576809s` and
/// `fast success waited too long: 11.16815s`, i.e. startup alone exceeded the
/// very interval the assertion exists to detect. No value between 0 and 10s could
/// have separated the two outcomes there, which is why the bound was not simply
/// raised again — see the `slow` test below for the previous attempt at that.
///
/// A 60s deadline with a 30s bound sits in the middle of a 60s gap: 27s of
/// headroom above a healthy run, which costs ~3.2s locally, and 30s below the
/// failure mode. A parent that waits for its deadline takes at least 60s and
/// fails; runner slowness would have to add ~27s to produce a false red.
/// The cost is that a genuinely hung parent now takes up to 60s to be reported
/// instead of 10s, which is the right trade against a gate that blocks unrelated
/// changes.
const HEADLESS_FAST_TIMEOUT_MS: u64 = 60_000;
const HEADLESS_FAST_MAX_ELAPSED: Duration = Duration::from_secs(30);

#[test]
fn headless_capture_fast_success_does_not_wait_for_timeout() {
    let fake_bin = create_fake_codex_bin();
    let output_path = fake_bin.path().join("success.jsonl");

    let started = Instant::now();
    headless_capture_command(
        fake_bin.path(),
        &output_path,
        "success",
        HEADLESS_FAST_TIMEOUT_MS,
    )
    .assert()
    .success();
    let elapsed = started.elapsed();

    assert!(
        elapsed < HEADLESS_FAST_MAX_ELAPSED,
        "fast success waited too long: {elapsed:?}"
    );
    assert_eq!(fs::read_to_string(output_path).unwrap(), "captured ok");
}

#[test]
fn headless_capture_fast_nonzero_preserves_exit_code() {
    let fake_bin = create_fake_codex_bin();
    let output_path = fake_bin.path().join("fail.jsonl");

    let started = Instant::now();
    headless_capture_command(
        fake_bin.path(),
        &output_path,
        "fail",
        HEADLESS_FAST_TIMEOUT_MS,
    )
    .assert()
    .failure()
    .code(17);
    let elapsed = started.elapsed();

    assert!(
        elapsed < HEADLESS_FAST_MAX_ELAPSED,
        "fast failure waited too long: {elapsed:?}"
    );
    assert_eq!(fs::read_to_string(output_path).unwrap(), "captured fail");
}

/// The parent deadline for the `slow` test, and the window the elapsed time has
/// to land in. Kept deliberately far from `FAKE_CODEX_SLOW_SLEEP_SECS`, which is
/// the child's own sleep in `src/bin/fake_codex.rs`.
///
/// This window answers "did the *parent* end this run", not "did it end it on
/// time". `headless_capture_timeout_fires_near_its_deadline` below answers the
/// second question, because this window deliberately cannot: it is 50s wide
/// above the deadline, so a regression that stretched the effective deadline to
/// 50s would still kill the 120s child, still report 124, and still land inside
/// it. The two assertions are complementary and both are needed.
const HEADLESS_SLOW_TIMEOUT_MS: u64 = 10_000;
const HEADLESS_SLOW_MIN_ELAPSED: Duration = Duration::from_secs(10);
const HEADLESS_SLOW_MAX_ELAPSED: Duration = Duration::from_secs(60);

#[test]
fn headless_capture_slow_command_times_out() {
    let fake_bin = create_fake_codex_bin();
    let output_path = fake_bin.path().join("slow.jsonl");

    let started = Instant::now();
    headless_capture_command(
        fake_bin.path(),
        &output_path,
        "slow",
        HEADLESS_SLOW_TIMEOUT_MS,
    )
    .assert()
    .failure()
    .code(124);
    let elapsed = started.elapsed();

    // The discriminating fact is that the *parent's* 10s timeout ended this run,
    // not the child's own sleep. The lower bound proves the parent waited for
    // its deadline instead of failing early; the upper bound proves it did not
    // simply outlive the child.
    //
    // The upper bound is therefore set against the child's sleep, not against a
    // teardown budget. It has been wrong twice, in the same way both times:
    //
    //   - 14s left only 4s for everything outside the deadline — `tokscale`'s own
    //     process startup, `child.kill()`, `child.wait()` and joining the stdout
    //     pump — and on a Windows runner startup alone can consume most of that.
    //     It failed at 14.83s in CI (job 92167428621) while still killing the
    //     child correctly, so the bound was measuring runner speed rather than
    //     the behaviour this test is named for.
    //   - 18s, its replacement, kept a 2s margin below the child's then-20s sleep
    //     and failed at 21.17s in CI (run 31196968982, job 93170461116) — past the
    //     child's sleep entirely. At that point the bound could no longer separate
    //     "the parent killed the child at its deadline" from "the parent outlived
    //     the child", which is the one thing it exists to do, so raising it to 22s
    //     would have kept the test green while making it vacuous.
    //
    // Both failures came from squeezing the bound into a narrow gap between the
    // deadline and the child's sleep. The fix is to widen that gap instead: the
    // child now sleeps `FAKE_CODEX_SLOW_SLEEP_SECS` (120s) against the parent's
    // 10s deadline, so the two outcomes are 110s apart and the 60s bound has 50s
    // of headroom above the deadline and 60s below the child's sleep. A parent
    // that outlived the child cannot finish before 120s, and the ~11s of runner
    // startup that broke the 18s bound is a fifth of the headroom here.
    //
    // The lower bound stays at the parent's own deadline. It is exact by
    // construction: the child cannot exit on its own before then, so anything
    // faster means the parent gave up early.
    //
    // Note this test cannot catch #1049: the stand-in spawns nothing, so its pipe
    // closes the moment it is killed, and the drain after the kill returns at
    // once instead of waiting out `STDOUT_DRAIN_GRACE`. Widening the gap does not
    // hide that — it was never covered here. The descendant shape that #1049
    // actually reports is covered by
    // `headless_capture_descendant_holding_stdout_still_times_out` below.
    assert!(
        elapsed >= HEADLESS_SLOW_MIN_ELAPSED && elapsed < HEADLESS_SLOW_MAX_ELAPSED,
        "slow command timeout duration was unexpected: {elapsed:?}"
    );
}

/// The #1049 shape: the child is killed at the deadline but a *descendant* still
/// holds the write end of the stdout pipe, so the pump never sees EOF.
///
/// Unix-only. The grandchild is the stand-in binary itself, and Windows keeps a
/// running image locked, which would leave the fixture's TempDir undeletable for
/// the two minutes the grandchild lives. #1049 is a Linux report and
/// `run_capture_command` is ordinary `std::process` code, so ubuntu and macOS
/// coverage is what this needs to be worth its cost.
///
/// The bound this asserts is the point: a parent that waits for EOF without one
/// cannot finish before the grandchild's 120s sleep, well past
/// `HEADLESS_SLOW_MAX_ELAPSED`.
#[test]
#[cfg(unix)]
fn headless_capture_descendant_holding_stdout_still_times_out() {
    let fake_bin = create_fake_codex_bin();
    let output_path = fake_bin.path().join("descendant.jsonl");

    let started = Instant::now();
    headless_capture_command(
        fake_bin.path(),
        &output_path,
        "descendant",
        HEADLESS_SLOW_TIMEOUT_MS,
    )
    .assert()
    .failure()
    .code(124);
    let elapsed = started.elapsed();

    assert!(
        elapsed >= HEADLESS_SLOW_MIN_ELAPSED && elapsed < HEADLESS_SLOW_MAX_ELAPSED,
        "a descendant holding stdout must not extend the timeout: {elapsed:?}"
    );
}

/// Reaps the stdout holder `fake_codex` publishes to `TOKSCALE_FAKE_CODEX_PIDFILE`.
///
/// The holder deliberately outlives the direct child, so without this it would
/// sit in the process table for the rest of its 120s sleep. Parallel and retried
/// CI runs accumulate those, so teardown kills it rather than waiting it out.
#[cfg(unix)]
struct HolderReaper {
    pidfile: std::path::PathBuf,
}

#[cfg(unix)]
impl Drop for HolderReaper {
    fn drop(&mut self) {
        let Ok(raw) = std::fs::read_to_string(&self.pidfile) else {
            return;
        };
        let Ok(pid) = raw.trim().parse::<i32>() else {
            return;
        };
        // Shelling out to kill(1) keeps this dependency-free -- tokscale-cli does
        // not depend on libc, and pulling it in for one test teardown is not
        // worth it. An already-exited pid just makes kill exit non-zero.
        let _ = std::process::Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// The other half of #1049: the direct child exits *successfully* before the
/// deadline while a descendant keeps stdout open.
///
/// `try_wait` sees a normal exit, so `timed_out` stays false and the drain takes
/// the non-timeout branch. That branch waited unboundedly -- both before #1166
/// (an unconditional `join`) and after it -- so this hung forever with no
/// deadline to rescue it. The timeout branch's `STDOUT_DRAIN_GRACE` does not
/// apply here, which is exactly why this needs its own test.
#[test]
#[cfg(unix)]
fn headless_capture_descendant_after_clean_exit_does_not_hang() {
    let fake_bin = create_fake_codex_bin();
    let output_path = fake_bin.path().join("descendant-exit.jsonl");
    let pidfile = fake_bin.path().join("holder.pid");
    let _reaper = HolderReaper {
        pidfile: pidfile.clone(),
    };

    let started = Instant::now();
    headless_capture_command(
        fake_bin.path(),
        &output_path,
        "descendant-exit",
        HEADLESS_SLOW_TIMEOUT_MS,
    )
    .env("TOKSCALE_FAKE_CODEX_PIDFILE", &pidfile)
    .assert()
    .failure();
    let elapsed = started.elapsed();

    // The bound is the assertion. The holder sleeps 120s, so anything that waits
    // for EOF cannot return before then; finishing inside the 60s ceiling proves
    // the wait is bounded rather than merely slow.
    assert!(
        elapsed < HEADLESS_SLOW_MAX_ELAPSED,
        "a descendant holding stdout after a clean child exit must not hang: {elapsed:?}"
    );
}

/// How far the measured deadline may sit from the configured one in
/// `headless_capture_timeout_fires_near_its_deadline`.
///
/// See that test for why five seconds against a ten second deadline is both
/// generous and meaningful.
const HEADLESS_DEADLINE_TOLERANCE: Duration = Duration::from_secs(5);

/// Time one `headless capture` run of `mode` under `timeout_ms`, asserting the
/// outcome the mode is defined to produce, and return how long it took.
///
/// Shared by `headless_capture_timeout_fires_near_its_deadline` so its baseline
/// and its timed-out run differ in exactly one thing — the deadline — and every
/// other cost is measured the same way on both sides of the subtraction.
fn time_headless_capture(fake_bin: &Path, label: &str, mode: &str, timeout_ms: u64) -> Duration {
    let output_path = fake_bin.join(format!("{label}.jsonl"));
    let started = Instant::now();
    let assertion = headless_capture_command(fake_bin, &output_path, mode, timeout_ms).assert();
    match mode {
        "success" => {
            assertion.success();
        }
        "slow" => {
            assertion.failure().code(124);
        }
        other => panic!("time_headless_capture does not know mode {other:?}"),
    }
    let elapsed = started.elapsed();
    if mode == "success" {
        assert_eq!(fs::read_to_string(&output_path).unwrap(), "captured ok");
    }
    elapsed
}

#[test]
fn headless_capture_timeout_fires_near_its_deadline() {
    // `headless_capture_slow_command_times_out` proves the parent killed the
    // child rather than outliving it. It cannot prove the parent killed it *at
    // the deadline it was given*: its window is `[10s, 60s)`, so a regression
    // that stretched the effective deadline to 50s would still kill the 120s
    // child, still report 124, and still land inside that window. This test pins
    // the deadline itself.
    //
    // It does so by subtracting two runs instead of bounding one. Everything a
    // run pays outside the deadline — `tokscale`'s process spawn, dynamic
    // linking, argument parsing, settings load, spawning and reaping the
    // stand-in on PATH — is paid by a fast run and a timed-out run alike, so it
    // cancels. What is left is the wait on the deadline.
    //
    // The numbers from the CI failure that forced the previous bound open (run
    // 31196968982, job 93170461116) show how completely it cancels. Against a
    // configured 10s deadline:
    //
    //     fast_success 11.16815s    fast_fail 11.0576809s    slow 21.173129s
    //     slow - fast_success = 10.0050s
    //     slow - fast_fail    = 10.1154s
    //
    // Every absolute number there is ~11s of runner overhead away from anything
    // a fixed threshold could use — that overhead alone is longer than the
    // deadline being measured, which is why no absolute bound survived. The
    // differences recover the deadline to within 0.12s, on the very runner that
    // was too slow for a bound of any width. The ~11s of startup noise is not
    // something this assertion tolerates; it is something it subtracts away.
    let fake_bin = create_fake_codex_bin();

    // The overhead is only common-mode once it is in steady state, and the first
    // run of the test binary is not. Measured locally: the first `tokscale`
    // spawn in this test costs 3.5-6.1s (paging in a debug binary, linking,
    // first execution of the freshly copied stand-in), and every spawn after it
    // costs ~45ms. Subtracting a cold baseline from a warm timed-out run charges
    // that one-off to the deadline and understates it — an early draft of this
    // test failed exactly that way, reporting a 3.96s deadline for a 10s
    // configured one because its single baseline happened to cost 6.09s.
    //
    // So the baseline is the minimum of two samples rather than one sample. The
    // noise here is one-sided — nothing can make a run finish faster than the
    // work it has to do, only slower — so the smallest sample is the best
    // estimate of the floor, and one unlucky sample no longer moves it. The
    // symmetric risk, a one-off landing on the timed-out run instead, is what
    // the tolerance below is sized for.
    let baseline = time_headless_capture(
        fake_bin.path(),
        "deadline-baseline-a",
        "success",
        HEADLESS_FAST_TIMEOUT_MS,
    )
    .min(time_headless_capture(
        fake_bin.path(),
        "deadline-baseline-b",
        "success",
        HEADLESS_FAST_TIMEOUT_MS,
    ));

    let configured_deadline = Duration::from_millis(HEADLESS_SLOW_TIMEOUT_MS);

    // Five seconds against a ten second deadline, i.e. an accepted band of
    // `[5s, 15s]`.
    //
    // Generous: the largest residual ever measured is the 0.12s above, on a
    // `windows-latest` runner loaded enough to fail three tests at once, and a
    // warm local run lands within ~50ms. Five seconds is roughly forty times the
    // worst of those, so this is not the next bound that gets widened.
    //
    // Meaningful: it still fails loudly for the case the coarse window misses.
    // An effective deadline of 50s lands 35s outside the band; so does 2x; so
    // does anything past 1.5x. Five seconds is the largest tolerance that keeps
    // "the deadline is half again as long as it was configured to be" a red
    // test. It catches the opposite regression too — a deadline collapsing
    // toward zero — which the `[10s, 60s)` lower bound only appears to catch
    // because startup pads the measurement past 10s on its own.
    //
    // Widening this is a change of meaning, not of margin: the check is worth
    // having only while `configured * 1.5` stays outside the band.
    let low = configured_deadline.saturating_sub(HEADLESS_DEADLINE_TOLERANCE);
    let high = configured_deadline + HEADLESS_DEADLINE_TOLERANCE;

    // The timed-out run: the same overhead, plus one wait on
    // `HEADLESS_SLOW_TIMEOUT_MS`. Run adjacent in time to the baseline on the
    // same machine, so the overhead the subtraction removes is the overhead that
    // was actually paid.
    //
    // The one-sided-noise argument that makes the baseline a minimum of two
    // samples applies to this side too, and this is the side that drives the
    // upper bound: a scheduling hit worth more than the tolerance would report a
    // deadline longer than the one that actually fired. So this side is a
    // minimum of samples as well — it is just sampled lazily, because each
    // sample costs a full `HEADLESS_SLOW_TIMEOUT_MS` and the baseline's cost
    // ~45ms once warm. The second sample is taken only when the first disagrees
    // with the configured deadline, so the steady-state cost stays one wait.
    //
    // Sampling lazily cannot hide a regression. A second sample is taken only
    // when the test is already failing, and a minimum can only move the estimate
    // down, so the retry can rescue a spike that inflated the measurement and
    // nothing else: a deadline that really is 5x too long measures ~50s twice
    // and still fails.
    //
    // That the minimum only moves down is also why the resample is conditioned
    // on the upper bound alone. A measurement below `low` — a deadline that
    // fired early, or a baseline that padded the run — cannot be moved back up
    // into the band by a minimum, so resampling it would spend another full
    // `HEADLESS_SLOW_TIMEOUT_MS` to reach the failure it had already reached.
    // The too-short case fails on the first sample.
    //
    // `saturating_sub` rather than `-`: a baseline longer than the timed-out run
    // means the deadline was not waited on at all, which is a failure to report,
    // not a subtraction overflow to panic on.
    let mut slow = time_headless_capture(
        fake_bin.path(),
        "deadline-slow",
        "slow",
        HEADLESS_SLOW_TIMEOUT_MS,
    );
    let mut measured_deadline = slow.saturating_sub(baseline);
    if measured_deadline > high {
        slow = slow.min(time_headless_capture(
            fake_bin.path(),
            "deadline-slow-resample",
            "slow",
            HEADLESS_SLOW_TIMEOUT_MS,
        ));
        measured_deadline = slow.saturating_sub(baseline);
    }

    assert!(
        measured_deadline >= low && measured_deadline <= high,
        "timeout did not fire near its configured deadline: \
         configured {configured_deadline:?}, measured {measured_deadline:?} \
         (timed-out run {slow:?} - baseline run {baseline:?}), \
         accepted {low:?}..={high:?}"
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

#[cfg(unix)]
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

#[cfg(unix)]
fn create_positive_utc_offset_submit_fixture_dir() -> (TempDir, String) {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();
    prime_pricing_cache(base);

    let utc_today = chrono::Utc::now().date_naive();
    let utc_noon = utc_today.and_hms_opt(12, 0, 0).unwrap().and_utc();
    let local_date = utc_today.succ_opt().unwrap().format("%Y-%m-%d").to_string();
    let session = base.join(".local/share/opencode/storage/message/session1");
    fs::create_dir_all(&session).unwrap();

    let message = serde_json::json!({
        "id": "msg_ahead_of_utc",
        "sessionID": "session1",
        "role": "assistant",
        "modelID": "gpt-4o",
        "providerID": "openai",
        "cost": 0.02,
        "tokens": {
            "input": 1000,
            "output": 500,
            "reasoning": 0,
            "cache": { "read": 200, "write": 50 }
        },
        "time": { "created": utc_noon.timestamp_millis() as f64 }
    });
    fs::write(session.join("msg_ahead_of_utc.json"), message.to_string()).unwrap();

    (tmp, local_date)
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
/// The config root every `cmd_with_home` fixture writes into, and the value
/// the child is told to use.
///
/// `HOME` plus the `XDG_*` vars locate the config dir on Unix and reach nothing
/// on Windows: `paths::get_config_dir` resolves the Windows root through
/// `dirs::config_dir()`, a `SHGetKnownFolderPath` call no environment variable
/// redirects. The child therefore read the *runner's* real
/// `%APPDATA%\tokscale\` — so a fixture's settings.json was never seen (model
/// aliases went unfolded, `scanner.extraScanPaths` came back null, auto-pinning
/// had nothing to pin into) and the pricing cache the fixture primed with an
/// empty catalog was replaced by whatever real prices happened to be on the
/// machine.
///
/// `TOKSCALE_CONFIG_DIR` is the one override consulted first on every platform,
/// and on Unix it names the directory the `XDG_CONFIG_HOME` pin already
/// produced, so nothing moves there. Setting it also means these runs no longer
/// depend on the legacy macOS settings fallback, which is correct for a
/// hermetic fixture: `paths.rs` documents the override as meaning exactly "do
/// not ingest anything from outside this root".
fn sandbox_config_dir(tmp: &Path) -> std::path::PathBuf {
    tmp.join(".config").join("tokscale")
}

fn cmd_with_home(tmp: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.env("HOME", tmp)
        .env("XDG_CONFIG_HOME", tmp.join(".config"))
        .env("XDG_DATA_HOME", tmp.join(".local/share"))
        .env("XDG_CACHE_HOME", tmp.join(".cache"))
        .env("TOKSCALE_CONFIG_DIR", sandbox_config_dir(tmp))
        // `pricing` intentionally bypasses TOKSCALE_PRICING_CACHE_ONLY; the
        // loopback proxies below are the offline guarantee for every command.
        .env("TOKSCALE_PRICING_CACHE_ONLY", "1")
        // Point every API call at a dead loopback port. `get_api_base_url`
        // reads TOKSCALE_API_URL and falls back to https://tokscale.ai, and
        // submit, autosubmit run, login and delete-submitted-data all format
        // their endpoint onto it. Unpinned, those commands hit production with
        // whatever token the test set, stopped only by the proxies below — a
        // guard a `.no_proxy()` on one client would silently remove. Pinning it
        // here also drops any TOKSCALE_API_URL a developer has exported.
        .env("TOKSCALE_API_URL", "http://127.0.0.1:9")
        // Keep cache-only fixtures offline even if a future code path ignores
        // the cache-only switch or a developer has proxy variables configured.
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env("http_proxy", "http://127.0.0.1:9")
        .env("https_proxy", "http://127.0.0.1:9")
        .env("all_proxy", "http://127.0.0.1:9")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        // Clear scan-path overrides inherited from the dev's shell, otherwise a
        // developer who exports e.g. TOKSCALE_EXTRA_DIRS=~/.codex/sessions (for
        // codefuse mirror tracking) makes the scanner read real session data
        // and breaks fixture-count assertions. Hermetic on CI either way.
        .env_remove("TOKSCALE_EXTRA_DIRS")
        .env_remove("TOKSCALE_HEADLESS_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("COPILOT_OTEL_FILE_EXPORTER_PATH")
        .env_remove("GOOSE_PATH_ROOT")
        .env_remove("CODEBUFF_DATA_DIR")
        .env_remove("GEMINI_CLI_HOME")
        .env_remove("HERMES_HOME")
        .env_remove("PRIME_AGENT_CODING_AGENT_DIR")
        .env_remove("PRIME_AGENT_SESSION_DIR")
        .env_remove("PRIME_AGENT_CODING_AGENT_SESSION_DIR")
        .env_remove("DSH_HOME");
    cmd
}

fn cmd_with_conflicting_env(tmp: &Path) -> Command {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.env("HOME", tmp)
        .env("XDG_CONFIG_HOME", tmp.join(".config"))
        .env("XDG_DATA_HOME", tmp.join(".local/share"))
        .env("XDG_CACHE_HOME", tmp.join(".cache"))
        .env("TOKSCALE_CONFIG_DIR", sandbox_config_dir(tmp));
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
        .env("TOKSCALE_CONFIG_DIR", sandbox_config_dir(tmp))
        // Same production-endpoint pin as cmd_with_home.
        .env("TOKSCALE_API_URL", "http://127.0.0.1:9")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env("http_proxy", "http://127.0.0.1:9")
        .env("https_proxy", "http://127.0.0.1:9")
        .env("all_proxy", "http://127.0.0.1:9")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        // Clear scan-path overrides (mirrors cmd_with_home)
        .env_remove("TOKSCALE_EXTRA_DIRS")
        .env_remove("TOKSCALE_HEADLESS_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("COPILOT_OTEL_FILE_EXPORTER_PATH")
        .env_remove("GOOSE_PATH_ROOT")
        .env_remove("CODEBUFF_DATA_DIR")
        .env_remove("GEMINI_CLI_HOME")
        .env_remove("HERMES_HOME")
        .env_remove("PRIME_AGENT_CODING_AGENT_DIR")
        .env_remove("PRIME_AGENT_SESSION_DIR")
        .env_remove("PRIME_AGENT_CODING_AGENT_SESSION_DIR");
    cmd
}

fn write_pricing_cache(base: &Path, timestamp: u64) {
    let litellm = format!(
        r#"{{"timestamp":{},"data":{{"gpt-4o":{{"input_cost_per_token":0.0000025,"output_cost_per_token":0.00001}},"claude-sonnet-4-20250514":{{"input_cost_per_token":0.000003,"output_cost_per_token":0.000015}}}}}}"#,
        timestamp
    );
    let openrouter = format!(r#"{{"timestamp":{},"data":{{}}}}"#, timestamp);

    write_canonical_pricing_cache_files(base, &litellm, &openrouter, &litellm);
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

/// Add an assistant message whose usage is entirely zero (tokens, cost, no
/// duration) in its own month/hour, for `--hide-zero` tests. Uses a distinct
/// model and a 2023-03-15 timestamp so it forms an all-zero row in the
/// models, monthly, and hourly reports without touching other buckets.
fn add_zero_usage_opencode_message(base: &Path) {
    let session3 = base.join(".local/share/opencode/storage/message/session3");
    fs::create_dir_all(&session3).unwrap();

    // 2023-03-15 12:00:00 UTC = 1678881600000 ms
    let msg_z = r#"{
        "id": "msg_z",
        "sessionID": "session3",
        "role": "assistant",
        "modelID": "zero-model",
        "providerID": "openai",
        "cost": 0.0,
        "tokens": {
            "input": 0,
            "output": 0,
            "reasoning": 0,
            "cache": { "read": 0, "write": 0 }
        },
        "time": { "created": 1678881600000.0 }
    }"#;
    fs::write(session3.join("msg_z.json"), msg_z).unwrap();
}

/// Add a month whose only non-zero token bucket is reasoning. This catches
/// callers that accidentally fall back to the original MonthlyUsage shape.
fn add_reasoning_only_opencode_message(base: &Path) {
    let session = base.join(".local/share/opencode/storage/message/reasoning-session");
    fs::create_dir_all(&session).unwrap();

    // 2026-02-15 12:00:00 UTC = 1771156800000 ms
    let message = r#"{
        "id": "reasoning-msg",
        "sessionID": "reasoning-session",
        "role": "assistant",
        "modelID": "gpt-4o",
        "providerID": "openai",
        "cost": 0.0321,
        "tokens": {
            "input": 0,
            "output": 0,
            "reasoning": 321,
            "cache": { "read": 0, "write": 0 }
        },
        "time": { "created": 1771156800000.0 }
    }"#;
    fs::write(session.join("reasoning-msg.json"), message).unwrap();
}

fn add_same_named_idless_opencode_messages(base: &Path) {
    let root = base.join(".local/share/opencode/storage/message");
    for (session_id, input, output) in [("idless-session-a", 13, 2), ("idless-session-b", 17, 3)] {
        let session = root.join(session_id);
        fs::create_dir_all(&session).unwrap();
        fs::write(
            session.join("same-name.json"),
            format!(
                r#"{{"sessionID":"{session_id}","role":"assistant","modelID":"gpt-4o","providerID":"openai","tokens":{{"input":{input},"output":{output},"reasoning":0,"cache":{{"read":0,"write":0}}}},"time":{{"created":1700000000000}}}}"#
            ),
        )
        .unwrap();
    }
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

    let litellm_payload = serde_json::to_string(&litellm).unwrap();
    let openrouter_payload = serde_json::to_string(&openrouter).unwrap();
    write_canonical_pricing_cache_files(
        base,
        &litellm_payload,
        &openrouter_payload,
        &litellm_payload,
    );
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
    write_settings_json_at(settings_json_path(base), body);
}

/// Settings for a run that passes `--home <base>` rather than inheriting the
/// sandbox config dir.
///
/// An explicit `--home` bypasses `TOKSCALE_CONFIG_DIR` entirely:
/// `Settings::load_for_home_override` reads
/// `ExplicitHomeConfigLayout::current()` under the given home, which is
/// `.config/tokscale` on Unix and `AppData\Roaming\tokscale` on Windows. That
/// branch is real product behavior, unlike the one [`settings_json_path`] used
/// to carry, so this mirrors it.
fn write_explicit_home_settings_json(base: &Path, body: &str) {
    let path = if cfg!(target_os = "windows") {
        base.join("AppData")
            .join("Roaming")
            .join("tokscale")
            .join("settings.json")
    } else {
        base.join(".config").join("tokscale").join("settings.json")
    };
    write_settings_json_at(path, body);
}

fn write_settings_json_at(path: std::path::PathBuf, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

/// One path on every platform: the child is given `TOKSCALE_CONFIG_DIR`, so
/// where it looks no longer depends on the OS. The Windows arm this replaces
/// mirrored the real `%APPDATA%` layout under the fixture home, which the child
/// never consulted — `dirs::config_dir()` reads the known folder, not `HOME` —
/// so the fixture and the reader disagreed and every settings-driven assertion
/// on Windows saw an absent file.
fn settings_json_path(base: &Path) -> std::path::PathBuf {
    sandbox_config_dir(base).join("settings.json")
}

/// A scan path spelled the way the binary spells it.
///
/// `tokscale clients --json` emits every client's scan root with native
/// separators throughout: the root half comes from the platform (`C:\Users\me`
/// on Windows), and the relative half is pushed component-by-component so no
/// forward slash survives from the `/`-joined client-table literal. An
/// expectation built with `Path::join` would disagree on the relative half's
/// separators (`...\.codex/sessions` against the emitted
/// `...\.codex\sessions`), because `join` only normalizes the junction.
/// Changing the emitter means changing this helper in the same commit (#1048).
fn client_scan_path(home: &Path, relative: &str) -> String {
    let mut path = home.to_path_buf();
    for component in Path::new(relative).components() {
        path.push(component.as_os_str());
    }
    path.to_string_lossy().into_owned()
}

/// Writes a minimal clawdboard account export to `<dir>/export.json` and
/// returns its path, for exercising `tokscale import`.
fn write_clawdboard_export_fixture(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("export.json");
    fs::write(
        &path,
        r#"{
          "dailyAggregates": [
            {
              "date": "2026-05-11",
              "source": "codex",
              "machineId": "m1",
              "inputTokens": 100,
              "outputTokens": 50,
              "cacheCreationTokens": 0,
              "cacheReadTokens": 10,
              "totalCost": "0.50",
              "modelsUsed": ["gpt-5.5"],
              "modelBreakdowns": [
                { "modelName": "gpt-5.5", "cost": 0.5, "inputTokens": 100,
                  "outputTokens": 50, "cacheReadTokens": 10, "cacheCreationTokens": 0 }
              ]
            }
          ]
        }"#,
    )
    .unwrap();
    path
}

/// Writes a `.claude/.mcp.json` under `home` declaring a locally configured
/// MCP server, so tests can verify that data derived purely from an
/// external export (e.g. `tokscale import`) does not leak it.
fn write_local_mcp_config(home: &Path) {
    let dir = home.join(".claude");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(".mcp.json"),
        r#"{"mcpServers":{"local-only-test-server":{"command":"echo"}}}"#,
    )
    .unwrap();
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

/// The V1 Cherry Studio transcript root under a sandboxed home, spelled the
/// way `PathRoot::AppData` resolves it per platform.
///
/// Built component-by-component rather than from a single `/`-joined literal:
/// `Path::join` only normalizes the junction, so a `"AppData/Roaming/..."`
/// literal keeps its own forward slashes on Windows and the fixture would not
/// exercise the real `AppData\Roaming\CherryStudio\.claude\projects`
/// layout the scanner walks (#1048).
fn cherrystudio_projects_root(base: &Path) -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    let relative = "AppData/Roaming/CherryStudio/.claude/projects";
    #[cfg(target_os = "macos")]
    let relative = "Library/Application Support/CherryStudio/.claude/projects";
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let relative = ".config/CherryStudio/.claude/projects";

    let mut path = base.to_path_buf();
    for component in relative.split('/') {
        path.push(component);
    }
    path
}

/// Four snapshots of one streamed Cherry Studio API call. The final snapshot
/// connects the UUID-only, message-only, and request-only partial records and
/// carries the final streamed output count.
fn write_cherrystudio_connected_alias_transcript(base: &Path) {
    let path = cherrystudio_projects_root(base)
        .join("workspace")
        .join("session.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        concat!(
            r#"{"type":"assistant","uuid":"u","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            "\n",
            r#"{"type":"assistant","message":{"id":"m","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            "\n",
            r#"{"type":"assistant","requestId":"r","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            "\n",
            r#"{"type":"assistant","uuid":"u","requestId":"r","message":{"id":"m","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":300}}}"#,
            "\n"
        ),
    )
    .unwrap();
}

/// A historical Cherry Studio call followed by a timestamp-less streamed
/// replay. The replay grows output usage but must not take the transcript mtime
/// as the call timestamp when its component has a valid event timestamp.
fn write_cherrystudio_historical_replay_without_timestamp(base: &Path) {
    let path = cherrystudio_projects_root(base)
        .join("workspace")
        .join("historical.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        concat!(
            r#"{"type":"assistant","requestId":"historical-request","timestamp":"2024-01-02T03:04:05.000Z","message":{"id":"historical-message","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":10}}}"#,
            "\n",
            r#"{"type":"assistant","requestId":"historical-request","message":{"id":"historical-message","model":"deepseek-v4-pro","usage":{"input_tokens":100,"output_tokens":300}}}"#,
            "\n"
        ),
    )
    .unwrap();
}

/// The DSH transcript root under a sandboxed home: `<home>/.dsh/sessions`.
///
/// DSH keeps all user data under one root, resolved as an explicit config
/// path, then `$DSH_HOME`, then `~/.dsh` (`util/home-paths`, `resolveDshHome`);
/// the shipped base composition pins the session store to `sessions` beneath
/// it. `cmd_with_home` clears `DSH_HOME` so this test reads the sandbox.
fn dsh_sessions_root(base: &Path) -> std::path::PathBuf {
    base.join(".dsh").join("sessions")
}

/// One zstd DSH transcript with a reasoning-bearing call, written to
/// `<home>/.dsh/sessions/<encoded-cwd>/<session-id>/session.jsonl.zstd`.
///
/// Usage numbers are the committed vendor pair from
/// `examples/acp-agent/tests/snapshots/subagent-fork-in-process`.
fn write_dsh_zstd_session(base: &Path) {
    let dir = dsh_sessions_root(base)
        .join("-tmp-dsh-workspace")
        .join("96cf59c9-b347-48b9-b234-a5200913ad05");
    fs::create_dir_all(&dir).unwrap();
    let payload = concat!(
        r#"{"type":"session","version":0,"id":"96cf59c9-b347-48b9-b234-a5200913ad05","createdAt":1783352134832,"cwd":"/tmp/dsh-workspace","delegationDepth":0}"#,
        "
",
        r#"{"type":"assistant/message","seq":39,"time":1785730448979,"data":{"turn":1,"message":{"id":"7ac2e3d7-d558-4b24-b71e-40fc2f42216d","source":{"kind":"model","provider":"deepseek","model":"deepseek-reasoner"}},"usage":{"inputTokens":2885,"outputTokens":25,"cacheReadTokens":0,"reasoningTokens":23}}}"#,
        "
"
    );
    fs::write(
        dir.join("session.jsonl.zstd"),
        zstd::encode_all(payload.as_bytes(), 3).unwrap(),
    )
    .unwrap();
}

/// A DSH fork pair: the parent transcript plus the child whose seeded prefix
/// repeats the parent's seq-39 call verbatim under a different session id.
fn write_dsh_fork_pair(base: &Path) {
    write_dsh_zstd_session(base);

    let dir = dsh_sessions_root(base)
        .join("-tmp-dsh-workspace")
        .join("ada8966c-9fa3-441b-8721-37ff1e795e6a");
    fs::create_dir_all(&dir).unwrap();
    let payload = concat!(
        r#"{"type":"session","version":0,"id":"ada8966c-9fa3-441b-8721-37ff1e795e6a","createdAt":1783352137161,"cwd":"/tmp/dsh-workspace","parentSession":"96cf59c9-b347-48b9-b234-a5200913ad05","seedLength":42,"origin":"subagent","delegationDepth":1}"#,
        "
",
        r#"{"type":"assistant/message","seq":39,"time":1785730448979,"data":{"turn":1,"message":{"id":"7ac2e3d7-d558-4b24-b71e-40fc2f42216d","source":{"kind":"model","provider":"deepseek","model":"deepseek-reasoner"}},"usage":{"inputTokens":2885,"outputTokens":25,"cacheReadTokens":0,"reasoningTokens":23}}}"#,
        "
",
        r#"{"type":"assistant/message","seq":96,"time":1786358035361,"data":{"turn":2,"message":{"id":"cdc56e00-c648-4669-92b2-7299e41cb743","source":{"kind":"model","provider":"deepseek","model":"deepseek-reasoner"}},"usage":{"inputTokens":97,"outputTokens":39,"cacheReadTokens":2816,"reasoningTokens":34}}}"#,
        "
"
    );
    fs::write(
        dir.join("session.jsonl.zstd"),
        zstd::encode_all(payload.as_bytes(), 3).unwrap(),
    )
    .unwrap();
}

/// A DSH transcript pair whose child header carries no `seedLength`, so only
/// the per-call `message.id` marks the seeded rows as the parent's work.
fn write_dsh_seeded_pair_without_seed_length(base: &Path) {
    let row = concat!(
        r#"{"type":"assistant/message","seq":39,"time":1785730448979,"data":{"turn":1,"message":{"id":"7ac2e3d7-d558-4b24-b71e-40fc2f42216d","source":{"kind":"model","provider":"deepseek","model":"deepseek-reasoner"}},"usage":{"inputTokens":2885,"outputTokens":25,"cacheReadTokens":0,"reasoningTokens":23}}}"#,
        "\n"
    );

    for (session_id, header) in [
        (
            "96cf59c9-b347-48b9-b234-a5200913ad05",
            concat!(
                r#"{"type":"session","version":0,"id":"96cf59c9-b347-48b9-b234-a5200913ad05","createdAt":1783352134832,"cwd":"/tmp/dsh-workspace"}"#,
                "\n"
            ),
        ),
        (
            "ada8966c-9fa3-441b-8721-37ff1e795e6a",
            concat!(
                r#"{"type":"session","version":0,"id":"ada8966c-9fa3-441b-8721-37ff1e795e6a","createdAt":1783352137161,"cwd":"/tmp/dsh-workspace","parentSession":"96cf59c9-b347-48b9-b234-a5200913ad05"}"#,
                "\n"
            ),
        ),
    ] {
        let dir = dsh_sessions_root(base)
            .join("-tmp-dsh-workspace")
            .join(session_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("session.jsonl.zstd"),
            zstd::encode_all(format!("{header}{row}").as_bytes(), 3).unwrap(),
        )
        .unwrap();
    }
}

/// Two unrelated summaries with identical local coordinates but distinct
/// compaction UUIDs, plus one parent/child replay sharing a UUID. This is the
/// complete #1187 collision boundary: retain two calls from the first leg and
/// collapse the copied call from the second.
fn write_dsh_compaction_identity_fixture(base: &Path) {
    let fixtures = [
        (
            "unrelated-a",
            r#"{"type":"session","id":"unrelated-a","createdAt":1,"cwd":"/tmp/dsh-workspace"}"#,
            r#"{"type":"compaction/summary","seq":4,"time":1786669450002,"data":{"compactionId":"compact-a","message":{"source":{"provider":"p","model":"m"}},"usage":{"inputTokens":10,"outputTokens":20}}}"#,
        ),
        (
            "unrelated-b",
            r#"{"type":"session","id":"unrelated-b","createdAt":2,"cwd":"/tmp/dsh-workspace"}"#,
            r#"{"type":"compaction/summary","seq":4,"time":1786669450002,"data":{"compactionId":"compact-b","message":{"source":{"provider":"p","model":"m"}},"usage":{"inputTokens":10,"outputTokens":20}}}"#,
        ),
        (
            "fork-parent",
            r#"{"type":"session","id":"fork-parent","createdAt":3,"cwd":"/tmp/dsh-workspace"}"#,
            r#"{"type":"compaction/summary","seq":9,"time":1786669450003,"data":{"compactionId":"compact-copied","message":{"source":{"provider":"p","model":"m"}},"usage":{"inputTokens":30,"outputTokens":40}}}"#,
        ),
        (
            "fork-child",
            r#"{"type":"session","id":"fork-child","createdAt":4,"cwd":"/tmp/dsh-workspace","parentSession":"fork-parent"}"#,
            r#"{"type":"compaction/summary","seq":9,"time":1786669450003,"data":{"compactionId":"compact-copied","message":{"source":{"provider":"p","model":"m"}},"usage":{"inputTokens":30,"outputTokens":40}}}"#,
        ),
    ];

    for (session_id, header, summary) in fixtures {
        let dir = dsh_sessions_root(base)
            .join("-tmp-dsh-workspace")
            .join(session_id);
        fs::create_dir_all(&dir).unwrap();
        let payload = format!("{header}\n{summary}\n");
        fs::write(
            dir.join("session.jsonl.zstd"),
            zstd::encode_all(payload.as_bytes(), 3).unwrap(),
        )
        .unwrap();
    }
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
    fs::write(cache_dir.join("usage.json"), r#"{"usageEventsDisplay":[]}"#).unwrap();
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
fn test_codex_activity_command_help() {
    let mut cmd = cargo_bin_cmd!("tokscale");
    cmd.args(["codex", "activity", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "opt-in Codex account-activity snapshot",
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
fn test_import_stdout_is_pure_json() {
    // `tokscale import export.json > out.json` must produce a valid JSON
    // file: no human-readable banners/summaries/warnings on stdout, only
    // the serialized payload (matching how `tokscale graph` behaves).
    let home = TempDir::new().unwrap();
    let export_path = write_clawdboard_export_fixture(home.path());

    let output = cmd_with_home(home.path())
        .args(["import", export_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout was not pure JSON: {e}\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert!(json["summary"]["totalTokens"].as_i64().unwrap() > 0);

    // The human-readable banner belongs on stderr, not stdout.
    assert!(String::from_utf8_lossy(&output.stderr).contains("Import Usage Data"));
}

#[test]
fn test_import_does_not_leak_local_mcp_servers() {
    // Reusing the graph/submit converter must not embed the local
    // machine's configured MCP server names into data derived purely from
    // a third-party clawdboard export.
    let home = TempDir::new().unwrap();
    write_local_mcp_config(home.path());
    let export_path = write_clawdboard_export_fixture(home.path());

    let output = cmd_with_home(home.path())
        .args(["import", export_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json.get("mcpServers").is_none() || json["mcpServers"].is_null(),
        "import output should not carry mcpServers: {json}"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("local-only-test-server"));
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
    let mut cmd = cmd_with_home(tmp.path());
    cmd.args(["codex", "accounts", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""accounts": []"#));
}

#[test]
fn test_pricing_command_missing_model() {
    let tmp = TempDir::new().expect("failed to create temp home");
    cmd_with_home(tmp.path()).arg("pricing").assert().failure();
}

#[test]
fn test_headless_command_missing_client() {
    let tmp = TempDir::new().expect("failed to create temp home");
    cmd_with_home(tmp.path()).arg("headless").assert().failure();
}

#[test]
fn test_headless_command_invalid_client() {
    let tmp = TempDir::new().expect("failed to create temp home");
    cmd_with_home(tmp.path())
        .args(["headless", "invalid-client", "test"])
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
        serde_json::json!(client_scan_path(real_home.path(), ".codex/sessions"))
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

/// Unix-only: the premise is a host in `America/Los_Angeles`, and `TZ` does not
/// move `chrono::Local` on Windows. See `graph_day_buckets` for the full note.
#[test]
#[cfg(unix)]
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
fn test_opencode_same_named_idless_files_survive_cold_and_warm_cache() {
    let tmp = create_empty_fixture_dir();
    add_same_named_idless_opencode_messages(tmp.path());

    for pass in ["cold", "warm"] {
        let output = cmd_with_home(tmp.path())
            .args(["models", "--json", "--client", "opencode", "--no-spinner"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{pass} cache pass failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["totalMessages"].as_i64(), Some(2), "{pass} cache pass");
        assert_eq!(json["totalInput"].as_i64(), Some(30), "{pass} cache pass");
        assert_eq!(json["totalOutput"].as_i64(), Some(5), "{pass} cache pass");
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

#[test]
fn test_mcode_headless_stream_counts_identically_cold_and_warm_cache() {
    let tmp = create_empty_fixture_dir();
    let stream_dir = tmp.path().join(".config/tokscale/headless/mcode");
    fs::create_dir_all(&stream_dir).unwrap();
    fs::write(
        stream_dir.join("run.jsonl"),
        concat!(
            r#"{"type":"message","message":{"turnId":"turn-1","role":"assistant","timestamp":1786800000000,"usage":{"inputTokens":1000,"outputTokens":250,"cacheReadTokens":400,"cacheWriteTokens":50}}}"#,
            "\n",
            r#"{"schemaVersion":1,"type":"exec.result","sessionId":"session-1","turnId":"turn-1","status":"succeeded","model":{"providerId":"minimax","modelId":"MiniMax-M2.5","variant":"fast"},"durationMs":10}"#,
            "\n"
        ),
    )
    .unwrap();

    for pass in ["cold", "warm"] {
        let output = cmd_with_home(tmp.path())
            .args(["models", "--json", "--client", "mcode", "--no-spinner"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{pass} cache pass failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let entries = json["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{pass} cache pass");
        let entry = &entries[0];
        assert_eq!(entry["client"].as_str(), Some("mcode"), "{pass} cache pass");
        assert_eq!(
            entry["provider"].as_str(),
            Some("minimax"),
            "{pass} cache pass"
        );
        assert_eq!(
            entry["model"].as_str(),
            Some("minimax-m2.5"),
            "{pass} cache pass"
        );
        assert_eq!(entry["input"].as_i64(), Some(1000), "{pass} cache pass");
        assert_eq!(entry["output"].as_i64(), Some(250), "{pass} cache pass");
        assert_eq!(entry["cacheRead"].as_i64(), Some(400), "{pass} cache pass");
        assert_eq!(entry["cacheWrite"].as_i64(), Some(50), "{pass} cache pass");
        assert_eq!(json["totalMessages"].as_i64(), Some(1), "{pass} cache pass");
    }
}

/// The Windows regression guard for the two tests below.
///
/// Both of them assert on parsed totals, so when discovery silently reads the
/// wrong root they fail as `Some(0)` vs `Some(1)` with nothing pointing at the
/// path. `PathRoot::AppData` used to resolve to the `FOLDERID_RoamingAppData`
/// known folder under env roots, which no environment variable can redirect,
/// so on Windows the scan walked the live profile instead of this sandbox and
/// the fixture was never seen. Assert the emitted scan root directly.
#[test]
fn test_clients_json_cherrystudio_scan_root_stays_inside_the_sandboxed_home() {
    let tmp = create_empty_fixture_dir();
    write_cherrystudio_connected_alias_transcript(tmp.path());

    let output = cmd_with_home(tmp.path())
        .args(["clients", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let cherry = json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["client"] == "cherrystudio")
        .expect("cherrystudio must appear in `clients --json`");

    assert_eq!(
        cherry["sessionsPath"],
        serde_json::json!(cherrystudio_projects_root(tmp.path()).to_string_lossy()),
        "Cherry Studio's scan root must follow the sandboxed home, not the machine's app-data folder"
    );
}

/// DSH resolves its root from `$DSH_HOME` with a `~/.dsh` fallback, so a
/// sandboxed `HOME` must move the scan root with it. Asserting the emitted
/// path directly keeps a discovery regression from surfacing as a bare
/// `Some(0)` vs `Some(1)` totals mismatch in the tests below.
#[test]
fn test_clients_json_dsh_scan_root_stays_inside_the_sandboxed_home() {
    let tmp = create_empty_fixture_dir();
    write_dsh_zstd_session(tmp.path());

    let output = cmd_with_home(tmp.path())
        .args(["clients", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let dsh = json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["client"] == "dsh")
        .expect("dsh must appear in `clients --json`");

    assert_eq!(
        dsh["sessionsPath"],
        serde_json::json!(dsh_sessions_root(tmp.path()).to_string_lossy()),
        "DSH's scan root must follow the sandboxed home, not the machine's ~/.dsh"
    );
}

/// The zstd lane's cache parity guard.
///
/// The first pass decodes the transcript and writes the source cache; the
/// second serves it warm. Both must report the same reasoning-corrected
/// buckets — `reasoningTokens` is a subset of `outputTokens`, so 25 output
/// tokens with 23 reasoning tokens leave 2 in the additive output bucket.
#[test]
fn test_dsh_zstd_transcript_counts_identically_cold_and_warm_cache() {
    let tmp = create_empty_fixture_dir();
    write_dsh_zstd_session(tmp.path());

    for pass in ["cold", "warm"] {
        let output = cmd_with_home(tmp.path())
            .args(["models", "--json", "--client", "dsh", "--no-spinner"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{pass} cache pass failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let entries = json["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{pass} cache pass");
        let entry = &entries[0];
        assert_eq!(entry["client"].as_str(), Some("dsh"), "{pass} cache pass");
        assert_eq!(
            entry["provider"].as_str(),
            Some("deepseek"),
            "{pass} cache pass"
        );
        assert_eq!(
            entry["model"].as_str(),
            Some("deepseek-reasoner"),
            "{pass} cache pass"
        );
        assert_eq!(entry["input"].as_i64(), Some(2885), "{pass} cache pass");
        assert_eq!(entry["output"].as_i64(), Some(2), "{pass} cache pass");
        assert_eq!(entry["reasoning"].as_i64(), Some(23), "{pass} cache pass");
        assert_eq!(json["totalMessages"].as_i64(), Some(1), "{pass} cache pass");
    }
}

/// A forked DSH session copies the parent's completed prefix into the child
/// transcript verbatim, so the pair must still bill the seeded call once.
#[test]
fn test_dsh_forked_session_counts_the_seeded_prefix_once_cold_and_warm_cache() {
    let tmp = create_empty_fixture_dir();
    write_dsh_fork_pair(tmp.path());

    for pass in ["cold", "warm"] {
        let output = cmd_with_home(tmp.path())
            .args(["models", "--json", "--client", "dsh", "--no-spinner"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{pass} cache pass failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        // Two distinct calls: the parent's seq-39 message and the child's own
        // seq-96 message. The child's copy of seq 39 is the parent's work.
        assert_eq!(json["totalMessages"].as_i64(), Some(2), "{pass} cache pass");
        assert_eq!(
            json["totalInput"].as_i64(),
            Some(2885 + 97),
            "{pass} cache pass"
        );
        assert_eq!(
            json["totalOutput"].as_i64(),
            Some(2 + 5),
            "{pass} cache pass"
        );
    }
}

/// The lane's cross-file dedup pass, isolated from the seq boundary.
///
/// Without `seedLength` in the header the parser cannot tell a seeded row from
/// an owned one, so the duplicate is only collapsed by the per-call
/// `message.id` dedup key, cold and warm alike.
#[test]
fn test_dsh_repeated_message_ids_across_transcripts_count_once_cold_and_warm_cache() {
    let tmp = create_empty_fixture_dir();
    write_dsh_seeded_pair_without_seed_length(tmp.path());

    for pass in ["cold", "warm"] {
        let output = cmd_with_home(tmp.path())
            .args(["models", "--json", "--client", "dsh", "--no-spinner"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{pass} cache pass failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["totalMessages"].as_i64(), Some(1), "{pass} cache pass");
        assert_eq!(json["totalInput"].as_i64(), Some(2885), "{pass} cache pass");
        assert_eq!(json["totalOutput"].as_i64(), Some(2), "{pass} cache pass");
    }
}

#[test]
fn test_dsh_compaction_identity_is_global_cold_and_warm_cache() {
    let tmp = create_empty_fixture_dir();
    write_dsh_compaction_identity_fixture(tmp.path());

    for pass in ["cold", "warm"] {
        let output = cmd_with_home(tmp.path())
            .args(["models", "--json", "--client", "dsh", "--no-spinner"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{pass} cache pass failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["totalMessages"].as_i64(), Some(3), "{pass} cache pass");
        assert_eq!(json["totalInput"].as_i64(), Some(50), "{pass} cache pass");
        assert_eq!(json["totalOutput"].as_i64(), Some(80), "{pass} cache pass");
    }
}

#[test]
fn test_cherrystudio_connected_aliases_count_once_cold_and_warm_cache() {
    let tmp = create_empty_fixture_dir();
    write_cherrystudio_connected_alias_transcript(tmp.path());

    // The first invocation parses the transcript and writes the source cache;
    // the second reads that cache. Both must retain the parser's one-call
    // contribution rather than reviving the three partial snapshots.
    for pass in ["cold", "warm"] {
        let output = cmd_with_home(tmp.path())
            .args([
                "models",
                "--json",
                "--client",
                "cherrystudio",
                "--no-spinner",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{pass} cache pass failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["totalMessages"].as_i64(), Some(1), "{pass} cache pass");
        assert_eq!(json["totalInput"].as_i64(), Some(100), "{pass} cache pass");
        assert_eq!(json["totalOutput"].as_i64(), Some(300), "{pass} cache pass");
    }
}

#[test]
fn test_cherrystudio_historical_timestamp_survives_timestampless_replay_cold_and_warm() {
    let tmp = create_empty_fixture_dir();
    write_cherrystudio_historical_replay_without_timestamp(tmp.path());

    // The first pass parses the source; the second reads its cache. Neither
    // may promote the historical call into today because a later replay lacks
    // an event timestamp. The replay's cumulative output still contributes.
    for pass in ["cold", "warm"] {
        let output = cmd_with_home(tmp.path())
            .args([
                "models",
                "--json",
                "--today",
                "--client",
                "cherrystudio",
                "--no-spinner",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{pass} cache pass failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["totalMessages"].as_i64(), Some(0), "{pass} cache pass");
        assert_eq!(json["totalInput"].as_i64(), Some(0), "{pass} cache pass");
        assert_eq!(json["totalOutput"].as_i64(), Some(0), "{pass} cache pass");
    }

    let output = cmd_with_home(tmp.path())
        .args([
            "models",
            "--json",
            "--client",
            "cherrystudio",
            "--no-spinner",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "totals report failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["totalMessages"].as_i64(), Some(1));
    assert_eq!(json["totalInput"].as_i64(), Some(100));
    assert_eq!(json["totalOutput"].as_i64(), Some(300));
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

/// Unix-only: the fixture's expected date is UTC+1 day, which only holds if the
/// child really runs in `Pacific/Kiritimati`. `TZ` does not move
/// `chrono::Local` on Windows — see the note on `graph_day_buckets`.
#[test]
#[cfg(unix)]
fn test_submit_dry_run_preserves_local_date_ahead_of_utc() {
    let (tmp, expected_local_date) = create_positive_utc_offset_submit_fixture_dir();

    cmd_with_home(tmp.path())
        .env("TZ", "Pacific/Kiritimati")
        .env("TOKSCALE_API_TOKEN", "test-token")
        .args(["submit", "--client", "opencode", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Date range: {expected_local_date} to {expected_local_date}"
        )))
        .stdout(predicate::str::contains("Total tokens: 1,750"));
}

/// Regression: an unbounded `submit` re-scans every client directory, and the
/// silent wait used to give no hint of what was being scanned or how long it
/// took. The scope label and the elapsed time are the observability for that
/// wait; neither changes what gets submitted.
#[test]
fn test_submit_reports_scan_scope_and_elapsed() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .env("TOKSCALE_API_TOKEN", "test-token")
        .args([
            "--no-spinner",
            "submit",
            "--client",
            "opencode",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Scanning local session data (1 client, full history)...",
        ))
        .stdout(predicate::str::is_match(r"Scanned in \d+\.\d+s\.").unwrap())
        // The scan is already narrowed to one client, so the tip has nothing
        // left to suggest.
        .stdout(predicate::str::contains("Tip:").not());

    cmd_with_home(tmp.path())
        .env("TOKSCALE_API_TOKEN", "test-token")
        .args([
            "--no-spinner",
            "submit",
            "--client",
            "opencode",
            "--since",
            "2026-01-01",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Scanning local session data (1 client, since 2026-01-01)...",
        ))
        .stdout(predicate::str::contains("Tip:").not());

    cmd_with_home(tmp.path())
        .env("TOKSCALE_API_TOKEN", "test-token")
        .args([
            "--no-spinner",
            "submit",
            "--client",
            "synthetic",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Scanning local session data ({} clients, full history)...",
            tokscale_core::ClientId::COUNT
        )));
}

/// The tip may name `--client` and nothing else.
///
/// `--since` does not shorten the scan: the date filters are `retain`
/// predicates run over already-parsed messages, so the same files are read
/// either way. It also clears `fullHistory` on the scan scope, and
/// `planParserHighWaterSubmission` (packages/frontend/src/lib/db/parserHighWater.ts)
/// freezes a partial snapshot for every client in `SUPPORTED_VERSIONED_PARSERS`
/// — copilot, droid, antigravity-cli and antigravity. Advertising it as a
/// speedup costs the user data.
#[test]
fn test_submit_tip_recommends_only_the_client_filter() {
    let tmp = create_temp_fixture_dir();
    cmd_with_home(tmp.path())
        .env("TOKSCALE_API_TOKEN", "test-token")
        .args(["--no-spinner", "submit", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Tip: narrow the scan with `--client <id>` for a faster submit.",
        ))
        .stdout(predicate::str::is_match(r"Tip:.*--since").unwrap().not());
}

/// Autosubmit's stdout is the scheduler's log file (`StandardOutPath` in the
/// launchd plist, the systemd/cron redirect elsewhere), so the tip would be
/// appended to it on every scheduled run with nobody at a prompt to act on it.
///
/// This covers the observable behavior end to end. It does not isolate the mode
/// gate on its own — `submit_filters` always hands `run_submit_command` a
/// client list, so the already-narrowed gate would suppress the tip here too.
/// `client_scope_tip_is_interactive_only` in main.rs is the test that flips
/// only the mode.
///
/// The home is deliberately empty of session data, and that is load-bearing.
/// `autosubmit run` is the one submit path in this file that is not a dry run
/// (`run_submit_command(.., dry_run = false, ..)` at the `AutosubmitRunDecision`
/// call site), so any usage at all carries it past the `total_tokens == 0`
/// short-circuit and into a real `POST {api}/api/submit` with
/// `Authorization: Bearer test-token`. With `create_temp_fixture_dir` here the
/// run reached "Submitting to server..." and issued that POST against
/// `https://tokscale.ai`; the loopback proxies were all that kept the packet on
/// the machine. `cmd_with_home` now also pins `TOKSCALE_API_URL` to a dead
/// loopback port, so no test resolves the production endpoint any more — but
/// an empty home is still what keeps *this* test off the submit path entirely.
/// It ends the run at "No usage data found to submit." instead, which is what
/// the last assertion pins, and the scope line and the tip gate are observable
/// either way.
///
/// `prime_pricing_cache` is still needed: the pricing load runs before the
/// usage check and ignores `TOKSCALE_PRICING_CACHE_ONLY`.
#[test]
fn test_autosubmit_run_omits_the_scan_scope_tip() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    prime_pricing_cache(tmp.path());
    write_settings_json(tmp.path(), r#"{"autosubmit":{"enabled":true}}"#);

    cmd_with_home(tmp.path())
        .env("TOKSCALE_API_TOKEN", "test-token")
        .args(["--no-spinner", "autosubmit", "run", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Scanning local session data ("))
        .stdout(predicate::str::is_match(r"Scanned in \d+\.\d+s\.").unwrap())
        .stdout(predicate::str::contains("Tip:").not())
        .stdout(predicate::str::contains("No usage data found to submit."));
}

/// Every command `cmd_with_home` builds must resolve the API to a dead loopback
/// port instead of production.
///
/// `auth::get_api_base_url` reads `TOKSCALE_API_URL` and falls back to
/// `https://tokscale.ai`, and `submit`, `autosubmit run`, `login` and
/// `delete-submitted-data` all format their endpoint onto whatever it returns.
/// Before the pin the harness neither set nor removed that variable, so a
/// non-dry-run submit under a fixture with usage sent
/// `POST https://tokscale.ai/api/submit` carrying the test's bearer token, with
/// only the loopback proxies stopping it, and a developer with
/// `TOKSCALE_API_URL` exported silently redirected every test to their own
/// server.
///
/// `login --token` is the cheapest command that reaches the API with no fixture
/// at all: the `tt_` prefix check passes locally, then it GETs
/// `{api}/api/auth/token` and prints the request URL when the connection fails.
/// Asserting on that URL proves the child process resolved the pinned base,
/// which reading the env map back off the `Command` would not.
#[test]
fn test_cmd_with_home_keeps_api_calls_off_production() {
    let tmp = TempDir::new().expect("failed to create temp dir");

    cmd_with_home(tmp.path())
        .args(["--no-spinner", "login", "--token", "tt_not_a_real_token"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "url (http://127.0.0.1:9/api/auth/token)",
        ))
        .stderr(predicate::str::contains("tokscale.ai").not());
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
fn test_empty_report_total_cost_is_positive_zero() {
    // f64's Sum identity is -0.0; without normalization an empty report
    // serializes as "totalCost": -0.0.
    let tmp = TempDir::new().unwrap();
    for subcmd in ["models", "monthly", "hourly"] {
        let output = offline_cmd_with_home(tmp.path())
            .args([subcmd, "--json", "--client", "crush", "--no-spinner"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{subcmd} stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("-0.0"),
            "{subcmd} JSON contains negative zero: {stdout}"
        );

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let total_cost = json["totalCost"].as_f64().unwrap();
        assert_eq!(total_cost, 0.0, "{subcmd} totalCost should be zero");
        assert!(
            total_cost.is_sign_positive(),
            "{subcmd} totalCost serialized as -0.0"
        );
    }
}

#[test]
fn test_hide_zero_drops_all_zero_entries_but_keeps_totals() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    add_zero_usage_opencode_message(tmp.path());
    add_reasoning_only_opencode_message(tmp.path());

    let run = |args: &[&str]| -> serde_json::Value {
        let output = offline_cmd_with_home(tmp.path())
            .args(args)
            .args(["--json", "--client", "opencode", "--no-spinner"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?} stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    };

    // models: the all-zero (opencode, zero-model) row disappears with the flag
    let full = run(&["models"]);
    let filtered = run(&["models", "--hide-zero"]);
    let has_zero_model = |json: &serde_json::Value| {
        json["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["model"] == "zero-model")
    };
    assert!(has_zero_model(&full), "zero row must show without the flag");
    assert!(!has_zero_model(&filtered), "--hide-zero must drop the row");
    assert!(filtered["entries"].as_array().unwrap().iter().all(|e| {
        e["input"].as_i64().unwrap() != 0
            || e["output"].as_i64().unwrap() != 0
            || e["cost"].as_f64().unwrap() != 0.0
    }));
    // totals are display-independent: hidden rows still count
    assert_eq!(full["totalMessages"], filtered["totalMessages"]);
    assert_eq!(full["totalCost"], filtered["totalCost"]);

    // monthly: the all-zero 2023-03 bucket disappears with the flag
    let full = run(&["monthly"]);
    let filtered = run(&["monthly", "--hide-zero"]);
    let months = |json: &serde_json::Value| -> Vec<String> {
        json["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["month"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(months(&full).contains(&"2023-03".to_string()));
    assert!(!months(&filtered).contains(&"2023-03".to_string()));
    assert!(
        months(&filtered).contains(&"2026-02".to_string()),
        "--hide-zero must retain a reasoning-only month"
    );
    let reasoning_month = filtered["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["month"] == "2026-02")
        .unwrap();
    assert_eq!(reasoning_month["reasoning"], 321);
    assert_eq!(full["totalCost"], filtered["totalCost"]);

    // hourly: exactly one all-zero hour bucket disappears with the flag
    let full = run(&["hourly"]);
    let filtered = run(&["hourly", "--hide-zero"]);
    assert_eq!(
        full["entries"].as_array().unwrap().len(),
        filtered["entries"].as_array().unwrap().len() + 1
    );
    assert_eq!(full["totalCost"], filtered["totalCost"]);
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
    assert!(first.get("reasoning").is_some());
    assert!(first.get("messageCount").is_some());
    assert!(first.get("cost").is_some());
}

#[test]
fn test_monthly_v2_outputs_reasoning_in_json_and_table() {
    let tmp = create_temp_fixture_dir();
    add_reasoning_only_opencode_message(tmp.path());

    let output = cmd_with_home(tmp.path())
        .args(["monthly", "--json", "--client", "opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "monthly JSON failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let reasoning_month = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["month"] == "2026-02")
        .expect("reasoning-only month must be retained");
    assert_eq!(reasoning_month["input"], 0);
    assert_eq!(reasoning_month["output"], 0);
    assert_eq!(reasoning_month["cacheRead"], 0);
    assert_eq!(reasoning_month["cacheWrite"], 0);
    assert_eq!(reasoning_month["reasoning"], 321);

    cmd_with_home(tmp.path())
        .args(["monthly", "--client", "opencode", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2026-02"))
        // With no input/output/cache tokens, this finite Cost/1M value proves
        // the table's token total includes the 321 reasoning tokens.
        .stdout(predicate::str::contains("$100.00/M"));
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
    write_explicit_home_settings_json(
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

/// Adds a third OpenCode session whose single message reports the same physical
/// model as session 1 (claude sonnet 4) under a channel-variant name string
/// (`claude-sonnet-4-cc`), so model-alias folding can be exercised end to end.
fn add_alias_variant_message(tmp: &Path) {
    let session3 = tmp.join(".local/share/opencode/storage/message/session3");
    fs::create_dir_all(&session3).unwrap();
    let msg = r#"{
        "id": "msg_d",
        "sessionID": "session3",
        "role": "assistant",
        "modelID": "claude-sonnet-4-cc",
        "providerID": "anthropic",
        "cost": 0.04,
        "tokens": {
            "input": 400,
            "output": 100,
            "reasoning": 0,
            "cache": { "read": 0, "write": 0 }
        },
        "time": { "created": 1718460000000.0, "completed": 1718460002000.0 }
    }"#;
    fs::write(session3.join("msg_d.json"), msg).unwrap();
}

/// Writes a tokscale `settings.json` with the given `modelAliases` object into
/// the sandbox config dir `cmd_with_home` points the child at.
fn write_model_aliases(tmp: &Path, aliases_json: &str) {
    write_settings_json(tmp, &format!(r#"{{"modelAliases": {aliases_json}}}"#));
}

fn models_by_name(tmp: &Path) -> serde_json::Value {
    let output = cmd_with_home(tmp)
        .args(["models", "--json", "--client", "opencode", "--no-spinner"])
        .args(["--group-by", "model"])
        .output()
        .unwrap();
    assert!(output.status.success(), "command failed: {output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn test_models_alias_folds_variants_into_one_row() {
    let tmp = create_temp_fixture_dir();
    add_alias_variant_message(tmp.path());
    write_model_aliases(tmp.path(), r#"{"claude-sonnet-4-cc": "claude-sonnet-4"}"#);

    let json = models_by_name(tmp.path());
    let entries = json["entries"].as_array().unwrap();
    let models: Vec<&str> = entries
        .iter()
        .map(|e| e["model"].as_str().unwrap())
        .collect();

    // The -cc variant folded into the canonical model: exactly one
    // claude-sonnet-4 row and no claude-sonnet-4-cc row.
    assert!(
        models.contains(&"claude-sonnet-4"),
        "expected folded model, got {models:?}"
    );
    assert!(
        !models.contains(&"claude-sonnet-4-cc"),
        "variant should have folded away, got {models:?}"
    );

    // The variant's tokens merged INTO the canonical row — the fold-sensitive
    // check: session1 (input 1000 + 800) plus the folded -cc variant (400) =
    // 2200. A no-op fold would leave this row at 1800 and emit a separate
    // claude-sonnet-4-cc row instead.
    let folded = entries
        .iter()
        .find(|e| e["model"] == "claude-sonnet-4")
        .unwrap();
    assert_eq!(
        folded["input"].as_i64().unwrap(),
        2200,
        "folded row must include the variant's tokens, got {folded:?}"
    );
    assert!(
        folded.get("mergedClients").is_some(),
        "folded entry should retain mergedClients"
    );

    // Per-entry sums still reconcile with report totals — the fold must not
    // double-count or drop tokens.
    let sum_input: i64 = entries.iter().map(|e| e["input"].as_i64().unwrap()).sum();
    assert_eq!(sum_input, json["totalInput"].as_i64().unwrap());
}

#[test]
fn test_models_alias_folds_in_monthly_report() {
    // The fold happens inside normalize_model_for_grouping, so it must apply at
    // every grouping call site, not only the models report. Prove it on the
    // monthly report (a distinct call site) too.
    let tmp = create_temp_fixture_dir();
    add_alias_variant_message(tmp.path());
    write_model_aliases(tmp.path(), r#"{"claude-sonnet-4-cc": "claude-sonnet-4"}"#);

    let output = cmd_with_home(tmp.path())
        .args(["monthly", "--json", "--client", "opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success(), "command failed: {output:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let models: Vec<&str> = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|entry| entry["models"].as_array().unwrap())
        .map(|m| m.as_str().unwrap())
        .collect();
    assert!(
        models.contains(&"claude-sonnet-4"),
        "monthly models should include the canonical name, got {models:?}"
    );
    assert!(
        !models.contains(&"claude-sonnet-4-cc"),
        "monthly report must fold the -cc variant too, got {models:?}"
    );
}

#[test]
fn test_models_alias_absent_is_noop() {
    let tmp = create_temp_fixture_dir();
    add_alias_variant_message(tmp.path());
    // No settings.json / no modelAliases configured.

    let json = models_by_name(tmp.path());
    let models: Vec<&str> = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["model"].as_str().unwrap())
        .collect();

    // Without aliases the variant stays a separate row (opt-in default off).
    assert!(
        models.contains(&"claude-sonnet-4"),
        "expected base model, got {models:?}"
    );
    assert!(
        models.contains(&"claude-sonnet-4-cc"),
        "variant should remain separate without aliases, got {models:?}"
    );
}

#[test]
fn test_models_alias_totals_unchanged() {
    // Folding relabels/merges buckets whose costs were already computed
    // per-message, so grand totals must be identical with and without aliases.
    let without = create_temp_fixture_dir();
    add_alias_variant_message(without.path());
    let base = models_by_name(without.path());

    let with = create_temp_fixture_dir();
    add_alias_variant_message(with.path());
    write_model_aliases(with.path(), r#"{"claude-sonnet-4-cc": "claude-sonnet-4"}"#);
    let aliased = models_by_name(with.path());

    assert_eq!(
        base["totalInput"].as_i64(),
        aliased["totalInput"].as_i64(),
        "totalInput must be unchanged by aliasing"
    );
    assert_eq!(
        base["totalOutput"].as_i64(),
        aliased["totalOutput"].as_i64(),
        "totalOutput must be unchanged by aliasing"
    );
    let base_cost = base["totalCost"].as_f64().unwrap();
    let aliased_cost = aliased["totalCost"].as_f64().unwrap();
    assert!(
        (base_cost - aliased_cost).abs() < 1e-9,
        "totalCost must be unchanged by aliasing: {base_cost} vs {aliased_cost}"
    );
}

#[test]
fn test_alias_folds_local_report_but_not_submitted_payload() {
    // Finding B: a machine-local `modelAliases` config must fold ONLY local
    // presentation/grouping. The model identity that leaves the machine
    // (submit/upload/export payload) must stay raw, or a per-device alias config
    // would rewrite and fragment uploaded history across a user's machines. The
    // `graph` command emits the exact byte shape that `submit` POSTs, so it is
    // the faithful stand-in for the submitted payload.
    let tmp = create_temp_fixture_dir();
    add_alias_variant_message(tmp.path());
    write_model_aliases(tmp.path(), r#"{"claude-sonnet-4-cc": "claude-sonnet-4"}"#);

    // Local models report: the alias DOES fold (canonical name only, no variant).
    let models = models_by_name(tmp.path());
    let displayed: Vec<&str> = models["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["model"].as_str().unwrap())
        .collect();
    assert!(
        displayed.contains(&"claude-sonnet-4"),
        "local report must show the canonical name, got {displayed:?}"
    );
    assert!(
        !displayed.contains(&"claude-sonnet-4-cc"),
        "local report must fold the -cc variant away, got {displayed:?}"
    );

    // Submit/export payload (`graph` prints the submit shape to stdout): the raw
    // variant MUST survive unfolded, both in the models summary and per-day.
    let output = cmd_with_home(tmp.path())
        .args(["graph", "--client", "opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(output.status.success(), "graph command failed: {output:?}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let submitted_models: Vec<&str> = payload["summary"]["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    assert!(
        submitted_models.contains(&"claude-sonnet-4-cc"),
        "submitted payload must keep the RAW model id (alias must not leak), got {submitted_models:?}"
    );

    let per_contribution_models: Vec<&str> = payload["contributions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["clients"].as_array().unwrap())
        .map(|s| s["modelId"].as_str().unwrap())
        .collect();
    assert!(
        per_contribution_models.contains(&"claude-sonnet-4-cc"),
        "submitted per-day contributions must carry the RAW model id, got {per_contribution_models:?}"
    );
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
    let tmp = TempDir::new().expect("failed to create temp dir");
    prime_canonical_sonnet_pricing_cache(tmp.path(), "claude-sonnet-4-20250514");
    let mut cmd = cmd_with_home(tmp.path());
    cmd.args(["pricing", "claude-sonnet-4-20250514", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Pricing for:"))
        .stdout(predicate::str::contains(
            "Matched key: claude-sonnet-4-20250514",
        ))
        .stdout(predicate::str::contains("Source: LiteLLM"))
        .stdout(predicate::str::contains("Resolution:"))
        .stdout(predicate::str::contains("submission-safe"))
        .stdout(predicate::str::contains("Input:  $3.00 / 1M tokens"))
        .stdout(predicate::str::contains("Output: $15.00 / 1M tokens"));
}

/// Sub-cent prices have to survive all the way to the terminal, not just
/// through the formatter. Both keys below are real LiteLLM rows.
#[test]
fn test_pricing_command_renders_sub_cent_prices() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs();
    let payload = serde_json::to_string(&serde_json::json!({
        "timestamp": now,
        "data": {
            // Ordinary input and output, cache read at $0.003625 / 1M. Two
            // decimals blanked only the cache line, so the zero read as a real
            // price sitting next to two believable ones.
            "tencent/deepseek-v4-pro": {
                "input_cost_per_token": 0.000000435,
                "output_cost_per_token": 0.00000087,
                "cache_read_input_token_cost": 0.000000003625,
            },
            // Cache read is exactly $0.005 / 1M, which two decimals rounded up
            // to twice the real price rather than down to zero.
            "gpt-5-nano": {
                "input_cost_per_token": 0.00000005,
                "output_cost_per_token": 0.0000004,
                "cache_read_input_token_cost": 0.000000005,
            },
        },
    }))
    .unwrap();
    let empty = format!(r#"{{"timestamp":{now},"data":{{}}}}"#);
    write_canonical_pricing_cache_files(tmp.path(), &payload, &payload, &empty);

    cmd_with_home(tmp.path())
        .args(["pricing", "tencent/deepseek-v4-pro", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Input:  $0.435 / 1M tokens"))
        .stdout(predicate::str::contains("Output: $0.87 / 1M tokens"))
        .stdout(predicate::str::contains(
            "Cache Read:  $0.003625 / 1M tokens",
        ));

    cmd_with_home(tmp.path())
        .args(["pricing", "gpt-5-nano", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Input:  $0.05 / 1M tokens"))
        .stdout(predicate::str::contains("Cache Read:  $0.005 / 1M tokens"));
}

#[test]
fn test_pricing_list_overrides_renders_sub_cent_prices() {
    // `pricing list-overrides` reads per-1M values straight out of
    // custom-pricing.json without scaling, so it shares the lookup path's
    // formatter but not its arithmetic. Exercise it end to end: a cache-read
    // override below a cent used to print $0.00, which is indistinguishable
    // from an override the user never set.
    let tmp = TempDir::new().expect("failed to create temp dir");
    let config_dir = sandbox_config_dir(tmp.path());
    fs::create_dir_all(&config_dir).expect("failed to create config dir");
    fs::write(
        config_dir.join("custom-pricing.json"),
        serde_json::to_string(&serde_json::json!({
            "models": {
                "acme/sub-cent": {
                    "input_cost_per_million_tokens": 0.435,
                    "output_cost_per_million_tokens": 0.87,
                    "cache_read_input_token_cost_per_million_tokens": 0.003625,
                    "cache_creation_input_token_cost_per_million_tokens": 0.005,
                }
            }
        }))
        .unwrap(),
    )
    .expect("failed to write custom-pricing.json");

    cmd_with_home(tmp.path())
        .args(["pricing", "list-overrides", "--no-spinner"])
        .assert()
        .success()
        .stdout(predicate::str::contains("acme/sub-cent"))
        .stdout(predicate::str::contains("Input:  $0.435 / 1M tokens"))
        .stdout(predicate::str::contains("Output: $0.87 / 1M tokens"))
        .stdout(predicate::str::contains(
            "Cache Read:  $0.003625 / 1M tokens",
        ))
        .stdout(predicate::str::contains("Cache Write: $0.005 / 1M tokens"));
}

#[test]
fn test_pricing_command_json() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    prime_canonical_sonnet_pricing_cache(tmp.path(), "claude-sonnet-4-20250514");
    let output = cmd_with_home(tmp.path())
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
    assert!(json.get("resolution").is_some(), "Missing resolution");
    assert!(json.get("pricing").is_some(), "Missing pricing");

    assert_eq!(json["modelId"], "claude-sonnet-4-20250514");
    assert_eq!(json["matchedKey"], "claude-sonnet-4-20250514");
    assert_eq!(json["source"], "LiteLLM");
    let resolution = &json["resolution"];
    assert!(resolution.get("kind").is_some());
    assert!(resolution.get("candidateCount").is_some());
    assert_eq!(resolution["submissionSafe"].as_bool(), Some(true));

    let pricing = &json["pricing"];
    assert_eq!(pricing["inputCostPerToken"], 0.000003);
    assert_eq!(pricing["outputCostPerToken"], 0.000015);
    assert_eq!(pricing["cacheReadInputTokenCost"], 0.0000003);
    assert_eq!(pricing["cacheCreationInputTokenCost"], 0.00000375);
}

#[test]
fn test_pricing_command_with_provider() {
    let tmp = TempDir::new().expect("failed to create temp dir");
    prime_canonical_sonnet_pricing_cache(tmp.path(), "claude-sonnet-4-20250514");
    let mut cmd = cmd_with_home(tmp.path());
    cmd.args([
        "pricing",
        "claude-sonnet-4-20250514",
        "--provider",
        "litellm",
        "--no-spinner",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "Matched key: claude-sonnet-4-20250514",
    ))
    .stdout(predicate::str::contains("Source: LiteLLM"))
    .stdout(predicate::str::contains("Input:  $3.00 / 1M tokens"))
    .stdout(predicate::str::contains("Output: $15.00 / 1M tokens"));
}

#[test]
fn test_pricing_command_invalid_provider() {
    let tmp = TempDir::new().expect("failed to create temp home");
    cmd_with_home(tmp.path())
        .args([
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
        serde_json::json!(client_scan_path(tmp.path(), ".claude/transcripts"))
    );
    assert_eq!(claude["additionalPaths"][0]["exists"], true);
}

#[test]
fn test_clients_command_includes_claude_transcripts_text() {
    let tmp = create_empty_fixture_dir();
    fs::create_dir_all(tmp.path().join(".claude/transcripts")).unwrap();

    // The transcripts path is spelled with native separators (#1048): on
    // Windows the `~` root renders with backslashes throughout.
    #[cfg(windows)]
    let expected = "additional: ~\\.claude\\transcripts ✓";
    #[cfg(not(windows))]
    let expected = "additional: ~/.claude/transcripts ✓";

    cmd_with_home(tmp.path())
        .arg("clients")
        .assert()
        .success()
        .stdout(predicate::str::contains(expected));
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
fn test_clients_json_finds_stats_cache_under_claude_config_dir() {
    let home = create_empty_fixture_dir();
    let default_cache = home.path().join(".claude/stats-cache.json");
    fs::create_dir_all(default_cache.parent().unwrap()).unwrap();
    fs::write(&default_cache, "{}").unwrap();

    let claude_config_dir = TempDir::new().expect("failed to create Claude config dir");
    let configured_cache = claude_config_dir.path().join("stats-cache.json");
    fs::write(&configured_cache, "{}").unwrap();

    let output = cmd_with_home(home.path())
        .env("CLAUDE_CONFIG_DIR", claude_config_dir.path())
        .args(["clients", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let claude = json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["client"] == "claude")
        .unwrap();
    let stats_cache = claude["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "claude_stats_cache_not_imported")
        .expect("configured stats-cache.json should produce a diagnostic");

    assert_eq!(
        stats_cache["paths"][0]["path"],
        configured_cache.to_string_lossy().as_ref()
    );
}

#[test]
fn test_clients_home_override_ignores_claude_config_dir_for_stats_cache() {
    let explicit_home = create_empty_fixture_dir();
    let expected_cache = explicit_home
        .path()
        .join(".claude")
        .join("stats-cache.json");
    fs::create_dir_all(expected_cache.parent().unwrap()).unwrap();
    fs::write(&expected_cache, "{}").unwrap();

    let process_home = TempDir::new().expect("failed to create process home");
    let conflicting_claude_dir = TempDir::new().expect("failed to create Claude config dir");
    fs::write(conflicting_claude_dir.path().join("stats-cache.json"), "{}").unwrap();

    let output = cmd_with_conflicting_env(process_home.path())
        .env("CLAUDE_CONFIG_DIR", conflicting_claude_dir.path())
        .args([
            "clients",
            "--json",
            "--home",
            explicit_home.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let claude = json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["client"] == "claude")
        .unwrap();
    let stats_cache = claude["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["code"] == "claude_stats_cache_not_imported")
        .expect("the --home stats-cache.json should produce a diagnostic");

    assert_eq!(
        stats_cache["paths"][0]["path"],
        expected_cache.to_string_lossy().as_ref()
    );
}

#[test]
fn test_clients_home_override_ignores_conflicting_claude_config_dir_env() {
    let real_home = create_empty_fixture_dir();
    fs::create_dir_all(real_home.path().join("Library/Application Support/Claude")).unwrap();
    let conflicting_home = TempDir::new().expect("failed to create temp dir");
    let conflicting_claude_dir = TempDir::new().expect("failed to create temp dir");

    let output = cmd_with_conflicting_env(conflicting_home.path())
        .env("CLAUDE_CONFIG_DIR", conflicting_claude_dir.path())
        .args([
            "clients",
            "--json",
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
    let claude = json["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["client"] == "claude")
        .unwrap();
    let diagnostics = claude["diagnostics"].as_array().unwrap();
    let desktop_diagnostic = diagnostics
        .iter()
        .find(|item| item["code"] == "claude_desktop_not_scanned")
        .expect("claude_desktop_not_scanned diagnostic should be present");
    let paths = desktop_diagnostic["paths"].as_array().unwrap();

    for label in ["claudeCodeProjects", "claudeCodeTranscripts"] {
        let path = paths
            .iter()
            .find(|p| p["label"] == label)
            .unwrap_or_else(|| panic!("expected a {label} diagnostic path"))["path"]
            .as_str()
            .unwrap();
        assert!(
            path.starts_with(real_home.path().to_str().unwrap()),
            "{label} should stay under the --home override, got {path}"
        );
        assert!(
            !path.contains(conflicting_claude_dir.path().to_str().unwrap()),
            "{label} must not follow CLAUDE_CONFIG_DIR when --home is explicit, got {path}"
        );
    }
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
fn test_monthly_light_title_uses_pinned_bucket_month() {
    let tmp = create_temp_fixture_dir();
    write_settings_json(
        tmp.path(),
        r#"{"scanner":{"bucketTimezone":"Pacific/Kiritimati"}}"#,
    );
    let expected_month =
        tokscale_core::BucketTimezone::from_pinned_name(Some("Pacific/Kiritimati"))
            .today()
            .format("%B %Y")
            .to_string();

    cmd_with_home(tmp.path())
        .args([
            "monthly",
            "--light",
            "--client",
            "opencode",
            "--no-spinner",
            "--month",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Monthly Token Usage Report ({expected_month})"
        )));
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
fn test_submit_includes_unpriced_usage_at_zero_and_keeps_the_rest() {
    let tmp = create_temp_fixture_dir();
    // Healthy pricing that does not happen to cover the unpriced model below.
    // Without this the fixture holds no pricing at all, which is a different
    // (fatal) condition — see test_submit_offline_without_pricing_cache_fails.
    prime_pricing_cache_with_a_priced_model(tmp.path());
    write_fake_credentials(tmp.path());
    let unpriced_dir = tmp
        .path()
        .join(".local/share/opencode/storage/message/unpriced");
    fs::create_dir_all(&unpriced_dir).unwrap();
    fs::write(
        unpriced_dir.join("unpriced.json"),
        r#"{
            "id": "unpriced",
            "sessionID": "unpriced",
            "role": "assistant",
            "modelID": "genuinely-unpriced-model",
            "providerID": "unknown-provider",
            "cost": 0,
            "tokens": { "input": 1, "output": 0, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1736510400000.0 }
        }"#,
    )
    .unwrap();

    let output = offline_cmd_with_home(tmp.path())
        .args([
            "--no-spinner",
            "submit",
            "--client",
            "opencode",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "one unpriced model should not block covered usage; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "submitting 1 unpriced unknown_provider/genuinely-unpriced-model message(s) (1 tokens) at $0.00"
        ),
        "the zeroed model must be named: {stdout}"
    );
    assert!(
        stdout.contains("Affected days are marked cost-incomplete"),
        "the warning must explain the server-side floor: {stdout}"
    );
    assert!(
        stdout.contains("Hint: unpriced usage is included in token totals with zero cost"),
        "the zero-cost fallback must be followed by a fix hint: {stdout}"
    );
    assert!(
        stdout.contains("custom-pricing.json"),
        "the hint must name the custom pricing file: {stdout}"
    );
    assert!(
        stdout.contains("keyed by the model id alone"),
        "the hint must state the key format, since the warning above prints provider/model but CustomPricing::lookup keys on the model id: {stdout}"
    );
    assert!(
        stdout.contains("submit --dry-run"),
        "the hint must point at the verification command: {stdout}"
    );
    assert!(
        stdout.contains("Dry run - not submitting data."),
        "dry-run must complete without submitting: {stdout}"
    );
}

/// Stealth preview shorthands (`ox-alpha`, `x-preview-f-free`) resolve to
/// their canonical upstream $0 rows, so they submit with the priced usage:
/// no per-row warning, no aggregate line, no fix hint.
#[test]
fn test_submit_stealth_preview_shorthands_upload_without_warnings() {
    let tmp = create_temp_fixture_dir();
    // Mirror the live models.dev rows (both deprecated $0, no cache-write
    // bucket). The shorthand ids below only resolve through them.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs();
    let priced_payload = format!(
        r#"{{"timestamp":{},"data":{{"gpt-4o":{{"input_cost_per_token":0.0000025,"output_cost_per_token":0.00001}}}}}}"#,
        now
    );
    let models_dev_payload = format!(
        r#"{{"timestamp":{},"data":{{"opencode-go/ox-alpha-free":{{"input_cost_per_token":0.0,"output_cost_per_token":0.0,"cache_read_input_token_cost":0.0}},"opencode/x-preview-f-free":{{"input_cost_per_token":0.0,"output_cost_per_token":0.0,"cache_read_input_token_cost":0.0}}}}}}"#,
        now
    );
    write_canonical_pricing_cache_files(
        tmp.path(),
        &priced_payload,
        &priced_payload,
        &models_dev_payload,
    );
    write_fake_credentials(tmp.path());
    let preview_dir = tmp
        .path()
        .join(".local/share/opencode/storage/message/stealth-preview");
    fs::create_dir_all(&preview_dir).unwrap();
    for (file, id, session, provider, model_id) in [
        (
            "ox.json",
            "ox",
            "stealth-preview-ox",
            "stealth",
            "stealth/ox-alpha",
        ),
        (
            "xpreview.json",
            "xpreview",
            "stealth-preview-x",
            "opencode-zen",
            "x-preview-f-free",
        ),
    ] {
        fs::write(
            preview_dir.join(file),
            format!(
                r#"{{
            "id": "{id}",
            "sessionID": "{session}",
            "role": "assistant",
            "modelID": "{model_id}",
            "providerID": "{provider}",
            "cost": 0,
            "tokens": {{ "input": 1000, "output": 500, "reasoning": 0, "cache": {{ "read": 0, "write": 0 }} }},
            "time": {{ "created": 1736510400000.0 }}
        }}"#,
            ),
        )
        .unwrap();
    }

    let output = offline_cmd_with_home(tmp.path())
        .args([
            "--no-spinner",
            "submit",
            "--client",
            "opencode",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "shorthand preview usage must submit cleanly; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("unpriced"),
        "upstream-$0 usage must not warn: {stdout}"
    );
    assert!(
        !stdout.contains("custom-pricing.json"),
        "no fix hint for priced usage: {stdout}"
    );
    assert!(
        stdout.contains("Total tokens: 6,950"),
        "both preview models (3,000 tokens) must be counted on top of the 3,950-token stock fixture: {stdout}"
    );
}

/// Regression: a long proxy-model history fans out to one warning row per
/// provider/model pair and used to bury the submittable summary. Detail rows
/// are capped (mirroring the tokenless-row reporter) with an aggregate total.
#[test]
fn test_submit_caps_unpriced_warning_rows_and_reports_the_total() {
    let tmp = create_temp_fixture_dir();
    prime_pricing_cache_with_a_priced_model(tmp.path());
    write_fake_credentials(tmp.path());
    let unpriced_dir = tmp
        .path()
        .join(".local/share/opencode/storage/message/unpriced-cap");
    fs::create_dir_all(&unpriced_dir).unwrap();
    for i in 0..21 {
        fs::write(
            unpriced_dir.join(format!("unpriced-{i:02}.json")),
            format!(
                r#"{{
            "id": "unpriced-{i:02}",
            "sessionID": "unpriced-cap",
            "role": "assistant",
            "modelID": "genuinely-unpriced-model-{i:02}",
            "providerID": "unknown-provider-{i:02}",
            "cost": 0,
            "tokens": {{ "input": 1, "output": 0, "reasoning": 0, "cache": {{ "read": 0, "write": 0 }} }},
            "time": {{ "created": 1736510400000.0 }}
        }}"#,
            ),
        )
        .unwrap();
    }

    let output = offline_cmd_with_home(tmp.path())
        .args([
            "--no-spinner",
            "submit",
            "--client",
            "opencode",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "capped warnings must not block covered usage; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("... and 1 more"),
        "rows past the cap must be acknowledged: {stdout}"
    );
    assert!(
        stdout.contains(
            "Unpriced total: 21 message(s) (21 tokens) at $0.00 across 21 provider/model(s)."
        ),
        "the aggregate must account for every row including capped ones: {stdout}"
    );
}

/// Regression: the hint tells the user to add custom pricing keyed by the ids
/// printed above it, so an id the cap swallows is an unfixable gap. Nothing
/// else surfaces the rows — `unpriced_submission_usage` is `#[serde(skip)]`
/// and `--dry-run` runs the same reporter — so the capped tail must still be
/// listed by id.
#[test]
fn test_submit_names_the_unpriced_models_past_the_detail_cap() {
    let tmp = create_temp_fixture_dir();
    prime_pricing_cache_with_a_priced_model(tmp.path());
    write_fake_credentials(tmp.path());
    let unpriced_dir = tmp
        .path()
        .join(".local/share/opencode/storage/message/unpriced-tail");
    fs::create_dir_all(&unpriced_dir).unwrap();
    for i in 0..22 {
        fs::write(
            unpriced_dir.join(format!("unpriced-{i:02}.json")),
            format!(
                r#"{{
            "id": "unpriced-{i:02}",
            "sessionID": "unpriced-tail",
            "role": "assistant",
            "modelID": "genuinely-unpriced-model-{i:02}",
            "providerID": "unknown-provider-{i:02}",
            "cost": 0,
            "tokens": {{ "input": 1, "output": 0, "reasoning": 0, "cache": {{ "read": 0, "write": 0 }} }},
            "time": {{ "created": 1736510400000.0 }}
        }}"#,
            ),
        )
        .unwrap();
    }

    let output = offline_cmd_with_home(tmp.path())
        .args([
            "--no-spinner",
            "submit",
            "--client",
            "opencode",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "capped warnings must not block covered usage; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("... and 2 more"),
        "rows past the cap must be acknowledged: {stdout}"
    );
    // Equal tokens and message counts, so the provider/model tiebreak decides:
    // `...-20` and `...-21` are the two rows that fall past the cap.
    assert!(
        stdout.contains("unknown-provider-20/genuinely-unpriced-model-20"),
        "a capped row must still be nameable for custom-pricing: {stdout}"
    );
    assert!(
        stdout.contains("unknown-provider-21/genuinely-unpriced-model-21"),
        "a capped row must still be nameable for custom-pricing: {stdout}"
    );
}

/// Regression: the capped ids were first printed as one unbroken line, so a
/// long unpriced history rebuilt the wall of text the cap exists to prevent.
/// Every capped id still has to appear -- the hint asks the user to price
/// them -- but wrapped, not on a single line.
#[test]
fn test_submit_wraps_the_capped_unpriced_model_names() {
    let tmp = create_temp_fixture_dir();
    prime_pricing_cache_with_a_priced_model(tmp.path());
    write_fake_credentials(tmp.path());
    let unpriced_dir = tmp
        .path()
        .join(".local/share/opencode/storage/message/unpriced-wrap");
    fs::create_dir_all(&unpriced_dir).unwrap();
    for i in 0..40 {
        fs::write(
            unpriced_dir.join(format!("unpriced-{i:02}.json")),
            format!(
                r#"{{
            "id": "unpriced-{i:02}",
            "sessionID": "unpriced-wrap",
            "role": "assistant",
            "modelID": "genuinely-unpriced-model-{i:02}",
            "providerID": "unknown-provider-{i:02}",
            "cost": 0,
            "tokens": {{ "input": 1, "output": 0, "reasoning": 0, "cache": {{ "read": 0, "write": 0 }} }},
            "time": {{ "created": 1736510400000.0 }}
        }}"#,
            ),
        )
        .unwrap();
    }

    let output = offline_cmd_with_home(tmp.path())
        .args([
            "--no-spinner",
            "submit",
            "--client",
            "opencode",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "capped warnings must not block covered usage; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("... and 20 more"),
        "rows past the cap must be acknowledged: {stdout}"
    );

    // Every capped id stays reachable: the hint tells the user to price them.
    for i in 20..40 {
        let id = format!("unknown-provider-{i:02}/genuinely-unpriced-model-{i:02}");
        assert!(
            stdout.contains(&id),
            "capped row {id} must still be nameable for custom-pricing: {stdout}"
        );
    }

    // ... but spread over several lines. Unwrapped, these 20 ids landed on one
    // ~1000-character line.
    let longest = stdout.lines().map(str::len).max().unwrap_or(0);
    assert!(
        longest < 300,
        "no output line may bury the screen; longest was {longest} chars: {stdout}"
    );
}

/// Regression: the cap used to keep the alphabetically first rows, which hid
/// the heaviest usage behind a provider id starting with `z`. The ids the hint
/// asks the user to price must be the ones pricing recovers the most tokens
/// for.
#[test]
fn test_submit_orders_unpriced_warning_rows_by_token_impact() {
    let tmp = create_temp_fixture_dir();
    prime_pricing_cache_with_a_priced_model(tmp.path());
    write_fake_credentials(tmp.path());
    let unpriced_dir = tmp
        .path()
        .join(".local/share/opencode/storage/message/unpriced-rank");
    fs::create_dir_all(&unpriced_dir).unwrap();
    for i in 0..20 {
        fs::write(
            unpriced_dir.join(format!("unpriced-{i:02}.json")),
            format!(
                r#"{{
            "id": "unpriced-{i:02}",
            "sessionID": "unpriced-rank",
            "role": "assistant",
            "modelID": "genuinely-unpriced-model-{i:02}",
            "providerID": "unknown-provider-{i:02}",
            "cost": 0,
            "tokens": {{ "input": 1, "output": 0, "reasoning": 0, "cache": {{ "read": 0, "write": 0 }} }},
            "time": {{ "created": 1736510400000.0 }}
        }}"#,
            ),
        )
        .unwrap();
    }
    // Sorts last by provider id, so the old alphabetical order pushed the
    // single row worth pricing past the 20-row cap.
    fs::write(
        unpriced_dir.join("unpriced-heavy.json"),
        r#"{
            "id": "unpriced-heavy",
            "sessionID": "unpriced-rank",
            "role": "assistant",
            "modelID": "zz-heavy-unpriced-model",
            "providerID": "zz-provider",
            "cost": 0,
            "tokens": { "input": 999, "output": 0, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1736510400000.0 }
        }"#,
    )
    .unwrap();

    let output = offline_cmd_with_home(tmp.path())
        .args([
            "--no-spinner",
            "submit",
            "--client",
            "opencode",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "capped warnings must not block covered usage; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let heaviest =
        "submitting 1 unpriced zz_provider/zz-heavy-unpriced-model message(s) (999 tokens)";
    let lightest = "submitting 1 unpriced unknown-provider-00/genuinely-unpriced-model-00";
    let heaviest_at = stdout
        .find(heaviest)
        .unwrap_or_else(|| panic!("the heaviest row must get a detail line: {stdout}"));
    let lightest_at = stdout
        .find(lightest)
        .unwrap_or_else(|| panic!("the lighter rows must still be listed: {stdout}"));
    assert!(
        heaviest_at < lightest_at,
        "detail rows must be ordered by token impact, heaviest first: {stdout}"
    );
    assert!(
        stdout.contains(
            "Unpriced total: 21 message(s) (1,019 tokens) at $0.00 across 21 provider/model(s)."
        ),
        "the aggregate must still cover every row: {stdout}"
    );
}

/// Regression: a cold cache with no network must not look like "no usage".
///
/// Every fetchable upstream is unreachable here and nothing is cached, so the
/// pricing service loads empty and covers nothing. Zeroing on that basis would
/// make every local cost ungrounded; the day floor prevents a server-side
/// decrease, but the CLI must still report the total pricing outage as failure.
#[test]
fn test_submit_offline_without_pricing_cache_fails() {
    let tmp = create_temp_fixture_dir_without_pricing_cache();
    write_fake_credentials(tmp.path());
    // Cost 0 keeps these messages non-authoritative, so they depend on pricing.
    // The stock fixture's messages carry a positive OpenCode cost, which is
    // provider-reported and would bypass pricing entirely.
    let unpriced_dir = tmp
        .path()
        .join(".local/share/opencode/storage/message/needs-pricing");
    fs::create_dir_all(&unpriced_dir).unwrap();
    fs::write(
        unpriced_dir.join("needs-pricing.json"),
        r#"{
            "id": "needs-pricing",
            "sessionID": "needs-pricing",
            "role": "assistant",
            "modelID": "claude-sonnet-4-20250514",
            "providerID": "anthropic",
            "cost": 0,
            "tokens": { "input": 1000, "output": 500, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1736510400000.0 }
        }"#,
    )
    .unwrap();

    let output = offline_cmd_with_home(tmp.path())
        .args([
            "--no-spinner",
            "submit",
            "--client",
            "opencode",
            "--dry-run",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "a total pricing outage must fail loudly, not report an empty submission;\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("pricing data is unavailable for submission"),
        "the failure must name the pricing outage: {stderr}"
    );
    assert!(
        !stdout.contains("No usage data found to submit."),
        "usage exists; it must not be reported as absent: {stdout}"
    );
}

#[test]
fn test_submit_with_only_generic_gemini_usage_keeps_its_tokens() {
    let tmp = create_empty_fixture_dir();
    let message_dir = tmp
        .path()
        .join(".local/share/opencode/storage/message/gemini-default");
    fs::create_dir_all(&message_dir).unwrap();
    fs::write(
        message_dir.join("gemini-default.json"),
        r#"{
            "id": "gemini-default",
            "sessionID": "gemini-default",
            "role": "assistant",
            "modelID": "gemini-default",
            "providerID": "google",
            "tokens": { "input": 1, "output": 0, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "time": { "created": 1736510400000.0 }
        }"#,
    )
    .unwrap();

    cmd_with_home(tmp.path())
        .env("TOKSCALE_API_TOKEN", "test-token")
        .args(["submit", "--client", "opencode", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "submitting 1 unpriced google/gemini-default message(s) (1 tokens) at $0.00",
        ))
        .stdout(predicate::str::contains("Total tokens: 1"))
        .stdout(predicate::str::contains("No usage data found to submit.").not());
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

/// A session whose two turns land on different calendar days depending on the
/// zone reading them. Chosen so all three zones used below disagree:
///
/// | zone                  | 11:30Z     | 18:00Z     | day buckets              |
/// |-----------------------|------------|------------|--------------------------|
/// | `America/Los_Angeles` | 03-02 03:30| 03-02 10:00| `{03-02: 2}`             |
/// | `Asia/Seoul`          | 03-02 20:30| 03-03 03:00| `{03-02: 1, 03-03: 1}`   |
/// | `Pacific/Kiritimati`  | 03-03 01:30| 03-03 08:00| `{03-03: 2}`             |
///
/// Three distinct shapes means a test can tell which zone actually did the
/// bucketing, rather than only that two runs happened to agree.
fn create_bucket_timezone_fixture_dir() -> TempDir {
    let tmp = TempDir::new().expect("failed to create temp dir");
    let base = tmp.path();
    prime_pricing_cache(base);

    let session = base.join(".local/share/opencode/storage/message/session1");
    fs::create_dir_all(&session).unwrap();

    for (id, created_ms) in [
        ("msg_a", 1_772_451_000_000i64),
        ("msg_b", 1_772_474_400_000),
    ] {
        let msg = format!(
            r#"{{
                "id": "{id}",
                "sessionID": "session1",
                "role": "assistant",
                "modelID": "claude-sonnet-4-20250514",
                "providerID": "anthropic",
                "cost": 0.05,
                "tokens": {{
                    "input": 1000,
                    "output": 500,
                    "reasoning": 0,
                    "cache": {{ "read": 200, "write": 50 }}
                }},
                "time": {{ "created": {created_ms}.0 }}
            }}"#
        );
        fs::write(session.join(format!("{id}.json")), msg).unwrap();
    }

    tmp
}

fn pin_bucket_timezone(base: &Path, zone: &str) {
    pin_bucket_timezone_field(base, &format!(r#""{zone}""#));
}

/// [`pin_bucket_timezone`] with the raw JSON value, so a test can write `null`
/// as well as a string.
fn pin_bucket_timezone_field(base: &Path, json_value: &str) {
    write_settings_json(
        base,
        &format!(r#"{{ "scanner": {{ "bucketTimezone": {json_value} }} }}"#),
    );
}

/// Day buckets that actually carry messages, as `(date, message_count)`.
/// The graph zero-fills a calendar, so the empty days carry no signal.
///
/// # Why the `TZ`-driven tests below are unix-only
///
/// `TZ` is the instrument these tests pose their question with, and it is a
/// POSIX instrument. `chrono::Local` honors it on Unix; on Windows `Local`
/// reads `GetTimeZoneInformation` — the machine's own zone, which no
/// environment variable overrides. A test whose premise is "run this from Los
/// Angeles, then from Seoul" therefore runs twice from the runner's own zone
/// on Windows and asserts against buckets the host never produced. Concretely:
/// this fixture's two messages sit at 2026-03-02T11:30Z and 2026-03-02T18:00Z,
/// so a UTC runner buckets both into `2026-03-02` while the expectation is
/// Seoul's `03-02`/`03-03` split.
///
/// That is a statement about the instrument, and only about the instrument. It
/// is *not* a claim that the product was fine on Windows. It was not:
/// `detect_local_iana_name` declining to pin while a foreign `TZ` was set was
/// a real bug — the device never pinned, on any run, for as long as the
/// variable stayed set, and so kept the rescan-splits-history behaviour that
/// pinning exists to remove. It is fixed in `bucket_tz::tz_env_zone`, which no
/// longer offers `TZ` as the pin candidate on the platform where
/// `chrono::Local` does not read it.
///
/// That fix does not hand these tests their instrument back. `TZ` still cannot
/// move the zone the child buckets in on Windows, so these tests still cannot
/// place the child anywhere, and they stay gated. What the fix restored is
/// covered directly by the unit test
/// `bucket_tz::tests::a_foreign_tz_does_not_make_a_windows_host_unpinnable`,
/// which asserts the pinning behaviour itself rather than through `TZ`.
///
/// Tests that pin a zone through settings.json rather than through `TZ` stay
/// on every platform: the pin outranks the host, which is the whole claim.
fn graph_day_buckets(base: &Path, timezone: &str) -> Vec<(String, i64)> {
    let output = cmd_with_home(base)
        .env("TZ", timezone)
        .args(["graph", "--client", "opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut buckets: Vec<(String, i64)> = json["contributions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| {
            let messages = c["totals"]["messages"].as_i64().unwrap_or(0);
            (messages > 0).then(|| (c["date"].as_str().unwrap().to_string(), messages))
        })
        .collect();
    buckets.sort();
    buckets
}

/// The regression this whole feature exists for.
///
/// Day keys used to be derived from `chrono::Local` on every scan, so
/// rescanning the same unchanged history from another timezone re-split it
/// across days. The server's monotonic per-day guard then kept the stale value
/// on one day and accepted the new one on its neighbour, and the device's total
/// was inflated permanently with no way to walk it back.
///
/// With a zone pinned, the same bytes produce the same day keys no matter what
/// `TZ` says — and specifically the *pinned* zone's keys, not either host's.
#[test]
fn test_pinned_bucket_timezone_survives_a_host_timezone_change() {
    let tmp = create_bucket_timezone_fixture_dir();
    pin_bucket_timezone(tmp.path(), "Pacific/Kiritimati");

    let from_los_angeles = graph_day_buckets(tmp.path(), "America/Los_Angeles");
    let from_seoul = graph_day_buckets(tmp.path(), "Asia/Seoul");

    assert_eq!(
        from_los_angeles, from_seoul,
        "a pinned zone must key the same history the same way from any host timezone"
    );
    // Not just equal to each other — equal to what the pinned zone says.
    // Los Angeles would report {03-02: 2} and Seoul {03-02: 1, 03-03: 1}, so
    // agreeing on {03-03: 2} can only come from Pacific/Kiritimati.
    assert_eq!(
        from_los_angeles,
        vec![("2026-03-03".to_string(), 2)],
        "buckets must follow the pinned zone, not either host"
    );
}

/// The other half of the contract: a device that has never pinned reports
/// exactly what it reported before this change — day keys follow the machine.
///
/// Separate homes because the first run on a home pins it; this asserts the
/// unpinned/first-scan semantics, which are the ones that must not move.
#[test]
#[cfg(unix)]
fn test_unpinned_first_scan_still_buckets_by_the_host_timezone() {
    let los_angeles = create_bucket_timezone_fixture_dir();
    let seoul = create_bucket_timezone_fixture_dir();

    assert_eq!(
        graph_day_buckets(los_angeles.path(), "America/Los_Angeles"),
        vec![("2026-03-02".to_string(), 2)],
        "an unpinned device must bucket by its own timezone, unchanged"
    );
    assert_eq!(
        graph_day_buckets(seoul.path(), "Asia/Seoul"),
        vec![("2026-03-02".to_string(), 1), ("2026-03-03".to_string(), 1)],
        "an unpinned device must bucket by its own timezone, unchanged"
    );
}

/// The first run records the zone, and recording it changes nothing about what
/// that run reports. If pinning moved the numbers on the machine doing the
/// pinning, every user would see a one-off jump on upgrade.
///
/// This is the integration witness to the Windows bug fixed in
/// `bucket_tz::tz_env_zone`, and it stays unix-only anyway. Both of its
/// assertions are addressed to a zone this test cannot put a Windows child in:
/// the buckets it expects are Seoul's, and the zone it expects on disk is
/// `Asia/Seoul`, whereas a fixed Windows host pins the Win32 zone it is
/// actually in (`Etc/UTC` on the runner) and buckets accordingly. Passing there
/// would require `TZ` to move `chrono::Local`, which is the one thing Windows
/// does not do. See [`graph_day_buckets`].
#[test]
#[cfg(unix)]
fn test_first_run_pins_the_host_timezone_without_changing_its_own_output() {
    let tmp = create_bucket_timezone_fixture_dir();
    let settings_path = settings_json_path(tmp.path());
    assert!(!settings_path.exists(), "fixture must start unpinned");

    let buckets = graph_day_buckets(tmp.path(), "Asia/Seoul");
    assert_eq!(
        buckets,
        vec![("2026-03-02".to_string(), 1), ("2026-03-03".to_string(), 1)],
        "the run that pins must report what it would have reported unpinned"
    );

    let settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
    assert_eq!(
        settings["scanner"]["bucketTimezone"].as_str(),
        Some("Asia/Seoul"),
        "the first run must record the zone it bucketed into"
    );

    // And the recorded zone is what the *next* run uses, even from elsewhere.
    assert_eq!(
        graph_day_buckets(tmp.path(), "America/Los_Angeles"),
        buckets,
        "the recorded zone must survive a host timezone change"
    );
}

#[test]
fn test_config_rejects_rekeying_or_unsetting_an_established_timezone() {
    let tmp = create_bucket_timezone_fixture_dir();
    pin_bucket_timezone(tmp.path(), "America/Los_Angeles");

    cmd_with_home(tmp.path())
        .args(["config", "set", "timezone", "Pacific/Kiritimati"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "historical submitted day rows are monotonic",
        ));

    cmd_with_home(tmp.path())
        .args(["config", "set", "timezone", "auto"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "server resync/replacement transition",
        ));

    cmd_with_home(tmp.path())
        .args(["config", "unset", "timezone"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "historical submitted day rows are monotonic",
        ));

    cmd_with_home(tmp.path())
        .args(["config", "set", "timezone", "America/Los_Angeles"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(unchanged)"));
}

#[test]
fn test_config_can_recover_an_invalid_or_unpinned_timezone() {
    let tmp = create_bucket_timezone_fixture_dir();
    let config_dir = tmp.path().join(".config/tokscale");
    pin_bucket_timezone(tmp.path(), "Mars/Olympus_Mons");

    cmd_with_home(tmp.path())
        // macOS resolves its config root from the account home, not `HOME`.
        // Pin the command to this fixture so a prior test cannot auto-pin a
        // shared profile before this invalid value is read.
        .env("TOKSCALE_CONFIG_DIR", &config_dir)
        .args(["config", "set", "timezone", "Asia/Seoul"])
        .assert()
        .success();
}

#[test]
fn test_config_set_can_initialize_or_recover_a_timezone_before_auto_pinning() {
    for existing in [None, Some("Mars/Olympus_Mons")] {
        let tmp = create_bucket_timezone_fixture_dir();
        let config_dir = tmp.path().join(".config/tokscale");
        if let Some(existing) = existing {
            pin_bucket_timezone(tmp.path(), existing);
        }

        cmd_with_home(tmp.path())
            .env("TOKSCALE_CONFIG_DIR", &config_dir)
            .args(["config", "set", "timezone", "Asia/Seoul"])
            .assert()
            .success();

        let settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(config_dir.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            settings["scanner"]["bucketTimezone"].as_str(),
            Some("Asia/Seoul"),
            "config set must operate on a {existing:?} value before startup auto-pinning"
        );
    }
}

#[test]
fn test_config_set_auto_can_initialize_or_recover_before_auto_pinning() {
    for existing in [None, Some("Mars/Olympus_Mons")] {
        let tmp = create_bucket_timezone_fixture_dir();
        let config_dir = tmp.path().join(".config/tokscale");
        if let Some(existing) = existing {
            pin_bucket_timezone(tmp.path(), existing);
        }

        cmd_with_home(tmp.path())
            .env("TOKSCALE_CONFIG_DIR", &config_dir)
            .args(["config", "set", "timezone", "auto"])
            .assert()
            .success();

        let settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(config_dir.join("settings.json")).unwrap())
                .unwrap();
        assert!(
            settings["scanner"]["bucketTimezone"].is_string(),
            "config set auto must write a detected timezone for a {existing:?} value"
        );
    }
}

#[test]
fn test_config_unset_can_initialize_or_recover_a_timezone_before_auto_pinning() {
    for existing in [None, Some("Mars/Olympus_Mons")] {
        let tmp = create_bucket_timezone_fixture_dir();
        let config_dir = tmp.path().join(".config/tokscale");
        if let Some(existing) = existing {
            pin_bucket_timezone(tmp.path(), existing);
        }

        cmd_with_home(tmp.path())
            .env("TOKSCALE_CONFIG_DIR", &config_dir)
            .args(["config", "unset", "timezone"])
            .assert()
            .success();
    }
}

#[test]
fn test_read_only_config_commands_still_auto_pin_the_timezone() {
    let tmp = create_bucket_timezone_fixture_dir();
    let config_dir = tmp.path().join(".config/tokscale");

    cmd_with_home(tmp.path())
        .env("TOKSCALE_CONFIG_DIR", &config_dir)
        .args(["config", "get", "timezone"])
        .assert()
        .success();

    let settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(config_dir.join("settings.json")).unwrap())
            .unwrap();
    assert!(
        settings["scanner"]["bucketTimezone"].is_string(),
        "read-only config commands must retain startup auto-pinning"
    );
}

/// A fixed UTC offset cannot follow daylight saving time, so pinning one would
/// reintroduce a bounded version of the bug pinning removes. Reject it at the
/// boundary rather than storing a value that silently degrades to unpinned.
#[test]
fn test_config_set_timezone_rejects_a_fixed_offset() {
    let tmp = create_bucket_timezone_fixture_dir();

    cmd_with_home(tmp.path())
        .args(["config", "set", "timezone", "+09:00"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a known IANA timezone name"));
}

/// A hand-edited settings.json with a bad zone must not break scanning. It
/// degrades to the pre-pinning behaviour, which is wrong in the old way rather
/// than a hard failure on every command.
///
/// The run also re-detects a real zone for the field (see
/// `test_auto_pinning_recovers_from_a_bucket_timezone_that_names_no_zone`), and
/// the buckets are unchanged either way: auto-pinning only ever records a zone
/// that reproduces `chrono::Local`, which is what the fallback uses.
#[test]
fn test_unparseable_pinned_timezone_falls_back_to_the_host_timezone() {
    let tmp = create_bucket_timezone_fixture_dir();
    pin_bucket_timezone(tmp.path(), "Mars/Olympus_Mons");

    assert_eq!(
        graph_day_buckets(tmp.path(), "America/Los_Angeles"),
        vec![("2026-03-02".to_string(), 2)],
        "an unknown zone name must fall back to the host, not fail the scan"
    );
}

/// Naming the local zone and bucketing in it go through different code with
/// different rules. `chrono::Local` honors `TZ`; `iana-time-zone` does not read
/// `TZ` at all on Linux — it resolves `/etc/localtime`. A host where those two
/// disagree is ordinary (any `TZ=...` in a shell profile, any CI container),
/// and pinning the detected name there would re-key the whole history on the
/// first run after upgrading.
///
/// `TZ=XYZ8` is a POSIX rule string, not a zone name: `chrono::Local` honors it
/// as UTC-8, and no detector will ever return it. So either the guard declines
/// to pin, or it pins something that buckets identically — and the assertion
/// below holds under both without caring which happened, which is the actual
/// contract.
#[test]
fn test_first_run_never_rebuckets_when_the_detector_disagrees_with_local() {
    let tmp = create_bucket_timezone_fixture_dir();

    // UTC-8: 03:30 and 10:00 on 2026-03-02 — one day, both messages.
    assert_eq!(
        graph_day_buckets(tmp.path(), "XYZ8"),
        vec![("2026-03-02".to_string(), 2)],
        "the first run must bucket by chrono::Local no matter what the zone \
         detector reports"
    );

    // Whatever it recorded, a second run has to agree with the first. A pin
    // that moved the buckets would show up here even if the first run's
    // assertion happened to match by luck.
    assert_eq!(
        graph_day_buckets(tmp.path(), "XYZ8"),
        vec![("2026-03-02".to_string(), 2)],
        "the recorded zone must reproduce what the first run reported"
    );
}

/// Hour keys that actually carry messages, sorted ascending.
fn hourly_keys(base: &Path, timezone: &str) -> Vec<String> {
    let output = cmd_with_home(base)
        .env("TZ", timezone)
        .args(["hourly", "--json", "--client", "opencode", "--no-spinner"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut keys: Vec<String> = json["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["hour"].as_str().unwrap().to_string())
        .collect();
    keys.sort();
    keys
}

/// The hourly key embeds a date, and the rebucket pass has already moved every
/// message's `date` into the pinned zone. Deriving the hour from the host
/// instead lets one report disagree with itself about which day an hour belongs
/// to — and a `--today` filter resolved in the pinned zone then selects rows
/// labelled with the neighbouring host-local date.
#[test]
fn test_hourly_keys_follow_the_pinned_bucket_timezone() {
    let tmp = create_bucket_timezone_fixture_dir();
    pin_bucket_timezone(tmp.path(), "Pacific/Kiritimati");

    // 11:30Z and 18:00Z are 01:30 and 08:00 on 03-03 in Kiritimati (UTC+14),
    // but 03:30 and 10:00 on 03-02 in Los Angeles — a different day *and* a
    // different hour, so neither half can match by accident.
    let expected = vec![
        "2026-03-03 01:00".to_string(),
        "2026-03-03 08:00".to_string(),
    ];

    assert_eq!(
        hourly_keys(tmp.path(), "America/Los_Angeles"),
        expected,
        "hour keys must be built in the pinned zone, not the host's"
    );
    assert_eq!(
        hourly_keys(tmp.path(), "Asia/Seoul"),
        expected,
        "and they must not move when the host timezone does"
    );
}

/// `Settings::load()` answers an unparseable settings.json with
/// `Settings::default()`. Auto-pinning is the first path in the CLI that loads
/// and then unconditionally saves, so on its own it would replace a
/// hand-edited or truncated settings.json with defaults plus a timezone —
/// erasing scanner paths, aliases, autosubmit config and UI preferences on a
/// plain `tokscale graph`, with no prompt and no way back.
///
/// The positive control matters: if this host cannot name its own zone the pin
/// is skipped for every home and the negative cases would pass vacuously.
#[test]
fn test_auto_pinning_never_overwrites_a_settings_file_it_could_not_read() {
    // Positive control — a readable file on this host really does get pinned.
    let control = create_bucket_timezone_fixture_dir();
    let control_settings = settings_json_path(control.path());
    fs::create_dir_all(control_settings.parent().unwrap()).unwrap();
    fs::write(&control_settings, r#"{"colorPalette":"green"}"#).unwrap();
    graph_day_buckets(control.path(), "Asia/Seoul");
    let control_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&control_settings).unwrap()).unwrap();
    assert!(
        control_json["scanner"]["bucketTimezone"].is_string(),
        "auto-pinning must work on this host for the assertions below to mean \
         anything; got {control_json}"
    );
    assert_eq!(
        control_json["colorPalette"].as_str(),
        Some("green"),
        "pinning a readable file must preserve everything else in it"
    );

    // Truncated JSON, and valid JSON with a field of the wrong type — both make
    // `serde_json::from_str` fail, and both leave real user data on disk.
    for unreadable in [
        r#"{"colorPalette": "green", "scanner": {"#,
        r#"{"colorPalette": 42, "scanner": {"extraScanPaths": {"claude": ["/data"]}}}"#,
    ] {
        let tmp = create_bucket_timezone_fixture_dir();
        let settings_path = settings_json_path(tmp.path());
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        fs::write(&settings_path, unreadable).unwrap();

        cmd_with_home(tmp.path())
            .env("TZ", "Asia/Seoul")
            .args(["graph", "--client", "opencode", "--no-spinner"])
            .assert()
            .success();

        assert_eq!(
            fs::read_to_string(&settings_path).unwrap(),
            unreadable,
            "a settings.json that could not be read must be left byte-identical, \
             not replaced with defaults plus a timezone"
        );
    }
}

/// `tokscale config` loads and saves too, and it is the one place a user is
/// watching. Refuse out loud rather than silently replacing a file whose
/// contents were never recovered.
#[test]
fn test_config_set_refuses_to_overwrite_unreadable_settings() {
    let tmp = create_bucket_timezone_fixture_dir();
    let settings_path = settings_json_path(tmp.path());
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    let unreadable = r#"{"scanner": {"extraScanPaths": "not-a-map"}}"#;
    fs::write(&settings_path, unreadable).unwrap();

    cmd_with_home(tmp.path())
        .args(["config", "set", "timezone", "Asia/Seoul"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "could not read this machine's tokscale settings",
        ));

    assert_eq!(
        fs::read_to_string(&settings_path).unwrap(),
        unreadable,
        "a refused write must leave the file untouched"
    );
}

/// A `bucketTimezone` that does not name a zone the tz database knows is not a
/// pin: bucketing degrades to host-local, so the device keeps exactly the
/// exposure auto-pinning exists to close. Treating "present" as "pinned" made
/// one bad hand edit suppress the fix permanently.
#[test]
fn test_auto_pinning_recovers_from_a_bucket_timezone_that_names_no_zone() {
    let pinned_zone = |base: &Path| -> String {
        let settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(settings_json_path(base)).unwrap()).unwrap();
        settings["scanner"]["bucketTimezone"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };
    let scan = |base: &Path| {
        cmd_with_home(base)
            .env("TZ", "Asia/Seoul")
            .args(["graph", "--client", "opencode", "--no-spinner"])
            .assert()
            .success();
    };

    // What a real recovery looks like on this host, from a file that is simply
    // unpinned. Comparing against it keeps the test from passing on a host
    // where nothing can be detected and every case recovers to nothing.
    let baseline = create_bucket_timezone_fixture_dir();
    pin_bucket_timezone_field(baseline.path(), "null");
    scan(baseline.path());
    let detected = pinned_zone(baseline.path());
    assert!(
        !detected.is_empty(),
        "auto-pinning must work on this host for the assertions below to mean \
         anything"
    );

    for junk in ["", "   ", "Mars/Olympus_Mons", "+09:00"] {
        let tmp = create_bucket_timezone_fixture_dir();
        pin_bucket_timezone(tmp.path(), junk);

        scan(tmp.path());
        assert_eq!(
            pinned_zone(tmp.path()),
            detected,
            "`{junk}` names no zone, so bucketing already falls back to the host \
             — the next run must re-detect instead of leaving the device \
             permanently unpinned"
        );

        // And the recovered value is a real pin, so it is not re-detected again
        // on every subsequent run.
        scan(tmp.path());
        assert_eq!(pinned_zone(tmp.path()), detected);
    }
}

/// Whether mode 000 actually denies a read here. Root ignores it, and so do
/// some container filesystems, and the tests below have nothing to assert when
/// the file they made unreadable can still be read. Probes the real behaviour
/// rather than inferring it from the uid.
#[cfg(unix)]
fn mode_000_denies_reads(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let probe = dir.join(".mode-probe");
    fs::write(&probe, "probe").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o000)).unwrap();
    let denied = fs::read_to_string(&probe).is_err();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o644)).unwrap();
    fs::remove_file(&probe).unwrap();
    denied
}

/// `read_to_string(..).ok()` collapses "no such file" into the same `None` as
/// "this file exists and I could not open it". Auto-pinning has to tell them
/// apart: the first is safe to write, the second is a file whose contents are
/// still on disk and still unknown.
///
/// A parse failure is covered by
/// `test_auto_pinning_never_overwrites_a_settings_file_it_could_not_read`; this
/// is the I/O half, which never reaches the parser at all.
#[cfg(unix)]
#[test]
fn test_auto_pinning_declines_when_settings_json_cannot_be_opened() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = create_bucket_timezone_fixture_dir();
    if !mode_000_denies_reads(tmp.path()) {
        return;
    }

    let settings_path = settings_json_path(tmp.path());
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    fs::write(&settings_path, r#"{"colorPalette":"green"}"#).unwrap();
    fs::set_permissions(&settings_path, fs::Permissions::from_mode(0o000)).unwrap();

    cmd_with_home(tmp.path())
        .env("TZ", "Asia/Seoul")
        .args(["graph", "--client", "opencode", "--no-spinner"])
        .assert()
        .success();

    fs::set_permissions(&settings_path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        fs::read_to_string(&settings_path).unwrap(),
        r#"{"colorPalette":"green"}"#,
        "a settings.json that could not be opened must be left byte-identical"
    );
}

/// The macOS fallback only fires while the canonical path is absent, so writing
/// a primary settings.json shadows the legacy file for good — a legacy file the
/// user could otherwise have repaired. Auto-pinning must not create one on top
/// of a legacy file it could not open.
///
/// macOS-only: `legacy_macos_config_dir()` returns `None` everywhere else, so
/// there is no fallback to shadow. Uses a permissions failure rather than bad
/// JSON because unparseable legacy *content* is read successfully and blocked
/// one step later, by the parse arm.
#[cfg(target_os = "macos")]
#[test]
fn test_auto_pinning_does_not_shadow_a_legacy_settings_file_it_cannot_open() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = create_bucket_timezone_fixture_dir();
    if !mode_000_denies_reads(tmp.path()) {
        return;
    }

    let legacy = tmp
        .path()
        .join("Library/Application Support/tokscale/settings.json");
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, r#"{"colorPalette":"green"}"#).unwrap();
    fs::set_permissions(&legacy, fs::Permissions::from_mode(0o000)).unwrap();

    let primary = settings_json_path(tmp.path());
    assert!(!primary.exists(), "fixture must start with no primary file");

    cmd_with_home(tmp.path())
        // The one test here that must run *without* the config-dir override:
        // `Settings::load_with_origin` skips the legacy macOS read whenever
        // `TOKSCALE_CONFIG_DIR` is set, because the override means "this root
        // and nothing outside it". With it set there is no fallback left to
        // shadow and the assertion below could never fail. Clearing it leaves
        // the resolver on `$HOME/.config/tokscale`, which is the same sandbox
        // directory — this is macOS-only, so no known-folder lookup is in play.
        .env_remove("TOKSCALE_CONFIG_DIR")
        .env("TZ", "Asia/Seoul")
        .args(["graph", "--client", "opencode", "--no-spinner"])
        .assert()
        .success();

    fs::set_permissions(&legacy, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        !primary.exists(),
        "writing a primary settings.json would shadow the legacy file the user \
         still has to repair"
    );
    assert_eq!(
        fs::read_to_string(&legacy).unwrap(),
        r#"{"colorPalette":"green"}"#,
        "and the legacy file itself must be left alone"
    );
}
