use crate::clients::ClientId;
use crate::sessions::codex::CodexParseState;
use crate::UnifiedMessage;
use bincode::Options;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

// CACHE_FORMAT_VERSION changes only when the serialized storage layout or a
// cross-client type such as UnifiedMessage changes incompatibly. Parser-only
// changes belong in parser_version() so one client cannot evict every other
// client's cached transcripts.
// 2: Related-file fingerprints now retain their paths and whether they were
// absent when cached. Claude sidechain parent candidates can therefore be
// revalidated without reparsing the sidechain on every warm scan, while a
// later-created parent transcript still invalidates the entry.
// 3: UnifiedMessage gained session_title, changing the bincode payload layout.
// Old shards must read as Stale (silent rebuild), not Invalid (corruption
// warning), so the format version moves with the struct.
// 4: UnifiedMessage gained model_attribution_conflicted, changing the bincode
// payload layout. Old shards must be silently rebuilt rather than decoded.
// 5: Prime Agent entries cache reconciliation accounting beside their messages.
// Version-4 shards have an explicit wire migration below, so other clients stay
// warm and Prime entries need only one rebuild/backfill.
const CACHE_FORMAT_VERSION: u32 = 5;
const LEGACY_CACHE_FORMAT_VERSION: u32 = 4;
// V2 intentionally starts cold and leaves source-message-cache.bin untouched:
// the monolith did not record a trustworthy parser owner for migration.
const CACHE_SHARD_DIRNAME: &str = "source-message-cache-v2";
const CACHE_LOCK_FILENAME: &str = "source-message-cache.lock";
const CACHE_SHARD_COUNT: usize = 256;
const MAX_CACHE_SHARD_BYTES: u64 = 256 * 1024 * 1024;
const FINGERPRINT_SAMPLE_BYTES: usize = 4096;
const FINGERPRINT_SAMPLE_POINTS: usize = 5;
const HASH_BUFFER_BYTES: usize = 64 * 1024;

#[cfg(test)]
thread_local! {
    static FULL_HASH_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn cache_dir() -> Option<PathBuf> {
    if crate::paths::is_config_dir_overridden()
        || dirs::config_dir().is_some()
        || cfg!(target_os = "macos") && crate::paths::home_dir().is_some()
    {
        Some(crate::paths::get_cache_dir())
    } else {
        fallback_cache_dir()
    }
}

fn cache_shard_dir() -> Option<PathBuf> {
    Some(cache_dir()?.join(CACHE_SHARD_DIRNAME))
}

fn cache_lock_path() -> Option<PathBuf> {
    Some(cache_dir()?.join(CACHE_LOCK_FILENAME))
}

fn fallback_cache_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|path| path.join("tokscale"))
        .or_else(user_scoped_temp_dir)
}

#[cfg(unix)]
fn user_scoped_temp_dir() -> Option<PathBuf> {
    let uid = unsafe { libc::geteuid() };
    Some(std::env::temp_dir().join(format!("tokscale-uid-{uid}")))
}

#[cfg(not(unix))]
fn user_scoped_temp_dir() -> Option<PathBuf> {
    std::env::var_os("USERNAME")
        .or_else(|| std::env::var_os("USER"))
        .map(|user| {
            let mut path = std::env::temp_dir();
            path.push(format!("tokscale-user-{}", user.to_string_lossy()));
            path
        })
}

fn ensure_cache_dir(dir: &Path) -> std::io::Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(dir) {
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            return Err(std::io::Error::other(
                "cache directory is not a real directory",
            ));
        }
    }
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

static WARNED_CONTEXTS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();

