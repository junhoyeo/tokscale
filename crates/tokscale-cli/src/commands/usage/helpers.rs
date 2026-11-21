use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Local, Utc};

/// Bounds each usage HTTP request, including response body reads, to 30 seconds,
/// the value opencode_go, sakana and cursor already use. Refreshes and retries
/// each receive their own request deadline.
///
/// A timeout only ends the wait: a POST that changes server state (an OAuth
/// refresh that rotates the refresh token, a Codex reset-credit redemption) may
/// still have been applied when the client gives up.
pub(super) fn http_client() -> Result<reqwest::Client> {
    http_client_with_timeout(std::time::Duration::from_secs(30))
}

fn http_client_with_timeout(timeout: std::time::Duration) -> Result<reqwest::Client> {
    Ok(tokscale_core::http::client_builder()
        .timeout(timeout)
        .build()?)
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s.to_string(),
    }
}

/// Reads a secret from the platform credential store.
///
/// `service` means different things per platform, because the two stores have
/// different lookup semantics and the same string cannot serve both:
///
/// * macOS: `security find-generic-password -s <service>` matches on the
///   *service attribute alone* and ignores the account, so a bare service name
///   resolves whatever account the item was stored under.
/// * Windows: the Credential Manager has no service attribute. `CredReadW` is
///   an exact match on the full `TargetName` and supports no wildcards, so the
///   caller must pass a complete target name. For anything written by Go's
///   `go-keyring` (which is what `gh` uses) that is `"<service>:<username>"`,
///   never the bare `"<service>"` — see `copilot::gh_wincred_targets`.
pub fn read_keychain(service: &str) -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("security")
            .args(["find-generic-password", "-s", service, "-w"])
            .output()?;
        if !out.status.success() {
            anyhow::bail!("Keychain lookup failed for service '{service}'");
        }
        Ok(String::from_utf8(out.stdout)?.trim_end().to_string())
    }

    #[cfg(target_os = "windows")]
    {
        // The caller supplies a full Windows target name here; this helper
        // does not compose one, because the composition rule belongs to
        // whichever library wrote the credential.
        read_wincred(service)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = service;
        anyhow::bail!("Keychain lookup is only available on macOS and Windows");
    }
}

/// Outcome of a credential-store lookup that keeps "absent" apart from
/// "present but unreadable".
///
/// On macOS a locked keychain, or a session that may not show the access
/// prompt, makes `security ... -w` fail with exit 36 even though the item
/// exists. Callers that treat every failure as "not logged in" hide the
/// provider without saying why; `Unreadable` lets them report it instead.
/// It never carries stdout, which is where `security -w` writes the secret,
/// and its `Debug` output redacts the secret held by `Found`.
#[derive(Clone, PartialEq, Eq)]
pub enum KeychainLookup {
    Found(String),
    Missing,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Unreadable {
        code: Option<i32>,
        stderr: String,
    },
}

impl std::fmt::Debug for KeychainLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Found(_) => f.debug_tuple("Found").field(&"<redacted>").finish(),
            Self::Missing => f.write_str("Missing"),
            Self::Unreadable { code, stderr } => f
                .debug_struct("Unreadable")
                .field("code", code)
                .field("stderr", stderr)
                .finish(),
        }
    }
}

/// One `security` invocation, reduced to what classification needs.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) struct SecurityRun {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Classifies a `security find-generic-password -s <service> -w` result.
///
/// Any failure, including no exit code at all, runs `probe` -- the same
/// lookup without `-w`, which reads only attributes and keeps working while
/// the secret is locked away. Exit 44 is not special-cased: `security` keeps
/// only the low byte of the OSStatus, so errSecItemNotFound (-25300) shares it
/// with other statuses. Only a probe that succeeds proves the item exists;
/// anything else stays `Missing` so users who never stored the item are not
/// shown a warning about it. Exit 0 with a payload that is not UTF-8 also
/// proves the item exists, so it is `Unreadable` rather than `Missing`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn classify_keychain_lookup(
    read: std::io::Result<SecurityRun>,
    probe: impl FnOnce() -> std::io::Result<SecurityRun>,
) -> KeychainLookup {
    let Ok(read) = read else {
        return KeychainLookup::Missing;
    };
    match read.code {
        Some(0) => match String::from_utf8(read.stdout) {
            Ok(secret) => KeychainLookup::Found(secret.trim_end().to_string()),
            Err(_) => KeychainLookup::Unreadable {
                code: Some(0),
                stderr: String::from_utf8_lossy(&read.stderr).trim().to_string(),
            },
        },
        code => match probe() {
            Ok(SecurityRun { code: Some(0), .. }) => KeychainLookup::Unreadable {
                code,
                stderr: String::from_utf8_lossy(&read.stderr).trim().to_string(),
            },
            _ => KeychainLookup::Missing,
        },
    }
}

