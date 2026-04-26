//! Cross-platform resolution for tokscale's user config and cache dirs.
//!
//! Tokscale-core needs the same path helpers tokscale-cli uses (settings
//! and message/pricing caches read from related directories), so the
//! resolver lives here and is re-exported from tokscale-cli for callers
//! that already imported it from there. macOS users following the docs
//! expect `~/.config/tokscale/` because that is what `auth.rs`,
//! `cursor.rs`, and `antigravity.rs` already write to.
//! `dirs::config_dir()` would instead return `~/Library/Application Support/`
//! on macOS, splitting state across two roots and silently ignoring
//! settings.json edits the user made via the documented path. This module
//! enforces the unified `~/.config/tokscale/` location on macOS + Linux,
//! while keeping the platform default on Windows.

use std::path::PathBuf;

/// Resolve the tokscale config dir, honoring `TOKSCALE_CONFIG_DIR` first.
///
/// Resolution order:
/// 1. `TOKSCALE_CONFIG_DIR` taken verbatim when set to a non-empty value.
///    Absolute paths are recommended; relative paths are accepted and
///    resolved against the process CWD. Empty strings are treated as
///    unset so the user gets the platform default instead of a surprise
///    `./` write — keeps the resolver consistent with
///    [`is_config_dir_overridden`], which also rejects empty strings.
/// 2. macOS: `$HOME/.config/tokscale` (overrides `dirs::config_dir()`,
///    which would return `~/Library/Application Support/` and split state
///    across two roots — see module docs).
/// 3. Linux: `dirs::config_dir().join("tokscale")` so XDG_CONFIG_HOME is
///    honored. Falls through to `$HOME/.config/tokscale` when neither
///    `XDG_CONFIG_HOME` nor `HOME` resolve.
/// 4. Windows (and any other platform): `dirs::config_dir().join("tokscale")`.
/// 5. Last-ditch fallback: `./.tokscale` so a missing HOME never panics.
pub fn get_config_dir() -> PathBuf {
    if let Some(custom) = std::env::var_os("TOKSCALE_CONFIG_DIR") {
        if !custom.is_empty() {
            return PathBuf::from(custom);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(".config").join("tokscale");
        }
    }

    dirs::config_dir()
        .map(|d| d.join("tokscale"))
        .unwrap_or_else(|| PathBuf::from(".tokscale"))
}

/// Resolve the tokscale cache dir as `<config_dir>/cache`.
///
/// Caches (TUI display data, source-message bincode, pricing JSON, the
/// OpenCode migration record, Wrapped fonts/images) all live under this
/// single subdirectory so an isolated profile (`TOKSCALE_CONFIG_DIR=...`)
/// covers everything in one shot, and so `rm -rf <cache_dir>` is always
/// safe — no durable state mixed in.
pub fn get_cache_dir() -> PathBuf {
    get_config_dir().join("cache")
}

/// Whether `TOKSCALE_CONFIG_DIR` is explicitly set in the environment.
///
/// Callers that want to read a legacy on-disk location during a path
/// transition MUST gate that fallback on this returning `false`. When the
/// override is set (CI sandbox, tests, isolated profile), the user has
/// asked for an explicit, hermetic root — silently ingesting files from
/// the historic `~/.cache/tokscale/` or `~/Library/Caches/tokscale/`
/// locations defeats that contract.
pub fn is_config_dir_overridden() -> bool {
    std::env::var_os("TOKSCALE_CONFIG_DIR").is_some_and(|v| !v.is_empty())
}

/// Pre-#470 cache directory at `dirs::cache_dir()/tokscale`.
///
/// On macOS this resolves to `~/Library/Caches/tokscale/` (where the
/// source-message-cache, pricing caches, and opencode-migration.json
/// historically lived). On Linux this resolves to `$XDG_CACHE_HOME/tokscale`
/// or `~/.cache/tokscale/`.
///
/// Returns `None` when `TOKSCALE_CONFIG_DIR` is set so the override stays
/// hermetic (no legacy-data leak into isolated profiles).
pub fn legacy_dirs_cache_dir() -> Option<PathBuf> {
    if is_config_dir_overridden() {
        return None;
    }
    dirs::cache_dir().map(|d| d.join("tokscale"))
}

