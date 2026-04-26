use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokscale_core::scanner::ScannerSettings;

use super::themes::ThemeName;

const DEFAULT_AUTO_REFRESH_MS: u64 = 60_000;
const MIN_AUTO_REFRESH_MS: u64 = 30_000;
const MAX_AUTO_REFRESH_MS: u64 = 3_600_000;

const DEFAULT_NATIVE_TIMEOUT_MS: u64 = 300_000;
const MIN_NATIVE_TIMEOUT_MS: u64 = 5_000;
const MAX_NATIVE_TIMEOUT_MS: u64 = 3_600_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LightSettings {
    /// When true, every `tokscale --light` run atomically overwrites the
    /// TUI cache (same semantics as `--light --write-cache`). The CLI
    /// flags `--write-cache` / `--no-write-cache` override this per-invocation.
    #[serde(default)]
    pub write_cache: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "default_color_palette")]
    pub color_palette: String,
    #[serde(default)]
    pub auto_refresh_enabled: bool,
    #[serde(default = "default_auto_refresh_ms")]
    pub auto_refresh_ms: u64,
    #[serde(default)]
    pub include_unused_models: bool,
    #[serde(default = "default_native_timeout_ms")]
    pub native_timeout_ms: u64,
    /// Persistent scanner configuration. Allows users to pin additional
    /// OpenCode SQLite paths (and, in future, other scanner overrides)
    /// without having to set env vars on every invocation.
    ///
    /// `#[serde(default)]` makes this a drop-in addition — settings.json
    /// files written before the field existed still load cleanly, and an
    /// empty `"scanner": {}` is equivalent to not setting it at all.
    #[serde(default)]
    pub scanner: ScannerSettings,
    /// Default `--client` filter applied when the user does not pass any
    /// CLI client flag. Lets people pin "I only care about my OpenCode and
    /// Claude usage" without typing `--client opencode,claude` on every
    /// invocation.
    ///
    /// Stored as canonical lowercase ids matching `ClientFilter::as_filter_str`
    /// (e.g. `["opencode", "claude", "synthetic"]`). Unknown ids are dropped
    /// silently at load time so a typo or stale entry never breaks tokscale.
    /// CLI flags always override this list completely — no merging.
    #[serde(default, deserialize_with = "deserialize_string_array_lossy")]
    pub default_clients: Vec<String>,
    #[serde(default)]
    pub light: LightSettings,
}

/// Lossy deserializer for `defaultClients`: accepts an array of arbitrary
/// JSON values, keeps only string elements, and silently drops anything
/// else. Hand-edited settings.json files sometimes end up with stray nulls,
/// numbers, or trailing trash; failing the whole load over one bad element
/// would silently fall back to defaults for *every* setting in the file
/// (theme, scanner paths, etc.), which is a much worse user experience
/// than dropping the bad entry.
fn deserialize_string_array_lossy<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<Vec<serde_json::Value>> = Option::deserialize(deserializer).ok().flatten();
    Ok(value
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect())
}

fn default_color_palette() -> String {
    "blue".to_string()
}

fn default_auto_refresh_ms() -> u64 {
    DEFAULT_AUTO_REFRESH_MS
}

fn default_native_timeout_ms() -> u64 {
    DEFAULT_NATIVE_TIMEOUT_MS
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            color_palette: default_color_palette(),
            auto_refresh_enabled: false,
            auto_refresh_ms: DEFAULT_AUTO_REFRESH_MS,
            include_unused_models: false,
            native_timeout_ms: DEFAULT_NATIVE_TIMEOUT_MS,
            scanner: ScannerSettings::default(),
            default_clients: Vec::new(),
            light: LightSettings::default(),
        }
    }
}

/// Thin helper that loads settings and returns just the scanner portion.
///
/// Every CLI entry point that builds `LocalParseOptions`/`ReportOptions`
/// calls this so user-configured scanner paths are honored on every
/// invocation. Errors during load fall through to
/// [`ScannerSettings::default`] — a missing or malformed settings.json
/// should never break `tokscale` runs.
pub fn load_scanner_settings() -> ScannerSettings {
    Settings::load().scanner
}

/// Returns the user's configured `defaultClients` list as raw lowercase
/// ids. Validation against the live `ClientFilter` enum happens at the
/// CLI boundary so this module stays independent of the CLI types.
///
/// Returns an empty `Vec` when settings.json is missing, malformed, or
/// the field is unset — never errors.
pub fn load_default_clients() -> Vec<String> {
    Settings::load().default_clients
}

