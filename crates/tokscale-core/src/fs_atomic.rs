use std::io;
use std::path::Path;

pub fn replace_file(tmp_path: &Path, final_path: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        windows_replace_file(tmp_path, final_path)
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::fs::rename(tmp_path, final_path)
    }
}

#[cfg(target_os = "windows")]
fn windows_replace_file(tmp_path: &Path, final_path: &Path) -> io::Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    unsafe extern "system" {
        fn MoveFileExW(
            lp_existing_file_name: *const u16,
            lp_new_file_name: *const u16,
            dw_flags: u32,
        ) -> i32;
    }

    fn encode(path: &Path) -> Vec<u16> {
        OsStr::new(path.as_os_str())
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    // MoveFileExW replacing an existing file is a well-known source of
    // transient ERROR_ACCESS_DENIED (5) / ERROR_SHARING_VIOLATION (32) on
    // Windows: antivirus, indexing, and cloud-sync agents routinely hold a
    // brief scan handle open on a just-written file. Retry a handful of
    // times with a short backoff before giving up, rather than surfacing a
    // one-shot failure for what is usually a few-millisecond lock.
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const MAX_ATTEMPTS: u32 = 5;

    let existing = encode(tmp_path);
    let new = encode(final_path);

    for attempt in 1..=MAX_ATTEMPTS {
        let result = unsafe {
            MoveFileExW(
                existing.as_ptr(),
                new.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result != 0 {
            return Ok(());
        }

        let error = io::Error::last_os_error();
        let is_retryable = matches!(
            error.raw_os_error(),
            Some(ERROR_ACCESS_DENIED) | Some(ERROR_SHARING_VIOLATION)
        );
        if !is_retryable || attempt == MAX_ATTEMPTS {
            return Err(error);
        }
        std::thread::sleep(std::time::Duration::from_millis(10 * attempt as u64));
    }

    unreachable!("loop always returns on its final attempt")
}