/// Pre-#470 cache directory at `~/.cache/tokscale`.
///
/// This is where the TUI display cache (`tui-data-cache.json`) and the
/// Wrapped image / font caches lived before #470 consolidated everything
/// under `<config_dir>/cache`. On Linux this typically equals
/// [`legacy_dirs_cache_dir`]; on macOS it does NOT (Library/Caches vs
/// `.cache`), so both legacy probes need to run during migration.
///
/// Returns `None` when `TOKSCALE_CONFIG_DIR` is set or HOME cannot be
/// resolved.
pub fn legacy_dot_cache_tokscale_dir() -> Option<PathBuf> {
    if is_config_dir_overridden() {
        return None;
    }
    dirs::home_dir().map(|h| h.join(".cache").join("tokscale"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;

    fn save_env() -> (
        Option<std::ffi::OsString>,
        Option<std::ffi::OsString>,
        Option<std::ffi::OsString>,
    ) {
        (
            env::var_os("TOKSCALE_CONFIG_DIR"),
            env::var_os("HOME"),
            env::var_os("XDG_CONFIG_HOME"),
        )
    }

    fn restore_env(
        prev: (
            Option<std::ffi::OsString>,
            Option<std::ffi::OsString>,
            Option<std::ffi::OsString>,
        ),
    ) {
        unsafe {
            match prev.0 {
                Some(v) => env::set_var("TOKSCALE_CONFIG_DIR", v),
                None => env::remove_var("TOKSCALE_CONFIG_DIR"),
            }
            match prev.1 {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
            match prev.2 {
                Some(v) => env::set_var("XDG_CONFIG_HOME", v),
                None => env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }

    #[test]
    #[serial]
    fn env_override_is_returned_verbatim() {
        let prev = save_env();
        unsafe {
            env::set_var("TOKSCALE_CONFIG_DIR", "/tmp/tokscale-custom");
        }
        assert_eq!(get_config_dir(), PathBuf::from("/tmp/tokscale-custom"));
        restore_env(prev);
    }

    #[test]
    #[serial]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn unix_default_is_dot_config_tokscale_under_home() {
        let prev = save_env();
        unsafe {
            env::remove_var("TOKSCALE_CONFIG_DIR");
            env::remove_var("XDG_CONFIG_HOME");
            env::set_var("HOME", "/tmp/tokscale-core-paths-home");
        }
        assert_eq!(
            get_config_dir(),
            PathBuf::from("/tmp/tokscale-core-paths-home/.config/tokscale"),
        );
        restore_env(prev);
    }

    #[test]
    #[serial]
    #[cfg(target_os = "linux")]
    fn linux_honors_xdg_config_home_when_set() {
        let prev = save_env();
        unsafe {
            env::remove_var("TOKSCALE_CONFIG_DIR");
            env::set_var("XDG_CONFIG_HOME", "/tmp/tokscale-core-paths-xdg");
        }
        assert_eq!(
            get_config_dir(),
            PathBuf::from("/tmp/tokscale-core-paths-xdg/tokscale"),
        );
        restore_env(prev);
    }

    #[test]
    #[serial]
    fn cache_dir_is_cache_subdir_of_config_dir() {
        let prev = save_env();
        unsafe {
            env::set_var("TOKSCALE_CONFIG_DIR", "/tmp/tokscale-cache-test");
        }
        assert_eq!(
            get_cache_dir(),
            PathBuf::from("/tmp/tokscale-cache-test/cache")
        );
        restore_env(prev);
    }

    #[test]
    #[serial]
    fn legacy_helpers_return_none_when_overridden() {
        let prev = save_env();
        unsafe {
            env::set_var("TOKSCALE_CONFIG_DIR", "/tmp/tokscale-override");
        }
        assert!(legacy_dirs_cache_dir().is_none());
        assert!(legacy_dot_cache_tokscale_dir().is_none());
        restore_env(prev);
    }

    #[test]
    #[serial]
    fn legacy_helpers_return_some_when_not_overridden() {
        let prev = save_env();
        unsafe {
            env::remove_var("TOKSCALE_CONFIG_DIR");
        }
        assert!(
            legacy_dirs_cache_dir().is_some(),
            "dirs::cache_dir always resolves on test platforms"
        );
        assert!(
            legacy_dot_cache_tokscale_dir().is_some(),
            "HOME is set in test environments"
        );
        restore_env(prev);
    }

    #[test]
    #[serial]
    fn get_config_dir_treats_empty_override_as_unset() {
        // Empty TOKSCALE_CONFIG_DIR previously slipped through and
        // produced PathBuf::from(""), which silently relocated cache
        // writes to ./cache and ./.tokscale. The resolver must agree
        // with `is_config_dir_overridden`: empty == unset.
        let prev = save_env();
        unsafe {
            env::set_var("TOKSCALE_CONFIG_DIR", "");
        }
        let resolved = get_config_dir();
        assert_ne!(
            resolved,
            PathBuf::from(""),
            "empty override must not resolve to the empty path"
        );
        assert!(
            resolved.is_absolute() || resolved == PathBuf::from(".tokscale"),
            "empty override must fall through to platform default, got {resolved:?}"
        );
        restore_env(prev);
    }

    #[test]
    #[serial]
    fn is_config_dir_overridden_treats_empty_string_as_unset() {
        let prev = save_env();
        unsafe {
            env::set_var("TOKSCALE_CONFIG_DIR", "");
        }
        assert!(!is_config_dir_overridden());
        restore_env(prev);
    }
}