impl Settings {
    fn config_path() -> Result<PathBuf> {
        let config_dir = crate::paths::get_config_dir();

        if !config_dir.exists() {
            fs::create_dir_all(&config_dir)?;
        }

        Ok(config_dir.join("settings.json"))
    }

    /// Returns the legacy `~/Library/Application Support/tokscale/settings.json`
    /// path on macOS so `load()` can fall back to it during the transition.
    /// Returns `None` on other platforms or when HOME cannot be resolved.
    fn legacy_macos_path() -> Option<PathBuf> {
        crate::paths::legacy_macos_config_dir().map(|d| d.join("settings.json"))
    }

    pub fn load() -> Self {
        let primary = Self::config_path()
            .ok()
            .and_then(|path| fs::read_to_string(path).ok());

        // Transparent macOS fallback: pre-fix releases wrote settings.json under
        // `~/Library/Application Support/tokscale/`. Read it once if the new
        // path is empty so users don't lose theme / scanner / defaultClients
        // preferences after upgrading. The next `save()` lands at the new
        // canonical path under `~/.config/tokscale/`. Skipped when the user
        // has explicitly pinned a config root via `TOKSCALE_CONFIG_DIR` so
        // CI sandboxes and isolated profiles stay hermetic instead of
        // silently ingesting personal settings from the legacy macOS path.
        let raw = primary.or_else(|| {
            if crate::paths::is_config_dir_overridden() {
                return None;
            }
            Self::legacy_macos_path().and_then(|legacy| fs::read_to_string(legacy).ok())
        });

        raw.and_then(|content| serde_json::from_str(&content).ok())
            .map(|mut s: Settings| {
                s.auto_refresh_ms = s
                    .auto_refresh_ms
                    .clamp(MIN_AUTO_REFRESH_MS, MAX_AUTO_REFRESH_MS);
                s.native_timeout_ms = s
                    .native_timeout_ms
                    .clamp(MIN_NATIVE_TIMEOUT_MS, MAX_NATIVE_TIMEOUT_MS);
                s
            })
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        let content = serde_json::to_string_pretty(self)?;

        // Atomic write: write to temp file, sync, then rename
        // Matches the pattern used in tui/cache.rs and pricing/cache.rs
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let tmp_filename = format!(".settings.{}.{:x}.tmp", std::process::id(), nanos);
        let temp_path = path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(&tmp_filename);

        let write_result = (|| -> Result<()> {
            let mut file = fs::File::create(&temp_path)?;
            use std::io::Write;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            if fs::rename(&temp_path, &path).is_err() {
                // Windows: rename can't overwrite; copy then cleanup so destination is never removed first.
                fs::copy(&temp_path, &path)?;
                let _ = fs::remove_file(&temp_path);
            }
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }

        write_result
    }

    pub fn theme_name(&self) -> ThemeName {
        self.color_palette.parse().unwrap_or(ThemeName::Blue)
    }

    pub fn set_theme(&mut self, theme: ThemeName) {
        self.color_palette = theme.as_str().to_string();
    }

    pub fn get_auto_refresh_interval(&self) -> Option<Duration> {
        if self.auto_refresh_enabled && self.auto_refresh_ms > 0 {
            Some(Duration::from_millis(self.auto_refresh_ms))
        } else {
            None
        }
    }

    pub fn get_native_timeout(&self) -> Duration {
        let timeout_ms = if let Ok(env_val) = std::env::var("TOKSCALE_NATIVE_TIMEOUT_MS") {
            env_val.parse::<u64>().unwrap_or(self.native_timeout_ms)
        } else {
            self.native_timeout_ms
        };

        let clamped = timeout_ms.clamp(MIN_NATIVE_TIMEOUT_MS, MAX_NATIVE_TIMEOUT_MS);
        Duration::from_millis(clamped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[cfg(target_os = "macos")]
    #[serial_test::serial]
    fn load_falls_back_to_legacy_macos_path_when_new_path_missing() {
        // Sandbox HOME so the test never reads or writes a real user's
        // settings.json. Existing macOS users upgrading to the unified
        // path must keep the theme + scanner settings they already have
        // under `~/Library/Application Support/tokscale/`.
        use std::env;
        let temp = tempfile::TempDir::new().unwrap();
        let prev_home = env::var_os("HOME");
        let prev_override = env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            env::set_var("HOME", temp.path());
            env::remove_var("TOKSCALE_CONFIG_DIR");
        }

        let legacy_dir = temp.path().join("Library/Application Support/tokscale");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join("settings.json"),
            r#"{"colorPalette":"halloween","defaultClients":["opencode"]}"#,
        )
        .unwrap();

        // Sanity: new path must be empty so the fallback is what we exercise.
        let new_path = temp.path().join(".config/tokscale/settings.json");
        assert!(!new_path.exists());

        let loaded = Settings::load();
        assert_eq!(loaded.color_palette, "halloween");
        assert_eq!(loaded.default_clients, vec!["opencode".to_string()]);

        unsafe {
            match prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
            match prev_override {
                Some(v) => env::set_var("TOKSCALE_CONFIG_DIR", v),
                None => env::remove_var("TOKSCALE_CONFIG_DIR"),
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[serial_test::serial]
    fn load_skips_legacy_macos_fallback_when_config_dir_overridden() {
        // The whole point of TOKSCALE_CONFIG_DIR is hermeticity. CI sandboxes,
        // tests, and isolated profiles MUST NOT silently inherit theme /
        // scanner / defaultClients from `~/Library/Application Support/`
        // when the user explicitly pinned a config root.
        use std::env;
        let temp = tempfile::TempDir::new().unwrap();
        let legacy_root = tempfile::TempDir::new().unwrap();
        let prev_home = env::var_os("HOME");
        let prev_override = env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            env::set_var("HOME", legacy_root.path());
            env::set_var("TOKSCALE_CONFIG_DIR", temp.path());
        }

        let legacy_dir = legacy_root
            .path()
            .join("Library/Application Support/tokscale");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join("settings.json"),
            r#"{"colorPalette":"halloween","defaultClients":["opencode"]}"#,
        )
        .unwrap();

        let loaded = Settings::load();
        assert_eq!(
            loaded.color_palette,
            Settings::default().color_palette,
            "override must yield default settings, not the legacy file's halloween palette"
        );
        assert!(
            loaded.default_clients.is_empty(),
            "override must not leak defaultClients from the legacy macOS path"
        );

        unsafe {
            match prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
            match prev_override {
                Some(v) => env::set_var("TOKSCALE_CONFIG_DIR", v),
                None => env::remove_var("TOKSCALE_CONFIG_DIR"),
            }
        }
    }

    #[test]
    fn settings_load_backfills_scanner_when_missing_from_json() {
        // Older settings.json files predate the `scanner` key. They must
        // still deserialize cleanly and fall through to ScannerSettings::default.
        let json = r#"{
            "colorPalette": "blue",
            "autoRefreshEnabled": false,
            "autoRefreshMs": 60000,
            "includeUnusedModels": false,
            "nativeTimeoutMs": 300000
        }"#;
        let parsed: Settings = serde_json::from_str(json).unwrap();
        assert!(parsed.scanner.opencode_db_paths.is_empty());
    }

    #[test]
    fn settings_load_reads_scanner_opencode_db_paths() {
        let json = r#"{
            "colorPalette": "blue",
            "autoRefreshEnabled": false,
            "autoRefreshMs": 60000,
            "includeUnusedModels": false,
            "nativeTimeoutMs": 300000,
            "scanner": {
                "opencodeDbPaths": [
                    "/custom/one.db",
                    "/custom/opencode-stable.db"
                ]
            }
        }"#;
        let parsed: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.scanner.opencode_db_paths,
            vec![
                PathBuf::from("/custom/one.db"),
                PathBuf::from("/custom/opencode-stable.db"),
            ]
        );
    }

    #[test]
    fn settings_load_reads_scanner_extra_scan_paths() {
        let json = r#"{
            "colorPalette": "blue",
            "autoRefreshEnabled": false,
            "autoRefreshMs": 60000,
            "includeUnusedModels": false,
            "nativeTimeoutMs": 300000,
            "scanner": {
                "extraScanPaths": {
                    "codex": ["/tmp/project-a/.codex/sessions"],
                    "openclaw": ["/tmp/imports/openclaw/agents"]
                }
            }
        }"#;
        let parsed: Settings = serde_json::from_str(json).unwrap();
        let serialized = serde_json::to_value(&parsed).unwrap();

        assert_eq!(
            serialized["scanner"]["extraScanPaths"]["codex"][0],
            serde_json::json!("/tmp/project-a/.codex/sessions")
        );
        assert_eq!(
            serialized["scanner"]["extraScanPaths"]["openclaw"][0],
            serde_json::json!("/tmp/imports/openclaw/agents")
        );
    }

    #[test]
    fn settings_accepts_empty_scanner_object() {
        // `"scanner": {}` is the documented "no-op" form; must be valid.
        let json = r#"{
            "colorPalette": "blue",
            "autoRefreshEnabled": false,
            "autoRefreshMs": 60000,
            "includeUnusedModels": false,
            "nativeTimeoutMs": 300000,
            "scanner": {}
        }"#;
        let parsed: Settings = serde_json::from_str(json).unwrap();
        assert!(parsed.scanner.opencode_db_paths.is_empty());
    }

    #[test]
    fn settings_round_trips_scanner_section_through_json() {
        // Saving and loading must preserve scanner paths verbatim so that
        // the TUI settings save flow never drops the key silently.
        let mut settings = Settings::default();
        settings.scanner.opencode_db_paths = vec![PathBuf::from("/a/b/opencode.db")];
        let serialized = serde_json::to_string(&settings).unwrap();
        let parsed: Settings = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            parsed.scanner.opencode_db_paths,
            vec![PathBuf::from("/a/b/opencode.db")]
        );
    }

    #[test]
    fn settings_round_trips_scanner_extra_scan_paths_through_json() {
        let json = r#"{
            "colorPalette": "blue",
            "autoRefreshEnabled": false,
            "autoRefreshMs": 60000,
            "includeUnusedModels": false,
            "nativeTimeoutMs": 300000,
            "scanner": {
                "extraScanPaths": {
                    "gemini": ["/tmp/imports/gemini/tmp"]
                }
            }
        }"#;

        let parsed: Settings = serde_json::from_str(json).unwrap();
        let serialized = serde_json::to_string(&parsed).unwrap();
        let round_trip: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(
            round_trip["scanner"]["extraScanPaths"]["gemini"][0],
            serde_json::json!("/tmp/imports/gemini/tmp")
        );
    }

    #[test]
    fn settings_default_clients_defaults_to_empty() {
        // Older settings.json files have no `defaultClients` key — they
        // must still parse and yield the "no defaults configured" state.
        let json = r#"{
            "colorPalette": "blue",
            "autoRefreshEnabled": false,
            "autoRefreshMs": 60000,
            "includeUnusedModels": false,
            "nativeTimeoutMs": 300000
        }"#;
        let parsed: Settings = serde_json::from_str(json).unwrap();
        assert!(parsed.default_clients.is_empty());
    }

    #[test]
    fn settings_default_clients_round_trips() {
        // User-configured list must survive load+save unchanged. This is
        // what `tokscale --client opencode,claude` consults when no CLI
        // flag is present.
        let json = r#"{
            "colorPalette": "blue",
            "autoRefreshEnabled": false,
            "autoRefreshMs": 60000,
            "includeUnusedModels": false,
            "nativeTimeoutMs": 300000,
            "defaultClients": ["opencode", "claude", "synthetic"]
        }"#;
        let parsed: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed.default_clients,
            vec![
                "opencode".to_string(),
                "claude".to_string(),
                "synthetic".to_string()
            ]
        );

        let serialized = serde_json::to_string(&parsed).unwrap();
        let round_trip: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            round_trip["defaultClients"],
            serde_json::json!(["opencode", "claude", "synthetic"])
        );
    }

    #[test]
    fn settings_default_clients_drops_non_string_elements_silently() {
        let json = r#"{
            "colorPalette": "halloween",
            "defaultClients": ["opencode", 123, null, "claude", true, {"x":1}]
        }"#;
        let parsed: Settings = serde_json::from_str(json).expect("settings should still load");
        assert_eq!(parsed.color_palette, "halloween");
        assert_eq!(
            parsed.default_clients,
            vec!["opencode".to_string(), "claude".to_string()]
        );
    }

    #[test]
    fn settings_load_accepts_legacy_json_without_light_section() {
        let json = r#"{
            "colorPalette": "blue",
            "autoRefreshEnabled": false,
            "autoRefreshMs": 60000,
            "includeUnusedModels": false,
            "nativeTimeoutMs": 300000
        }"#;
        let parsed: Settings = serde_json::from_str(json).unwrap();
        assert!(!parsed.light.write_cache);
    }

    #[test]
    fn light_settings_round_trip() {
        let light = LightSettings { write_cache: true };
        let serialized = serde_json::to_string(&light).unwrap();
        let parsed: LightSettings = serde_json::from_str(&serialized).unwrap();
        assert!(parsed.write_cache);
    }
}