/// Looks up `service` and reports whether it is found, missing, or present
/// but unreadable. Unlike [`read_keychain`], this distinguishes the last two
/// on macOS. Other platforms keep [`read_keychain`]'s lookup unchanged and
/// report every failure as `Missing`.
pub fn keychain_lookup(service: &str) -> KeychainLookup {
    #[cfg(target_os = "macos")]
    {
        let security = |args: &[&str]| {
            std::process::Command::new("security")
                .args(args)
                .output()
                .map(|out| SecurityRun {
                    code: out.status.code(),
                    stdout: out.stdout,
                    stderr: out.stderr,
                })
        };
        classify_keychain_lookup(
            security(&["find-generic-password", "-s", service, "-w"]),
            || security(&["find-generic-password", "-s", service]),
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        read_keychain(service)
            .map(KeychainLookup::Found)
            .unwrap_or(KeychainLookup::Missing)
    }
}

/// Reads a credential blob from the Windows Credential Manager.
///
/// `target` must be the *full* `TargetName` exactly as stored: `CredReadW`
/// with `CRED_TYPE_GENERIC` is an exact lookup with no prefix or wildcard
/// matching, so a target that is one character short simply reports
/// `ERROR_NOT_FOUND`.
#[cfg(target_os = "windows")]
pub(super) fn read_wincred(target: &str) -> Result<String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::Credentials::{CredFree, CredReadW, CRED_TYPE_GENERIC};

    // CredReadW expects a null-terminated UTF-16 target name.
    let wide: Vec<u16> = OsStr::new(target)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut cred_ptr: *mut windows_sys::Win32::Security::Credentials::CREDENTIALW =
        std::ptr::null_mut();

    // On windows-sys CredReadW returns BOOL (i32); 0 means failure and the
    // error code is in GetLastError (e.g. ERROR_NOT_FOUND when the user never
    // ran `gh auth login`).
    unsafe {
        let ok = CredReadW(wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred_ptr);
        if ok == 0 {
            anyhow::bail!(
                "CredReadW failed for target '{target}': {}",
                std::io::Error::last_os_error()
            );
        }
        if cred_ptr.is_null() {
            anyhow::bail!("CredReadW returned null credential for target '{target}'");
        }
        let cred = &*cred_ptr;
        let blob_size = cred.CredentialBlobSize as usize;
        let blob_ptr = cred.CredentialBlob;
        if blob_ptr.is_null() || blob_size == 0 {
            CredFree(cred_ptr as *const std::ffi::c_void);
            anyhow::bail!("Empty credential blob for target '{target}'");
        }
        let slice = std::slice::from_raw_parts(blob_ptr, blob_size);
        // Decode before freeing, but bind the result so the allocation is
        // released on the error path too.
        let decoded = decode_wincred_blob(slice);
        CredFree(cred_ptr as *const std::ffi::c_void);
        decoded.map_err(|e| anyhow::anyhow!("Credential blob for target '{target}': {e}"))
    }
}

