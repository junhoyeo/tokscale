use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Local, Utc};

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

    fn utc(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
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
