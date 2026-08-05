use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

const CACHE_TTL_SECS: u64 = 3600;

pub fn get_cache_dir() -> PathBuf {
    crate::paths::get_cache_dir()
}

pub fn get_cache_path(filename: &str) -> PathBuf {
    get_cache_dir().join(filename)
}

#[derive(Serialize, Deserialize)]
pub struct CachedData<T> {
    pub timestamp: u64,
    pub data: T,
}

fn load_cache_with_policy<T: for<'de> Deserialize<'de>>(
    filename: &str,
    allow_stale: bool,
) -> Option<T> {
    let canonical_path = get_cache_path(filename);
    let cached: CachedData<T> = match fs::read_to_string(&canonical_path) {
        Ok(content) => serde_json::from_str(&content).ok()?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            legacy_cache_paths(filename).into_iter().find_map(|path| {
                let content = fs::read_to_string(&path).ok()?;
                serde_json::from_str(&content).ok()
            })?
        }
        Err(_) => return None,
    };

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();

    if cached.timestamp > now {
        return None;
    }

    if !allow_stale && now.saturating_sub(cached.timestamp) > CACHE_TTL_SECS {
        return None;
    }

    Some(cached.data)
}

pub fn load_cache<T: for<'de> Deserialize<'de>>(filename: &str) -> Option<T> {
    load_cache_with_policy(filename, false)
}

pub fn load_cache_any_age<T: for<'de> Deserialize<'de>>(filename: &str) -> Option<T> {
    load_cache_with_policy(filename, true)
}

pub fn save_cache<T: Serialize>(filename: &str, data: &T) -> Result<(), std::io::Error> {
    let dir = get_cache_dir();
    fs::create_dir_all(&dir)?;

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
        .as_secs();

    let cached = CachedData {
        timestamp: now,
        data,
    };
    let content = serde_json::to_string(&cached)?;

    let final_path = get_cache_path(filename);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let tmp_filename = format!(".{}.{}.{:x}.tmp", filename, std::process::id(), nanos);
    let tmp_path = dir.join(&tmp_filename);

    use std::io::Write;
    // INVARIANT: All cache writes use atomic temp-file rename. NEVER delete
    // the canonical cache file before writing — a partial save or process
    // crash between delete and rename would lose the cache. The temp-file
    // pattern makes corruption-on-crash impossible.
    let write_result = (|| {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        crate::fs_atomic::replace_file(&tmp_path, &final_path)?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }

    write_result
}

fn legacy_cache_paths(filename: &str) -> Vec<PathBuf> {
    if crate::paths::is_config_dir_overridden() {
        return Vec::new();
    }

    [
        crate::paths::legacy_dirs_cache_dir().map(|d| d.join(filename)),
        crate::paths::legacy_dot_cache_tokscale_dir().map(|d| d.join(filename)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(test)]
mod tests {
    // The imports the one test here needs live in its body: the test is
    // `#[cfg(unix)]`, and at module scope they would be unused imports on
    // Windows.
    #[allow(unused_imports)]
    use super::*;

    /// Unix-only, because the fixture this test needs cannot be placed on
    /// Windows without leaving the sandbox.
    ///
    /// The test has to write a pricing file into `legacy_dirs_cache_dir()`,
    /// which is `dirs::cache_dir()/tokscale`. `dirs::cache_dir()` reads
    /// `XDG_CACHE_HOME` on Linux and `$HOME` on macOS, so the redirects below
    /// move it into the temp dir; on Windows it is a `SHGetKnownFolderPath`
    /// call that no environment variable reaches, so it stays at the real
    /// `%LOCALAPPDATA%\tokscale\`. Writing there would drop a pricing file in
    /// the actual profile of whatever machine ran the suite — the sandbox
    /// escape #997 was about — and the assertion would be meaningless anyway,
    /// since that directory may already hold a cache the canonical lookup
    /// finds first, so the fallback under test would never run.
    ///
    /// `#[cfg(unix)]` rather than a runtime `return`: the previous form was an
    /// early return with no marker, so on Windows this reported as a passing
    /// test that asserted nothing. The condition is not dynamic — it is the
    /// platform — so the gate belongs where a reader can see it. The sandbox
    /// check itself is kept below as an assertion, which fails loudly instead
    /// of skipping if a Unix host ever resolves the legacy root outside the
    /// temp dirs.
    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn load_falls_back_to_legacy_dirs_cache_path() {
        use crate::paths::test_env::EnvGuard;
        use tempfile::TempDir;

        let temp_home = TempDir::new().unwrap();
        let temp_xdg_cache = TempDir::new().unwrap();
        let mut env = EnvGuard::capture(&[
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_CONFIG_HOME",
            "TOKSCALE_CONFIG_DIR",
        ]);
        env.set("HOME", temp_home.path());
        env.set("XDG_CACHE_HOME", temp_xdg_cache.path());
        // Pin XDG_CONFIG_HOME so paths::get_cache_dir() stays inside
        // the sandboxed HOME on Linux CI runners that set this var
        // globally — without the pin, the canonical path resolves
        // outside the temp dir and the legacy fallback never gets
        // exercised because the binary never tries the right legacy
        // root either.
        env.set("XDG_CONFIG_HOME", temp_home.path().join(".config"));
        env.remove("TOKSCALE_CONFIG_DIR");

        let legacy_path = crate::paths::legacy_dirs_cache_dir()
            .unwrap()
            .join("pricing-litellm.json");

        // Never write the fixture outside the sandbox: this file lands in a
        // real user profile if the redirects above did not take. Assert rather
        // than skip, so a host where they stop working reports a failure
        // instead of a silently empty test. See the note on this fn for why
        // Windows is excluded at compile time rather than caught here.
        assert!(
            legacy_path.starts_with(temp_home.path())
                || legacy_path.starts_with(temp_xdg_cache.path()),
            "legacy cache root resolved outside the sandbox ({}); writing the \
             fixture there would touch a real profile",
            legacy_path.display()
        );

        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(
            &legacy_path,
            format!(r#"{{"timestamp":{now},"data":{{"ok":true}}}}"#),
        )
        .unwrap();

        let loaded: Option<serde_json::Value> = load_cache("pricing-litellm.json");
        assert_eq!(loaded.unwrap()["ok"], serde_json::json!(true));
    }
}