/// Turns a raw Credential Manager blob into a token string.
///
/// `go-keyring` stores the secret as plain UTF-8 bytes and `wincred` writes
/// `CredentialBlob` verbatim with no transcoding and no length prefix, so the
/// blob is read as UTF-8 and stripped of the trailing NUL padding some writers
/// add. A blob that still holds a NUL after trimming came from something else
/// — a UTF-16LE writer, most likely, whose ASCII text is accidentally valid
/// UTF-8 — and is rejected instead of being handed to the API as a corrupt
/// token that would come back as a puzzling HTTP 401.
#[cfg(any(target_os = "windows", test))]
fn decode_wincred_blob(bytes: &[u8]) -> Result<String> {
    let raw = String::from_utf8(bytes.to_vec())?;
    let token = raw.trim_end_matches('\0').trim_end().to_string();
    if token.is_empty() {
        anyhow::bail!("blob is empty after trimming NUL padding");
    }
    if token.contains('\0') {
        anyhow::bail!("blob contains an interior NUL byte, so it is not UTF-8 text");
    }
    Ok(token)
}

pub fn format_reset_time(resets_at: &str) -> String {
    let dt = match DateTime::parse_from_rfc3339(resets_at) {
        Ok(d) => d.with_timezone(&Utc),
        Err(_) => return resets_at.into(),
    };
    let local_dt = dt.with_timezone(&Local);
    let now = Utc::now();
    let display_time = compact_reset_time(local_dt, now.with_timezone(&Local), dt - now);
    format_reset_time_with_now(dt, now, &display_time)
}

fn format_reset_time_with_now(
    reset_at: DateTime<Utc>,
    now: DateTime<Utc>,
    display_time: &str,
) -> String {
    let diff = reset_at - now;
    if diff <= Duration::zero() {
        return "resets now".into();
    }
    let total_mins = diff.num_minutes();
    if total_mins < 60 {
        format!("resets in {total_mins}m")
    } else if total_mins < 24 * 60 {
        let h = diff.num_hours();
        let m = (diff - Duration::hours(h)).num_minutes();
        if m > 0 {
            format!("resets in {h}h {m}m")
        } else {
            format!("resets in {h}h")
        }
    } else {
        format!("resets {display_time}")
    }
}

fn compact_reset_time(reset_at: DateTime<Local>, now: DateTime<Local>, diff: Duration) -> String {
    if reset_at.year() != now.year() {
        reset_at.format("%Y-%m-%d %H:%M").to_string()
    } else if diff.num_days() < 7 {
        reset_at.format("%a %b %-d %H:%M").to_string()
    } else {
        reset_at.format("%b %-d %H:%M").to_string()
    }
}

pub fn render_ascii_bar(remaining_percent: f64, width: usize) -> String {
    let filled = (remaining_percent.clamp(0.0, 100.0) / 100.0 * width as f64).round() as usize;
    format!("[{}{}]", "=".repeat(filled), "-".repeat(width - filled))
}

