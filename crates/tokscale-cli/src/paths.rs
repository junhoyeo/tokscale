//! CLI-side path helpers.
//!
//! The cross-platform config and cache directory resolution lives in
//! `tokscale_core::paths` so the core crate's caches can resolve the same
//! locations without depending on tokscale-cli. This module re-exports
//! those helpers and adds the macOS legacy-config helper that
//! `Settings::load()` and `load_star_cache()` need (they have to read
//! `~/Library/Application Support/tokscale/` once on upgrade — see #468).

use std::path::PathBuf;

#[allow(unused_imports)]
pub use tokscale_core::paths::{
    get_cache_dir, get_config_dir, is_config_dir_overridden, legacy_dirs_cache_dir,
    legacy_dot_cache_tokscale_dir,
};

/// Legacy macOS config dir (`~/Library/Application Support/tokscale`).
///
/// Returns `None` off macOS, when HOME cannot be resolved, or when
/// `TOKSCALE_CONFIG_DIR` is set (so the env override stays hermetic).
/// Used by `Settings::load()` and `load_star_cache()` so users upgrading
/// from a release that wrote files under `~/Library/Application Support/`
/// keep their preferences on first launch after upgrade.
#[cfg(target_os = "macos")]
pub fn legacy_macos_config_dir() -> Option<PathBuf> {
    if is_config_dir_overridden() {
        return None;
    }
    dirs::config_dir().map(|d| d.join("tokscale"))
}

#[cfg(not(target_os = "macos"))]
pub fn legacy_macos_config_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;

    #[test]
    #[serial]
    #[cfg(target_os = "macos")]
    fn legacy_macos_returns_none_when_overridden() {
        let prev = env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            env::set_var("TOKSCALE_CONFIG_DIR", "/tmp/tokscale-cli-paths-override");
        }
        assert!(legacy_macos_config_dir().is_none());
        unsafe {
            match prev {
                Some(v) => env::set_var("TOKSCALE_CONFIG_DIR", v),
                None => env::remove_var("TOKSCALE_CONFIG_DIR"),
            }
        }
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn legacy_macos_returns_none_off_macos() {
        assert!(legacy_macos_config_dir().is_none());
    }

    #[test]
    fn re_exports_compile_and_match_core() {
        let _config: PathBuf = get_config_dir();
        let _cache: PathBuf = get_cache_dir();
        let _: bool = is_config_dir_overridden();
        let _: Option<PathBuf> = legacy_dirs_cache_dir();
        let _: Option<PathBuf> = legacy_dot_cache_tokscale_dir();
    }
}
