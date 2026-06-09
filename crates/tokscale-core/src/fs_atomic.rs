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

    let existing = encode(tmp_path);
    let new = encode(final_path);
    let result = unsafe {
        MoveFileExW(
            existing.as_ptr(),
            new.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