pub fn atomic_write_secret(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path has no parent directory",
        )
    })?;
    std::fs::create_dir_all(dir)?;
    let temp_path = path.with_extension(format!("{}.tmp", std::process::id()));
    {
        #[cfg(unix)]
        let mut opts = {
            use std::os::unix::fs::OpenOptionsExt;
            let mut o = std::fs::OpenOptions::new();
            o.mode(0o600);
            o
        };
        #[cfg(not(unix))]
        let mut opts = std::fs::OpenOptions::new();
        let mut f = match opts.write(true).create_new(true).open(&temp_path) {
            Ok(f) => f,
            Err(e) => {
                let _ = std::fs::remove_file(&temp_path);
                return Err(e);
            }
        };
        if let Err(e) = std::io::Write::write_all(&mut f, data) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(e);
        }
    }
    if let Err(e) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::time::Duration as StdDuration;

    fn utc(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn run(code: Option<i32>, out: &str, err: &str) -> std::io::Result<SecurityRun> {
        Ok(SecurityRun {
            code,
            stdout: out.as_bytes().to_vec(),
            stderr: err.as_bytes().to_vec(),
        })
    }

    fn never() -> std::io::Result<SecurityRun> {
        panic!("probe must not run for this read result")
    }

    #[test]
    fn classify_found_trims_trailing_newline() {
        let result = classify_keychain_lookup(run(Some(0), "secret\n", ""), never);
        assert_eq!(result, KeychainLookup::Found("secret".into()));
    }

    #[test]
    fn classify_exit_44_is_confirmed_by_probe() {
        // The exit status keeps only the OSStatus low byte, so 44 is not
        // proof of errSecItemNotFound; the probe decides.
        let absent = classify_keychain_lookup(run(Some(44), "", "x"), || run(Some(44), "", ""));
        assert_eq!(absent, KeychainLookup::Missing);
        let present =
            classify_keychain_lookup(run(Some(44), "", "x"), || run(Some(0), "attrs", ""));
        assert_eq!(
            present,
            KeychainLookup::Unreadable {
                code: Some(44),
                stderr: "x".into()
            }
        );
    }

    #[test]
    fn found_debug_redacts_secret() {
        let rendered = format!("{:?}", KeychainLookup::Found("oauth-secret".into()));
        assert!(!rendered.contains("oauth-secret"), "leaked: {rendered}");
        assert!(rendered.contains("<redacted>"), "unexpected: {rendered}");
    }

    #[test]
    fn classify_locked_but_present_is_unreadable() {
        let result = classify_keychain_lookup(run(Some(36), "", ""), || run(Some(0), "attrs", ""));
        assert_eq!(
            result,
            KeychainLookup::Unreadable {
                code: Some(36),
                stderr: String::new()
            }
        );
    }

    #[test]
    fn classify_locked_but_absent_is_missing() {
        let result = classify_keychain_lookup(run(Some(36), "", ""), || run(Some(44), "", ""));
        assert_eq!(result, KeychainLookup::Missing);
    }

    #[test]
    fn classify_probe_other_failure_is_missing() {
        let other = classify_keychain_lookup(run(Some(36), "", ""), || run(Some(36), "", ""));
        assert_eq!(other, KeychainLookup::Missing);
        let no_code = classify_keychain_lookup(run(Some(36), "", ""), || run(None, "", ""));
        assert_eq!(no_code, KeychainLookup::Missing);
        let spawn = classify_keychain_lookup(run(Some(36), "", ""), || {
            Err(std::io::ErrorKind::NotFound.into())
        });
        assert_eq!(spawn, KeychainLookup::Missing);
    }

    #[test]
    fn classify_no_exit_code_probes() {
        let result = classify_keychain_lookup(run(None, "", ""), || run(Some(0), "attrs", ""));
        assert_eq!(
            result,
            KeychainLookup::Unreadable {
                code: None,
                stderr: String::new()
            }
        );
    }

    #[test]
    fn classify_spawn_error_is_missing() {
        let result = classify_keychain_lookup(Err(std::io::ErrorKind::NotFound.into()), never);
        assert_eq!(result, KeychainLookup::Missing);
    }

    #[test]
    fn unreadable_never_captures_stdout() {
        let result = classify_keychain_lookup(run(Some(51), "LEAKED-TOKEN", "  denied \n"), || {
            run(Some(0), "attrs", "")
        });
        assert_eq!(
            result,
            KeychainLookup::Unreadable {
                code: Some(51),
                stderr: "denied".into()
            }
        );
        assert!(!format!("{result:?}").contains("LEAKED-TOKEN"));
    }

    #[test]
    fn classify_non_utf8_secret_is_unreadable() {
        // Exit 0 proves the item exists, so a payload that is not text must
        // surface as unreadable rather than hide the provider.
        let read = Ok(SecurityRun {
            code: Some(0),
            stdout: vec![0xff, 0xfe],
            stderr: vec![],
        });
        assert_eq!(
            classify_keychain_lookup(read, never),
            KeychainLookup::Unreadable {
                code: Some(0),
                stderr: String::new()
            }
        );
    }

    #[test]
    fn wincred_blob_decodes_a_plain_utf8_token() {
        // go-keyring writes the secret as raw UTF-8 with no transcoding.
        assert_eq!(decode_wincred_blob(b"gho_example").unwrap(), "gho_example");
    }

    #[test]
    fn wincred_blob_trims_nul_padding_and_trailing_whitespace() {
        assert_eq!(
            decode_wincred_blob(b"gho_example\0\0").unwrap(),
            "gho_example"
        );
        assert_eq!(
            decode_wincred_blob(b"gho_example\r\n").unwrap(),
            "gho_example"
        );
    }

    #[test]
    fn wincred_blob_rejects_an_interior_nul_from_a_utf16_writer() {
        // "gho_x" as UTF-16LE is accidentally valid UTF-8, so from_utf8 alone
        // would hand back a mangled token and the API would answer 401 with no
        // clue why. Reject it and let the caller fall through to hosts.yml.
        let utf16le = b"g\0h\0o\0_\0x\0";
        let err = decode_wincred_blob(utf16le).unwrap_err().to_string();
        assert!(err.contains("interior NUL"), "got: {err}");
    }

    #[test]
    fn wincred_blob_rejects_empty_and_non_utf8_payloads() {
        assert!(decode_wincred_blob(b"").is_err());
        assert!(decode_wincred_blob(b"\0\0\0").is_err());
        assert!(decode_wincred_blob(b"   ").is_err());
        assert!(decode_wincred_blob(&[0xff, 0xfe, 0x41]).is_err());
    }

    #[test]
    fn reset_time_keeps_short_windows_relative() {
        let label = format_reset_time_with_now(
            utc("2026-06-25T02:45:00Z"),
            utc("2026-06-25T01:30:00Z"),
            "2026-06-25 10:45 +08:00",
        );

        assert_eq!(label, "resets in 1h 15m");
    }

    #[test]
    fn reset_time_shows_absolute_local_time_for_daily_or_longer_windows() {
        let label = format_reset_time_with_now(
            utc("2026-06-27T01:30:00Z"),
            utc("2026-06-25T01:30:00Z"),
            "Sat Jun 27 09:30",
        );

        assert_eq!(label, "resets Sat Jun 27 09:30");
    }

    #[test]
    fn reset_time_omits_weekday_for_long_windows() {
        let label = format_reset_time_with_now(
            utc("2026-07-18T00:43:00Z"),
            utc("2026-06-25T01:30:00Z"),
            "Jul 18 08:43",
        );

        assert_eq!(label, "resets Jul 18 08:43");
    }

    /// Accepts one request and then never finishes the response: with
    /// `body == false` it sends nothing, with `body == true` it sends headers
    /// and one byte of a ten-byte body.
    struct StalledServer {
        addr: std::net::SocketAddr,
        release: Option<mpsc::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl StalledServer {
        fn start(body: bool) -> (Self, tokio::sync::oneshot::Receiver<()>) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let (release, released) = mpsc::channel();
            let (ready, observed) = tokio::sync::oneshot::channel();
            let thread = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(StdDuration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while stream.read_exact(&mut byte).is_ok() {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        if body {
                            stream
                                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n{")
                                .unwrap();
                            stream.flush().unwrap();
                        }
                        let _ = ready.send(());
                        let _ = released.recv();
                        return;
                    }
                }
            });
            (
                Self {
                    addr,
                    release: Some(release),
                    thread: Some(thread),
                },
                observed,
            )
        }
    }

    impl Drop for StalledServer {
        fn drop(&mut self) {
            self.release.take();
            // Wake accept if the client failed before establishing a connection.
            let _ = TcpStream::connect(self.addr);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    /// Sends one GET to a stalled fixture and returns the outcome, or `None`
    /// when `watchdog` elapses first.
    async fn request_stalled_fixture(
        client: reqwest::Client,
        body: bool,
        watchdog: StdDuration,
    ) -> Option<reqwest::Result<String>> {
        let (server, observed) = StalledServer::start(body);
        let url = format!("http://{}/", server.addr);
        let request = async {
            let response = client.get(url).send().await?;
            response.text().await
        };
        let (observed, result) = tokio::join!(
            tokio::time::timeout(StdDuration::from_secs(2), observed),
            tokio::time::timeout(watchdog, request),
        );
        observed.expect("fixture must observe the request").unwrap();
        result.ok()
    }

    async fn assert_bounded_client_times_out(body: bool) {
        let client = http_client_with_timeout(StdDuration::from_millis(100)).unwrap();
        let error = request_stalled_fixture(client, body, StdDuration::from_secs(2))
            .await
            .expect("usage client must time out before the outer watchdog")
            .expect_err("stalled response must fail");
        assert!(
            error.is_timeout(),
            "expected a reqwest timeout, got {error}"
        );
    }

    /// Negative control: on the same fixture, the shared builder without a
    /// timeout is still waiting when the watchdog fires, so the bound in
    /// `assert_bounded_client_times_out` comes from `http_client_with_timeout`
    /// and not from the fixture closing the connection.
    async fn assert_unbounded_client_hangs(body: bool) {
        let client = tokscale_core::http::client_builder().build().unwrap();
        let outcome = request_stalled_fixture(client, body, StdDuration::from_millis(300)).await;
        assert!(
            outcome.is_none(),
            "a client without a timeout must not finish against a stalled server, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn usage_http_timeout_bounds_stalled_headers() {
        assert_bounded_client_times_out(false).await;
        assert_unbounded_client_hangs(false).await;
    }

    #[tokio::test]
    async fn usage_http_timeout_bounds_stalled_body() {
        assert_bounded_client_times_out(true).await;
        assert_unbounded_client_hangs(true).await;
    }
}

/// Exercises the real Credential Manager. Compiled and run only on Windows,
/// where `.github/workflows/test_coverage.yml` runs the suite as a hard gate —
/// that CI leg is the only place the Windows lookup can actually be executed.
#[cfg(all(test, target_os = "windows"))]
mod windows_wincred_tests {
    use super::read_wincred;
    use windows_sys::Win32::Security::Credentials::{
        CredDeleteW, CredWriteW, CREDENTIALW, CRED_PERSIST_SESSION, CRED_TYPE_GENERIC,
    };

    fn wide(value: &str) -> Vec<u16> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        OsStr::new(value)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Removes the credential on drop, so a failing assertion cannot leave an
    /// entry behind in the runner's (or a developer's) Credential Manager.
    struct TestCredential {
        target: String,
    }

    impl TestCredential {
        /// Returns `None` when the write is refused, so a locked-down runner
        /// skips instead of failing for an unrelated reason.
        fn write(target: &str, secret: &[u8]) -> Option<Self> {
            let mut target_wide = wide(target);
            let mut cred: CREDENTIALW = unsafe { std::mem::zeroed() };
            cred.Type = CRED_TYPE_GENERIC;
            cred.TargetName = target_wide.as_mut_ptr();
            cred.CredentialBlobSize = secret.len() as u32;
            cred.CredentialBlob = secret.as_ptr() as *mut u8;
            // Session persistence: the entry disappears at logoff even if the
            // Drop below never runs.
            cred.Persist = CRED_PERSIST_SESSION;
            let ok = unsafe { CredWriteW(&cred, 0) };
            if ok == 0 {
                return None;
            }
            Some(Self {
                target: target.to_string(),
            })
        }
    }

    impl Drop for TestCredential {
        fn drop(&mut self) {
            let target_wide = wide(&self.target);
            unsafe {
                CredDeleteW(target_wide.as_ptr(), CRED_TYPE_GENERIC, 0);
            }
        }
    }

    #[test]
    fn credreadw_matches_the_full_target_name_and_not_a_prefix() {
        // Deliberately shaped like the go-keyring composition: `base` stands
        // in for the service name and `composed` for what go-keyring actually
        // writes. Reading `base` must fail — that is #1194 in one assertion.
        let base = format!("tokscale-test:{}:gh", std::process::id());
        let composed = format!("{base}:");
        let Some(_guard) = TestCredential::write(&composed, b"gho_wincred_roundtrip") else {
            eprintln!("skipping: CredWriteW was refused on this runner");
            return;
        };

        assert_eq!(
            read_wincred(&composed).unwrap(),
            "gho_wincred_roundtrip",
            "the composed target must read back verbatim"
        );
        assert!(
            read_wincred(&base).is_err(),
            "CredReadW must not resolve a prefix of the stored target"
        );
    }

    #[test]
    fn read_wincred_trims_nul_padding_written_by_other_tools() {
        let target = format!("tokscale-test:{}:padded:", std::process::id());
        let Some(_guard) = TestCredential::write(&target, b"gho_padded\0") else {
            eprintln!("skipping: CredWriteW was refused on this runner");
            return;
        };
        assert_eq!(read_wincred(&target).unwrap(), "gho_padded");
    }

    #[test]
    fn read_wincred_reports_a_missing_target_as_an_error() {
        let target = format!("tokscale-test:{}:absent:", std::process::id());
        assert!(read_wincred(&target).is_err());
    }
}