fn warned_contexts() -> &'static Mutex<HashSet<&'static str>> {
    WARNED_CONTEXTS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn warn_cache_failure_once(context: &'static str, path: &Path, error: &impl std::fmt::Display) {
    warn_cache_failure_once_in(warned_contexts(), context, path, error);
}

/// The once-only set is a parameter purely so the poisoned-set regression test
/// can supply its own. Mutex poisoning is irreversible, so a test that poisoned
/// the process-global set would leave every later test in the binary depending
/// on the very recovery it is checking. Production has exactly one caller and
/// it always passes `warned_contexts()`, so the once-per-process,
/// once-per-context semantics are unchanged.
fn warn_cache_failure_once_in(
    warned: &Mutex<HashSet<&'static str>>,
    context: &'static str,
    path: &Path,
    error: &impl std::fmt::Display,
) {
    tracing::warn!(path = %path.display(), %error, %context, "source message cache failure");

    // Most non-TUI commands (including `submit`) do not install a tracing
    // subscriber. Surface persistence failures directly once per process so a
    // permanently cold cache can never fail silently again. The TUI owns raw
    // mode and the alternate screen for its whole run, so a raw stdio write
    // there corrupts the rendered display. Defer that fallback until the TUI
    // restores the terminal instead of consuming the once-only warning while
    // leaving the user with no visible diagnostic (#941).
    // Recover from a poisoned set the way tui_signal does: an unrelated panic
    // elsewhere must not be what silences the diagnostic this block exists to
    // guarantee. The set only tracks which contexts were already reported, so
    // its contents stay meaningful across an unwind.
    if warned
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(context)
    {
        crate::tui_signal::emit_or_defer_stderr(format!(
            "tokscale: warning: {context} ({}): {error}",
            path.display()
        ));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct FileSampleHash {
    pub offset: u64,
    pub len: u64,
    pub hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SourceFingerprint {
    pub size: u64,
    pub modified_ns: u64,
    pub sample_hashes: Vec<FileSampleHash>,
    pub content_hash: [u8; 32],
    pub related_files: Vec<RelatedFileFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct RelatedFileFingerprint {
    pub suffix: String,
    pub path: CachedPath,
    pub exists: bool,
    pub size: u64,
    pub modified_ns: u64,
    pub sample_hashes: Vec<FileSampleHash>,
    pub content_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FingerprintStatus {
    /// Size and nanosecond mtime still match for the source and every parser
    /// sidecar, and their bounded samples still match. No full-file SHA-256 was
    /// computed, so a warm scan reads at most 20 KiB per watched file.
    Unchanged,
    /// Metadata changed, so a complete fingerprint was rebuilt to distinguish
    /// a real content change from a metadata-only touch.
    Changed(SourceFingerprint),
}

impl SourceFingerprint {
    pub(crate) fn from_path(path: &Path) -> Option<Self> {
        Self::from_path_with_related(path, std::iter::empty())
    }

    #[cfg(test)]
    pub(crate) fn from_sqlite_path(path: &Path) -> Option<Self> {
        let related_paths = ["-wal"]
            .into_iter()
            .map(|suffix| (suffix.to_string(), append_path_suffix(path, suffix)));
        Self::from_path_with_related_mode(path, related_paths, ContentHashMode::SamplesOnly)
    }

    /// Fingerprint for a Jcode session snapshot and its append-only journal
    /// sidecar. Jcode persists recent changes in `session_*.journal.jsonl`
    /// until the next checkpoint rewrites the snapshot, so the source-message
    /// cache must invalidate when either file changes.
    #[cfg(test)]
    pub(crate) fn from_jcode_path(path: &Path) -> Option<Self> {
        let related_paths = std::iter::once((
            ".journal.jsonl".to_string(),
            crate::sessions::jcode::jcode_journal_path(path),
        ));
        Self::from_path_with_related(path, related_paths)
    }

    /// Fingerprint for a Roo-family task (`ui_messages.json`) and its sibling
    /// `api_conversation_history.json`. `parse_roo_kilo_file` reads the history
    /// sibling for the model and agent, so a history-only rewrite (the UI file
    /// unchanged) must still invalidate the cache or reports keep stale
    /// model/agent/pricing.
    #[cfg(test)]
    pub(crate) fn from_roo_path(path: &Path) -> Option<Self> {
        let history = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("api_conversation_history.json");
        let related_paths = std::iter::once(("api_conversation_history.json".to_string(), history));
        Self::from_path_with_related(path, related_paths)
    }

    /// Fingerprint for a Claude Code JSONL file that may have a sibling `.meta.json`
    /// sidecar. When the sidecar appears or changes (e.g. after a Claude Code upgrade),
    /// the fingerprint changes and the cache invalidates.
    #[cfg(test)]
    pub(crate) fn from_claude_code_path_with_home(
        path: &Path,
        home_dir: Option<&Path>,
    ) -> Option<Self> {
        let mut related = Vec::new();

        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            let meta_filename = format!("{}.meta.json", stem);
            related.push((".meta.json".to_string(), path.with_file_name(meta_filename)));
        }

        if let Some(variant_path) = crate::cc_mirror::variant_file_for_session_path(path, home_dir)
        {
            related.push(("cc-mirror/variant.json".to_string(), variant_path));
        }
        for (index, parent_path) in
            crate::sessions::claudecode::parent_session_paths_for_cache(path)
                .into_iter()
                .enumerate()
        {
            related.push((format!("parent-session-{index}.jsonl"), parent_path));
        }

        Self::from_path_with_related(path, related)
    }

    /// Fingerprint for a Grok source and every file or directory read by its
    /// parser for rollup and session metadata. Unified-log parsing also reads
    /// metadata across the complete sessions tree.
    #[cfg(test)]
    pub(crate) fn from_grok_path(path: &Path) -> Option<Self> {
        Self::from_path_with_related(path, crate::sessions::grok::grok_related_paths(path))
    }

    /// Fingerprint for a Kiro source file. IDE sessions consume a sibling
    /// `messages.jsonl`, while CLI `*.json` headers consume same-stem `*.jsonl`.
    /// Global-storage and `.chat` snapshots are self-contained.
    #[cfg(test)]
    pub(crate) fn from_kiro_path(path: &Path) -> Option<Self> {
        let Some(messages) = crate::sessions::kiro::kiro_related_messages_path(path) else {
            return Self::from_path(path);
        };
        let related_paths = std::iter::once(("messages.jsonl".to_string(), messages));
        Self::from_path_with_related(path, related_paths)
    }

    #[cfg(test)]
    pub(crate) fn from_droid_path(path: &Path) -> Option<Self> {
        let Some(jsonl) = crate::sessions::droid::droid_jsonl_path(path) else {
            return Self::from_path(path);
        };
        let related_paths = std::iter::once(("session.jsonl".to_string(), jsonl));
        Self::from_path_with_related(path, related_paths)
    }

    #[cfg(test)]
    pub(crate) fn from_kimi_path(path: &Path) -> Option<Self> {
        if crate::sessions::kimi::is_kimi_code_path(path) {
            return Self::from_path(path);
        }
        let Some(config) = crate::sessions::kimi::kimi_config_path(path) else {
            return Self::from_path(path);
        };
        let related_paths = std::iter::once(("config.json".to_string(), config));
        Self::from_path_with_related(path, related_paths)
    }

    pub(crate) fn check_path(path: &Path, cached: Option<&Self>) -> Option<FingerprintStatus> {
        Self::check_path_with_related(path, std::iter::empty(), cached)
    }

    /// Check a non-Codex source without rebuilding its write-only whole-file
    /// hash when metadata or samples changed. Codex uses `check_path` because
    /// its incremental resume state compares the full content hash; generic
    /// parsers only need the bounded samples for invalidation.
    pub(crate) fn check_path_samples_only(
        path: &Path,
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus> {
        Self::check_path_with_related_mode(
            path,
            std::iter::empty(),
            cached,
            ContentHashMode::SamplesOnly,
        )
    }

    pub(crate) fn check_sqlite_path(
        path: &Path,
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus> {
        let related_paths = ["-wal"]
            .into_iter()
            .map(|suffix| (suffix.to_string(), append_path_suffix(path, suffix)));
        // SQLite databases can be tens of GB; skip the whole-file content hash
        // (size + mtime + samples detect changes, and no SQLite source reads
        // content_hash). See ContentHashMode.
        Self::check_path_with_related_mode(
            path,
            related_paths,
            cached,
            ContentHashMode::SamplesOnly,
        )
    }

    /// Fingerprint a Devin Desktop ACP stream together with every CLI database
    /// that can resolve its title to a model/session id. A database or WAL
    /// change can alter a cached Desktop message even when the NDJSON stream is
    /// untouched, so the lookup inputs must be watched as related files.
    pub(crate) fn check_devin_desktop_path_samples_only(
        path: &Path,
        devin_db_paths: &[PathBuf],
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus> {
        let related_paths = devin_db_paths
            .iter()
            .enumerate()
            .flat_map(|(index, db_path)| {
                let prefix = format!("devin-cli-db-{index}");
                [
                    (prefix.clone(), db_path.clone()),
                    (format!("{prefix}-wal"), append_path_suffix(db_path, "-wal")),
                ]
            });
        Self::check_path_with_related_mode(
            path,
            related_paths,
            cached,
            ContentHashMode::SamplesOnly,
        )
    }

    pub(crate) fn check_jcode_path_samples_only(
        path: &Path,
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus> {
        Self::check_jcode_path_with_mode(path, cached, ContentHashMode::SamplesOnly)
    }

    fn check_jcode_path_with_mode(
        path: &Path,
        cached: Option<&Self>,
        mode: ContentHashMode,
    ) -> Option<FingerprintStatus> {
        let related_paths = std::iter::once((
            ".journal.jsonl".to_string(),
            crate::sessions::jcode::jcode_journal_path(path),
        ));
        Self::check_path_with_related_mode(path, related_paths, cached, mode)
    }

    pub(crate) fn check_roo_path_samples_only(
        path: &Path,
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus> {
        Self::check_roo_path_with_mode(path, cached, ContentHashMode::SamplesOnly)
    }

    pub(crate) fn check_cline_path_samples_only(
        path: &Path,
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus> {
        let related_paths = if crate::sessions::cline::is_cline_cli_messages_path(path) {
            std::iter::once((
                "manifest.json".to_string(),
                crate::sessions::cline::cline_cli_manifest_path(path),
            ))
            .collect::<Vec<_>>()
        } else {
            let history = path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("api_conversation_history.json");
            vec![("api_conversation_history.json".to_string(), history)]
        };
        Self::check_path_with_related_mode(
            path,
            related_paths,
            cached,
            ContentHashMode::SamplesOnly,
        )
    }

    fn check_roo_path_with_mode(
        path: &Path,
        cached: Option<&Self>,
        mode: ContentHashMode,
    ) -> Option<FingerprintStatus> {
        let history = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("api_conversation_history.json");
        let related_paths = std::iter::once(("api_conversation_history.json".to_string(), history));
        Self::check_path_with_related_mode(path, related_paths, cached, mode)
    }

    pub(crate) fn check_claude_code_path_with_home_samples_only(
        path: &Path,
        cached: Option<&Self>,
        home_dir: Option<&Path>,
    ) -> Option<FingerprintStatus> {
        Self::check_claude_code_path_with_home_mode(
            path,
            cached,
            home_dir,
            ContentHashMode::SamplesOnly,
        )
    }

    fn check_claude_code_path_with_home_mode(
        path: &Path,
        cached: Option<&Self>,
        home_dir: Option<&Path>,
        mode: ContentHashMode,
    ) -> Option<FingerprintStatus> {
        let mut related = Vec::new();

        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            let meta_filename = format!("{}.meta.json", stem);
            related.push((".meta.json".to_string(), path.with_file_name(meta_filename)));
        }

        if let Some(variant_path) = crate::cc_mirror::variant_file_for_session_path(path, home_dir)
        {
            related.push(("cc-mirror/variant.json".to_string(), variant_path));
        }

        let primary_matches =
            cached.and_then(|fingerprint| primary_fingerprint_matches(path, fingerprint));
        let parent_paths = cached
            .filter(|_| primary_matches == Some(true))
            .map(cached_claude_parent_paths)
            .unwrap_or_else(|| {
                crate::sessions::claudecode::parent_session_paths_for_cache(path)
                    .into_iter()
                    .enumerate()
                    .map(|(index, parent_path)| {
                        (format!("parent-session-{index}.jsonl"), parent_path)
                    })
                    .collect()
            });
        related.extend(parent_paths);

        Self::check_path_with_related_mode_and_primary(path, related, cached, mode, primary_matches)
    }

    pub(crate) fn check_grok_path_samples_only(
        path: &Path,
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus> {
        Self::check_grok_path_with_mode(path, cached, ContentHashMode::SamplesOnly)
    }

    fn check_grok_path_with_mode(
        path: &Path,
        cached: Option<&Self>,
        mode: ContentHashMode,
    ) -> Option<FingerprintStatus> {
        let related_paths = crate::sessions::grok::grok_related_paths(path);
        Self::check_path_with_related_mode(path, related_paths, cached, mode)
    }

    pub(crate) fn check_kiro_path_samples_only(
        path: &Path,
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus> {
        Self::check_kiro_path_with_mode(path, cached, ContentHashMode::SamplesOnly)
    }

    fn check_kiro_path_with_mode(
        path: &Path,
        cached: Option<&Self>,
        mode: ContentHashMode,
    ) -> Option<FingerprintStatus> {
        let Some(messages) = crate::sessions::kiro::kiro_related_messages_path(path) else {
            return Self::check_path_with_related_mode(path, std::iter::empty(), cached, mode);
        };
        let related_paths = std::iter::once(("messages.jsonl".to_string(), messages));
        Self::check_path_with_related_mode(path, related_paths, cached, mode)
    }

    pub(crate) fn check_droid_path_samples_only(
        path: &Path,
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus> {
        Self::check_droid_path_with_mode(path, cached, ContentHashMode::SamplesOnly)
    }

    fn check_droid_path_with_mode(
        path: &Path,
        cached: Option<&Self>,
        mode: ContentHashMode,
    ) -> Option<FingerprintStatus> {
        let Some(jsonl) = crate::sessions::droid::droid_jsonl_path(path) else {
            return Self::check_path_with_related_mode(path, std::iter::empty(), cached, mode);
        };
        let related_paths = std::iter::once(("session.jsonl".to_string(), jsonl));
        Self::check_path_with_related_mode(path, related_paths, cached, mode)
    }

    pub(crate) fn check_kimi_path_samples_only(
        path: &Path,
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus> {
        Self::check_kimi_path_with_mode(path, cached, ContentHashMode::SamplesOnly)
    }

    /// Stats are append-only JSONL; use bounded samples to avoid hashing a
    /// growing daily log on every warm scan.
    pub(crate) fn check_reasonix_path_samples_only(
        path: &Path,
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus> {
        Self::check_path_with_related_mode(
            path,
            std::iter::empty(),
            cached,
            ContentHashMode::SamplesOnly,
        )
    }

    fn check_kimi_path_with_mode(
        path: &Path,
        cached: Option<&Self>,
        mode: ContentHashMode,
    ) -> Option<FingerprintStatus> {
        if crate::sessions::kimi::is_kimi_code_path(path) {
            return Self::check_path_with_related_mode(path, std::iter::empty(), cached, mode);
        }
        let Some(config) = crate::sessions::kimi::kimi_config_path(path) else {
            return Self::check_path_with_related_mode(path, std::iter::empty(), cached, mode);
        };
        let related_paths = std::iter::once(("config.json".to_string(), config));
        Self::check_path_with_related_mode(path, related_paths, cached, mode)
    }

    fn check_path_with_related<I>(
        path: &Path,
        related_paths: I,
        cached: Option<&Self>,
    ) -> Option<FingerprintStatus>
    where
        I: IntoIterator<Item = (String, PathBuf)>,
    {
        Self::check_path_with_related_mode(path, related_paths, cached, ContentHashMode::Full)
    }

    fn check_path_with_related_mode<I>(
        path: &Path,
        related_paths: I,
        cached: Option<&Self>,
        mode: ContentHashMode,
    ) -> Option<FingerprintStatus>
    where
        I: IntoIterator<Item = (String, PathBuf)>,
    {
        Self::check_path_with_related_mode_and_primary(path, related_paths, cached, mode, None)
    }

    fn check_path_with_related_mode_and_primary<I>(
        path: &Path,
        related_paths: I,
        cached: Option<&Self>,
        mode: ContentHashMode,
        primary_matches: Option<bool>,
    ) -> Option<FingerprintStatus>
    where
        I: IntoIterator<Item = (String, PathBuf)>,
    {
        let related_paths: Vec<(String, PathBuf)> = related_paths.into_iter().collect();
        let cache_hit = cached.is_some_and(|fingerprint| {
            primary_matches
                .unwrap_or_else(|| primary_fingerprint_matches(path, fingerprint).unwrap_or(false))
                && related_fingerprint_metadata_matches(&related_paths, fingerprint)
                    .unwrap_or(false)
        });
        if cache_hit {
            return Some(FingerprintStatus::Unchanged);
        }

        Self::from_path_with_related_mode(path, related_paths, mode).map(FingerprintStatus::Changed)
    }

    fn from_path_with_related<I>(path: &Path, related_paths: I) -> Option<Self>
    where
        I: IntoIterator<Item = (String, PathBuf)>,
    {
        Self::from_path_with_related_mode(path, related_paths, ContentHashMode::Full)
    }

    fn from_path_with_related_mode<I>(
        path: &Path,
        related_paths: I,
        mode: ContentHashMode,
    ) -> Option<Self>
    where
        I: IntoIterator<Item = (String, PathBuf)>,
    {
        let (size, modified_ns, sample_hashes, content_hash) = file_fingerprint_parts(path, mode)?;
        let mut related_files: Vec<RelatedFileFingerprint> = related_paths
            .into_iter()
            .map(|(suffix, related_path)| {
                RelatedFileFingerprint::from_path(suffix, &related_path, mode)
            })
            .collect::<Option<_>>()?;
        related_files.sort_by(|left, right| left.suffix.cmp(&right.suffix));

        Some(Self {
            size,
            modified_ns,
            sample_hashes,
            content_hash,
            related_files,
        })
    }
}

impl RelatedFileFingerprint {
    fn from_path(suffix: String, path: &Path, mode: ContentHashMode) -> Option<Self> {
        let cached_path = CachedPath::from_path(path);
        match path.metadata() {
            Ok(_) => {
                let (size, modified_ns, sample_hashes, content_hash) =
                    file_fingerprint_parts(path, mode)?;
                Some(Self {
                    suffix,
                    path: cached_path,
                    exists: true,
                    size,
                    modified_ns,
                    sample_hashes,
                    content_hash,
                })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(Self {
                suffix,
                path: cached_path,
                exists: false,
                size: 0,
                modified_ns: 0,
                sample_hashes: Vec::new(),
                content_hash: [0; 32],
            }),
            Err(_) => None,
        }
    }
}

fn cached_claude_parent_paths(cached: &SourceFingerprint) -> Vec<(String, PathBuf)> {
    cached
        .related_files
        .iter()
        .filter(|related| related.suffix.starts_with("parent-session-"))
        .map(|related| (related.suffix.clone(), related.path.to_path_buf()))
        .collect()
}

fn primary_fingerprint_matches(path: &Path, cached: &SourceFingerprint) -> Option<bool> {
    let (size, modified_ns) = metadata_signature(path).ok()?;
    if size != cached.size || modified_ns != cached.modified_ns {
        return Some(false);
    }
    Some(compute_sample_hashes(path, size)? == cached.sample_hashes)
}

fn metadata_signature(path: &Path) -> std::io::Result<(u64, u64)> {
    let metadata = path.metadata()?;
    let modified_ns = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos() as u64;
    Ok((metadata.len(), modified_ns))
}

fn related_fingerprint_metadata_matches(
    related_paths: &[(String, PathBuf)],
    cached: &SourceFingerprint,
) -> Option<bool> {
    if cached.related_files.len() != related_paths.len() {
        return Some(false);
    }

    for (suffix, related_path) in related_paths {
        let Some(related) = cached
            .related_files
            .iter()
            .find(|related| related.suffix == *suffix)
        else {
            return Some(false);
        };
        if related.path != CachedPath::from_path(related_path) {
            return Some(false);
        }
        match metadata_signature(related_path) {
            Ok((size, modified_ns)) => {
                if !related.exists || related.size != size || related.modified_ns != modified_ns {
                    return Some(false);
                }
                if compute_sample_hashes(related_path, size)? != related.sample_hashes {
                    return Some(false);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if related.exists {
                    return Some(false);
                }
            }
            Err(_) => return None,
        }
    }

    Some(true)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CodexIncrementalCache {
    pub state: CodexParseState,
    pub consumed_offset: u64,
    pub ends_with_newline: bool,
    pub prefix_hash: [u8; 32],
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct CachedPath(Vec<u8>);

#[cfg(unix)]
impl CachedPath {
    pub(crate) fn from_path(path: &Path) -> Self {
        use std::os::unix::ffi::OsStrExt;

        Self(path.as_os_str().as_bytes().to_vec())
    }

    pub(crate) fn to_path_buf(&self) -> PathBuf {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        PathBuf::from(OsString::from_vec(self.0.clone()))
    }

    fn update_digest(&self, hasher: &mut Sha256) {
        hasher.update(&self.0);
    }
}

/// `/` and `\` as UTF-16 code units, and the `\\?\` verbatim prefix.
#[cfg(windows)]
const FORWARD_SLASH_UTF16: u16 = b'/' as u16;
#[cfg(windows)]
const BACKSLASH_UTF16: u16 = b'\\' as u16;
#[cfg(windows)]
const VERBATIM_PREFIX_UTF16: [u16; 4] = [
    BACKSLASH_UTF16,
    BACKSLASH_UTF16,
    b'?' as u16,
    BACKSLASH_UTF16,
];

/// The stored spelling is kept verbatim so [`CachedPath::to_path_buf`] hands
/// back exactly the path that was cached, but *identity* — equality, hashing
/// and the shard digest — folds `/` into `\` first. See [`CachedPath::
/// identity_units`] for why.
#[cfg(windows)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedPath(Vec<u16>);

#[cfg(windows)]
impl CachedPath {
    pub(crate) fn from_path(path: &Path) -> Self {
        use std::os::windows::ffi::OsStrExt;

        Self(path.as_os_str().encode_wide().collect())
    }

    pub(crate) fn to_path_buf(&self) -> PathBuf {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        PathBuf::from(OsString::from_wide(&self.0))
    }

    /// The code units this path is *identified* by: the stored ones, with `/`
    /// folded to `\`.
    ///
    /// On Windows both characters are directory separators, so `C:\a/b\f.jsonl`
    /// and `C:\a\b\f.jsonl` name one file — and a scan produces both spellings
    /// for that one file. `ClientDef::resolve_path` assembles every scan root by
    /// string concatenation (`format!("{root}/{relative}")`), so the root half
    /// carries forward slashes, while `WalkDir` appends each child below it with
    /// the platform separator. Hashing the units as written therefore gave one
    /// file two cache keys.
    ///
    /// That is not only a test artifact. `tokscale --home C:/Users/me` and a
    /// default run (where `dirs` yields `C:\Users\me`) disagree on every key, so
    /// neither run can ever read the other's entries: the cache stays cold and
    /// the shards accumulate a duplicate copy of every file. Git Bash and MSYS2
    /// export `HOME` with forward slashes, so this is reachable without anyone
    /// typing an unusual path.
    ///
    /// Paths in the verbatim namespace are exempt. After `\\?\` the object
    /// manager performs no translation at all, so `/` there is an ordinary
    /// character in a name rather than a separator, and folding it would merge
    /// two genuinely different paths.
    ///
    /// Case is deliberately *not* folded. Windows filesystems are usually but
    /// not always case-insensitive — NTFS supports per-directory sensitivity —
    /// so folding case could merge two real files. Separator folding has no such
    /// exception outside the verbatim namespace, which is why only it is safe.
    fn identity_units(&self) -> impl Iterator<Item = u16> + '_ {
        let verbatim = self.0.starts_with(&VERBATIM_PREFIX_UTF16);
        self.0.iter().map(move |unit| {
            if !verbatim && *unit == FORWARD_SLASH_UTF16 {
                BACKSLASH_UTF16
            } else {
                *unit
            }
        })
    }

    fn update_digest(&self, hasher: &mut Sha256) {
        for code_unit in self.identity_units() {
            hasher.update(code_unit.to_le_bytes());
        }
    }
}

#[cfg(windows)]
impl PartialEq for CachedPath {
    fn eq(&self, other: &Self) -> bool {
        self.identity_units().eq(other.identity_units())
    }
}

#[cfg(windows)]
impl Eq for CachedPath {}

#[cfg(windows)]
impl std::hash::Hash for CachedPath {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Length first, mirroring the `Vec<u16>` derive this replaces. Folding
        // `/` to `\` never changes the length, so this stays consistent with
        // the `PartialEq` above.
        state.write_usize(self.0.len());
        for code_unit in self.identity_units() {
            state.write_u16(code_unit);
        }
    }
}

#[cfg(not(any(unix, windows)))]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct CachedPath(String);

#[cfg(not(any(unix, windows)))]
impl CachedPath {
    pub(crate) fn from_path(path: &Path) -> Self {
        Self(path.to_string_lossy().into_owned())
    }

    pub(crate) fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(&self.0)
    }

    fn update_digest(&self, hasher: &mut Sha256) {
        hasher.update(self.0.as_bytes());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CacheIdentity {
    namespace: &'static str,
    parser_version: u32,
}

impl CacheIdentity {
    pub(crate) fn for_client(client: ClientId) -> Self {
        Self {
            namespace: client.as_str(),
            parser_version: parser_version(client),
        }
    }

    pub(crate) const fn synthetic() -> Self {
        Self {
            namespace: "synthetic",
            parser_version: 1,
        }
    }

    fn current_for_namespace(namespace: &str) -> Option<Self> {
        if namespace == "synthetic" {
            return Some(Self::synthetic());
        }
        ClientId::from_str(namespace).map(Self::for_client)
    }

    #[cfg(test)]
    fn all() -> impl Iterator<Item = Self> {
        ClientId::iter()
            .map(Self::for_client)
            .chain(std::iter::once(Self::synthetic()))
    }
}

fn parser_version(client: ClientId) -> u32 {
    match client {
        // These clients accumulated parser-only invalidations under the old
        // global schema. Their independent counters start from those histories
        // so future changes have an obvious local version to increment.
        // v6->v7: `reasoning_output_tokens` is a subset of `output_tokens`, so
        // it is now subtracted out of the output bucket instead of being
        // carried in both. Without this bump an existing cache keeps replaying
        // pre-split rows, and those sessions stay double-priced while looking
        // fixed.
        ClientId::Codex => 7,
        // v4->v5: jcode's assistant-message timestamp is now back-calculated
        // to the turn start (timestamp - tool_duration_ms) instead of using
        // the recorded (end-anchored) timestamp directly. Follow-up to #890.
        // v5->v6: OpenAI-style Jcode usage now removes cache-read overlap from
        // input_tokens before pricing and aggregation.
        // v6->v7: snapshot and journal message arrays are now parsed
        // leniently (a single wrong-typed token_usage no longer drops the
        // whole session or its journal line), and a journal replay of an
        // already-seen user message id no longer re-arms pending_turn_start
        // and mints a spurious turn.
        ClientId::Jcode => 7,
        // v5->v6: merge same-dedup-key Copilot spans before emitting messages.
        // v6->v7: all-zero trace/span ids (the W3C sentinel for "no recording
        // span context") are now treated as absent instead of as a real,
        // shared identity, and a valid span_id alone (no trace_id) is now a
        // stable dedup key instead of falling through to the line-index key.
        // v7->v8: stabilize duplicate agent attribution and partial timing boundaries.
        ClientId::Copilot => 8,
        // Pi subagent sessions now derive agent attribution from session_info
        // names; version-1 caches carry those messages without agent metadata.
        ClientId::Pi => 2,
        // Devin CLI v1 could stop at a malformed chat_message. v2->v3:
        // message timestamp is now back-calculated to the turn start
        // (created_at - total_time_ms) instead of the recorded (end-anchored)
        // created_at. Follow-up to #890.
        ClientId::DevinCli => 3,
        // Desktop v1 parsed a non-ACP shape and did not track its CLI title
        // lookup; its timestamp handling is unaffected by the #890 follow-up.
        ClientId::DevinDesktop => 2,
        // WARNING — bumping this discards data that is not recoverable by
        // re-parsing. Claude Code rewrites a transcript in place on
        // resume/compact, and since #994 a Claude entry deliberately carries
        // assistant turns the live file no longer contains (see
        // `HistoryRetention::RetainObserved`). A bump drops every entry, and
        // the cold rebuild that follows sees only the compacted file — so it
        // silently retires those turns from every user's totals and
        // reintroduces the exact drift #994 reported. Bumping for a real
        // parser change is still correct; do it knowing the cost, and prefer
        // a fix that does not need one.
        ClientId::Claude => 2,
        // Junie's usage-event timestamp is now back-calculated to the call
        // start (timestampMs - usage.time) instead of the recorded
        // (end-anchored) timestampMs. Follow-up to #890. v2->v3: preserve
        // provider-reported cost provenance, including explicit zeroes, so
        // strict submission does not reject valid cached unknown-model usage.
        ClientId::Junie => 3,
        // zcode's model_usage timestamp now prefers `started_at` over
        // `completed_at`. Follow-up to #890. v2->v3: rows with a NULL
        // `started_at` now back-calculate `completed_at - duration_ms`
        // instead of staying end-anchored at `completed_at`, and
        // `is_turn_start` is now assigned to the earliest-STARTED request
        // per turn instead of the first one seen in completed_at order.
        // Second-round follow-up to #890.
        ClientId::Zcode => 3,
        // opencodereview's llm_response timestamp is now back-calculated to
        // the call start (timestamp - duration_ms) instead of the recorded
        // (end-anchored) timestamp. Follow-up to #890. v2->v3: records without
        // their own `timestamp` now carry a line-number discriminator in the
        // dedup key, so distinct calls sharing a model and token counts no
        // longer collapse into one under the shared file-mtime fallback.
        // Both bumps were precautionary rather than corrective: until the
        // submit path learned to parse opencodereview, the only reader was
        // parse_local_clients, which does not go through this cache, so no
        // entry was ever written under this namespace. 3 is therefore the
        // first version to describe real cache entries — it is not carrying
        // an invalidation debt forward, and a further bump would have had
        // nothing to invalidate.
        ClientId::OpenCodeReview => 3,
        // Kiro's structured messages.jsonl turns now back-calculate the
        // start anchor from `turn_end - elapsedTime` when the user prompt's
        // own timestamp is missing/unparseable, instead of falling through
        // to the (end-anchored) turn_end timestamp. Second-round follow-up
        // to #890.
        ClientId::Kiro => 2,
        // Kimi v2 checks token buckets without an overflowing sum. v2->v3:
        // symbolic usage-record models now resolve from the latest llm.request.
        // v3->v4: non-positive wire timestamps (kimi-cli `timestamp`,
        // kimi-code `time`) now fall back to the file mtime instead of
        // anchoring the message in a pre-epoch bucket.
        ClientId::Kimi => 4,
        // v1->v2: standalone Cline messages subtract cache buckets from gross
        // input tokens, reject non-finite costs, and preserve zero-cost reports.
        // v2->v3: content-aware Cline CLI turn-start classification now
        // recognizes user tool-result records as continuations instead of
        // beginning a new turn, so cached turns must be reparsed.
        ClientId::Cline => 3,
        // v1->v2: cache-write now maps directly from `Input (w/ Cache Write)`
        // instead of subtracting `Input (w/o Cache Write)`, and a numeric CSV
        // `Cost` (including explicit zero) is retained as provider-reported so
        // repricing no longer drops the Team/Enterprise Cursor Token Rate.
        ClientId::Cursor => 2,
        // v1->v2: Kimchi's Pi-compatible messages now carry stable namespaced
        // deduplication keys.
        ClientId::Kimchi => 2,
        // v1->v2: Prime Agent now strips a leading BOM and recovers records
        // containing undecodable bytes; its accounting scan also continues past
        // those records instead of truncating and misaligning message indices.
        // The bump intentionally forces a full re-decode and accounting/matching
        // rebuild; it makes the legacy v4 accounting-backfill path unreachable
        // for live caches, but avoids mixing v1 cached messages with v2 scans.
        // Without the bump, malformed-line loss could be mistaken for complete
        // accounting rather than a truncated source. v2->v3 rejects damaged
        // lineage and usage structural keys before reconciliation bookkeeping.
        // v3->v4 rejects damaged lineage values and matching-critical child
        // timestamps while preserving unrelated damaged usage extensions.
        ClientId::PrimeAgent => 4,
        // Initial Reasonix implementation. The fingerprint samples the
        // append-only stats JSONL source so appended records are reparsed.
        // v1->v2: strip a leading BOM and recover records containing
        // undecodable bytes instead of silently dropping the whole record.
        // v2->v3: damaged providers use delimiter-aware family inference.
        // v3->v4 applies that recovery to every family with version-aware
        // boundaries, rejecting family-name substrings inside ordinary words.
        ClientId::Reasonix => 4,
        // v1->v2: per-model token attribution now comes from
        // session_model_usage instead of crediting the whole session to
        // sessions.model, and dedup keys are namespaced per (session, model).
        ClientId::Hermes => 2,
        // v2 added per-turn usage records. v3 adds the canonical unified log,
        // non-overlapping output/cache/reasoning buckets, and session metadata.
        // v4 scopes unified model attribution by PID generation and exact child
        // session, so the same source can now produce different model IDs.
        // v5 preserves distinct unified events when timestamps and token
        // buckets repeat, and fingerprints the complete sessions metadata tree.
        // v6 persists whether an unknown unified model was deliberately
        // fail-closed due to conflicting child attribution evidence.
        // v6->v7: session files are now parsed past undecodable lines instead
        // of stopping at the first one, and usage dedup keys carry the record's
        // file position. Both change the parse of byte-identical input, so
        // cached entries hold truncated and under-deduplicated output (#1031).
        ClientId::Grok => 7,
        // v1 retained MiMo's embedded `cost` value but did not preserve its
        // provider-reported provenance. Reparse cached rows so strict submit
        // validation does not reject valid unknown-model MiMo usage offline.
        // v2->v3: duplicate merging now upgrades the retained row when a later
        // copy carries an explicit cost, including zero.
        ClientId::MiMoCode => 3,
        // Droid's cumulative session totals now anchor on the settings file's
        // mtime (floored at providerLockTimestamp) instead of the lock
        // timestamp alone, so a long-running session stops reporting every
        // token it ever spent against the day it was started. A session that
        // has since ended never changes its bytes again, so its fingerprint
        // stays valid forever and only this bump discards the v1 anchor.
        // v2->v3: a session's cumulative total is no longer one record at one
        // instant. It is now apportioned across the assistant replies in the
        // sibling transcript, weighted by the context each reply read, so a
        // multi-day session reports against the days it actually ran.
        // v3->v4: a reply is weighted by the context standing before it rather
        // than including its own bytes, so a long answer no longer charges
        // itself for its own output.
        // v4->v5: output and reasoning follow the reply's own size instead of
        // the context it read, a pre-epoch transcript timestamp no longer
        // anchors a share of the session in 1969, and an oversized transcript
        // takes the single-record path. Versions 2 through 4 only ever existed
        // in pre-release builds of this change; the bump past them keeps anyone
        // who ran one from holding a superseded split.
        // v5->v6: a coalesced run of replies now reports how many calls it
        // stands for instead of one, an unreadable file size no longer waives
        // the transcript ceiling, and a transcript that could not be read whole
        // takes the single-record path rather than apportioning the session
        // over the prefix that was read.
        // v6->v7: the apportioned records are attribution fragments of one
        // session, so the session's reply count now rides on exactly one of
        // them. v6 entries carry a count on every record, which `sessionize`
        // reads as one session per record.
        ClientId::Droid => 7,
        // First version of the fx (vercel-labs) usage-v2.json parser. Entries
        // are versioned from the start so later parser changes have an
        // obvious local counter to bump, like every other client here.
        ClientId::Fx => 1,
        _ => 1,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CacheKey {
    namespace: String,
    path: CachedPath,
}

impl CacheKey {
    fn new(identity: CacheIdentity, path: &Path) -> Self {
        Self {
            namespace: identity.namespace.to_string(),
            path: CachedPath::from_path(path),
        }
    }

    fn from_entry(entry: &CachedSourceEntry) -> Self {
        Self {
            namespace: entry.parser_namespace.clone(),
            path: entry.path.clone(),
        }
    }

    fn shard(&self) -> CacheShardKey {
        let mut hasher = Sha256::new();
        hasher.update(self.namespace.as_bytes());
        hasher.update([0]);
        self.path.update_digest(&mut hasher);
        let digest = hasher.finalize();
        CacheShardKey {
            namespace: self.namespace.clone(),
            index: usize::from(digest[0]) % CACHE_SHARD_COUNT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheShardKey {
    namespace: String,
    index: usize,
}

/// Marks a Claude entry as carrying retention provenance.
///
/// A Claude entry's `fallback_timestamp_indices` lists the messages the live
/// transcript no longer contains. An entry with nothing retained and an entry
/// written before provenance existed both leave that vector empty, so the
/// vector alone cannot answer "is this legacy?" — and answering it wrong
/// either strands stale rows (see
/// [`CachedSourceEntry::needs_retention_provenance_migration`]) or re-parses
/// every Claude transcript on every scan forever.
///
/// `usize::MAX` is never a real message index, so appending it records
/// "provenance is present, retained set may be empty" without changing the
/// serialized layout.
const CLAUDE_RETENTION_PROVENANCE_MARKER: usize = usize::MAX;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedSourceEntry {
    parser_namespace: String,
    parser_version: u32,
    pub path: CachedPath,
    pub fingerprint: SourceFingerprint,
    /// Not always a pure function of the source file. For a namespace that
    /// `retained_history_key_filter` covers, this can hold messages the live
    /// file no longer contains, and re-parsing will not reproduce them — the
    /// cache is the only copy. That is what makes a parser_version bump for
    /// those namespaces lossy rather than merely cold.
    pub messages: Vec<UnifiedMessage>,
    /// Namespace-specific indices that have to survive with the message vector.
    ///
    /// For Codex these identify fallback-timestamp messages. For Claude they
    /// identify messages retained after the live transcript stopped containing
    /// them, plus a trailing [`CLAUDE_RETENTION_PROVENANCE_MARKER`]. Claude
    /// never used this vector before retention provenance, so the second
    /// interpretation preserves the existing bincode layout and avoids a
    /// cache-format bump that would discard unrecoverable compacted history.
    pub fallback_timestamp_indices: Vec<usize>,
    pub codex_incremental: Option<CodexIncrementalCache>,
    /// Prime-only metadata used to reconcile fork aggregates with child
    /// transcripts. It shares this entry's parser identity and fingerprint, so
    /// a message cache hit can never pair with accounting from different bytes.
    pub prime_accounting: Option<crate::sessions::prime_agent::PrimeFileAccounting>,
}

/// Exact version-4 entry layout. Keeping this wire type lets existing shards
/// migrate without discarding cached messages for unrelated clients. Prime
/// entries convert with no accounting and are backfilled once on their next
/// unchanged scan.
#[derive(Debug, Serialize, Deserialize)]
struct LegacyCachedSourceEntryV4 {
    parser_namespace: String,
    parser_version: u32,
    path: CachedPath,
    fingerprint: SourceFingerprint,
    messages: Vec<UnifiedMessage>,
    fallback_timestamp_indices: Vec<usize>,
    codex_incremental: Option<CodexIncrementalCache>,
}

impl From<LegacyCachedSourceEntryV4> for CachedSourceEntry {
    fn from(entry: LegacyCachedSourceEntryV4) -> Self {
        Self {
            parser_namespace: entry.parser_namespace,
            parser_version: entry.parser_version,
            path: entry.path,
            fingerprint: entry.fingerprint,
            messages: entry.messages,
            fallback_timestamp_indices: entry.fallback_timestamp_indices,
            codex_incremental: entry.codex_incremental,
            prime_accounting: None,
        }
    }
}

impl CachedSourceEntry {
    pub(crate) fn new(
        identity: CacheIdentity,
        path: &Path,
        fingerprint: SourceFingerprint,
        messages: Vec<UnifiedMessage>,
        fallback_timestamp_indices: Vec<usize>,
        codex_incremental: Option<CodexIncrementalCache>,
    ) -> Self {
        Self {
            parser_namespace: identity.namespace.to_string(),
            parser_version: identity.parser_version,
            path: CachedPath::from_path(path),
            fingerprint,
            messages,
            fallback_timestamp_indices,
            codex_incremental,
            prime_accounting: None,
        }
    }

    pub(crate) fn with_prime_accounting(
        mut self,
        accounting: crate::sessions::prime_agent::PrimeFileAccounting,
    ) -> Self {
        self.prime_accounting = Some(accounting);
        self
    }

    fn identity_is_current(&self) -> bool {
        CacheIdentity::current_for_namespace(&self.parser_namespace)
            .is_some_and(|identity| identity.parser_version == self.parser_version)
    }

    fn matches_identity(&self, identity: CacheIdentity) -> bool {
        self.parser_namespace == identity.namespace
            && self.parser_version == identity.parser_version
    }

    /// Moves everything that scales with the transcript out of this entry,
    /// leaving the metadata a later `remove` or shard rewrite still needs.
    ///
    /// The husk left behind reports no messages, which every warm-hit check
    /// already treats as "not usable" — so a second lookup degrades to a
    /// re-parse rather than serving a truncated entry. See
    /// [`SourceMessageCache::take`] for why that second lookup cannot happen
    /// for the namespaces where a re-parse would lose history.
    fn take_payload(&mut self) -> Self {
        Self {
            parser_namespace: self.parser_namespace.clone(),
            parser_version: self.parser_version,
            path: self.path.clone(),
            fingerprint: self.fingerprint.clone(),
            messages: std::mem::take(&mut self.messages),
            fallback_timestamp_indices: std::mem::take(&mut self.fallback_timestamp_indices),
            codex_incremental: self.codex_incremental.take(),
            prime_accounting: self.prime_accounting.take(),
        }
    }

    pub(crate) fn is_claude_namespace(&self) -> bool {
        self.parser_namespace == ClientId::Claude.as_str()
    }

    /// Whether this Claude entry predates retention provenance.
    ///
    /// Entries written before the provenance marker existed carry retained
    /// turns mixed in with live ones and no way to tell them apart, so reading
    /// one as-is presents a stale copy of a response as if the live transcript
    /// still contained it. The reader rebuilds those entries once (see
    /// `lib.rs`), which re-derives the retained set from the live bytes and
    /// writes the marker, and this then reports `false` forever after.
    ///
    /// The distinction has to survive on disk, and it cannot be a new struct
    /// field: `CachedSourceEntry` is bincode-encoded without field names, so
    /// adding one needs a `CACHE_FORMAT_VERSION` bump, and that discards every
    /// Claude entry — including the compacted assistant turns only the cache
    /// still holds. The marker rides inside the existing index vector instead.
    pub(crate) fn needs_retention_provenance_migration(&self) -> bool {
        self.is_claude_namespace()
            && !self
                .fallback_timestamp_indices
                .contains(&CLAUDE_RETENTION_PROVENANCE_MARKER)
    }

    /// Whether this entry is carrying rows the live file may no longer
    /// contain — either an identified retained set, or a pre-provenance entry
    /// that cannot say which of its rows are retained and has to be assumed to
    /// hold some.
    ///
    /// Only meaningful for a namespace [`retained_history_key_filter`] covers;
    /// elsewhere the index vector means something else entirely.
    pub(crate) fn holds_retained_history(&self) -> bool {
        self.is_claude_namespace()
            && (self.needs_retention_provenance_migration()
                || self
                    .fallback_timestamp_indices
                    .iter()
                    .any(|index| *index != CLAUDE_RETENTION_PROVENANCE_MARKER))
    }

    pub(crate) fn retained_message_keys(&self) -> HashSet<String> {
        if !self.is_claude_namespace() {
            return HashSet::new();
        }
        self.fallback_timestamp_indices
            .iter()
            .filter(|index| **index != CLAUDE_RETENTION_PROVENANCE_MARKER)
            .filter_map(|index| self.messages.get(*index))
            .filter_map(|message| message.dedup_key.clone())
            .collect()
    }

    fn claude_retained_indices(
        messages: &[UnifiedMessage],
        retained_message_keys: &HashSet<String>,
    ) -> Vec<usize> {
        let mut indices: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                message
                    .dedup_key
                    .as_ref()
                    .is_some_and(|key| retained_message_keys.contains(key))
                    .then_some(index)
            })
            .collect();
        indices.push(CLAUDE_RETENTION_PROVENANCE_MARKER);
        indices
    }

    pub(crate) fn new_with_retained_message_keys(
        identity: CacheIdentity,
        path: &Path,
        fingerprint: SourceFingerprint,
        messages: Vec<UnifiedMessage>,
        retained_message_keys: &HashSet<String>,
    ) -> Self {
        debug_assert_eq!(identity.namespace, ClientId::Claude.as_str());
        let retained_indices = Self::claude_retained_indices(&messages, retained_message_keys);
        Self::new(
            identity,
            path,
            fingerprint,
            messages,
            retained_indices,
            None,
        )
    }

    fn remove_claude_synthetic_placeholders(&mut self) -> bool {
        if !self.is_claude_namespace() {
            return false;
        }
        let retained_keys = self.retained_message_keys();
        // Repairing placeholder rows tells us nothing about which turns are
        // retained, so an entry that arrived without provenance still needs
        // the rebuild afterwards.
        let had_provenance = !self.needs_retention_provenance_migration();
        let changed =
            crate::sessions::claudecode::remove_synthetic_placeholder_messages(&mut self.messages);
        if changed {
            self.fallback_timestamp_indices =
                Self::claude_retained_indices(&self.messages, &retained_keys);
            if !had_provenance {
                self.fallback_timestamp_indices
                    .retain(|index| *index != CLAUDE_RETENTION_PROVENANCE_MARKER);
            }
        }
        changed
    }

    /// Carry forward keyed messages an entry already on disk holds for this
    /// same path and this one does not.
    ///
    /// Two processes can scan at once — a running TUI and a `tokscale submit`,
    /// say. Each loads the entry, parses, and saves back, and the last writer
    /// replaces the other's entry wholesale. For most namespaces that is
    /// harmless: the loser's messages come from the same bytes and reappear on
    /// the next scan. For a namespace that retains history it is not, because
    /// the messages the loser observed are gone from the live file too, so
    /// nothing will ever put them back.
    ///
    /// Same filter as the parse-time merge: a key that is only unique within
    /// one file must not outlive the bytes that produced it.
    fn absorb_retained_history(&mut self, stored: &CachedSourceEntry) {
        let Some(key_is_globally_stable) = retained_history_key_filter(&self.parser_namespace)
        else {
            return;
        };
        // A stored entry from a different parser version describes a layout
        // this one does not agree with; let the wholesale replace stand.
        if stored.parser_namespace != self.parser_namespace
            || stored.parser_version != self.parser_version
        {
            return;
        }

        let mut keyed_indices: HashMap<String, usize> = self
            .messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| message.dedup_key.clone().map(|key| (key, index)))
            .collect();
        for message in &stored.messages {
            let Some(key) = message.dedup_key.as_ref() else {
                continue;
            };
            if !key_is_globally_stable(key) {
                continue;
            }
            if let Some(index) = keyed_indices.get(key).copied() {
                crate::sessions::claudecode::merge_message_completeness(
                    &mut self.messages[index],
                    message,
                );
                continue;
            }
            let index = self.messages.len();
            self.messages.push(message.clone());
            // Relative to this writer's current source fingerprint, a row only
            // the stored entry knew about is retained history even if it was
            // live when the concurrent writer observed it.
            self.fallback_timestamp_indices.push(index);
            keyed_indices.insert(key.clone(), index);
        }
    }
}

/// The dedup-key filter for namespaces whose entries carry history the live
/// file may no longer contain, or `None` for namespaces that do not retain
/// history.
///
/// Mirrors the `HistoryRetention` choice each lane makes in `lib.rs`. It has to
/// exist here as well because the save merge is the other place a retained
/// message can be dropped, and it must honor the same contract.
fn retained_history_key_filter(namespace: &str) -> Option<fn(&str) -> bool> {
    (namespace == ClientId::Claude.as_str())
        .then_some(crate::sessions::claudecode::dedup_key_is_globally_stable)
}

/// The envelope is deliberately independent from CachedSourceEntry's binary
/// layout. A parser version can therefore be checked before its payload is
/// deserialized, so (for example) a CodexParseState layout change cannot make
/// Claude's independently sharded cache unreadable.
#[derive(Debug, Serialize, Deserialize)]
struct CachedShardEnvelope {
    format_version: u32,
    parser_namespace: String,
    parser_version: u32,
    payload: Vec<u8>,
}

#[derive(Debug, Clone)]
enum DeletionReason {
    Invalidated(SourceFingerprint),
    Missing,
}

/// The mutable half of [`SourceMessageCache`].
///
/// It lives behind one mutex because the parse lanes hold the cache by shared
/// reference from inside `rayon` closures, and both of the memory properties
/// this type is responsible for need to mutate through that shared reference:
/// a namespace's shards are read on first use, and an entry's message payload
/// is handed to its one consumer instead of being cloned out from under a copy
/// the cache keeps forever. Every critical section is a hash lookup plus a
/// couple of `mem::take`s, except the once-per-namespace shard read.
#[derive(Default)]
struct CacheState {
    entries: HashMap<CacheKey, CachedSourceEntry>,
    /// Namespaces whose shards were already read (or attempted). Recorded even
    /// when the read fails so a broken cache directory cannot make every file
    /// in a lane retry the same I/O.
    loaded_namespaces: HashSet<&'static str>,
    dirty: bool,
    dirty_keys: HashSet<CacheKey>,
    deleted_keys: HashMap<CacheKey, DeletionReason>,
    rewrite_shards: HashSet<CacheShardKey>,
}

/// The persisted parse cache, read lazily and drained as it is consumed.
///
/// Both behaviours exist for one reason: a scan's memory must be proportional
/// to what it actually reads, not to everything the machine has ever cached.
/// Loading every namespace up front and keeping every entry alive for the
/// whole scan made a `tokscale` run cost the size of the entire cache plus the
/// size of its own output — a single-message `-c droid` scan peaked at 1.16 GB
/// against a 358 MB cache, and the TUI paid that peak again on every
/// auto-refresh until the process was killed (#1100).
#[derive(Default)]
pub(crate) struct SourceMessageCache {
    state: Mutex<CacheState>,
    /// `false` for [`SourceCachePolicy::InMemory`] callers, who must never
    /// touch the on-disk shards.
    persistent: bool,
}

impl SourceMessageCache {
    /// Opens the persistent cache without reading any shard.
    ///
    /// Shards are read per namespace on first access, so a scan that only
    /// looks at one client never deserializes another client's history.
    pub(crate) fn load() -> Self {
        Self {
            state: Mutex::new(CacheState::default()),
            persistent: true,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, CacheState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Reads `namespace`'s shards into `state` the first time it is needed.
    ///
    /// Missing-source pruning happens here rather than in one pass over the
    /// whole cache, so it stays scoped to the namespaces this scan touched.
    fn ensure_namespace_loaded(&self, state: &mut CacheState, namespace: &'static str) {
        if !self.persistent || !state.loaded_namespaces.insert(namespace) {
            return;
        }
        let Some(identity) = CacheIdentity::current_for_namespace(namespace) else {
            return;
        };
        let Some(shard_root) = cache_shard_dir() else {
            return;
        };
        let Some(lock_path) = cache_lock_path() else {
            return;
        };
        if let Err(error) = ensure_cache_dir(&shard_root) {
            warn_cache_failure_once(
                "source message cache directory is unavailable",
                &shard_root,
                &error,
            );
            return;
        }
        let lock_file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
        {
            Ok(file) => file,
            Err(error) => {
                warn_cache_failure_once(
                    "source message cache lock is unavailable",
                    &lock_path,
                    &error,
                );
                return;
            }
        };
        if let Err(error) = fs2::FileExt::lock_shared(&lock_file) {
            warn_cache_failure_once("source message cache lock failed", &lock_path, &error);
            return;
        }

        let parser_dir = shard_root.join(namespace);
        let read_dir = match fs::read_dir(&parser_dir) {
            Ok(read_dir) => read_dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                warn_cache_failure_once(
                    "source message cache parser directory is unreadable",
                    &parser_dir,
                    &error,
                );
                return;
            }
        };

        for dir_entry in read_dir.filter_map(Result::ok) {
            let Some(index) = parse_shard_filename(&dir_entry.file_name()) else {
                continue;
            };
            let shard_key = CacheShardKey {
                namespace: namespace.to_string(),
                index,
            };
            let path = dir_entry.path();
            let (entries, migrated) = match read_shard(&path, identity) {
                ShardReadStatus::Loaded(entries) => (entries, false),
                ShardReadStatus::Migrated(entries) => (entries, true),
                ShardReadStatus::Missing => continue,
                ShardReadStatus::Stale => {
                    state.rewrite_shards.insert(shard_key);
                    state.dirty = true;
                    continue;
                }
                ShardReadStatus::Invalid(error) => {
                    warn_cache_failure_once("source message cache shard is invalid", &path, &error);
                    state.rewrite_shards.insert(shard_key);
                    state.dirty = true;
                    continue;
                }
            };
            if migrated {
                state.rewrite_shards.insert(shard_key.clone());
                state.dirty = true;
            }
            for mut entry in entries {
                let key = CacheKey::from_entry(&entry);
                if key.shard() != shard_key || !entry.identity_is_current() {
                    state.rewrite_shards.insert(shard_key.clone());
                    state.dirty = true;
                    continue;
                }
                // A source that no longer exists can never be scanned again,
                // so its entry is dead weight in memory and on disk.
                if !key.path.to_path_buf().exists() {
                    state.deleted_keys.insert(key, DeletionReason::Missing);
                    state.dirty = true;
                    continue;
                }
                // This scan already produced (or deleted) something for this
                // source, and that is newer than the bytes on disk.
                if state.entries.contains_key(&key)
                    || state.dirty_keys.contains(&key)
                    || state.deleted_keys.contains_key(&key)
                {
                    continue;
                }
                if entry.remove_claude_synthetic_placeholders() {
                    // Do not bump Claude's parser version here: compacted
                    // transcripts rely on cached assistant history that a
                    // full invalidation cannot recover. Repair only the bad
                    // `<synthetic>` rows and persist that narrow migration.
                    state.dirty_keys.insert(key.clone());
                    state.dirty = true;
                }
                state.entries.insert(key, entry);
            }
        }
    }

    #[cfg(test)]
    fn load_all_namespaces(&self, state: &mut CacheState) {
        for identity in CacheIdentity::all() {
            self.ensure_namespace_loaded(state, identity.namespace);
        }
    }

    pub(crate) fn insert(&mut self, entry: CachedSourceEntry) {
        let key = CacheKey::from_entry(&entry);
        let state = self.state.get_mut().unwrap_or_else(|p| p.into_inner());
        state.entries.insert(key.clone(), entry);
        state.deleted_keys.remove(&key);
        state.dirty_keys.insert(key);
        state.dirty = true;
    }

    /// Reads an entry without disturbing the cache's copy.
    ///
    /// Production scans use [`Self::take`]; this exists so tests can assert on
    /// what a load produced without consuming it.
    #[cfg(test)]
    pub(crate) fn get(&self, identity: CacheIdentity, path: &Path) -> Option<CachedSourceEntry> {
        let key = CacheKey::new(identity, path);
        let mut state = self.state();
        self.ensure_namespace_loaded(&mut state, identity.namespace);
        state
            .entries
            .get(&key)
            .filter(|entry| entry.matches_identity(identity))
            .cloned()
    }

    /// Hands the entry for `path` to its one consumer, moving the message
    /// payload out of the cache and leaving the entry's metadata behind.
    ///
    /// Every source is looked up at most once per scan, and
    /// [`Self::save_if_dirty`] re-reads each shard it rewrites from disk and
    /// only overlays the entries this scan marked dirty — so a clean entry's
    /// messages are never needed again in memory, and releasing them here is
    /// what keeps a scan from holding the whole cache and its own output at
    /// the same time.
    ///
    /// Two cases keep their payload:
    ///
    /// * An entry that is actually carrying retained history (see
    ///   [`CachedSourceEntry::holds_retained_history`]) holds messages the live
    ///   file no longer contains, and the cache is the only copy. A second
    ///   lookup of a drained entry would look like a cold source and re-derive
    ///   history from the live bytes alone, retiring rows that can never come
    ///   back (#994). An entry in the same namespace whose retained set is
    ///   empty is fully reproducible from the live bytes, so it drains like any
    ///   other.
    /// * An entry already marked dirty is written back from memory by
    ///   `save_if_dirty`, so its payload has to still be there.
    pub(crate) fn take(&self, identity: CacheIdentity, path: &Path) -> Option<CachedSourceEntry> {
        let key = CacheKey::new(identity, path);
        let mut state = self.state();
        self.ensure_namespace_loaded(&mut state, identity.namespace);
        let retains_history = retained_history_key_filter(identity.namespace).is_some();
        let dirty = state.dirty_keys.contains(&key);
        let entry = state.entries.get_mut(&key)?;
        if !entry.matches_identity(identity) {
            return None;
        }
        if dirty || (retains_history && entry.holds_retained_history()) {
            return Some(entry.clone());
        }
        Some(entry.take_payload())
    }

    pub(crate) fn remove(&mut self, identity: CacheIdentity, path: &Path) {
        let key = CacheKey::new(identity, path);
        // Load first: an invalidation has to record the fingerprint it is
        // replacing, and an unread namespace holds no entry to record.
        let mut state = self.state();
        self.ensure_namespace_loaded(&mut state, identity.namespace);
        if let Some(entry) = state.entries.remove(&key) {
            state.dirty_keys.remove(&key);
            state
                .deleted_keys
                .insert(key, DeletionReason::Invalidated(entry.fingerprint));
            state.dirty = true;
        }
    }

    #[cfg(test)]
    pub(crate) fn loaded_namespace_count(&self) -> usize {
        self.state().loaded_namespaces.len()
    }

    #[cfg(test)]
    pub(crate) fn namespace_is_loaded(&self, namespace: &str) -> bool {
        self.state().loaded_namespaces.contains(namespace)
    }

    /// Whether the cache is holding a husk: an entry it still knows about
    /// whose messages were handed to their consumer.
    #[cfg(test)]
    pub(crate) fn entry_messages_released(&self, identity: CacheIdentity, path: &Path) -> bool {
        let state = self.state();
        state
            .entries
            .get(&CacheKey::new(identity, path))
            .is_some_and(|entry| entry.messages.is_empty())
    }

    #[cfg(test)]
    pub(crate) fn entry_fingerprint(
        &self,
        identity: CacheIdentity,
        path: &Path,
    ) -> Option<SourceFingerprint> {
        let state = self.state();
        state
            .entries
            .get(&CacheKey::new(identity, path))
            .map(|entry| entry.fingerprint.clone())
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.all_entries().len()
    }

    #[cfg(test)]
    fn is_dirty(&self) -> bool {
        self.state().dirty
    }

    #[cfg(test)]
    fn has_rewrite_shard(&self, shard: &CacheShardKey) -> bool {
        let mut state = self.state();
        self.load_all_namespaces(&mut state);
        state.rewrite_shards.contains(shard)
    }

    /// Every entry this cache currently holds, loading every namespace first.
    #[cfg(test)]
    pub(crate) fn all_entries(&self) -> Vec<CachedSourceEntry> {
        let mut state = self.state();
        self.load_all_namespaces(&mut state);
        state.entries.values().cloned().collect()
    }

    #[cfg(test)]
    pub(crate) fn prune_missing_files(&mut self) {
        let mut state = self.state();
        self.load_all_namespaces(&mut state);
        let removed_keys: Vec<CacheKey> = state
            .entries
            .keys()
            .filter(|key| !key.path.to_path_buf().exists())
            .cloned()
            .collect();

        for key in removed_keys {
            state.entries.remove(&key);
            state.dirty_keys.remove(&key);
            state.deleted_keys.insert(key, DeletionReason::Missing);
            state.dirty = true;
        }
    }

    pub(crate) fn save_if_dirty(&mut self) {
        self.save_if_dirty_with_limit(MAX_CACHE_SHARD_BYTES);
    }

    fn save_if_dirty_with_limit(&mut self, max_shard_bytes: u64) {
        let state = self.state.get_mut().unwrap_or_else(|p| p.into_inner());
        if !state.dirty {
            return;
        }

        let Some(shard_root) = cache_shard_dir() else {
            return;
        };
        if let Err(error) = ensure_cache_dir(&shard_root) {
            warn_cache_failure_once(
                "source message cache directory is unavailable",
                &shard_root,
                &error,
            );
            return;
        }
        let Some(lock_path) = cache_lock_path() else {
            return;
        };
        let lock_file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
        {
            Ok(file) => file,
            Err(error) => {
                warn_cache_failure_once(
                    "source message cache lock is unavailable",
                    &lock_path,
                    &error,
                );
                return;
            }
        };
        if let Err(error) = fs2::FileExt::lock_exclusive(&lock_file) {
            warn_cache_failure_once("source message cache lock failed", &lock_path, &error);
            return;
        }

        // Bucket dirty and deleted keys by shard up front. CacheKey::shard()
        // computes a SHA-256 digest, so grouping once keeps hashing at O(keys).
        // The previous per-shard `.filter(|k| k.shard() == shard_key)` recomputed
        // that digest for every key on every shard — O(shards * keys) — which
        // dominated cold-cache builds (hundreds of shards * tens of thousands of
        // files re-hashed).
        let mut dirty_by_shard: HashMap<CacheShardKey, Vec<CacheKey>> = HashMap::new();
        for key in &state.dirty_keys {
            dirty_by_shard
                .entry(key.shard())
                .or_default()
                .push(key.clone());
        }
        let mut deleted_by_shard: HashMap<CacheShardKey, Vec<(CacheKey, DeletionReason)>> =
            HashMap::new();
        for (key, reason) in &state.deleted_keys {
            deleted_by_shard
                .entry(key.shard())
                .or_default()
                .push((key.clone(), reason.clone()));
        }

        let mut affected_shards = state.rewrite_shards.clone();
        affected_shards.extend(dirty_by_shard.keys().cloned());
        affected_shards.extend(deleted_by_shard.keys().cloned());

        let mut successful_shards = HashSet::new();
        for shard_key in affected_shards {
            let Some(identity) = CacheIdentity::current_for_namespace(&shard_key.namespace) else {
                continue;
            };
            let parser_dir = shard_root.join(identity.namespace);
            if let Err(error) = ensure_cache_dir(&parser_dir) {
                warn_cache_failure_once(
                    "source message cache parser directory is unavailable",
                    &parser_dir,
                    &error,
                );
                continue;
            }
            let final_path = shard_path(&shard_root, &shard_key);

            let mut merged_entries: HashMap<CacheKey, CachedSourceEntry> =
                match read_shard_with_limit(&final_path, identity, max_shard_bytes) {
                    ShardReadStatus::Loaded(entries) | ShardReadStatus::Migrated(entries) => {
                        entries
                            .into_iter()
                            .filter(|entry| entry.identity_is_current())
                            .map(|entry| (CacheKey::from_entry(&entry), entry))
                            .filter(|(key, _)| key.shard() == shard_key)
                            .collect()
                    }
                    ShardReadStatus::Missing | ShardReadStatus::Stale => HashMap::new(),
                    ShardReadStatus::Invalid(error) => {
                        warn_cache_failure_once(
                            "source message cache shard is invalid",
                            &final_path,
                            &error,
                        );
                        HashMap::new()
                    }
                };

            if let Some(deleted) = deleted_by_shard.get(&shard_key) {
                for (key, reason) in deleted {
                    let should_remove = match reason {
                        DeletionReason::Missing => !key.path.to_path_buf().exists(),
                        DeletionReason::Invalidated(expected) => merged_entries
                            .get(key)
                            .is_some_and(|entry| entry.fingerprint == *expected),
                    };
                    if should_remove {
                        merged_entries.remove(key);
                    }
                }
            }
            if let Some(dirty) = dirty_by_shard.get(&shard_key) {
                for key in dirty {
                    if let Some(entry) = state.entries.get(key) {
                        let mut entry = entry.clone();
                        // Another process holding the lock before us may have
                        // stored history for this same path that our in-memory
                        // entry never saw. Union it in rather than replacing
                        // wholesale — see `absorb_retained_history`.
                        if let Some(stored) = merged_entries.remove(key) {
                            entry.absorb_retained_history(&stored);
                        }
                        entry.remove_claude_synthetic_placeholders();
                        merged_entries.insert(key.clone(), entry);
                    }
                }
            }

            let mut entries: Vec<CachedSourceEntry> = merged_entries.into_values().collect();
            entries.sort_by_key(|left| left.path.to_path_buf());
            match write_shard_with_limit(&final_path, identity, &entries, max_shard_bytes) {
                Ok(()) => {
                    successful_shards.insert(shard_key);
                }
                Err(error) => {
                    warn_cache_failure_once(
                        "source message cache shard could not be saved; future scans may remain cold",
                        &final_path,
                        &error,
                    );
                }
            }
        }

        state
            .dirty_keys
            .retain(|key| !successful_shards.contains(&key.shard()));
        state
            .deleted_keys
            .retain(|key, _| !successful_shards.contains(&key.shard()));
        state
            .rewrite_shards
            .retain(|shard| !successful_shards.contains(shard));
        state.dirty = !(state.dirty_keys.is_empty()
            && state.deleted_keys.is_empty()
            && state.rewrite_shards.is_empty());
    }
}

fn shard_filename(index: usize) -> String {
    format!("shard-{index:02x}.bin")
}

fn parse_shard_filename(filename: &std::ffi::OsStr) -> Option<usize> {
    let filename = filename.to_str()?;
    let encoded = filename.strip_prefix("shard-")?.strip_suffix(".bin")?;
    let index = usize::from_str_radix(encoded, 16).ok()?;
    (index < CACHE_SHARD_COUNT).then_some(index)
}

fn shard_path(root: &Path, key: &CacheShardKey) -> PathBuf {
    root.join(&key.namespace).join(shard_filename(key.index))
}

enum ShardReadStatus {
    Missing,
    Stale,
    Invalid(String),
    Loaded(Vec<CachedSourceEntry>),
    Migrated(Vec<CachedSourceEntry>),
}

fn read_shard(path: &Path, identity: CacheIdentity) -> ShardReadStatus {
    read_shard_with_limit(path, identity, MAX_CACHE_SHARD_BYTES)
}

fn read_shard_with_limit(
    path: &Path,
    identity: CacheIdentity,
    max_shard_bytes: u64,
) -> ShardReadStatus {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ShardReadStatus::Missing
        }
        Err(error) => return ShardReadStatus::Invalid(error.to_string()),
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => return ShardReadStatus::Invalid(error.to_string()),
    };
    if metadata.len() > max_shard_bytes {
        return ShardReadStatus::Invalid(format!(
            "{} bytes exceeds the {}-byte shard limit",
            metadata.len(),
            max_shard_bytes
        ));
    }

    let envelope: CachedShardEnvelope = match bincode::options()
        .with_limit(max_shard_bytes)
        .deserialize_from(BufReader::new(file))
    {
        Ok(envelope) => envelope,
        Err(error) => return ShardReadStatus::Invalid(error.to_string()),
    };
    if envelope.parser_namespace != identity.namespace
        || envelope.parser_version != identity.parser_version
    {
        return ShardReadStatus::Stale;
    }

    if envelope.format_version == LEGACY_CACHE_FORMAT_VERSION {
        return match bincode::options()
            .with_limit(max_shard_bytes)
            .deserialize::<Vec<LegacyCachedSourceEntryV4>>(&envelope.payload)
        {
            Ok(entries) => ShardReadStatus::Migrated(
                entries.into_iter().map(CachedSourceEntry::from).collect(),
            ),
            Err(error) => ShardReadStatus::Invalid(error.to_string()),
        };
    }
    if envelope.format_version != CACHE_FORMAT_VERSION {
        return ShardReadStatus::Stale;
    }

    match bincode::options()
        .with_limit(max_shard_bytes)
        .deserialize(&envelope.payload)
    {
        Ok(entries) => ShardReadStatus::Loaded(entries),
        Err(error) => ShardReadStatus::Invalid(error.to_string()),
    }
}

fn write_shard_with_limit(
    final_path: &Path,
    identity: CacheIdentity,
    entries: &[CachedSourceEntry],
    max_shard_bytes: u64,
) -> std::io::Result<()> {
    let payload = bincode::options()
        .with_limit(max_shard_bytes)
        .serialize(entries)
        .map_err(std::io::Error::other)?;
    let envelope = CachedShardEnvelope {
        format_version: CACHE_FORMAT_VERSION,
        parser_namespace: identity.namespace.to_string(),
        parser_version: identity.parser_version,
        payload,
    };
    let parent = final_path
        .parent()
        .ok_or_else(|| std::io::Error::other("cache shard has no parent directory"))?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let tmp_path = parent.join(format!(
        ".{}.{}.{nanos:x}.tmp",
        final_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("source-message-cache"),
        std::process::id(),
    ));

    // INVARIANT: shard writes use atomic temp-file replacement. Never remove
    // the canonical shard before the replacement is completely serialized and
    // fsynced, or one failed large shard write could destroy its last good copy.
    let write_result = (|| -> std::io::Result<()> {
        let file = File::create(&tmp_path)?;
        let mut writer = BufWriter::new(file);
        bincode::options()
            .with_limit(max_shard_bytes)
            .serialize_into(&mut writer, &envelope)
            .map_err(std::io::Error::other)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        crate::fs_atomic::replace_file(&tmp_path, final_path)?;
        let final_file = OpenOptions::new().read(true).write(true).open(final_path)?;
        final_file.sync_all()?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_result
}

fn read_sample_hash(file: &mut File, offset: u64, len: usize) -> Option<FileSampleHash> {
    if len == 0 {
        return Some(FileSampleHash {
            offset,
            len: 0,
            hash: 0,
        });
    }

    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buffer = vec![0_u8; len];
    file.read_exact(&mut buffer).ok()?;

    Some(FileSampleHash {
        offset,
        len: len as u64,
        hash: hash_bytes(&buffer),
    })
}

fn compute_sample_hashes(path: &Path, size: u64) -> Option<Vec<FileSampleHash>> {
    if path.metadata().ok()?.is_dir() {
        return Some(Vec::new());
    }
    if size == 0 {
        return Some(Vec::new());
    }

    let mut file = File::open(path).ok()?;
    let offsets = sample_offsets(size);
    offsets
        .into_iter()
        .map(|(offset, len)| read_sample_hash(&mut file, offset, len))
        .collect()
}

fn sample_offsets(size: u64) -> Vec<(u64, usize)> {
    let sample_len = size.min(FINGERPRINT_SAMPLE_BYTES as u64) as usize;
    if sample_len == 0 {
        return Vec::new();
    }

    let max_offset = size.saturating_sub(sample_len as u64);
    let mut offsets = if max_offset == 0 {
        vec![0]
    } else {
        vec![
            0,
            max_offset / 4,
            max_offset / 2,
            max_offset.saturating_mul(3) / 4,
            max_offset,
        ]
    };
    offsets.sort_unstable();
    offsets.dedup();
    offsets.truncate(FINGERPRINT_SAMPLE_POINTS);
    offsets
        .into_iter()
        .map(|offset| (offset, sample_len))
        .collect()
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Whether a fingerprint carries a whole-file `content_hash`.
///
/// Most warm validation uses size + mtime + samples
/// ([`primary_fingerprint_matches`] and [`related_fingerprint_metadata_matches`]).
/// Codex reads `content_hash` for incremental resume, while Prime hashes the
/// complete transcript on every warm hit because its cached messages and
/// reconciliation accounting must describe one exact byte snapshot. Generic
/// parsers and SQLite sources store a zero sentinel so their changed or cold
/// files do not pay for a whole-file hash that cannot affect parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentHashMode {
    Full,
    SamplesOnly,
}

fn file_fingerprint_parts(
    path: &Path,
    mode: ContentHashMode,
) -> Option<(u64, u64, Vec<FileSampleHash>, [u8; 32])> {
    let metadata = path.metadata().ok()?;
    let size = metadata.len();
    let modified_ns = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    let sample_hashes = compute_sample_hashes(path, size)?;
    let content_hash = if metadata.is_dir() {
        [0_u8; 32]
    } else {
        match mode {
            ContentHashMode::Full => hash_prefix(path, size)?,
            ContentHashMode::SamplesOnly => [0_u8; 32],
        }
    };
    Some((size, modified_ns, sample_hashes, content_hash))
}

fn append_path_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = OsString::from(path.as_os_str());
    os.push(suffix);
    PathBuf::from(os)
}

fn hash_prefix(path: &Path, len: u64) -> Option<[u8; 32]> {
    #[cfg(test)]
    FULL_HASH_CALLS.with(|calls| calls.set(calls.get() + 1));

    let mut file = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut remaining = len;
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];

    while remaining > 0 {
        let bytes_to_read = remaining.min(HASH_BUFFER_BYTES as u64) as usize;
        let read = file.read(&mut buffer[..bytes_to_read]).ok()?;
        if read == 0 {
            return None;
        }
        hasher.update(&buffer[..read]);
        remaining -= read as u64;
    }

    Some(hasher.finalize().into())
}

#[cfg(test)]
fn full_hash_call_count() -> usize {
    FULL_HASH_CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn build_codex_incremental_cache(
    path: &Path,
    consumed_offset: u64,
    state: CodexParseState,
) -> Option<CodexIncrementalCache> {
    let ends_with_newline = consumed_offset == 0 || file_ends_with_newline(path, consumed_offset);
    if !ends_with_newline {
        return None;
    }

    Some(CodexIncrementalCache {
        state,
        consumed_offset,
        ends_with_newline,
        prefix_hash: hash_prefix(path, consumed_offset)?,
    })
}

/// Build Codex incremental state when the caller already hashed the complete
/// consumed prefix. Full-file Codex fingerprints are also the prefix hash when
/// `consumed_offset` equals the current file size, so accepting that digest
/// avoids a second read of the transcript.
pub(crate) fn build_codex_incremental_cache_with_prefix_hash(
    path: &Path,
    consumed_offset: u64,
    state: CodexParseState,
    prefix_hash: [u8; 32],
) -> Option<CodexIncrementalCache> {
    let ends_with_newline = consumed_offset == 0 || file_ends_with_newline(path, consumed_offset);
    if !ends_with_newline {
        return None;
    }

    Some(CodexIncrementalCache {
        state,
        consumed_offset,
        ends_with_newline,
        prefix_hash,
    })
}

fn file_ends_with_newline(path: &Path, size: u64) -> bool {
    if size == 0 {
        return true;
    }

    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    if file.seek(SeekFrom::Start(size.saturating_sub(1))).is_err() {
        return false;
    }

    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).is_ok() && byte[0] == b'\n'
}

pub(crate) fn codex_prefix_matches(path: &Path, cached: &CodexIncrementalCache) -> bool {
    if cached.consumed_offset > 0 && !cached.ends_with_newline {
        return false;
    }

    match hash_prefix(path, cached.consumed_offset) {
        Some(prefix_hash) => prefix_hash == cached.prefix_hash,
        None => false,
    }
}

pub(crate) fn codex_cache_entry_matches_fingerprint(
    cached: &CachedSourceEntry,
    fingerprint: &SourceFingerprint,
) -> bool {
    let Some(codex_incremental) = cached.codex_incremental.as_ref() else {
        return false;
    };

    codex_incremental.consumed_offset == fingerprint.size
        && codex_incremental.ends_with_newline
        && codex_incremental.prefix_hash == fingerprint.content_hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::json_path_literal;
    use crate::TokenBreakdown;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    #[serial_test::serial]
    fn cache_warning_is_deferred_once_while_the_tui_is_active() {
        const CONTEXT: &str = "test source cache warning deferral";
        let mut tui = crate::tui_signal::TuiActiveGuard::capture();
        assert!(
            crate::tui_signal::take_deferred_stderr_for_test().is_empty(),
            "the test must not inherit deferred diagnostics"
        );

        // Deliberately the real process-global set, so the production entry
        // point and its once-per-context bookkeeping stay covered. The
        // poisoning test below is the one that needs an isolated set.
        tui.set(true);
        let path = Path::new("cache-warning-test");
        let error = std::io::Error::other("simulated cache failure");
        warn_cache_failure_once(CONTEXT, path, &error);
        warn_cache_failure_once(CONTEXT, path, &error);

        assert_eq!(
            crate::tui_signal::take_deferred_stderr_for_test(),
            vec![format!(
                "tokscale: warning: {CONTEXT} ({}): {error}",
                path.display()
            )],
            "a repeated failure should leave one complete warning for terminal restore"
        );
    }

    #[test]
    #[serial_test::serial]
    fn cache_warning_survives_a_poisoned_once_only_set() {
        const CONTEXT: &str = "test source cache warning after poisoning";
        let mut tui = crate::tui_signal::TuiActiveGuard::capture();
        assert!(
            crate::tui_signal::take_deferred_stderr_for_test().is_empty(),
            "the test must not inherit deferred diagnostics"
        );

        // Poison a set scoped to this test rather than the process-global one:
        // poisoning cannot be undone, so poisoning the real set would make
        // every later test in this binary depend on the recovery under test.
        let warned: Mutex<HashSet<&'static str>> = Mutex::new(HashSet::new());

        // An unrelated panic while the once-only set is locked poisons the
        // mutex. The warning must still reach the user instead of being
        // silently swallowed by the poison.
        //
        // No panic hook is installed here. The hook is process-global, so
        // swapping it would suppress the diagnostics of whatever else runs in
        // parallel; this unwind happens on the test's own thread, which
        // libtest already captures, so the expected panic message is only
        // printed if this test fails.
        let poisoned = std::panic::catch_unwind(|| {
            let _guard = warned.lock().expect("set is not yet poisoned");
            panic!("unrelated panic while holding the once-only set");
        });
        assert!(poisoned.is_err(), "the helper panic must have unwound");
        assert!(
            warned.is_poisoned(),
            "the once-only set must be poisoned for this test to mean anything"
        );

        tui.set(true);
        let path = Path::new("cache-warning-poison-test");
        let error = std::io::Error::other("simulated cache failure");
        warn_cache_failure_once_in(&warned, CONTEXT, path, &error);
        warn_cache_failure_once_in(&warned, CONTEXT, path, &error);

        assert_eq!(
            crate::tui_signal::take_deferred_stderr_for_test(),
            vec![format!(
                "tokscale: warning: {CONTEXT} ({}): {error}",
                path.display()
            )],
            "a poisoned once-only set must still defer exactly one warning"
        );
    }

    #[test]
    fn from_roo_path_invalidates_on_history_only_change() {
        // parse_roo_kilo_file reads model/agent from the sibling
        // api_conversation_history.json, so a history-only rewrite (ui_messages
        // byte-identical) must change the fingerprint or the cache serves stale
        // model/agent/pricing.
        let dir = TempDir::new().unwrap();
        let ui = dir.path().join("ui_messages.json");
        std::fs::write(&ui, b"[]").unwrap();
        let history = dir.path().join("api_conversation_history.json");
        std::fs::write(&history, b"<model>claude-sonnet-4</model>").unwrap();

        let roo_before = SourceFingerprint::from_roo_path(&ui).unwrap();
        let plain_before = SourceFingerprint::from_path(&ui).unwrap();

        // Rewrite the history only; leave ui_messages.json byte-identical.
        std::fs::write(&history, b"<model>claude-opus-4</model>").unwrap();

        let roo_after = SourceFingerprint::from_roo_path(&ui).unwrap();
        let plain_after = SourceFingerprint::from_path(&ui).unwrap();

        assert_ne!(
            roo_before, roo_after,
            "a history-only change must alter the roo fingerprint"
        );
        assert_eq!(
            plain_before, plain_after,
            "from_path ignores the history sibling (control)"
        );
    }

    #[test]
    fn cline_cli_fingerprint_tracks_manifest_changes() {
        let dir = TempDir::new().unwrap();
        let messages = dir.path().join("session.messages.json");
        let manifest = dir.path().join("session.json");
        std::fs::write(&messages, br#"{"messages":[]}"#).unwrap();

        let initial = match SourceFingerprint::check_cline_path_samples_only(&messages, None) {
            Some(FingerprintStatus::Changed(fingerprint)) => fingerprint,
            other => panic!("expected an initial fingerprint, got {other:?}"),
        };
        assert!(initial.related_files.iter().any(|related| {
            related.suffix == "manifest.json"
                && related.path.to_path_buf() == manifest
                && !related.exists
        }));
        assert!(matches!(
            SourceFingerprint::check_cline_path_samples_only(&messages, Some(&initial)),
            Some(FingerprintStatus::Unchanged)
        ));

        std::fs::write(&manifest, br#"{"title":"first"}"#).unwrap();
        assert!(matches!(
            SourceFingerprint::check_cline_path_samples_only(&messages, Some(&initial)),
            Some(FingerprintStatus::Changed(_))
        ));

        let with_manifest =
            match SourceFingerprint::check_cline_path_samples_only(&messages, Some(&initial)) {
                Some(FingerprintStatus::Changed(fingerprint)) => fingerprint,
                other => panic!("expected a refreshed fingerprint, got {other:?}"),
            };
        std::fs::write(&manifest, br#"{"title":"second"}"#).unwrap();
        assert!(matches!(
            SourceFingerprint::check_cline_path_samples_only(&messages, Some(&with_manifest)),
            Some(FingerprintStatus::Changed(_))
        ));
    }

    fn restore_env_var(key: &str, value: Option<impl AsRef<std::ffi::OsStr>>) {
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    /// Pin every env var the cache resolvers consult so the test stays
    /// inside `temp_home`, until the returned guard drops. CI runners can leak
    /// `XDG_CONFIG_HOME` / `XDG_CACHE_HOME` from the host, which would resolve
    /// cache shards outside the sandbox.
    ///
    /// The restore has to be a `Drop` guard rather than a trailing call. A
    /// failing assertion panics before any trailing restore runs, and of the
    /// four keys here `TOKSCALE_CONFIG_DIR` is consulted first on every
    /// platform — so a leaked one aims every later test in this binary at a
    /// `TempDir` that has already been dropped, which is the contamination this
    /// sandbox exists to prevent. `serial_test` prevents overlap, not
    /// inheritance.
    #[must_use = "the sandbox is torn down as soon as the guard drops; bind it to a \
                  named variable that outlives the test body"]
    fn sandbox_cache_env(temp_home: &std::path::Path) -> crate::paths::test_env::EnvGuard {
        let mut env = crate::paths::test_env::EnvGuard::capture(&[
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "TOKSCALE_CONFIG_DIR",
        ]);
        env.set("HOME", temp_home);
        env.set("XDG_CONFIG_HOME", temp_home.join(".config"));
        env.set("XDG_CACHE_HOME", temp_home.join(".cache"));
        // The three above isolate the cache on Unix and none of them reach
        // it on Windows: `paths::get_config_dir` resolves the Windows root
        // with `dirs::config_dir()`, a known-folder lookup that reads no
        // environment variable. Without this line every test here shared
        // one real `%APPDATA%\tokscale\cache`, so `SourceMessageCache::load`
        // returned its neighbours' shards along with its own and the entry
        // counts came out too high. `TOKSCALE_CONFIG_DIR` is the override
        // paths.rs documents for this case and is consulted first
        // everywhere; on Unix it names the directory the redirects above
        // already produced.
        env.set(
            "TOKSCALE_CONFIG_DIR",
            temp_home.join(".config").join("tokscale"),
        );
        env
    }

    fn write_temp_file(content: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content).unwrap();
        file.flush().unwrap();
        file
    }

    fn test_entry(identity: CacheIdentity, path: &Path, session_id: &str) -> CachedSourceEntry {
        CachedSourceEntry::new(
            identity,
            path,
            SourceFingerprint::from_path(path).unwrap(),
            vec![UnifiedMessage::new(
                identity.namespace,
                "gpt-5",
                "provider",
                session_id,
                1,
                TokenBreakdown {
                    input: 1,
                    output: 2,
                    cache_read: 3,
                    cache_write: 0,
                    reasoning: 0,
                },
                0.0,
            )],
            Vec::new(),
            None,
        )
    }

    fn write_sources_in_distinct_shards(
        dir: &TempDir,
        identity: CacheIdentity,
    ) -> (PathBuf, PathBuf) {
        let first = dir.path().join("source-0.jsonl");
        std::fs::write(&first, b"source-0\n").unwrap();
        let first_shard = CacheKey::new(identity, &first).shard();

        for index in 1..=CACHE_SHARD_COUNT * 2 {
            let candidate = dir.path().join(format!("source-{index}.jsonl"));
            std::fs::write(&candidate, format!("source-{index}\n")).unwrap();
            if CacheKey::new(identity, &candidate).shard() != first_shard {
                return (first, candidate);
            }
        }

        panic!("failed to find paths in distinct cache shards");
    }

    fn write_sources_in_same_shard(dir: &TempDir, identity: CacheIdentity) -> (PathBuf, PathBuf) {
        let mut paths_by_shard = HashMap::new();
        for index in 0..=CACHE_SHARD_COUNT * 4 {
            let candidate = dir.path().join(format!("source-{index}.jsonl"));
            std::fs::write(&candidate, format!("source-{index}\n")).unwrap();
            let shard = CacheKey::new(identity, &candidate).shard();
            if let Some(first) = paths_by_shard.insert(shard, candidate.clone()) {
                return (first, candidate);
            }
        }

        panic!("failed to find paths in the same cache shard");
    }

    fn cache_shard_path(identity: CacheIdentity, path: &Path) -> PathBuf {
        let root = cache_shard_dir().unwrap();
        shard_path(&root, &CacheKey::new(identity, path).shard())
    }

    #[test]
    fn test_codex_prefix_matches_appended_file() {
        let file = write_temp_file(b"line-1\nline-2\n");
        let fingerprint = SourceFingerprint::from_path(file.path()).unwrap();
        let incremental_cache = build_codex_incremental_cache(
            file.path(),
            fingerprint.size,
            CodexParseState::default(),
        )
        .unwrap();

        let mut reopened = file.reopen().unwrap();
        reopened.seek(SeekFrom::End(0)).unwrap();
        reopened.write_all(b"line-3\n").unwrap();
        reopened.flush().unwrap();

        assert!(codex_prefix_matches(file.path(), &incremental_cache,));
    }

    #[test]
    fn test_codex_incremental_cache_reuses_full_hash() {
        let file = write_temp_file(b"line-1\nline-2\n");
        let fingerprint = SourceFingerprint::from_path(file.path()).unwrap();
        let full_hashes_before = full_hash_call_count();

        let incremental_cache = build_codex_incremental_cache_with_prefix_hash(
            file.path(),
            fingerprint.size,
            CodexParseState::default(),
            fingerprint.content_hash,
        )
        .unwrap();

        assert_eq!(
            full_hash_call_count(),
            full_hashes_before,
            "a supplied Codex fingerprint must avoid a second whole-file SHA-256"
        );
        assert_eq!(incremental_cache.prefix_hash, fingerprint.content_hash);
        assert!(incremental_cache.ends_with_newline);
    }

    #[test]
    fn test_check_path_returns_unchanged_for_matching_metadata_and_samples() {
        let file = write_temp_file(&vec![b'a'; 32 * 1024]);
        let fingerprint = SourceFingerprint::from_path(file.path()).unwrap();
        let full_hashes_before = full_hash_call_count();

        let status = SourceFingerprint::check_path(file.path(), Some(&fingerprint)).unwrap();

        assert!(matches!(status, FingerprintStatus::Unchanged));
        assert_eq!(
            full_hash_call_count(),
            full_hashes_before,
            "an unchanged fingerprint must not compute a full SHA-256"
        );
    }

    #[test]
    fn test_check_path_returns_changed_when_sample_changes_with_same_metadata() {
        let original = vec![b'a'; 32 * 1024];
        let file = write_temp_file(&original);
        let fingerprint = SourceFingerprint::from_path(file.path()).unwrap();
        let original_signature = metadata_signature(file.path()).unwrap();
        let original_modified = std::fs::metadata(file.path()).unwrap().modified().unwrap();

        let mut rewritten = original;
        rewritten[0] = b'z';
        std::fs::write(file.path(), rewritten).unwrap();
        File::options()
            .write(true)
            .open(file.path())
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(original_modified))
            .unwrap();
        assert_eq!(metadata_signature(file.path()).unwrap(), original_signature);
        let full_hashes_before = full_hash_call_count();

        let status = SourceFingerprint::check_path(file.path(), Some(&fingerprint)).unwrap();

        let FingerprintStatus::Changed(changed) = status else {
            panic!("changed sample must rebuild the full fingerprint");
        };
        assert_ne!(changed, fingerprint);
        assert_eq!(
            full_hash_call_count(),
            full_hashes_before + 1,
            "a changed sample must rebuild the full fingerprint"
        );
    }

    #[test]
    fn test_generic_sources_skip_full_hash() {
        let original = vec![b'a'; 64 * 1024];
        let file = write_temp_file(&original);
        let fingerprint = SourceFingerprint::from_path(file.path()).unwrap();
        let original_signature = metadata_signature(file.path()).unwrap();
        let original_modified = std::fs::metadata(file.path()).unwrap().modified().unwrap();

        let mut rewritten = original;
        rewritten[0] = b'z';
        std::fs::write(file.path(), rewritten).unwrap();
        File::options()
            .write(true)
            .open(file.path())
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(original_modified))
            .unwrap();
        assert_eq!(metadata_signature(file.path()).unwrap(), original_signature);

        let full_hashes_before = full_hash_call_count();
        let status =
            SourceFingerprint::check_path_samples_only(file.path(), Some(&fingerprint)).unwrap();
        let FingerprintStatus::Changed(changed) = status else {
            panic!("changed sample must invalidate a generic source");
        };
        assert_eq!(
            full_hash_call_count(),
            full_hashes_before,
            "generic source fingerprints must not compute a whole-file SHA-256"
        );
        assert_eq!(changed.content_hash, [0_u8; 32]);

        let full_hashes_before = full_hash_call_count();
        let cold = SourceFingerprint::check_path_samples_only(file.path(), None).unwrap();
        let FingerprintStatus::Changed(cold) = cold else {
            panic!("an uncached generic source must build a fingerprint");
        };
        assert_eq!(full_hash_call_count(), full_hashes_before);
        assert_eq!(cold.content_hash, [0_u8; 32]);
    }

    #[test]
    fn test_sqlite_fingerprint_skips_full_hash() {
        let file = write_temp_file(&vec![b'a'; 64 * 1024]);
        let full_hashes_before = full_hash_call_count();

        let fingerprint = SourceFingerprint::from_sqlite_path(file.path()).unwrap();

        assert_eq!(
            full_hash_call_count(),
            full_hashes_before,
            "a SQLite fingerprint must not compute a whole-file SHA-256"
        );
        assert_eq!(
            fingerprint.content_hash, [0_u8; 32],
            "a SQLite fingerprint stores a zero content_hash sentinel"
        );
        assert!(
            !fingerprint.sample_hashes.is_empty(),
            "samples still guard SQLite change detection"
        );
    }

    #[test]
    fn test_sqlite_check_detects_change_without_full_hash() {
        let original = vec![b'a'; 64 * 1024];
        let file = write_temp_file(&original);
        let fingerprint = SourceFingerprint::from_sqlite_path(file.path()).unwrap();

        // Unchanged: metadata + samples match, no full hash.
        let full_hashes_before = full_hash_call_count();
        let status = SourceFingerprint::check_sqlite_path(file.path(), Some(&fingerprint)).unwrap();
        assert!(matches!(status, FingerprintStatus::Unchanged));

        // Changed: a same-size rewrite with a rolled-back mtime is still caught
        // by the samples, and still without a whole-file hash.
        let original_modified = std::fs::metadata(file.path()).unwrap().modified().unwrap();
        let mut rewritten = original;
        rewritten[0] = b'z';
        std::fs::write(file.path(), rewritten).unwrap();
        File::options()
            .write(true)
            .open(file.path())
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(original_modified))
            .unwrap();

        let status = SourceFingerprint::check_sqlite_path(file.path(), Some(&fingerprint)).unwrap();
        assert!(matches!(status, FingerprintStatus::Changed(_)));
        assert_eq!(
            full_hash_call_count(),
            full_hashes_before,
            "SQLite change detection must never compute a whole-file SHA-256"
        );
    }

    #[test]
    fn test_source_fingerprint_changes_for_same_size_rewrite() {
        let file = write_temp_file(b"aaaa\nbbbb\ncccc\n");
        let before = SourceFingerprint::from_path(file.path()).unwrap();

        std::fs::write(file.path(), b"aaaa\nzzzz\ncccc\n").unwrap();

        let after = SourceFingerprint::from_path(file.path()).unwrap();
        assert_ne!(before, after);
    }

    #[test]
    fn test_source_fingerprint_changes_for_large_same_size_unsampled_rewrite() {
        let mut original = vec![b'a'; 128 * 1024];
        original.extend_from_slice(b"\n");
        let file = write_temp_file(&original);
        let before = SourceFingerprint::from_path(file.path()).unwrap();

        let mut rewritten = original.clone();
        rewritten[73 * 1024] = b'z';
        std::fs::write(file.path(), &rewritten).unwrap();

        let after = SourceFingerprint::from_path(file.path()).unwrap();
        assert_ne!(before, after);
    }

    #[test]
    fn test_sqlite_source_fingerprint_tracks_sidecar_changes() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("history.db");
        std::fs::write(&db_path, b"main-db").unwrap();

        let base = SourceFingerprint::from_sqlite_path(&db_path).unwrap();

        let wal_path = append_path_suffix(&db_path, "-wal");
        std::fs::write(&wal_path, b"wal-1").unwrap();
        let with_wal = SourceFingerprint::from_sqlite_path(&db_path).unwrap();
        assert_ne!(base, with_wal);

        std::fs::write(&wal_path, b"wal-2").unwrap();
        let updated_wal = SourceFingerprint::from_sqlite_path(&db_path).unwrap();
        assert_ne!(with_wal, updated_wal);

        let before_shm = SourceFingerprint::from_sqlite_path(&db_path).unwrap();
        let shm_path = append_path_suffix(&db_path, "-shm");
        std::fs::write(&shm_path, b"shm-1").unwrap();
        let with_shm = SourceFingerprint::from_sqlite_path(&db_path).unwrap();
        assert_eq!(before_shm, with_shm);
    }

    #[test]
    fn test_devin_desktop_fingerprint_tracks_cli_lookup_database_and_wal() {
        let dir = TempDir::new().unwrap();
        let desktop_path = dir.path().join("desktop.ndjson");
        let db_path = dir.path().join("sessions.db");
        std::fs::write(&desktop_path, b"desktop usage\n").unwrap();
        std::fs::write(&db_path, b"lookup-one").unwrap();

        let fingerprint = match SourceFingerprint::check_devin_desktop_path_samples_only(
            &desktop_path,
            std::slice::from_ref(&db_path),
            None,
        )
        .unwrap()
        {
            FingerprintStatus::Changed(fingerprint) => fingerprint,
            FingerprintStatus::Unchanged => panic!("an uncached source must build a fingerprint"),
        };
        assert!(matches!(
            SourceFingerprint::check_devin_desktop_path_samples_only(
                &desktop_path,
                std::slice::from_ref(&db_path),
                Some(&fingerprint),
            ),
            Some(FingerprintStatus::Unchanged)
        ));

        std::fs::write(&db_path, b"lookup-two").unwrap();
        let changed = SourceFingerprint::check_devin_desktop_path_samples_only(
            &desktop_path,
            std::slice::from_ref(&db_path),
            Some(&fingerprint),
        )
        .unwrap();
        let fingerprint = match changed {
            FingerprintStatus::Changed(fingerprint) => fingerprint,
            FingerprintStatus::Unchanged => panic!("a lookup database rewrite must invalidate"),
        };

        std::fs::write(append_path_suffix(&db_path, "-wal"), b"wal").unwrap();
        assert!(matches!(
            SourceFingerprint::check_devin_desktop_path_samples_only(
                &desktop_path,
                std::slice::from_ref(&db_path),
                Some(&fingerprint),
            ),
            Some(FingerprintStatus::Changed(_))
        ));
    }

    #[test]
    fn test_devin_parser_versions_invalidate_v1_entries() {
        assert_eq!(parser_version(ClientId::DevinCli), 3);
        assert_eq!(parser_version(ClientId::DevinDesktop), 2);
    }

    #[test]
    fn test_codex_duration_parser_version_invalidates_v4_entries() {
        // v6->v7 splits `reasoning_output_tokens` out of the Codex output
        // bucket. The bump is what stops an existing cache from replaying
        // pre-split rows, so it has to be asserted rather than assumed.
        assert_eq!(parser_version(ClientId::Codex), 7);
        assert_eq!(parser_version(ClientId::Claude), 2);
    }

    #[test]
    fn test_copilot_duplicate_metadata_parser_version_invalidates_v7_entries() {
        assert_eq!(parser_version(ClientId::Copilot), 8);
    }

    #[test]
    fn test_duration_anchor_audit_remaining_parsers_bumps_versions() {
        // Follow-up to #890: junie, jcode, devin-cli, zcode, and
        // opencodereview were re-anchored to start-anchored duration
        // timestamps; their cache-invalidating parser versions must bump so
        // stale end-anchored-timestamp cache entries are not reused.
        //
        // Second-round review found gaps in that first pass: zcode's
        // NULL-`started_at` fallback stayed end-anchored and its
        // `is_turn_start` marking didn't follow the new start-anchored
        // timestamps, and kiro's structured messages.jsonl turns stayed
        // end-anchored when the prompt timestamp was missing. Both bump
        // again here so those stale (start-anchored-but-still-wrong) v2/v1
        // cache entries are also invalidated.
        assert_eq!(parser_version(ClientId::Junie), 3);
        assert_eq!(parser_version(ClientId::Jcode), 7);
        assert_eq!(parser_version(ClientId::DevinCli), 3);
        assert_eq!(parser_version(ClientId::Zcode), 3);
        assert_eq!(parser_version(ClientId::OpenCodeReview), 3);
        assert_eq!(parser_version(ClientId::Kiro), 2);
    }

    #[test]
    fn test_kimi_parser_version_invalidates_v3_entries() {
        assert_eq!(parser_version(ClientId::Kimi), 4);
    }

    #[test]
    fn test_lossy_jsonl_parser_versions_invalidate_v3_entries() {
        assert_eq!(parser_version(ClientId::PrimeAgent), 4);
        assert_eq!(parser_version(ClientId::Reasonix), 4);
    }

    #[test]
    #[serial_test::serial]
    fn prime_and_reasonix_v3_shards_are_rejected_before_changed_bytes_are_parsed() {
        for (client, source_bytes, stale_provider, expected_provider) in [
            (
                ClientId::PrimeAgent,
                b"{\"type\":\"session\",\"version\":3,\"id\":\"root\",\"cwd\":\"/tmp/project\"}\n{\"type\":\"message\",\"id\":\"valid\",\"message\":{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-opus-5\",\"usage\":{\"input\":20,\"output\":8}}}\n".as_slice(),
                "stale-prime-v3",
                "anthropic",
            ),
            (
                ClientId::Reasonix,
                b"{\"ts\":\"2026-08-04T09:10:11Z\",\"model\":\"deepseek/chat\",\"prompt\":100,\"completion\":20,\"total\":120}\n".as_slice(),
                "stale-reasonix-v3",
                "deepseek",
            ),
        ] {
            let temp_home = TempDir::new().unwrap();
            let _cache_env = sandbox_cache_env(temp_home.path());
            let source = write_temp_file(source_bytes);
            let current_identity = CacheIdentity::for_client(client);
            assert_eq!(current_identity.parser_version, 4);
            let stale_identity = CacheIdentity {
                namespace: current_identity.namespace,
                parser_version: 3,
            };
            let fingerprint = SourceFingerprint::from_path(source.path()).unwrap();
            let stale_entry = CachedSourceEntry::new(
                stale_identity,
                source.path(),
                fingerprint.clone(),
                vec![UnifiedMessage::new(
                    current_identity.namespace,
                    "stale-model",
                    stale_provider,
                    "stale-session",
                    1,
                    TokenBreakdown {
                        input: 999,
                        output: 0,
                        cache_read: 0,
                        cache_write: 0,
                        reasoning: 0,
                    },
                    0.0,
                )],
                Vec::new(),
                None,
            );
            let stale_path = cache_shard_path(current_identity, source.path());
            ensure_cache_dir(stale_path.parent().unwrap()).unwrap();
            write_shard_with_limit(
                &stale_path,
                stale_identity,
                &[stale_entry],
                MAX_CACHE_SHARD_BYTES,
            )
            .unwrap();

            let mut cache = SourceMessageCache::load();
            assert!(cache.get(current_identity, source.path()).is_none());
            assert_eq!(SourceFingerprint::from_path(source.path()).unwrap(), fingerprint);

            let rebuilt = match client {
                ClientId::PrimeAgent => crate::sessions::prime_agent::parse_prime_agent_file(source.path()),
                ClientId::Reasonix => crate::sessions::reasonix::parse_reasonix_file(source.path()),
                _ => unreachable!(),
            };
            assert_eq!(rebuilt.len(), 1);
            assert_eq!(rebuilt[0].provider_id, expected_provider);
            assert_ne!(rebuilt[0].tokens.input, 999);
            cache.insert(CachedSourceEntry::new(
                current_identity,
                source.path(),
                fingerprint,
                rebuilt.clone(),
                Vec::new(),
                None,
            ));
            cache.save_if_dirty();

            let warm = SourceMessageCache::load();
            let cached = warm.get(current_identity, source.path()).unwrap();
            assert_eq!(cached.parser_version, 4);
            assert_eq!(cached.messages, rebuilt);
        }
    }

    #[test]
    fn test_hermes_parser_version_invalidates_v1_entries() {
        assert_eq!(parser_version(ClientId::Hermes), 2);
    }

    #[test]
    fn test_grok_resilient_line_reader_parser_version_invalidates_v6_entries() {
        // A Grok session file that is never appended to again keeps its
        // fingerprint forever, so only the version bump discards the truncated
        // v6 parse and forces a cold reparse.
        assert_eq!(parser_version(ClientId::Grok), 7);
    }

    #[test]
    fn test_droid_usage_anchor_parser_version_invalidates_v1_entries() {
        // A finished Droid session's settings.json is never rewritten again, so
        // its fingerprint keeps matching and only the version bump discards the
        // v1 lock-timestamp anchor.
        assert_eq!(parser_version(ClientId::Droid), 7);
    }

    #[test]
    fn test_micode_parser_version_invalidates_rows_without_cost_provenance() {
        assert_eq!(parser_version(ClientId::MiMoCode), 3);
    }

    #[test]
    fn test_cursor_parser_version_invalidates_incorrect_token_and_cost_rows() {
        assert_eq!(parser_version(ClientId::Cursor), 2);
    }

    #[test]
    fn test_junie_parser_version_invalidates_rows_without_cost_provenance() {
        assert_eq!(parser_version(ClientId::Junie), 3);
    }

    #[test]
    fn test_jcode_fingerprint_tracks_journal_sidecar_changes() {
        let dir = TempDir::new().unwrap();
        let session_path = dir.path().join("session_fixture.json");
        std::fs::write(&session_path, br#"{"messages":[]}"#).unwrap();

        let base = SourceFingerprint::from_jcode_path(&session_path).unwrap();

        let journal_path = dir.path().join("session_fixture.journal.jsonl");
        std::fs::write(
            &journal_path,
            br#"{"append_messages":[]}
"#,
        )
        .unwrap();
        let with_journal = SourceFingerprint::from_jcode_path(&session_path).unwrap();
        assert_ne!(base, with_journal);

        std::fs::write(
            &journal_path,
            br#"{"append_messages":[{"id":"assistant_1"}]}
"#,
        )
        .unwrap();
        let updated_journal = SourceFingerprint::from_jcode_path(&session_path).unwrap();
        assert_ne!(with_journal, updated_journal);
    }

    #[test]
    fn test_grok_fingerprint_tracks_signals_sidecar_changes() {
        let dir = TempDir::new().unwrap();
        let updates_path = dir.path().join("updates.jsonl");
        std::fs::write(&updates_path, b"update\n").unwrap();

        let base = SourceFingerprint::from_grok_path(&updates_path).unwrap();

        let signals_path = dir.path().join("signals.json");
        std::fs::write(&signals_path, br#"{"input":1}"#).unwrap();
        let with_signals = SourceFingerprint::from_grok_path(&updates_path).unwrap();
        assert_ne!(base, with_signals);

        std::fs::write(&signals_path, br#"{"input":2}"#).unwrap();
        let updated_signals = SourceFingerprint::from_grok_path(&updates_path).unwrap();
        assert_ne!(with_signals, updated_signals);
    }

    #[test]
    fn test_grok_fingerprint_tracks_summary_and_events_sidecar_changes() {
        let dir = TempDir::new().unwrap();
        let updates_path = dir.path().join("updates.jsonl");
        std::fs::write(&updates_path, b"update\n").unwrap();

        let base = SourceFingerprint::from_grok_path(&updates_path).unwrap();

        let summary_path = dir.path().join("summary.json");
        std::fs::write(&summary_path, br#"{"model":"grok-3"}"#).unwrap();
        let with_summary = SourceFingerprint::from_grok_path(&updates_path).unwrap();
        assert_ne!(base, with_summary);

        std::fs::write(&summary_path, br#"{"model":"grok-4"}"#).unwrap();
        let updated_summary = SourceFingerprint::from_grok_path(&updates_path).unwrap();
        assert_ne!(with_summary, updated_summary);

        let events_path = dir.path().join("events.jsonl");
        std::fs::write(&events_path, b"event-1\n").unwrap();
        let with_events = SourceFingerprint::from_grok_path(&updates_path).unwrap();
        assert_ne!(updated_summary, with_events);

        std::fs::write(&events_path, b"event-2\n").unwrap();
        let updated_events = SourceFingerprint::from_grok_path(&updates_path).unwrap();
        assert_ne!(with_events, updated_events);
    }

    #[test]
    fn test_reasonix_stats_fingerprint_tracks_appends() {
        let dir = TempDir::new().unwrap();
        let session_path = dir.path().join("2026-08-04.jsonl");
        std::fs::write(&session_path, b"{\"total\":1}\n").unwrap();

        let initial = match SourceFingerprint::check_reasonix_path_samples_only(&session_path, None)
        {
            Some(FingerprintStatus::Changed(fingerprint)) => fingerprint,
            _ => panic!("uncached Reasonix session must produce a fingerprint"),
        };
        assert!(matches!(
            SourceFingerprint::check_reasonix_path_samples_only(&session_path, Some(&initial)),
            Some(FingerprintStatus::Unchanged)
        ));

        std::fs::write(&session_path, b"{\"total\":1}\n{\"total\":2}\n").unwrap();
        match SourceFingerprint::check_reasonix_path_samples_only(&session_path, Some(&initial)) {
            Some(FingerprintStatus::Changed(fingerprint)) => fingerprint,
            _ => panic!("Reasonix stats append must invalidate"),
        };
    }

    #[test]
    fn test_grok_unified_fingerprint_tracks_session_metadata_tree_changes() {
        let dir = TempDir::new().unwrap();
        let logs_dir = dir.path().join(".grok/logs");
        let session_dir = dir.path().join(".grok/sessions/%2Ftmp%2Fproject/session-1");
        std::fs::create_dir_all(&logs_dir).unwrap();
        std::fs::create_dir_all(&session_dir).unwrap();
        let unified_path = logs_dir.join("unified.jsonl");
        std::fs::write(
            &unified_path,
            br#"{"sid":"session-1","msg":"shell.turn.inference_done","ctx":{"prompt_tokens":1,"completion_tokens":1}}"#,
        )
        .unwrap();
        let summary_path = session_dir.join("summary.json");
        std::fs::write(&summary_path, br#"{"current_model_id":"grok-4.5"}"#).unwrap();

        let base = SourceFingerprint::from_grok_path(&unified_path).unwrap();

        std::fs::write(&summary_path, br#"{"current_model_id":"grok-4.6"}"#).unwrap();
        let changed_summary = SourceFingerprint::from_grok_path(&unified_path).unwrap();
        assert_ne!(base, changed_summary);
        assert!(matches!(
            SourceFingerprint::check_grok_path_samples_only(&unified_path, Some(&base)),
            Some(FingerprintStatus::Changed(_))
        ));

        let second_session_dir = dir.path().join(".grok/sessions/%2Ftmp%2Fproject/session-2");
        std::fs::create_dir_all(&second_session_dir).unwrap();
        std::fs::write(
            second_session_dir.join("summary.json"),
            br#"{"current_model_id":"grok-4.7"}"#,
        )
        .unwrap();
        let changed_tree = SourceFingerprint::from_grok_path(&unified_path).unwrap();
        assert_ne!(changed_summary, changed_tree);
    }

    #[test]
    fn test_kiro_ide_fingerprint_tracks_messages_sidecar_changes() {
        let dir = TempDir::new().unwrap();
        let sess_dir = dir.path().join("workspace-a/sess_02f1c107");
        std::fs::create_dir_all(&sess_dir).unwrap();
        let session_path = sess_dir.join("session.json");
        std::fs::write(&session_path, br#"{"schemaVersion":"1.0.0"}"#).unwrap();

        let base = SourceFingerprint::from_kiro_path(&session_path).unwrap();

        // messages.jsonl appearing (session.json untouched) must invalidate.
        let messages_path = sess_dir.join("messages.jsonl");
        std::fs::write(
            &messages_path,
            br#"{"role":"user","content":"hello"}
"#,
        )
        .unwrap();
        let with_messages = SourceFingerprint::from_kiro_path(&session_path).unwrap();
        assert_ne!(base, with_messages);

        // An append landing after the last session.json write must invalidate.
        std::fs::write(
            &messages_path,
            br#"{"role":"user","content":"hello"}
{"role":"assistant","content":"world"}
"#,
        )
        .unwrap();
        let updated_messages = SourceFingerprint::from_kiro_path(&session_path).unwrap();
        assert_ne!(with_messages, updated_messages);

        // A CLI source records its absent same-stem JSONL sidecar so a later
        // creation invalidates the cache without reparsing the primary file.
        let cli_path = dir.path().join("cli-session.json");
        std::fs::write(&cli_path, b"{}").unwrap();
        let cli_fingerprint = SourceFingerprint::from_kiro_path(&cli_path).unwrap();
        assert!(cli_fingerprint.related_files.iter().any(|related| {
            related.suffix == "messages.jsonl"
                && related.path.to_path_buf() == dir.path().join("cli-session.jsonl")
                && !related.exists
        }));
    }

    #[test]
    fn test_kiro_cli_fingerprint_tracks_same_stem_jsonl_changes() {
        let dir = TempDir::new().unwrap();
        let session_path = dir.path().join("cli-session.json");
        std::fs::write(&session_path, br#"{"sessionId":"session-1"}"#).unwrap();

        let base = SourceFingerprint::from_kiro_path(&session_path).unwrap();

        let messages_path = dir.path().join("cli-session.jsonl");
        std::fs::write(&messages_path, b"message-1\n").unwrap();
        let with_messages = SourceFingerprint::from_kiro_path(&session_path).unwrap();
        assert_ne!(base, with_messages);

        std::fs::write(&messages_path, b"message-2\n").unwrap();
        let updated_messages = SourceFingerprint::from_kiro_path(&session_path).unwrap();
        assert_ne!(with_messages, updated_messages);
    }

    #[test]
    fn test_droid_fingerprint_tracks_fallback_jsonl_changes() {
        let dir = TempDir::new().unwrap();
        let settings_path = dir.path().join("session.settings.json");
        std::fs::write(&settings_path, br#"{"tokenUsage":{"inputTokens":1}}"#).unwrap();

        let base = SourceFingerprint::from_droid_path(&settings_path).unwrap();

        let jsonl_path = dir.path().join("session.jsonl");
        std::fs::write(&jsonl_path, b"Model: Claude Sonnet 4\n").unwrap();
        let with_jsonl = SourceFingerprint::from_droid_path(&settings_path).unwrap();
        assert_ne!(base, with_jsonl);

        std::fs::write(&jsonl_path, b"Model: Claude Opus 4\n").unwrap();
        let updated_jsonl = SourceFingerprint::from_droid_path(&settings_path).unwrap();
        assert_ne!(with_jsonl, updated_jsonl);
    }

    #[test]
    fn test_kimi_fingerprint_tracks_legacy_config_but_keeps_kimi_code_self_contained() {
        let dir = TempDir::new().unwrap();
        let legacy_path = dir.path().join(".kimi/sessions/group/session/wire.jsonl");
        std::fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        std::fs::write(&legacy_path, b"usage\n").unwrap();

        let legacy_base = SourceFingerprint::from_kimi_path(&legacy_path).unwrap();
        let legacy_config = dir.path().join(".kimi/config.json");
        std::fs::write(&legacy_config, br#"{"model":"kimi-k2"}"#).unwrap();
        let legacy_with_config = SourceFingerprint::from_kimi_path(&legacy_path).unwrap();
        assert_ne!(legacy_base, legacy_with_config);

        std::fs::write(&legacy_config, br#"{"model":"kimi-k3"}"#).unwrap();
        let legacy_updated_config = SourceFingerprint::from_kimi_path(&legacy_path).unwrap();
        assert_ne!(legacy_with_config, legacy_updated_config);

        let code_path = dir
            .path()
            .join(".kimi-code/sessions/workspace/session/agents/main/wire.jsonl");
        std::fs::create_dir_all(code_path.parent().unwrap()).unwrap();
        std::fs::write(&code_path, b"usage.record\n").unwrap();
        let code_base = SourceFingerprint::from_kimi_path(&code_path).unwrap();
        assert_eq!(code_base, SourceFingerprint::from_path(&code_path).unwrap());

        let would_be_config = crate::sessions::kimi::kimi_config_path(&code_path).unwrap();
        std::fs::create_dir_all(would_be_config.parent().unwrap()).unwrap();
        std::fs::write(&would_be_config, br#"{"model":"unrelated"}"#).unwrap();
        let code_with_config = SourceFingerprint::from_kimi_path(&code_path).unwrap();
        assert_eq!(code_base, code_with_config);
    }

    #[test]
    #[serial_test::serial]
    fn test_kimi_stale_parser_cache_is_rejected_and_rebuilt_with_same_fingerprint() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let source_home = TempDir::new().unwrap();
        // Spelled the way the scan will spell it. `ClientDef::resolve_path`
        // joins the root with `/` and `WalkDir` appends the components below it
        // with the platform separator, so on Windows the parse stores this
        // entry under `<home>/.kimi/sessions\group\session\wire.jsonl` while a
        // `Path::join` fixture asks for it back under all backslashes.
        // `CachedPath` keys on the OS string as written, so those are two keys
        // for one file and the lookup below found nothing.
        let wire_path = PathBuf::from(
            ClientId::Kimi
                .data()
                .resolve_path_with_env_strategy(&source_home.path().to_string_lossy(), false),
        )
        .join("group")
        .join("session")
        .join("wire.jsonl");
        std::fs::create_dir_all(wire_path.parent().unwrap()).unwrap();
        std::fs::write(
            &wire_path,
            concat!(
                r#"{"type":"metadata","protocol_version":"1.3"}"#,
                "\n",
                r#"{"timestamp":1770983410.0,"message":{"type":"StatusUpdate","payload":{"token_usage":{"input_other":9223372036854775807,"output":9223372036854775807,"input_cache_read":2,"input_cache_creation":0},"message_id":"msg-extreme"}}}"#,
                "\n",
            ),
        )
        .unwrap();

        let fingerprint = match SourceFingerprint::check_kimi_path_samples_only(&wire_path, None)
            .unwrap()
        {
            FingerprintStatus::Changed(fingerprint) => fingerprint,
            FingerprintStatus::Unchanged => panic!("an uncached source must build a fingerprint"),
        };
        let identity = CacheIdentity::for_client(ClientId::Kimi);
        let stale_identity = CacheIdentity {
            namespace: identity.namespace,
            parser_version: identity.parser_version.saturating_sub(1),
        };
        let stale_message = UnifiedMessage::new(
            "kimi",
            "stale-model",
            "moonshot",
            "stale-session",
            1,
            TokenBreakdown {
                input: 999,
                output: 1,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
        );
        let stale_entry = CachedSourceEntry::new(
            stale_identity,
            &wire_path,
            fingerprint.clone(),
            vec![stale_message],
            Vec::new(),
            None,
        );
        let stale_shard = cache_shard_path(identity, &wire_path);
        ensure_cache_dir(stale_shard.parent().unwrap()).unwrap();
        write_shard_with_limit(
            &stale_shard,
            stale_identity,
            &[stale_entry],
            MAX_CACHE_SHARD_BYTES,
        )
        .unwrap();

        let loaded = SourceMessageCache::load();
        assert!(loaded.get(identity, &wire_path).is_none());
        assert!(matches!(
            SourceFingerprint::check_kimi_path_samples_only(&wire_path, Some(&fingerprint)),
            Some(FingerprintStatus::Unchanged)
        ));

        let first = crate::parse_all_messages_with_pricing_with_env_strategy(
            source_home.path().to_str().unwrap(),
            &["kimi".to_string()],
            None,
            false,
            &crate::scanner::ScannerSettings::default(),
        );
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].tokens.input, i64::MAX);
        assert_eq!(first[0].tokens.output, i64::MAX);
        assert_eq!(first[0].tokens.cache_read, 2);
        assert_eq!(first[0].tokens.cache_write, 0);
        assert!(
            matches!(
                SourceFingerprint::check_kimi_path_samples_only(&wire_path, Some(&fingerprint)),
                Some(FingerprintStatus::Unchanged)
            ),
            "parser-version invalidation must not require a source rewrite"
        );

        let rebuilt = SourceMessageCache::load();
        let cached = rebuilt
            .get(identity, &wire_path)
            .expect("production loader should persist the reparsed Kimi entry");
        assert_eq!(cached.parser_version, identity.parser_version);
        assert_eq!(cached.fingerprint, fingerprint);
        assert_eq!(cached.messages.len(), 1);
        assert_eq!(cached.messages[0].tokens.input, i64::MAX);
        assert_eq!(cached.messages[0].tokens.output, i64::MAX);
        assert_eq!(cached.messages[0].tokens.cache_read, 2);
        assert_eq!(cached.messages[0].tokens.cache_write, 0);

        let second = crate::parse_all_messages_with_pricing_with_env_strategy(
            source_home.path().to_str().unwrap(),
            &["kimi".to_string()],
            None,
            false,
            &crate::scanner::ScannerSettings::default(),
        );
        assert_eq!(second, first);
    }

    #[test]
    fn test_claude_sidechain_fingerprint_tracks_nested_parent_session_changes() {
        let dir = TempDir::new().unwrap();
        let project_dir = dir.path().join("projects/project-one");
        let sidechain_path = project_dir
            .join("parent-session/subagents")
            .join("agent-child.jsonl");
        std::fs::create_dir_all(sidechain_path.parent().unwrap()).unwrap();
        std::fs::write(
            &sidechain_path,
            concat!(
                r#"{"type":"assistant","isSidechain":true,"sessionId":"parent-session","agentId":"child","timestamp":"2026-01-01T00:00:00Z","requestId":"req-1","message":{"id":"msg-1","model":"claude-sonnet-4","usage":{"input_tokens":1,"output_tokens":1}}}"#,
                "\n"
            ),
        )
        .unwrap();

        let parent_path =
            crate::sessions::claudecode::parent_session_paths_for_cache(&sidechain_path)
                .into_iter()
                .next()
                .unwrap();
        assert_eq!(parent_path, project_dir.join("parent-session.jsonl"));
        let base =
            SourceFingerprint::from_claude_code_path_with_home(&sidechain_path, None).unwrap();

        std::fs::write(&parent_path, b"parent transcript 1\n").unwrap();
        let with_parent =
            SourceFingerprint::from_claude_code_path_with_home(&sidechain_path, None).unwrap();
        assert_ne!(base, with_parent);

        std::fs::write(&parent_path, b"parent transcript 2\n").unwrap();
        let updated_parent =
            SourceFingerprint::from_claude_code_path_with_home(&sidechain_path, None).unwrap();
        assert_ne!(with_parent, updated_parent);
    }

    #[test]
    fn test_claude_sidechain_fingerprint_tracks_flat_parent_session_changes() {
        let dir = TempDir::new().unwrap();
        let project_dir = dir.path().join("projects/project-one");
        std::fs::create_dir_all(&project_dir).unwrap();
        let sidechain_path = project_dir.join("agent-child.jsonl");
        let mut sidechain = format!("{}\n", "x".repeat(4096)).repeat(65);
        sidechain.push_str(concat!(
            r#"{"type":"assistant","isSidechain":true,"sessionId":"flat-parent","agentId":"child","timestamp":"2026-01-01T00:00:00Z","requestId":"req-1","message":{"id":"msg-1","model":"claude-sonnet-4","usage":{"input_tokens":1,"output_tokens":1}}}"#,
            "\n"
        ));
        std::fs::write(&sidechain_path, sidechain).unwrap();

        let parent_path =
            crate::sessions::claudecode::parent_session_paths_for_cache(&sidechain_path)
                .into_iter()
                .next()
                .unwrap();
        assert_eq!(parent_path, project_dir.join("flat-parent.jsonl"));
        let base =
            SourceFingerprint::from_claude_code_path_with_home(&sidechain_path, None).unwrap();

        std::fs::write(&parent_path, b"flat parent 1\n").unwrap();
        let with_parent =
            SourceFingerprint::from_claude_code_path_with_home(&sidechain_path, None).unwrap();
        assert_ne!(base, with_parent);

        std::fs::write(&parent_path, b"flat parent 2\n").unwrap();
        let updated_parent =
            SourceFingerprint::from_claude_code_path_with_home(&sidechain_path, None).unwrap();
        assert_ne!(with_parent, updated_parent);
    }

    #[test]
    fn test_claude_sidechain_warm_check_reuses_cached_parent_dependencies() {
        let dir = TempDir::new().unwrap();
        let project_dir = dir.path().join("projects/project-one");
        std::fs::create_dir_all(&project_dir).unwrap();
        let sidechain_path = project_dir.join("agent-child.jsonl");
        let mut sidechain = format!("{}\n", "x".repeat(4096)).repeat(65);
        sidechain.push_str(concat!(
            r#"{"type":"assistant","isSidechain":true,"sessionId":"flat-parent","agentId":"child","timestamp":"2026-01-01T00:00:00Z","requestId":"req-1","message":{"id":"msg-1","model":"claude-sonnet-4","usage":{"input_tokens":1,"output_tokens":1}}}"#,
            "\n"
        ));
        std::fs::write(&sidechain_path, sidechain).unwrap();

        let cached =
            SourceFingerprint::from_claude_code_path_with_home(&sidechain_path, None).unwrap();
        let parent_path = project_dir.join("flat-parent.jsonl");
        assert!(cached.related_files.iter().any(|related| {
            related.suffix == "parent-session-0.jsonl"
                && related.path.to_path_buf() == parent_path
                && !related.exists
        }));
        assert!(matches!(
            SourceFingerprint::check_claude_code_path_with_home_samples_only(
                &sidechain_path,
                Some(&cached),
                None,
            ),
            Some(FingerprintStatus::Unchanged)
        ));

        std::fs::write(&parent_path, b"parent transcript\n").unwrap();
        assert!(matches!(
            SourceFingerprint::check_claude_code_path_with_home_samples_only(
                &sidechain_path,
                Some(&cached),
                None,
            ),
            Some(FingerprintStatus::Changed(_))
        ));
    }

    #[test]
    fn test_claude_code_fingerprint_tracks_meta_sidecar_changes() {
        let dir = TempDir::new().unwrap();
        let jsonl_path = dir.path().join("agent-abc123.jsonl");
        std::fs::write(&jsonl_path, b"jsonl-content").unwrap();

        // No meta sidecar → baseline fingerprint
        let base = SourceFingerprint::from_claude_code_path_with_home(&jsonl_path, None).unwrap();

        // Add meta sidecar → fingerprint changes
        let meta_path = dir.path().join("agent-abc123.meta.json");
        std::fs::write(&meta_path, br#"{"agentType":"explore"}"#).unwrap();
        let with_meta =
            SourceFingerprint::from_claude_code_path_with_home(&jsonl_path, None).unwrap();
        assert_ne!(
            base, with_meta,
            "Adding meta sidecar should change fingerprint"
        );

        // Update meta sidecar → fingerprint changes again
        std::fs::write(&meta_path, br#"{"agentType":"executor"}"#).unwrap();
        let updated_meta =
            SourceFingerprint::from_claude_code_path_with_home(&jsonl_path, None).unwrap();
        assert_ne!(
            with_meta, updated_meta,
            "Updating meta sidecar should change fingerprint"
        );

        // Main session file (no agent- prefix) → unaffected by unrelated meta files
        let main_path = dir.path().join("session-uuid.jsonl");
        std::fs::write(&main_path, b"main-session").unwrap();
        let main_fp1 =
            SourceFingerprint::from_claude_code_path_with_home(&main_path, None).unwrap();
        // Create a meta file with the main session stem (unlikely in practice)
        let main_meta = dir.path().join("session-uuid.meta.json");
        std::fs::write(&main_meta, br#"{"agentType":"x"}"#).unwrap();
        let main_fp2 =
            SourceFingerprint::from_claude_code_path_with_home(&main_path, None).unwrap();
        assert_ne!(
            main_fp1, main_fp2,
            "Claude Code fingerprints always track .meta.json if it exists"
        );
    }

    #[test]
    fn test_claude_code_fingerprint_tracks_cc_mirror_variant_metadata_changes() {
        let dir = TempDir::new().unwrap();
        let variant_dir = dir.path().join(".cc-mirror/kimi-code");
        let config_dir = variant_dir.join("config");
        let project_dir = config_dir.join("projects/project-one");
        std::fs::create_dir_all(&project_dir).unwrap();
        let jsonl_path = project_dir.join("session.jsonl");
        std::fs::write(&jsonl_path, b"jsonl-content").unwrap();

        let variant_path = variant_dir.join("variant.json");
        std::fs::write(
            &variant_path,
            format!(
                r#"{{"name":"kimi-code","provider":"kimi","configDir":{}}}"#,
                json_path_literal(&config_dir)
            ),
        )
        .unwrap();
        let with_kimi =
            SourceFingerprint::from_claude_code_path_with_home(&jsonl_path, None).unwrap();

        std::fs::write(
            &variant_path,
            format!(
                r#"{{"name":"kimi-code","provider":"minimax","configDir":{}}}"#,
                json_path_literal(&config_dir)
            ),
        )
        .unwrap();
        let with_minimax =
            SourceFingerprint::from_claude_code_path_with_home(&jsonl_path, None).unwrap();

        assert_ne!(
            with_kimi, with_minimax,
            "Changing cc-mirror provider metadata should invalidate parsed Claude cache entries"
        );
    }

    #[test]
    fn test_claude_code_fingerprint_tracks_cc_mirror_custom_config_dir_metadata_changes() {
        let dir = TempDir::new().unwrap();
        let variant_dir = dir.path().join(".cc-mirror/kimi-code");
        let config_dir = dir.path().join("mirror-configs/kimi-code");
        let project_dir = config_dir.join("projects/project-one");
        std::fs::create_dir_all(&project_dir).unwrap();
        let jsonl_path = project_dir.join("session.jsonl");
        std::fs::write(&jsonl_path, b"jsonl-content").unwrap();

        std::fs::create_dir_all(&variant_dir).unwrap();
        let variant_path = variant_dir.join("variant.json");
        std::fs::write(
            &variant_path,
            format!(
                r#"{{"name":"kimi-code","provider":"kimi","configDir":{}}}"#,
                json_path_literal(&config_dir)
            ),
        )
        .unwrap();
        let with_kimi =
            SourceFingerprint::from_claude_code_path_with_home(&jsonl_path, Some(dir.path()))
                .unwrap();

        std::fs::write(
            &variant_path,
            format!(
                r#"{{"name":"kimi-code","provider":"minimax","configDir":{}}}"#,
                json_path_literal(&config_dir)
            ),
        )
        .unwrap();
        let with_minimax =
            SourceFingerprint::from_claude_code_path_with_home(&jsonl_path, Some(dir.path()))
                .unwrap();

        assert_ne!(
            with_kimi, with_minimax,
            "Changing cc-mirror metadata should invalidate cache entries for custom configDir layouts"
        );
    }

    #[test]
    fn test_codex_incremental_cache_requires_newline_boundary() {
        let file = write_temp_file(b"line-1\nline-2");

        assert!(build_codex_incremental_cache(
            file.path(),
            file.as_file().metadata().unwrap().len(),
            CodexParseState::default(),
        )
        .is_none());
    }

    #[test]
    fn test_codex_prefix_matches_rejects_middle_rewrite_with_same_tail() {
        let file = write_temp_file(b"aaaa\nbbbb\ncccc\n");
        let fingerprint = SourceFingerprint::from_path(file.path()).unwrap();
        let incremental_cache = build_codex_incremental_cache(
            file.path(),
            fingerprint.size,
            CodexParseState::default(),
        )
        .unwrap();

        std::fs::write(file.path(), b"aaaa\nzzzz\ncccc\nmore\n").unwrap();

        assert!(!codex_prefix_matches(file.path(), &incremental_cache));
    }

    #[test]
    fn test_codex_prefix_matches_rejects_large_unsampled_rewrite() {
        let mut original = vec![b'a'; 128 * 1024];
        original.extend_from_slice(b"\n");
        let file = write_temp_file(&original);
        let fingerprint = SourceFingerprint::from_path(file.path()).unwrap();
        let incremental_cache = build_codex_incremental_cache(
            file.path(),
            fingerprint.size,
            CodexParseState::default(),
        )
        .unwrap();

        let mut rewritten = original.clone();
        rewritten[73 * 1024] = b'z';
        rewritten.extend_from_slice(b"appended\n");
        std::fs::write(file.path(), rewritten).unwrap();

        assert!(!codex_prefix_matches(file.path(), &incremental_cache));
    }

    #[test]
    fn test_write_shard_round_trips_after_atomic_replace() {
        let source = write_temp_file(b"{}\n");
        let identity = CacheIdentity::for_client(ClientId::Claude);
        let entry = test_entry(identity, source.path(), "session-1");
        let shard_dir = TempDir::new().unwrap();
        let shard_path = shard_dir.path().join("shard.bin");

        write_shard_with_limit(
            &shard_path,
            identity,
            std::slice::from_ref(&entry),
            MAX_CACHE_SHARD_BYTES,
        )
        .unwrap();

        assert!(matches!(
            read_shard(&shard_path, identity),
            ShardReadStatus::Loaded(entries)
                if entries.len() == 1 && entries[0].messages[0].session_id == "session-1"
        ));
    }

    #[test]
    #[serial_test::serial]
    fn test_source_message_cache_round_trips_across_distinct_shards() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let source_dir = TempDir::new().unwrap();
        let identity = CacheIdentity::for_client(ClientId::Claude);
        let (path_one, path_two) = write_sources_in_distinct_shards(&source_dir, identity);
        let shard_one = cache_shard_path(identity, &path_one);
        let shard_two = cache_shard_path(identity, &path_two);
        assert_ne!(shard_one, shard_two);

        let mut cache = SourceMessageCache::default();
        cache.insert(test_entry(identity, &path_one, "session-1"));
        cache.insert(test_entry(identity, &path_two, "session-2"));
        cache.save_if_dirty();

        assert!(shard_one.is_file());
        assert!(shard_two.is_file());
        let loaded = SourceMessageCache::load();
        assert_eq!(loaded.entry_count(), 2);
        assert!(loaded.get(identity, &path_one).is_some());
        assert!(loaded.get(identity, &path_two).is_some());
    }

    #[test]
    #[serial_test::serial]
    fn test_aggregate_cache_can_exceed_individual_shard_limit() {
        const TEST_SHARD_LIMIT: u64 = 32 * 1024;

        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let source_dir = TempDir::new().unwrap();
        let identity = CacheIdentity::for_client(ClientId::Claude);
        let (path_one, path_two) = write_sources_in_distinct_shards(&source_dir, identity);

        let mut entry_one = test_entry(identity, &path_one, "session-1");
        entry_one.messages[0].model_id = "a".repeat(20 * 1024);
        let mut entry_two = test_entry(identity, &path_two, "session-2");
        entry_two.messages[0].model_id = "b".repeat(20 * 1024);

        let mut cache = SourceMessageCache::default();
        cache.insert(entry_one);
        cache.insert(entry_two);
        cache.save_if_dirty_with_limit(TEST_SHARD_LIMIT);
        assert!(
            !cache.is_dirty(),
            "both independently bounded shards should save"
        );

        let shard_one = cache_shard_path(identity, &path_one);
        let shard_two = cache_shard_path(identity, &path_two);
        let size_one = std::fs::metadata(&shard_one).unwrap().len();
        let size_two = std::fs::metadata(&shard_two).unwrap().len();
        assert!(size_one <= TEST_SHARD_LIMIT);
        assert!(size_two <= TEST_SHARD_LIMIT);
        assert!(size_one + size_two > TEST_SHARD_LIMIT);

        let loaded = SourceMessageCache::load();
        assert!(loaded.get(identity, &path_one).is_some());
        assert!(loaded.get(identity, &path_two).is_some());
    }

    #[test]
    #[serial_test::serial]
    fn test_corrupt_shard_does_not_hide_entries_from_other_shards() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let source_dir = TempDir::new().unwrap();
        let identity = CacheIdentity::for_client(ClientId::Claude);
        let (corrupt_path, valid_path) = write_sources_in_distinct_shards(&source_dir, identity);

        let mut cache = SourceMessageCache::default();
        cache.insert(test_entry(identity, &corrupt_path, "corrupt-session"));
        cache.insert(test_entry(identity, &valid_path, "valid-session"));
        cache.save_if_dirty();

        let corrupt_shard = cache_shard_path(identity, &corrupt_path);
        std::fs::write(&corrupt_shard, b"not a bincode shard").unwrap();
        assert!(matches!(
            read_shard(&corrupt_shard, identity),
            ShardReadStatus::Invalid(_)
        ));

        let loaded = SourceMessageCache::load();
        assert!(loaded.get(identity, &corrupt_path).is_none());
        assert_eq!(
            loaded.get(identity, &valid_path).unwrap().messages[0].session_id,
            "valid-session"
        );
        assert!(
            loaded.is_dirty(),
            "the corrupt shard should be scheduled for rewrite"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_stale_parser_shard_is_skipped_before_decoding_garbage_payload() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let source = write_temp_file(b"claude\n");
        let claude = CacheIdentity::for_client(ClientId::Claude);
        let codex = CacheIdentity::for_client(ClientId::Codex);

        let mut seed = SourceMessageCache::default();
        seed.insert(test_entry(claude, source.path(), "claude-session"));
        seed.save_if_dirty();

        let stale_key = CacheShardKey {
            namespace: codex.namespace.to_string(),
            index: 0,
        };
        let stale_path = shard_path(&cache_shard_dir().unwrap(), &stale_key);
        ensure_cache_dir(stale_path.parent().unwrap()).unwrap();
        let stale_envelope = CachedShardEnvelope {
            format_version: CACHE_FORMAT_VERSION,
            parser_namespace: codex.namespace.to_string(),
            parser_version: codex.parser_version.saturating_sub(1),
            payload: b"deliberately invalid entry payload".to_vec(),
        };
        // Scoped, so the handle is closed before anything rewrites this shard:
        // the rewrite goes through an atomic replace, and Windows refuses to
        // replace a file another handle still has open (`Access is denied`, os
        // error 5). On Unix the rename succeeds with the handle open, which is
        // why the leak was invisible.
        {
            let mut writer = BufWriter::new(File::create(&stale_path).unwrap());
            bincode::options()
                .serialize_into(&mut writer, &stale_envelope)
                .unwrap();
            writer.flush().unwrap();
        }

        assert!(matches!(
            read_shard(&stale_path, codex),
            ShardReadStatus::Stale
        ));
        let mut loaded = SourceMessageCache::load();
        assert_eq!(loaded.entry_count(), 1);
        assert!(loaded.get(claude, source.path()).is_some());
        assert!(loaded.has_rewrite_shard(&stale_key));

        loaded.save_if_dirty();
        assert!(matches!(
            read_shard(&stale_path, codex),
            ShardReadStatus::Loaded(entries) if entries.is_empty()
        ));
        assert!(SourceMessageCache::load()
            .get(claude, source.path())
            .is_some());
    }

    #[test]
    #[serial_test::serial]
    fn test_prior_cache_format_shard_is_skipped_before_decoding_payload() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let codex = CacheIdentity::for_client(ClientId::Codex);
        let stale_key = CacheShardKey {
            namespace: codex.namespace.to_string(),
            index: 0,
        };
        let stale_path = shard_path(&cache_shard_dir().unwrap(), &stale_key);
        ensure_cache_dir(stale_path.parent().unwrap()).unwrap();
        let stale_envelope = CachedShardEnvelope {
            format_version: LEGACY_CACHE_FORMAT_VERSION - 1,
            parser_namespace: codex.namespace.to_string(),
            parser_version: codex.parser_version,
            payload: b"prior UnifiedMessage layout".to_vec(),
        };
        let mut writer = BufWriter::new(File::create(&stale_path).unwrap());
        bincode::options()
            .serialize_into(&mut writer, &stale_envelope)
            .unwrap();
        writer.flush().unwrap();

        assert!(matches!(
            read_shard(&stale_path, codex),
            ShardReadStatus::Stale
        ));
    }

    #[test]
    #[serial_test::serial]
    fn test_v4_shard_migrates_messages_and_rewrites_once() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let source = write_temp_file(b"{}\n");
        let identity = CacheIdentity::for_client(ClientId::PrimeAgent);
        let entry = test_entry(identity, source.path(), "legacy-prime");
        let key = CacheKey::from_entry(&entry);
        let shard_key = key.shard();
        let legacy_path = shard_path(&cache_shard_dir().unwrap(), &shard_key);
        ensure_cache_dir(legacy_path.parent().unwrap()).unwrap();
        let legacy_entry = LegacyCachedSourceEntryV4 {
            parser_namespace: entry.parser_namespace,
            parser_version: entry.parser_version,
            path: entry.path,
            fingerprint: entry.fingerprint,
            messages: entry.messages,
            fallback_timestamp_indices: entry.fallback_timestamp_indices,
            codex_incremental: entry.codex_incremental,
        };
        let envelope = CachedShardEnvelope {
            format_version: LEGACY_CACHE_FORMAT_VERSION,
            parser_namespace: identity.namespace.to_string(),
            parser_version: identity.parser_version,
            payload: bincode::options().serialize(&vec![legacy_entry]).unwrap(),
        };
        let mut writer = BufWriter::new(File::create(&legacy_path).unwrap());
        bincode::options()
            .serialize_into(&mut writer, &envelope)
            .unwrap();
        writer.flush().unwrap();
        drop(writer);

        assert!(matches!(
            read_shard(&legacy_path, identity),
            ShardReadStatus::Migrated(entries)
                if entries.len() == 1
                    && entries[0].messages[0].session_id == "legacy-prime"
                    && entries[0].prime_accounting.is_none()
        ));

        let mut cache = SourceMessageCache::load();
        assert_eq!(
            cache.get(identity, source.path()).unwrap().messages[0].session_id,
            "legacy-prime"
        );
        assert!(cache.has_rewrite_shard(&shard_key));
        cache.save_if_dirty();
        assert!(matches!(
            read_shard(&legacy_path, identity),
            ShardReadStatus::Loaded(entries)
                if entries.len() == 1 && entries[0].prime_accounting.is_none()
        ));
    }

    #[test]
    #[serial_test::serial]
    fn test_copilot_stale_cache_is_rejected_and_rebuilt_with_root_agent() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let source_dir = TempDir::new().unwrap();
        let source_path = source_dir.path().join("copilot-otel.jsonl");
        std::fs::write(
            &source_path,
            concat!(
                r#"{"type":"span","traceId":"trace-cache","spanId":"invoke-sub","parentSpanId":"tool-task","name":"invoke_agent","attributes":{"gen_ai.operation.name":"invoke_agent","gen_ai.agent.id":"github.copilot.subagent"}}"#,
                "\n",
                r#"{"type":"span","traceId":"trace-cache","spanId":"tool-task","parentSpanId":"invoke-root","name":"execute_tool task"}"#,
                "\n",
                r#"{"type":"span","traceId":"trace-cache","spanId":"invoke-root","name":"invoke_agent","attributes":{"gen_ai.operation.name":"invoke_agent","gen_ai.agent.id":"github.copilot.default"}}"#,
                "\n",
                r#"{"type":"span","traceId":"trace-cache","spanId":"chat","parentSpanId":"invoke-root","name":"chat gpt-5.4-mini","attributes":{"gen_ai.operation.name":"chat","gen_ai.request.model":"gpt-5.4-mini","gen_ai.response.model":"gpt-5.4-mini","gen_ai.usage.input_tokens":1,"gen_ai.usage.output_tokens":1}}"#,
                "\n",
                r#"{"type":"span","traceId":"trace-cache","spanId":"chat","parentSpanId":"invoke-root","name":"chat gpt-5.4-mini","attributes":{"gen_ai.operation.name":"chat","gen_ai.request.model":"gpt-5.4-mini","gen_ai.response.model":"gpt-5.4-mini","gen_ai.usage.input_tokens":9,"gen_ai.usage.output_tokens":8}}"#,
                "\n",
            ),
        )
        .unwrap();

        let current_identity = CacheIdentity::for_client(ClientId::Copilot);
        let stale_identity = CacheIdentity {
            namespace: current_identity.namespace,
            parser_version: current_identity.parser_version.saturating_sub(1),
        };
        let mut stale_message = UnifiedMessage::new_with_dedup(
            "copilot",
            "gpt-5.4-mini",
            "github-copilot",
            "trace-cache",
            1,
            TokenBreakdown {
                input: 1,
                output: 1,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
            Some("trace-cache:chat".to_string()),
        );
        stale_message.agent = Some("github.copilot.subagent".to_string());
        let stale_duplicate = UnifiedMessage::new_with_dedup(
            "copilot",
            "gpt-5.4-mini",
            "github-copilot",
            "trace-cache",
            2,
            TokenBreakdown {
                input: 9,
                output: 8,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
            Some("trace-cache:chat".to_string()),
        );
        let fingerprint = SourceFingerprint::from_path(&source_path).unwrap();
        let stale_entry = CachedSourceEntry::new(
            stale_identity,
            &source_path,
            fingerprint.clone(),
            vec![stale_message, stale_duplicate],
            Vec::new(),
            None,
        );
        let shard_key = CacheKey::new(current_identity, &source_path).shard();
        let stale_path = shard_path(&cache_shard_dir().unwrap(), &shard_key);
        ensure_cache_dir(stale_path.parent().unwrap()).unwrap();
        write_shard_with_limit(
            &stale_path,
            stale_identity,
            &[stale_entry],
            MAX_CACHE_SHARD_BYTES,
        )
        .unwrap();

        let mut loaded = SourceMessageCache::load();
        assert!(
            loaded.get(current_identity, &source_path).is_none(),
            "a stale Copilot cache entry must not be served after the parser output change"
        );
        assert!(loaded.has_rewrite_shard(&shard_key));
        assert_eq!(
            SourceFingerprint::from_path(&source_path).unwrap(),
            fingerprint,
            "the source fingerprint must remain unchanged; parser version causes invalidation"
        );

        let rebuilt = crate::sessions::copilot::parse_copilot_file(&source_path);
        assert_eq!(rebuilt.len(), 1);
        assert_eq!(rebuilt[0].dedup_key.as_deref(), Some("trace-cache:chat"));
        assert_eq!(rebuilt[0].tokens.input, 9);
        assert_eq!(rebuilt[0].tokens.output, 8);
        assert_eq!(
            rebuilt[0].agent.as_deref(),
            Some("github.copilot.default"),
            "a cold rebuild must use the root invoke_agent attribution"
        );
        loaded.insert(CachedSourceEntry::new(
            current_identity,
            &source_path,
            fingerprint,
            rebuilt,
            Vec::new(),
            None,
        ));
        loaded.save_if_dirty();

        let reloaded = SourceMessageCache::load();
        let cached = reloaded
            .get(current_identity, &source_path)
            .expect("rebuilt Copilot cache entry should survive reload");
        assert_eq!(cached.parser_version, current_identity.parser_version);
        assert_eq!(
            cached.messages[0].agent.as_deref(),
            Some("github.copilot.default")
        );
        assert!(matches!(
            read_shard(&stale_path, current_identity),
            ShardReadStatus::Loaded(entries)
                if entries.len() == 1
                    && entries[0].messages[0].tokens.input == 9
                    && entries[0].messages[0].agent.as_deref()
                        == Some("github.copilot.default")
        ));
    }

    /// #1100: a scan must not deserialize namespaces it never reads.
    ///
    /// Before this was lazy, `load` read every shard of every client up front,
    /// so a one-file `-c droid` scan paid for the whole machine's history —
    /// 1.16 GB against a 358 MB cache on the reporter's class of machine — and
    /// the TUI paid it again on every auto-refresh.
    #[test]
    #[serial_test::serial]
    fn load_reads_only_the_namespaces_a_scan_touches() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let claude = CacheIdentity::for_client(ClientId::Claude);
        let codex = CacheIdentity::for_client(ClientId::Codex);
        let claude_source = write_temp_file(b"claude\n");
        let codex_source = write_temp_file(b"codex\n");

        let mut seed = SourceMessageCache::default();
        seed.insert(test_entry(claude, claude_source.path(), "claude-session"));
        seed.insert(test_entry(codex, codex_source.path(), "codex-session"));
        seed.save_if_dirty();

        let cache = SourceMessageCache::load();
        assert_eq!(
            cache.loaded_namespace_count(),
            0,
            "opening the cache must not read a single shard"
        );

        assert!(cache.take(codex, codex_source.path()).is_some());
        assert_eq!(
            cache.loaded_namespace_count(),
            1,
            "only the namespace that was asked for may be deserialized"
        );
        assert!(
            !cache.namespace_is_loaded(claude.namespace),
            "an untouched client's history must stay on disk"
        );
    }

    /// #1100: the payload handed to a scan must leave the cache, not be copied
    /// out of it. Holding both made a run cost the whole cache plus its own
    /// output at the same time.
    #[test]
    #[serial_test::serial]
    fn take_releases_the_entry_payload_but_keeps_its_metadata() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let identity = CacheIdentity::for_client(ClientId::Codex);
        let source = write_temp_file(b"codex\n");

        let mut seed = SourceMessageCache::default();
        seed.insert(test_entry(identity, source.path(), "codex-session"));
        seed.save_if_dirty();

        let cache = SourceMessageCache::load();
        let taken = cache.take(identity, source.path()).expect("warm entry");
        assert_eq!(taken.messages.len(), 1);
        assert_eq!(taken.messages[0].session_id, "codex-session");

        assert!(
            cache.entry_messages_released(identity, source.path()),
            "the cache must not keep a second copy of a payload it handed out"
        );
        assert_eq!(
            cache.entry_fingerprint(identity, source.path()),
            Some(taken.fingerprint.clone()),
            "the husk still has to answer fingerprint and invalidation lookups"
        );
    }

    /// A drained clean entry is still on disk, so a shard rewrite driven by a
    /// different file must carry it forward untouched. If `save_if_dirty` ever
    /// starts writing shards from memory alone, this is the test that fails.
    #[test]
    #[serial_test::serial]
    fn saving_after_a_take_keeps_the_drained_entry_on_disk() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let identity = CacheIdentity::for_client(ClientId::Codex);
        let untouched = write_temp_file(b"untouched\n");
        let rewritten = write_temp_file(b"rewritten\n");

        let mut seed = SourceMessageCache::default();
        seed.insert(test_entry(identity, untouched.path(), "kept-session"));
        seed.save_if_dirty();

        let mut cache = SourceMessageCache::load();
        assert!(cache.take(identity, untouched.path()).is_some());
        cache.insert(test_entry(identity, rewritten.path(), "new-session"));
        cache.save_if_dirty();

        let reloaded = SourceMessageCache::load();
        assert_eq!(
            reloaded
                .get(identity, untouched.path())
                .expect("the drained entry must survive the save")
                .messages[0]
                .session_id,
            "kept-session"
        );
        assert_eq!(
            reloaded
                .get(identity, rewritten.path())
                .expect("the newly cached entry must be saved")
                .messages[0]
                .session_id,
            "new-session"
        );
    }

    /// A Claude entry holds turns the live transcript no longer contains, and
    /// the cache is the only copy (#994). Draining one would make a second
    /// lookup look like a cold source and retire that history for good, so an
    /// entry that is carrying retained rows is served by clone instead.
    #[test]
    #[serial_test::serial]
    fn take_keeps_the_payload_of_an_entry_carrying_retained_history() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let identity = CacheIdentity::for_client(ClientId::Claude);
        let source = write_temp_file(b"claude\n");

        let live = keyed_message(identity.namespace, "claude-session", "live:req_live");
        let retained = keyed_message(identity.namespace, "claude-session", "old:req_old");
        let mut seed = SourceMessageCache::default();
        seed.insert(CachedSourceEntry::new_with_retained_message_keys(
            identity,
            source.path(),
            SourceFingerprint::from_path(source.path()).unwrap(),
            vec![live, retained],
            &HashSet::from(["old:req_old".to_string()]),
        ));
        seed.save_if_dirty();

        let cache = SourceMessageCache::load();
        let first = cache.take(identity, source.path()).expect("warm entry");
        assert_eq!(first.messages.len(), 2);
        assert_eq!(
            cache
                .take(identity, source.path())
                .expect("a retained-history entry must survive being read")
                .messages
                .len(),
            2,
            "the only copy of a compacted turn must not leave the cache"
        );
    }

    /// The same namespace, but nothing was retained: the live file reproduces
    /// every row, so the entry drains like any other and Claude-heavy machines
    /// still get the memory back.
    #[test]
    #[serial_test::serial]
    fn take_drains_a_claude_entry_with_no_retained_history() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let identity = CacheIdentity::for_client(ClientId::Claude);
        let source = write_temp_file(b"claude\n");

        let live = keyed_message(identity.namespace, "claude-session", "live:req_live");
        let mut seed = SourceMessageCache::default();
        seed.insert(CachedSourceEntry::new_with_retained_message_keys(
            identity,
            source.path(),
            SourceFingerprint::from_path(source.path()).unwrap(),
            vec![live],
            &HashSet::new(),
        ));
        seed.save_if_dirty();

        let cache = SourceMessageCache::load();
        assert_eq!(
            cache
                .take(identity, source.path())
                .expect("warm entry")
                .messages
                .len(),
            1
        );
        assert!(
            cache.entry_messages_released(identity, source.path()),
            "an entry with an empty retained set is reproducible from the live \
             file, so it must not keep a second copy"
        );
    }

    /// An entry written before retention provenance existed cannot say which of
    /// its rows the live file already dropped, so it has to be treated as
    /// carrying history even though its retained set reads as empty.
    #[test]
    #[serial_test::serial]
    fn take_keeps_the_payload_of_a_pre_provenance_claude_entry() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let identity = CacheIdentity::for_client(ClientId::Claude);
        let source = write_temp_file(b"claude\n");

        let mut seed = SourceMessageCache::default();
        // `test_entry` builds the legacy shape: messages, no provenance marker.
        let legacy = test_entry(identity, source.path(), "claude-session");
        assert!(legacy.needs_retention_provenance_migration());
        seed.insert(legacy);
        seed.save_if_dirty();

        let cache = SourceMessageCache::load();
        assert!(cache.take(identity, source.path()).is_some());
        assert!(
            !cache.entry_messages_released(identity, source.path()),
            "a legacy entry may be hiding retained rows it cannot identify"
        );
    }

    /// Invalidating a source the current process never read still has to
    /// record the fingerprint it replaces, so the stale shard row is dropped.
    #[test]
    #[serial_test::serial]
    fn remove_loads_the_namespace_it_invalidates() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let identity = CacheIdentity::for_client(ClientId::Codex);
        let source = write_temp_file(b"codex\n");

        let mut seed = SourceMessageCache::default();
        seed.insert(test_entry(identity, source.path(), "codex-session"));
        seed.save_if_dirty();

        let mut cache = SourceMessageCache::load();
        cache.remove(identity, source.path());
        cache.save_if_dirty();

        assert!(SourceMessageCache::load()
            .get(identity, source.path())
            .is_none());
    }

    /// A source that vanished is dropped as its namespace loads, which is the
    /// only pruning pass left now that nothing reads the whole cache.
    #[test]
    #[serial_test::serial]
    fn loading_a_namespace_prunes_entries_whose_source_is_gone() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let identity = CacheIdentity::for_client(ClientId::Codex);
        let source = write_temp_file(b"codex\n");
        let path = source.path().to_path_buf();

        let mut seed = SourceMessageCache::default();
        seed.insert(test_entry(identity, &path, "codex-session"));
        seed.save_if_dirty();
        drop(source);
        assert!(!path.exists());

        let mut cache = SourceMessageCache::load();
        assert!(cache.take(identity, &path).is_none());
        cache.save_if_dirty();

        assert!(SourceMessageCache::load().get(identity, &path).is_none());
    }

    /// The shard on disk is older than what this scan produced, so loading the
    /// namespace afterwards must not resurrect the stale row.
    #[test]
    #[serial_test::serial]
    fn a_lazy_namespace_load_does_not_overwrite_this_scan_s_entries() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let identity = CacheIdentity::for_client(ClientId::Codex);
        let source = write_temp_file(b"codex\n");

        let mut seed = SourceMessageCache::default();
        seed.insert(test_entry(identity, source.path(), "stale-session"));
        seed.save_if_dirty();

        let mut cache = SourceMessageCache::load();
        cache.insert(test_entry(identity, source.path(), "fresh-session"));
        assert_eq!(
            cache
                .get(identity, source.path())
                .expect("the freshly inserted entry")
                .messages[0]
                .session_id,
            "fresh-session",
            "a later shard read must not clobber what this scan just produced"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_explicit_invalidation_of_existing_path_persists() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let source = write_temp_file(b"still exists\n");
        let identity = CacheIdentity::for_client(ClientId::Claude);

        let mut seed = SourceMessageCache::default();
        seed.insert(test_entry(identity, source.path(), "session-1"));
        seed.save_if_dirty();
        assert!(SourceMessageCache::load()
            .get(identity, source.path())
            .is_some());

        let mut cache = SourceMessageCache::load();
        cache.remove(identity, source.path());
        cache.save_if_dirty();

        assert!(
            source.path().is_file(),
            "invalidation must not remove the source"
        );
        assert!(SourceMessageCache::load()
            .get(identity, source.path())
            .is_none());
    }

    #[test]
    #[serial_test::serial]
    fn test_stale_invalidation_preserves_concurrently_refreshed_entry() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());
        let source_dir = TempDir::new().unwrap();
        let path = source_dir.path().join("session.jsonl");
        let identity = CacheIdentity::for_client(ClientId::Claude);
        std::fs::write(&path, b"old\n").unwrap();

        let mut seed = SourceMessageCache::default();
        seed.insert(test_entry(identity, &path, "old-session"));
        seed.save_if_dirty();

        let mut stale_invalidator = SourceMessageCache::load();
        stale_invalidator.remove(identity, &path);

        std::fs::write(&path, b"fresh-content\n").unwrap();
        let mut fresh_writer = SourceMessageCache::load();
        fresh_writer.insert(test_entry(identity, &path, "fresh-session"));
        fresh_writer.save_if_dirty();

        stale_invalidator.save_if_dirty();

        let loaded = SourceMessageCache::load();
        assert_eq!(
            loaded.get(identity, &path).unwrap().messages[0].session_id,
            "fresh-session"
        );
    }

    #[test]
    fn test_prune_missing_files_removes_deleted_entries() {
        let file = write_temp_file(b"{}\n");
        let path = file.path().to_path_buf();
        let identity = CacheIdentity::for_client(ClientId::Claude);

        let mut cache = SourceMessageCache::default();
        cache.insert(test_entry(identity, &path, "session-1"));

        std::fs::remove_file(&path).unwrap();
        cache.prune_missing_files();

        assert!(cache.all_entries().is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn test_fallback_cache_dir_prefers_runtime_dir() {
        let runtime_dir = TempDir::new().unwrap();
        let original_xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok();
        restore_env_var("XDG_RUNTIME_DIR", Some(runtime_dir.path()));

        {
            assert_eq!(
                fallback_cache_dir(),
                Some(runtime_dir.path().join("tokscale"))
            );
        }

        restore_env_var("XDG_RUNTIME_DIR", original_xdg_runtime_dir);
    }

    #[test]
    #[serial_test::serial]
    fn test_save_if_dirty_marks_cache_clean() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());

        let mut cache = SourceMessageCache::default();
        assert!(!cache.is_dirty());

        {
            let file = write_temp_file(b"{}\n");
            let identity = CacheIdentity::for_client(ClientId::Claude);
            cache.insert(test_entry(identity, file.path(), "session-1"));
            assert!(cache.is_dirty());

            cache.save_if_dirty();
            assert!(!cache.is_dirty());
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_save_if_dirty_merges_concurrent_writers() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());

        {
            let source_dir = TempDir::new().unwrap();
            let identity = CacheIdentity::for_client(ClientId::Claude);
            let (path_one, path_two) = write_sources_in_same_shard(&source_dir, identity);
            assert_eq!(
                CacheKey::new(identity, &path_one).shard(),
                CacheKey::new(identity, &path_two).shard()
            );

            let mut writer_one = SourceMessageCache::load();
            let mut writer_two = SourceMessageCache::load();

            writer_one.insert(test_entry(identity, &path_one, "session-1"));
            writer_two.insert(test_entry(identity, &path_two, "session-2"));

            writer_one.save_if_dirty();
            writer_two.save_if_dirty();

            let loaded = SourceMessageCache::load();
            assert!(loaded.get(identity, &path_one).is_some());
            assert!(loaded.get(identity, &path_two).is_some());
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_save_if_dirty_preserves_recreated_path_from_concurrent_writer() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());

        {
            let source_dir = TempDir::new().unwrap();
            let path = source_dir.path().join("session.jsonl");
            std::fs::write(&path, b"{\"id\":\"old\"}\n").unwrap();
            let identity = CacheIdentity::for_client(ClientId::Claude);

            let mut seed = SourceMessageCache::default();
            seed.insert(test_entry(identity, &path, "old-session"));
            seed.save_if_dirty();

            let mut stale_deleter = SourceMessageCache::load();
            std::fs::remove_file(&path).unwrap();
            stale_deleter.prune_missing_files();

            std::fs::write(&path, b"{\"id\":\"fresh\"}\n").unwrap();
            let mut fresh_writer = SourceMessageCache::load();
            fresh_writer.insert(test_entry(identity, &path, "fresh-session"));
            fresh_writer.save_if_dirty();

            stale_deleter.save_if_dirty();

            let loaded = SourceMessageCache::load();
            let entry = loaded
                .get(identity, &path)
                .expect("recreated source cache entry should survive stale delete");
            assert_eq!(entry.messages[0].session_id, "fresh-session");
        }
    }

    fn keyed_message(namespace: &str, session_id: &str, dedup_key: &str) -> UnifiedMessage {
        UnifiedMessage::new_with_dedup(
            namespace,
            "claude-3-5-sonnet",
            "anthropic",
            session_id,
            1,
            TokenBreakdown {
                input: 1,
                output: 2,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
            },
            0.0,
            Some(dedup_key.to_string()),
        )
    }

    fn entry_with_messages(
        identity: CacheIdentity,
        path: &Path,
        messages: Vec<UnifiedMessage>,
    ) -> CachedSourceEntry {
        CachedSourceEntry::new(
            identity,
            path,
            SourceFingerprint::from_path(path).unwrap(),
            messages,
            Vec::new(),
            None,
        )
    }

    fn synthetic_placeholder_message(session_id: &str, dedup_key: &str) -> UnifiedMessage {
        let mut message = keyed_message(ClientId::Claude.as_str(), session_id, dedup_key);
        message.model_id = " <SYNTHETIC> ".to_string();
        message.provider_id = "unknown".to_string();
        message
    }

    /// "Nothing was retained" and "this entry predates retention provenance"
    /// both leave the index vector without any real index, and they need
    /// opposite handling: the first is a warm hit, the second has to be
    /// rebuilt from the live transcript. Only the marker separates them.
    #[test]
    fn test_claude_entry_reports_retention_provenance_only_once_recorded() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "{}").unwrap();
        let identity = CacheIdentity::for_client(ClientId::Claude);
        let live = keyed_message(ClientId::Claude.as_str(), "session", "msg_live:req_live");
        let retained = keyed_message(ClientId::Claude.as_str(), "session", "msg_old:req_old");

        let legacy = entry_with_messages(identity, &path, vec![live.clone(), retained.clone()]);
        assert!(
            legacy.needs_retention_provenance_migration(),
            "an entry written before the marker existed has to be rebuilt"
        );
        assert!(legacy.retained_message_keys().is_empty());

        let nothing_retained = CachedSourceEntry::new_with_retained_message_keys(
            identity,
            &path,
            SourceFingerprint::from_path(&path).unwrap(),
            vec![live.clone()],
            &HashSet::new(),
        );
        assert!(
            !nothing_retained.needs_retention_provenance_migration(),
            "an entry that retained nothing is current, not legacy — rebuilding \
             it on every scan would make the upgrade cost permanent"
        );
        assert!(nothing_retained.retained_message_keys().is_empty());

        let with_retained = CachedSourceEntry::new_with_retained_message_keys(
            identity,
            &path,
            SourceFingerprint::from_path(&path).unwrap(),
            vec![live, retained],
            &HashSet::from(["msg_old:req_old".to_string()]),
        );
        assert!(!with_retained.needs_retention_provenance_migration());
        assert_eq!(
            with_retained.retained_message_keys(),
            HashSet::from(["msg_old:req_old".to_string()])
        );

        // A Codex entry uses the same vector for real fallback-timestamp
        // indices and must never be mistaken for a Claude rebuild candidate.
        let codex = entry_with_messages(
            CacheIdentity::for_client(ClientId::Codex),
            &path,
            vec![keyed_message(ClientId::Codex.as_str(), "session", "key")],
        );
        assert!(!codex.needs_retention_provenance_migration());
    }

    #[test]
    #[serial_test::serial]
    fn test_loading_claude_cache_removes_synthetic_placeholder_rows_without_retiring_history() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());

        {
            let source_dir = TempDir::new().unwrap();
            let path = source_dir.path().join("conversation.jsonl");
            std::fs::write(&path, b"{\"id\":\"live\"}\n").unwrap();
            let identity = CacheIdentity::for_client(ClientId::Claude);
            let real_live = keyed_message("claude", "session", "live:req_live");
            let real_retained = keyed_message("claude", "session", "old:req_old");
            let synthetic_assistant =
                synthetic_placeholder_message("session", "synthetic:req_synthetic");
            let mut synthetic_tool_result = synthetic_placeholder_message(
                "session",
                "claude:tool_result:conversation:tool_result:toolu_1",
            );
            synthetic_tool_result.tokens.input = 100;

            let mut seed = SourceMessageCache::default();
            seed.insert(entry_with_messages(
                identity,
                &path,
                vec![
                    real_live.clone(),
                    real_retained.clone(),
                    synthetic_assistant,
                    synthetic_tool_result,
                ],
            ));
            seed.save_if_dirty();

            let mut repaired = SourceMessageCache::load();
            let entry = repaired
                .get(identity, &path)
                .expect("current Claude cache entry should load");
            assert_eq!(entry.messages.len(), 2);
            assert_eq!(
                entry
                    .messages
                    .iter()
                    .filter_map(|message| message.dedup_key.as_deref())
                    .collect::<HashSet<_>>(),
                HashSet::from(["live:req_live", "old:req_old"]),
                "the targeted migration must retain real live and compacted history"
            );
            repaired.save_if_dirty();

            let shard_path = cache_shard_path(identity, &path);
            assert!(matches!(
                read_shard(&shard_path, identity),
                ShardReadStatus::Loaded(entries)
                    if entries.len() == 1
                        && entries[0].messages.len() == 2
                        && entries[0]
                            .messages
                            .iter()
                            .all(|message| message.model_id != " <SYNTHETIC> ")
            ));
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_claude_cache_save_does_not_restore_synthetic_history_from_another_writer() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());

        {
            let source_dir = TempDir::new().unwrap();
            let path = source_dir.path().join("conversation.jsonl");
            std::fs::write(&path, b"{\"id\":\"live\"}\n").unwrap();
            let identity = CacheIdentity::for_client(ClientId::Claude);
            let real = keyed_message("claude", "session", "live:req_live");
            let synthetic = synthetic_placeholder_message("session", "synthetic:req_synthetic");

            let mut seed = SourceMessageCache::default();
            seed.insert(entry_with_messages(
                identity,
                &path,
                vec![real.clone(), synthetic],
            ));
            seed.save_if_dirty();

            // Simulate a process that parsed the live source after a compaction
            // while an old on-disk entry still carries the synthetic notice.
            // The normal retained-history merge would bring the globally stable
            // synthetic key back, so sanitation must run after that merge too.
            let mut fresh_writer = SourceMessageCache::default();
            fresh_writer.insert(entry_with_messages(identity, &path, vec![real]));
            fresh_writer.save_if_dirty();

            let shard_path = cache_shard_path(identity, &path);
            assert!(matches!(
                read_shard(&shard_path, identity),
                ShardReadStatus::Loaded(entries)
                    if entries.len() == 1
                        && entries[0].messages.len() == 1
                        && entries[0].messages[0].dedup_key.as_deref() == Some("live:req_live")
            ));
        }
    }

    /// A Claude entry can hold assistant turns the live transcript no longer
    /// contains (an in-place compaction dropped them). Two processes scanning
    /// at once therefore hold genuinely different histories for one path, and
    /// the wholesale last-writer-wins replace would retire the loser's turns
    /// for good — the live file cannot hand them back.
    #[test]
    #[serial_test::serial]
    fn test_save_if_dirty_unions_retained_history_for_the_same_path() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());

        {
            let source_dir = TempDir::new().unwrap();
            let path = source_dir.path().join("conversation.jsonl");
            std::fs::write(&path, b"{\"id\":\"live\"}\n").unwrap();
            let identity = CacheIdentity::for_client(ClientId::Claude);
            let namespace = ClientId::Claude.as_str();

            // Both processes carry the turn the file still has. Only the first
            // ever observed the one a compaction later removed.
            let mut shared_complete = keyed_message(namespace, "session", "msg_shared:req_shared");
            shared_complete.tokens.input = 2_000;
            shared_complete.tokens.output = 999;
            let mut shared_partial = shared_complete.clone();
            shared_partial.tokens.input = 200;
            shared_partial.tokens.output = 60;
            let observed_only_by_first =
                keyed_message(namespace, "session", "msg_dropped:req_dropped");

            let mut observer = SourceMessageCache::load();
            observer.insert(entry_with_messages(
                identity,
                &path,
                vec![shared_complete, observed_only_by_first],
            ));
            observer.save_if_dirty();

            let mut latecomer = SourceMessageCache::load();
            latecomer.insert(entry_with_messages(identity, &path, vec![shared_partial]));
            latecomer.save_if_dirty();

            let loaded = SourceMessageCache::load();
            let entry = loaded.get(identity, &path).expect("entry should survive");
            let keys: HashSet<&str> = entry
                .messages
                .iter()
                .filter_map(|message| message.dedup_key.as_deref())
                .collect();
            assert!(
                keys.contains("msg_dropped:req_dropped"),
                "a concurrent writer must not discard history it never saw, got {keys:?}"
            );
            assert_eq!(
                entry.messages.len(),
                2,
                "and must not duplicate the shared turn"
            );
            let shared = entry
                .messages
                .iter()
                .find(|message| message.dedup_key.as_deref() == Some("msg_shared:req_shared"))
                .expect("the shared turn should survive");
            assert_eq!(shared.tokens.input, 2_000);
            assert_eq!(shared.tokens.output, 999);
            assert_eq!(
                entry.retained_message_keys(),
                HashSet::from(["msg_dropped:req_dropped".to_string()]),
                "only the row absent from the current source is retained"
            );
        }
    }

    /// The union is scoped to keys that stay valid wherever the message is
    /// written. A Claude tool-result key embeds the transcript's file stem, so
    /// a carried-forward copy could never collapse against the same tool
    /// result replayed into a forked transcript — both would count.
    #[test]
    #[serial_test::serial]
    fn test_save_if_dirty_does_not_union_path_scoped_keys() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());

        {
            let source_dir = TempDir::new().unwrap();
            let path = source_dir.path().join("conversation.jsonl");
            std::fs::write(&path, b"{\"id\":\"live\"}\n").unwrap();
            let identity = CacheIdentity::for_client(ClientId::Claude);
            let namespace = ClientId::Claude.as_str();

            let shared = keyed_message(namespace, "session", "msg_shared:req_shared");
            let tool_result = keyed_message(
                namespace,
                "session",
                "claude:tool_result:conversation:tool_result:toolu_1",
            );

            let mut observer = SourceMessageCache::load();
            observer.insert(entry_with_messages(
                identity,
                &path,
                vec![shared.clone(), tool_result],
            ));
            observer.save_if_dirty();

            let mut latecomer = SourceMessageCache::load();
            latecomer.insert(entry_with_messages(identity, &path, vec![shared]));
            latecomer.save_if_dirty();

            let loaded = SourceMessageCache::load();
            let entry = loaded.get(identity, &path).expect("entry should survive");
            assert_eq!(
                entry.messages.len(),
                1,
                "path-scoped keys must not outlive the bytes that produced them"
            );
        }
    }

    /// The union exists only for namespaces that retain history. Everywhere
    /// else the live file is the whole truth, so a stale entry must still be
    /// replaced outright rather than accumulating.
    #[test]
    #[serial_test::serial]
    fn test_save_if_dirty_still_replaces_entries_for_non_retaining_clients() {
        let temp_home = TempDir::new().unwrap();
        let _cache_env = sandbox_cache_env(temp_home.path());

        {
            let source_dir = TempDir::new().unwrap();
            let path = source_dir.path().join("rollout.jsonl");
            std::fs::write(&path, b"{\"id\":\"live\"}\n").unwrap();
            let identity = CacheIdentity::for_client(ClientId::Codex);
            let namespace = ClientId::Codex.as_str();

            let mut observer = SourceMessageCache::load();
            observer.insert(entry_with_messages(
                identity,
                &path,
                vec![
                    keyed_message(namespace, "session", "codex-key-a"),
                    keyed_message(namespace, "session", "codex-key-b"),
                ],
            ));
            observer.save_if_dirty();

            let mut latecomer = SourceMessageCache::load();
            latecomer.insert(entry_with_messages(
                identity,
                &path,
                vec![keyed_message(namespace, "session", "codex-key-a")],
            ));
            latecomer.save_if_dirty();

            let loaded = SourceMessageCache::load();
            let entry = loaded.get(identity, &path).expect("entry should survive");
            assert_eq!(entry.messages.len(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_cached_path_preserves_non_utf8_bytes() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]));
        let cached_path = CachedPath::from_path(&path);

        assert_eq!(cached_path.to_path_buf(), path);
    }

    /// One file reached under both separators is one cache entry.
    ///
    /// A scan spells a discovered transcript with both: the root half comes
    /// from `format!("{root}/{relative}")` and the children below it from
    /// `Path::join`. Keying on the raw code units made those two spellings two
    /// entries for one file, so the cache could never hit.
    #[cfg(windows)]
    #[test]
    fn cached_path_identity_folds_the_two_windows_separators() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn hash_of(path: &CachedPath) -> u64 {
            let mut hasher = DefaultHasher::new();
            path.hash(&mut hasher);
            hasher.finish()
        }

        let mixed = CachedPath::from_path(Path::new(r"C:\home/.claude/projects\demo\s.jsonl"));
        let native = CachedPath::from_path(Path::new(r"C:\home\.claude\projects\demo\s.jsonl"));

        assert_eq!(mixed, native, "both spellings name one file");
        assert_eq!(hash_of(&mixed), hash_of(&native), "Hash must match Eq");

        let mut digests = Vec::new();
        for path in [&mixed, &native] {
            let mut hasher = Sha256::new();
            path.update_digest(&mut hasher);
            digests.push(hasher.finalize());
        }
        assert_eq!(
            digests[0], digests[1],
            "the shard digest must agree too, or one file lands in two shards"
        );

        // The stored spelling is untouched: `to_path_buf` still round-trips,
        // which `SourceMessageCache` relies on to stat the file it cached.
        assert_eq!(
            mixed.to_path_buf(),
            PathBuf::from(r"C:\home/.claude/projects\demo\s.jsonl")
        );

        // Different files stay different.
        let other = CachedPath::from_path(Path::new(r"C:\home\.claude\projects\demo\t.jsonl"));
        assert_ne!(mixed, other);
    }

    /// After `\\?\` the object manager stops translating, so `/` is an ordinary
    /// character in a name rather than a separator. Folding it there would merge
    /// two genuinely different paths.
    #[cfg(windows)]
    #[test]
    fn cached_path_identity_leaves_verbatim_paths_alone() {
        let with_slash = CachedPath::from_path(Path::new(r"\\?\C:\dir/name\f.jsonl"));
        let with_backslash = CachedPath::from_path(Path::new(r"\\?\C:\dir\name\f.jsonl"));

        assert_ne!(
            with_slash, with_backslash,
            "inside the verbatim namespace `/` is part of the name, not a separator"
        );
    }
}
