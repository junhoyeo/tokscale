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
const CACHE_FORMAT_VERSION: u32 = 2;
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
        || cfg!(target_os = "macos") && dirs::home_dir().is_some()
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

fn warn_cache_failure_once(context: &'static str, path: &Path, error: &impl std::fmt::Display) {
    tracing::warn!(path = %path.display(), %error, %context, "source message cache failure");

    // Most non-TUI commands (including `submit`) do not install a tracing
    // subscriber. Surface persistence failures directly once per process so a
    // permanently cold cache can never fail silently again.
    static WARNED_CONTEXTS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let warned = WARNED_CONTEXTS.get_or_init(|| Mutex::new(HashSet::new()));
    if warned.lock().is_ok_and(|mut warned| warned.insert(context)) {
        eprintln!("tokscale: warning: {context} ({}): {error}", path.display());
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

    /// Fingerprint for a Grok `updates.jsonl` session and every sibling read by
    /// its parser for rollup and session metadata.
    #[cfg(test)]
    pub(crate) fn from_grok_path(path: &Path) -> Option<Self> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let related_paths = ["signals.json", "summary.json", "events.jsonl"]
            .into_iter()
            .map(|name| (name.to_string(), parent.join(name)));
        Self::from_path_with_related(path, related_paths)
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
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let related_paths = ["signals.json", "summary.json", "events.jsonl"]
            .into_iter()
            .map(|name| (name.to_string(), parent.join(name)));
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

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

    fn update_digest(&self, hasher: &mut Sha256) {
        for code_unit in &self.0 {
            hasher.update(code_unit.to_le_bytes());
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
        ClientId::Codex => 4,
        ClientId::Jcode => 4,
        ClientId::Copilot => 4,
        // Pi subagent sessions now derive agent attribution from session_info
        // names; version-1 caches carry those messages without agent metadata.
        ClientId::Pi => 2,
        // Devin CLI v1 could stop at a malformed chat_message. Desktop v1
        // parsed a non-ACP shape and did not track its CLI title lookup.
        ClientId::DevinCli | ClientId::DevinDesktop => 2,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedSourceEntry {
    parser_namespace: String,
    parser_version: u32,
    pub path: CachedPath,
    pub fingerprint: SourceFingerprint,
    pub messages: Vec<UnifiedMessage>,
    pub fallback_timestamp_indices: Vec<usize>,
    pub codex_incremental: Option<CodexIncrementalCache>,
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
        }
    }

    fn identity_is_current(&self) -> bool {
        CacheIdentity::current_for_namespace(&self.parser_namespace)
            .is_some_and(|identity| identity.parser_version == self.parser_version)
    }
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

#[derive(Default)]
pub(crate) struct SourceMessageCache {
    pub entries: HashMap<CacheKey, CachedSourceEntry>,
    dirty: bool,
    dirty_keys: HashSet<CacheKey>,
    deleted_keys: HashMap<CacheKey, DeletionReason>,
    rewrite_shards: HashSet<CacheShardKey>,
}

impl SourceMessageCache {
    pub(crate) fn load() -> Self {
        let Some(shard_root) = cache_shard_dir() else {
            return Self::default();
        };
        let Some(lock_path) = cache_lock_path() else {
            return Self::default();
        };
        if let Err(error) = ensure_cache_dir(&shard_root) {
            warn_cache_failure_once(
                "source message cache directory is unavailable",
                &shard_root,
                &error,
            );
            return Self::default();
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
                return Self::default();
            }
        };
        if let Err(error) = fs2::FileExt::lock_shared(&lock_file) {
            warn_cache_failure_once("source message cache lock failed", &lock_path, &error);
            return Self::default();
        }

        let mut cache = Self::default();
        for identity in CacheIdentity::all() {
            let parser_dir = shard_root.join(identity.namespace);
            let read_dir = match fs::read_dir(&parser_dir) {
                Ok(read_dir) => read_dir,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    warn_cache_failure_once(
                        "source message cache parser directory is unreadable",
                        &parser_dir,
                        &error,
                    );
                    continue;
                }
            };

            for dir_entry in read_dir.filter_map(Result::ok) {
                let Some(index) = parse_shard_filename(&dir_entry.file_name()) else {
                    continue;
                };
                let shard_key = CacheShardKey {
                    namespace: identity.namespace.to_string(),
                    index,
                };
                let path = dir_entry.path();
                match read_shard(&path, identity) {
                    ShardReadStatus::Loaded(entries) => {
                        for entry in entries {
                            let key = CacheKey::from_entry(&entry);
                            if key.shard() == shard_key && entry.identity_is_current() {
                                cache.entries.insert(key, entry);
                            } else {
                                cache.rewrite_shards.insert(shard_key.clone());
                            }
                        }
                    }
                    ShardReadStatus::Missing => {}
                    ShardReadStatus::Stale => {
                        cache.rewrite_shards.insert(shard_key);
                    }
                    ShardReadStatus::Invalid(error) => {
                        warn_cache_failure_once(
                            "source message cache shard is invalid",
                            &path,
                            &error,
                        );
                        cache.rewrite_shards.insert(shard_key);
                    }
                }
            }
        }

        cache.dirty = !cache.rewrite_shards.is_empty();
        cache
    }

    pub(crate) fn insert(&mut self, entry: CachedSourceEntry) {
        let key = CacheKey::from_entry(&entry);
        self.entries.insert(key.clone(), entry);
        self.deleted_keys.remove(&key);
        self.dirty_keys.insert(key);
        self.dirty = true;
    }

    pub(crate) fn get(&self, identity: CacheIdentity, path: &Path) -> Option<&CachedSourceEntry> {
        let key = CacheKey::new(identity, path);
        self.entries.get(&key).filter(|entry| {
            entry.parser_namespace == identity.namespace
                && entry.parser_version == identity.parser_version
        })
    }

    pub(crate) fn remove(&mut self, identity: CacheIdentity, path: &Path) {
        let key = CacheKey::new(identity, path);
        if let Some(entry) = self.entries.remove(&key) {
            self.dirty_keys.remove(&key);
            self.deleted_keys
                .insert(key, DeletionReason::Invalidated(entry.fingerprint));
            self.dirty = true;
        }
    }

    pub(crate) fn prune_missing_files(&mut self) {
        let removed_keys: Vec<CacheKey> = self
            .entries
            .keys()
            .filter(|key| !key.path.to_path_buf().exists())
            .cloned()
            .collect();

        for key in removed_keys {
            self.entries.remove(&key);
            self.dirty_keys.remove(&key);
            self.deleted_keys.insert(key, DeletionReason::Missing);
            self.dirty = true;
        }
    }

    pub(crate) fn save_if_dirty(&mut self) {
        self.save_if_dirty_with_limit(MAX_CACHE_SHARD_BYTES);
    }

    fn save_if_dirty_with_limit(&mut self, max_shard_bytes: u64) {
        if !self.dirty {
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
        for key in &self.dirty_keys {
            dirty_by_shard
                .entry(key.shard())
                .or_default()
                .push(key.clone());
        }
        let mut deleted_by_shard: HashMap<CacheShardKey, Vec<(CacheKey, DeletionReason)>> =
            HashMap::new();
        for (key, reason) in &self.deleted_keys {
            deleted_by_shard
                .entry(key.shard())
                .or_default()
                .push((key.clone(), reason.clone()));
        }

        let mut affected_shards = self.rewrite_shards.clone();
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
                    ShardReadStatus::Loaded(entries) => entries
                        .into_iter()
                        .filter(|entry| entry.identity_is_current())
                        .map(|entry| (CacheKey::from_entry(&entry), entry))
                        .filter(|(key, _)| key.shard() == shard_key)
                        .collect(),
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
                    if let Some(entry) = self.entries.get(key) {
                        merged_entries.insert(key.clone(), entry.clone());
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

        self.dirty_keys
            .retain(|key| !successful_shards.contains(&key.shard()));
        self.deleted_keys
            .retain(|key, _| !successful_shards.contains(&key.shard()));
        self.rewrite_shards
            .retain(|shard| !successful_shards.contains(shard));
        self.dirty = !(self.dirty_keys.is_empty()
            && self.deleted_keys.is_empty()
            && self.rewrite_shards.is_empty());
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
    if envelope.format_version != CACHE_FORMAT_VERSION {
        return ShardReadStatus::Stale;
    }
    if envelope.parser_namespace != identity.namespace
        || envelope.parser_version != identity.parser_version
    {
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
        crate::fs_atomic::replace_file(&tmp_path, final_path)?;
        File::open(final_path)?.sync_all()?;
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
/// Validation uses size + mtime + samples ([`primary_fingerprint_matches`] and
/// [`related_fingerprint_metadata_matches`]) for every source. Only Codex reads
/// `content_hash` for incremental resume;
/// generic parsers and SQLite sources store a zero sentinel so changed or cold
/// files do not pay for a second whole-file hash that cannot affect parsing.
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
    let content_hash = match mode {
        ContentHashMode::Full => hash_prefix(path, size)?,
        ContentHashMode::SamplesOnly => [0_u8; 32],
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
    use crate::TokenBreakdown;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

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

    fn restore_env_var(key: &str, value: Option<impl AsRef<std::ffi::OsStr>>) {
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    /// Pin every env var the cache resolvers consult so the test stays
    /// inside `temp_home`. CI runners can leak `XDG_CONFIG_HOME` /
    /// `XDG_CACHE_HOME` from the host, which would resolve cache shards outside
    /// the sandbox. Returns the previous values so the caller can restore.
    fn sandbox_cache_env(
        temp_home: &std::path::Path,
    ) -> (
        Option<std::ffi::OsString>,
        Option<std::ffi::OsString>,
        Option<std::ffi::OsString>,
        Option<std::ffi::OsString>,
    ) {
        let prev_home = std::env::var_os("HOME");
        let prev_xdg_config = std::env::var_os("XDG_CONFIG_HOME");
        let prev_xdg_cache = std::env::var_os("XDG_CACHE_HOME");
        let prev_override = std::env::var_os("TOKSCALE_CONFIG_DIR");
        unsafe {
            std::env::set_var("HOME", temp_home);
            std::env::set_var("XDG_CONFIG_HOME", temp_home.join(".config"));
            std::env::set_var("XDG_CACHE_HOME", temp_home.join(".cache"));
            std::env::remove_var("TOKSCALE_CONFIG_DIR");
        }
        (prev_home, prev_xdg_config, prev_xdg_cache, prev_override)
    }

    fn restore_cache_env(
        prev: (
            Option<std::ffi::OsString>,
            Option<std::ffi::OsString>,
            Option<std::ffi::OsString>,
            Option<std::ffi::OsString>,
        ),
    ) {
        restore_env_var("HOME", prev.0);
        restore_env_var("XDG_CONFIG_HOME", prev.1);
        restore_env_var("XDG_CACHE_HOME", prev.2);
        restore_env_var("TOKSCALE_CONFIG_DIR", prev.3);
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
        assert_eq!(parser_version(ClientId::DevinCli), 2);
        assert_eq!(parser_version(ClientId::DevinDesktop), 2);
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
                r#"{{"name":"kimi-code","provider":"kimi","configDir":"{}"}}"#,
                config_dir.display()
            ),
        )
        .unwrap();
        let with_kimi =
            SourceFingerprint::from_claude_code_path_with_home(&jsonl_path, None).unwrap();

        std::fs::write(
            &variant_path,
            format!(
                r#"{{"name":"kimi-code","provider":"minimax","configDir":"{}"}}"#,
                config_dir.display()
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
                r#"{{"name":"kimi-code","provider":"kimi","configDir":"{}"}}"#,
                config_dir.display()
            ),
        )
        .unwrap();
        let with_kimi =
            SourceFingerprint::from_claude_code_path_with_home(&jsonl_path, Some(dir.path()))
                .unwrap();

        std::fs::write(
            &variant_path,
            format!(
                r#"{{"name":"kimi-code","provider":"minimax","configDir":"{}"}}"#,
                config_dir.display()
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
    #[serial_test::serial]
    fn test_source_message_cache_round_trips_across_distinct_shards() {
        let temp_home = TempDir::new().unwrap();
        let prev_env = sandbox_cache_env(temp_home.path());
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
        assert_eq!(loaded.entries.len(), 2);
        assert!(loaded.get(identity, &path_one).is_some());
        assert!(loaded.get(identity, &path_two).is_some());

        restore_cache_env(prev_env);
    }

    #[test]
    #[serial_test::serial]
    fn test_aggregate_cache_can_exceed_individual_shard_limit() {
        const TEST_SHARD_LIMIT: u64 = 32 * 1024;

        let temp_home = TempDir::new().unwrap();
        let prev_env = sandbox_cache_env(temp_home.path());
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
            !cache.dirty,
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

        restore_cache_env(prev_env);
    }

    #[test]
    #[serial_test::serial]
    fn test_corrupt_shard_does_not_hide_entries_from_other_shards() {
        let temp_home = TempDir::new().unwrap();
        let prev_env = sandbox_cache_env(temp_home.path());
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
            loaded.dirty,
            "the corrupt shard should be scheduled for rewrite"
        );

        restore_cache_env(prev_env);
    }

    #[test]
    #[serial_test::serial]
    fn test_stale_parser_shard_is_skipped_before_decoding_garbage_payload() {
        let temp_home = TempDir::new().unwrap();
        let prev_env = sandbox_cache_env(temp_home.path());
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
        let mut writer = BufWriter::new(File::create(&stale_path).unwrap());
        bincode::options()
            .serialize_into(&mut writer, &stale_envelope)
            .unwrap();
        writer.flush().unwrap();

        assert!(matches!(
            read_shard(&stale_path, codex),
            ShardReadStatus::Stale
        ));
        let mut loaded = SourceMessageCache::load();
        assert_eq!(loaded.entries.len(), 1);
        assert!(loaded.get(claude, source.path()).is_some());
        assert!(loaded.rewrite_shards.contains(&stale_key));

        loaded.save_if_dirty();
        assert!(matches!(
            read_shard(&stale_path, codex),
            ShardReadStatus::Loaded(entries) if entries.is_empty()
        ));
        assert!(SourceMessageCache::load()
            .get(claude, source.path())
            .is_some());

        restore_cache_env(prev_env);
    }

    #[test]
    #[serial_test::serial]
    fn test_copilot_v3_cache_is_rejected_and_rebuilt_with_root_agent() {
        let temp_home = TempDir::new().unwrap();
        let prev_env = sandbox_cache_env(temp_home.path());
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
            ),
        )
        .unwrap();

        let current_identity = CacheIdentity::for_client(ClientId::Copilot);
        let stale_identity = CacheIdentity {
            namespace: current_identity.namespace,
            parser_version: 3,
        };
        let mut stale_message = UnifiedMessage::new(
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
        );
        stale_message.agent = Some("github.copilot.subagent".to_string());
        let fingerprint = SourceFingerprint::from_path(&source_path).unwrap();
        let stale_entry = CachedSourceEntry::new(
            stale_identity,
            &source_path,
            fingerprint.clone(),
            vec![stale_message],
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
            "a v3 Copilot cache entry must not be served after the parser output change"
        );
        assert!(loaded.rewrite_shards.contains(&shard_key));
        assert_eq!(
            SourceFingerprint::from_path(&source_path).unwrap(),
            fingerprint,
            "the source fingerprint must remain unchanged; parser version causes invalidation"
        );

        let rebuilt = crate::sessions::copilot::parse_copilot_file(&source_path);
        assert_eq!(rebuilt.len(), 1);
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
                    && entries[0].messages[0].agent.as_deref()
                        == Some("github.copilot.default")
        ));

        restore_cache_env(prev_env);
    }

    #[test]
    #[serial_test::serial]
    fn test_explicit_invalidation_of_existing_path_persists() {
        let temp_home = TempDir::new().unwrap();
        let prev_env = sandbox_cache_env(temp_home.path());
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

        restore_cache_env(prev_env);
    }

    #[test]
    #[serial_test::serial]
    fn test_stale_invalidation_preserves_concurrently_refreshed_entry() {
        let temp_home = TempDir::new().unwrap();
        let prev_env = sandbox_cache_env(temp_home.path());
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

        restore_cache_env(prev_env);
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

        assert!(cache.entries.is_empty());
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
        let prev_env = sandbox_cache_env(temp_home.path());

        let mut cache = SourceMessageCache::default();
        assert!(!cache.dirty);

        {
            let file = write_temp_file(b"{}\n");
            let identity = CacheIdentity::for_client(ClientId::Claude);
            cache.insert(test_entry(identity, file.path(), "session-1"));
            assert!(cache.dirty);

            cache.save_if_dirty();
            assert!(!cache.dirty);
        }

        restore_cache_env(prev_env);
    }

    #[test]
    #[serial_test::serial]
    fn test_save_if_dirty_merges_concurrent_writers() {
        let temp_home = TempDir::new().unwrap();
        let prev_env = sandbox_cache_env(temp_home.path());

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

        restore_cache_env(prev_env);
    }

    #[test]
    #[serial_test::serial]
    fn test_save_if_dirty_preserves_recreated_path_from_concurrent_writer() {
        let temp_home = TempDir::new().unwrap();
        let prev_env = sandbox_cache_env(temp_home.path());

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

        restore_cache_env(prev_env);
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
}
